use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use image::GenericImageView;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong};
use osm_core::TileId;
use osm_loader::{CachedTileSource, FileTileCache, HttpTileSource, TileSource};
use osm_renderer::{LayerId, MapCamera, MapLayer, RenderState, RenderViewport, TileLayer};

const DEFAULT_TILE_LAYER_ID: &str = "base";

pub struct AndroidMapRenderer {
    commands: Sender<RenderCommand>,
    worker: Option<JoinHandle<()>>,
}

impl AndroidMapRenderer {
    fn new(tile_url_template: String, cache_dir: String) -> Result<Self, String> {
        let http_source =
            HttpTileSource::new(tile_url_template.clone()).map_err(|error| error.to_string())?;
        let cache = FileTileCache::new(PathBuf::from(cache_dir));
        let source = CachedTileSource::new(http_source, cache);
        let mut state = RenderState::new();
        state
            .layers_mut()
            .add_or_replace(MapLayer::Tile(TileLayer::new(
                LayerId::new(DEFAULT_TILE_LAYER_ID).map_err(|error| error.to_string())?,
                tile_url_template,
                0,
            )))
            .map_err(|error| error.to_string())?;

        let (commands, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("osm-map-renderer".to_owned())
            .spawn(move || RendererWorker::new(source, state).run(receiver))
            .map_err(|error| error.to_string())?;

        Ok(Self {
            commands,
            worker: Some(worker),
        })
    }

    fn send(&self, command: RenderCommand) {
        let _ = self.commands.send(command);
    }
}

impl Drop for AndroidMapRenderer {
    fn drop(&mut self) {
        let _ = self.commands.send(RenderCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
enum RenderCommand {
    AttachSurface,
    DetachSurface,
    Resize {
        width_px: u32,
        height_px: u32,
        density: f64,
    },
    SetCamera(MapCamera),
    AddTileLayer {
        id: String,
        url_template: String,
        z_index: i32,
        opacity: f32,
    },
    RemoveLayer(String),
    SetLayerVisible {
        id: String,
        visible: bool,
    },
    SetLayerOpacity {
        id: String,
        opacity: f32,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
struct LoadedTile {
    _width: u32,
    _height: u32,
}

struct RendererWorker {
    source: CachedTileSource<HttpTileSource>,
    state: RenderState,
    attached: bool,
    loaded_tiles: HashMap<TileId, LoadedTile>,
    _gpu: Option<wgpu::Instance>,
}

impl RendererWorker {
    fn new(source: CachedTileSource<HttpTileSource>, state: RenderState) -> Self {
        let gpu = std::panic::catch_unwind(wgpu::Instance::default).ok();
        Self {
            source,
            state,
            attached: false,
            loaded_tiles: HashMap::new(),
            _gpu: gpu,
        }
    }

    fn run(mut self, receiver: Receiver<RenderCommand>) {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(_) => return,
        };

        while let Ok(command) = receiver.recv() {
            match command {
                RenderCommand::AttachSurface => {
                    self.attached = true;
                    self.render_frame(&runtime);
                }
                RenderCommand::DetachSurface => self.attached = false,
                RenderCommand::Resize {
                    width_px,
                    height_px,
                    density,
                } => {
                    if let Ok(viewport) = RenderViewport::new(width_px, height_px, density) {
                        let _ = self.state.set_viewport(viewport);
                        self.render_frame(&runtime);
                    }
                }
                RenderCommand::SetCamera(camera) => {
                    let _ = self.state.set_camera(camera);
                    self.render_frame(&runtime);
                }
                RenderCommand::AddTileLayer {
                    id,
                    url_template,
                    z_index,
                    opacity,
                } => {
                    if let Ok(id) = LayerId::new(id) {
                        let mut layer = TileLayer::new(id, url_template, z_index);
                        layer.common.opacity = opacity;
                        let _ = self
                            .state
                            .layers_mut()
                            .add_or_replace(MapLayer::Tile(layer));
                        self.render_frame(&runtime);
                    }
                }
                RenderCommand::RemoveLayer(id) => {
                    if let Ok(id) = LayerId::new(id) {
                        self.state.layers_mut().remove(&id);
                        self.render_frame(&runtime);
                    }
                }
                RenderCommand::SetLayerVisible { id, visible } => {
                    if let Ok(id) = LayerId::new(id) {
                        let _ = self.state.layers_mut().set_visible(&id, visible);
                        self.render_frame(&runtime);
                    }
                }
                RenderCommand::SetLayerOpacity { id, opacity } => {
                    if let Ok(id) = LayerId::new(id) {
                        let _ = self.state.layers_mut().set_opacity(&id, opacity);
                        self.render_frame(&runtime);
                    }
                }
                RenderCommand::Shutdown => break,
            }
        }
    }

    fn render_frame(&mut self, runtime: &tokio::runtime::Runtime) {
        if !self.attached || !self.has_visible_tile_layer() {
            return;
        }

        let Ok(tiles) = self.state.visible_tiles() else {
            return;
        };

        for tile in tiles {
            if self.loaded_tiles.contains_key(&tile.id) {
                continue;
            }

            let Ok(bytes) = runtime.block_on(self.source.load_tile(tile.id)) else {
                continue;
            };
            let Ok(image) = image::load_from_memory(&bytes) else {
                continue;
            };
            let (width, height) = image.dimensions();
            self.loaded_tiles.insert(
                tile.id,
                LoadedTile {
                    _width: width,
                    _height: height,
                },
            );
        }
    }

    fn has_visible_tile_layer(&self) -> bool {
        self.state.layers().layers().iter().any(|layer| {
            matches!(layer, MapLayer::Tile(_))
                && layer.common().visible
                && layer.common().opacity > 0.0
        })
    }
}

fn renderer_from_ptr<'a>(ptr: jlong) -> Option<&'a AndroidMapRenderer> {
    if ptr == 0 {
        return None;
    }

    Some(unsafe { &*(ptr as *const AndroidMapRenderer) })
}

fn take_renderer_from_ptr(ptr: jlong) {
    if ptr == 0 {
        return;
    }

    drop(unsafe { Box::from_raw(ptr as *mut AndroidMapRenderer) });
}

fn java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Option<String> {
    env.get_string(value).ok().map(|value| value.into())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeCreateRenderer(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    tile_url_template: JString<'_>,
    cache_dir: JString<'_>,
) -> jlong {
    let Some(tile_url_template) = java_string(&mut env, &tile_url_template) else {
        return 0;
    };
    let Some(cache_dir) = java_string(&mut env, &cache_dir) else {
        return 0;
    };

    match AndroidMapRenderer::new(tile_url_template, cache_dir) {
        Ok(renderer) => Box::into_raw(Box::new(renderer)) as jlong,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeAttachSurface(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    _surface: JObject<'_>,
) {
    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::AttachSurface);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeResize(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    width_px: jint,
    height_px: jint,
    density: jfloat,
) {
    if width_px <= 0 || height_px <= 0 {
        return;
    }

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::Resize {
            width_px: width_px as u32,
            height_px: height_px as u32,
            density: f64::from(density),
        });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSetCamera(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    center_lat: jdouble,
    center_lon: jdouble,
    zoom: jdouble,
    bearing: jdouble,
    pitch: jdouble,
) {
    let camera = MapCamera {
        center_lat,
        center_lon,
        zoom,
        bearing,
        pitch,
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::SetCamera(camera));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeDetachSurface(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
) {
    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::DetachSurface);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeDestroyRenderer(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
) {
    take_renderer_from_ptr(ptr);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeAddTileLayer(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    url_template: JString<'_>,
    z_index: jint,
    opacity: jfloat,
) {
    let Some(layer_id) = java_string(&mut env, &layer_id) else {
        return;
    };
    let Some(url_template) = java_string(&mut env, &url_template) else {
        return;
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::AddTileLayer {
            id: layer_id,
            url_template,
            z_index,
            opacity,
        });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeRemoveLayer(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
) {
    let Some(layer_id) = java_string(&mut env, &layer_id) else {
        return;
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::RemoveLayer(layer_id));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSetLayerVisible(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    visible: jboolean,
) {
    let Some(layer_id) = java_string(&mut env, &layer_id) else {
        return;
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::SetLayerVisible {
            id: layer_id,
            visible: visible != 0,
        });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSetLayerOpacity(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    opacity: jfloat,
) {
    let Some(layer_id) = java_string(&mut env, &layer_id) else {
        return;
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::SetLayerOpacity {
            id: layer_id,
            opacity,
        });
    }
}
