use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
#[cfg(feature = "mobile")]
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use image::GenericImageView;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong};
use osm_core::TileId;
use osm_loader::{CachedTileSource, FileTileCache, HttpTileSource, TileSource};
use osm_renderer::{LayerId, MapCamera, MapLayer, RenderState, RenderViewport, TileLayer};
use wgpu::util::DeviceExt;

#[cfg(feature = "mobile")]
use crate::mobile::OsmTileEngine;

const DEFAULT_TILE_LAYER_ID: &str = "base";
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.02,
    g: 0.05,
    b: 0.12,
    a: 1.0,
};
const TILE_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0)
var tile_texture: texture_2d<f32>;

@group(0) @binding(1)
var tile_sampler: sampler;

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tile_texture, tile_sampler, in.uv);
}
"#;
const TEXTURED_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

const MB: usize = 1024 * 1024;
const DEFAULT_RAM_TILE_CACHE_LIMIT_BYTES: usize = 96 * MB;
const DEFAULT_GPU_TILE_CACHE_LIMIT_BYTES: usize = 128 * MB;
const DEFAULT_MAX_ZOOM_DISTANCE_TO_KEEP: u8 = 2;

#[derive(Debug, Clone, Copy)]
struct TileCacheLimits {
    ram_bytes: usize,
    gpu_bytes: usize,
    max_zoom_distance_to_keep: u8,
}

impl Default for TileCacheLimits {
    fn default() -> Self {
        Self {
            ram_bytes: DEFAULT_RAM_TILE_CACHE_LIMIT_BYTES,
            gpu_bytes: DEFAULT_GPU_TILE_CACHE_LIMIT_BYTES,
            max_zoom_distance_to_keep: DEFAULT_MAX_ZOOM_DISTANCE_TO_KEEP,
        }
    }
}

#[derive(Debug, Default)]
struct TileTelemetry {
    decoded_hit: u64,
    decoded_miss: u64,
    decoded_evictions: u64,
    gpu_hit: u64,
    gpu_miss: u64,
    gpu_evictions: u64,
    upload_count: u64,
    upload_time_total_ns: u128,
}

impl TileTelemetry {
    fn record_upload(&mut self, elapsed: std::time::Duration) {
        self.upload_count += 1;
        self.upload_time_total_ns += elapsed.as_nanos();
    }
}

#[derive(Debug)]
struct SizedLruCache<T> {
    entries: HashMap<TileId, (T, usize)>,
    order: VecDeque<TileId>,
    current_size_bytes: usize,
    size_limit_bytes: usize,
}

impl<T> SizedLruCache<T> {
    fn new(size_limit_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            current_size_bytes: 0,
            size_limit_bytes,
        }
    }

    fn contains_key(&self, key: &TileId) -> bool {
        self.entries.contains_key(key)
    }

    fn touch(&mut self, key: TileId) {
        if let Some(pos) = self.order.iter().position(|candidate| *candidate == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    fn insert(&mut self, key: TileId, value: T, size_bytes: usize) {
        if let Some((_, (_, previous_size))) = self.entries.remove_entry(&key) {
            self.current_size_bytes = self.current_size_bytes.saturating_sub(previous_size);
            if let Some(pos) = self.order.iter().position(|candidate| *candidate == key) {
                self.order.remove(pos);
            }
        }
        self.current_size_bytes = self.current_size_bytes.saturating_add(size_bytes);
        self.order.push_back(key);
        self.entries.insert(key, (value, size_bytes));
    }

    fn get_cloned(&mut self, key: &TileId) -> Option<T>
    where
        T: Clone,
    {
        let value = self.entries.get(key).map(|(value, _)| value).cloned();
        if value.is_some() {
            self.touch(*key);
        }
        value
    }

    fn get(&mut self, key: &TileId) -> Option<&T> {
        if self.entries.contains_key(key) {
            self.touch(*key);
            return self.entries.get(key).map(|(value, _)| value);
        }
        None
    }

    fn peek(&self, key: &TileId) -> Option<&T> {
        self.entries.get(key).map(|(value, _)| value)
    }

    fn remove(&mut self, key: &TileId) -> Option<T> {
        let (value, size_bytes) = self.entries.remove(key)?;
        self.current_size_bytes = self.current_size_bytes.saturating_sub(size_bytes);
        if let Some(pos) = self.order.iter().position(|candidate| candidate == key) {
            self.order.remove(pos);
        }
        Some(value)
    }

    fn evict_one(&mut self, visible_tiles: &HashSet<TileId>) -> Option<TileId> {
        let mut fallback = None;
        for (idx, candidate) in self.order.iter().enumerate() {
            if fallback.is_none() {
                fallback = Some((idx, *candidate));
            }
            if !visible_tiles.contains(candidate) {
                return Some(self.evict_at(idx));
            }
        }
        fallback.map(|(idx, _)| self.evict_at(idx))
    }

    fn evict_at(&mut self, idx: usize) -> TileId {
        let key = self
            .order
            .remove(idx)
            .expect("cache order index should exist");
        if let Some((_, size_bytes)) = self.entries.remove(&key) {
            self.current_size_bytes = self.current_size_bytes.saturating_sub(size_bytes);
        }
        key
    }
}

type ANativeWindow = c_void;

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_fromSurface(
        env: *mut jni::sys::JNIEnv,
        surface: jni::sys::jobject,
    ) -> *mut ANativeWindow;
    fn ANativeWindow_release(window: *mut ANativeWindow);
}

#[derive(Debug)]
struct NativeWindow {
    ptr: NonNull<ANativeWindow>,
}

unsafe impl Send for NativeWindow {}

impl NativeWindow {
    fn from_surface(env: &JNIEnv<'_>, surface: &JObject<'_>) -> Option<Self> {
        let ptr =
            unsafe { ANativeWindow_fromSurface(env.get_native_interface(), surface.as_raw()) };
        NonNull::new(ptr).map(|ptr| Self { ptr })
    }

    fn raw_handle(&self) -> NonNull<c_void> {
        self.ptr.cast()
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe { ANativeWindow_release(self.ptr.as_ptr()) };
    }
}

pub struct AndroidMapRenderer {
    commands: Sender<RenderCommand>,
    tile_requests: Sender<TileLoaderRequest>,
    worker: Option<JoinHandle<()>>,
    tile_loader: Option<JoinHandle<()>>,
}

impl AndroidMapRenderer {
    fn new(tile_url_template: String, cache_dir: String) -> Result<Self, String> {
        let http_source =
            HttpTileSource::new(tile_url_template.clone()).map_err(|error| error.to_string())?;
        let cache = FileTileCache::new(PathBuf::from(cache_dir));
        let source = CachedTileSource::new(http_source, cache);
        Self::with_source(source, tile_url_template)
    }

    fn with_source(
        source: CachedTileSource<HttpTileSource>,
        tile_url_template: String,
    ) -> Result<Self, String> {
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
        let (tile_requests, tile_request_receiver) = mpsc::channel();
        let tile_loader_commands = commands.clone();
        let tile_loader_source = source.clone();
        let tile_loader = thread::Builder::new()
            .name("osm-map-tile-loader".to_owned())
            .spawn(move || {
                tile_loader_loop(
                    tile_loader_source,
                    tile_loader_commands,
                    tile_request_receiver,
                )
            })
            .map_err(|error| error.to_string())?;
        let worker_tile_requests = tile_requests.clone();
        let worker = thread::Builder::new()
            .name("osm-map-renderer".to_owned())
            .spawn(move || RendererWorker::new(state, worker_tile_requests).run(receiver))
            .map_err(|error| error.to_string())?;

        Ok(Self {
            commands,
            tile_requests,
            worker: Some(worker),
            tile_loader: Some(tile_loader),
        })
    }

    #[cfg(feature = "mobile")]
    fn from_engine(engine: Arc<OsmTileEngine>) -> Result<Self, String> {
        Self::with_source(engine.shared_source(), engine.tile_url_template())
    }

    fn send(&self, command: RenderCommand) {
        let _ = self.commands.send(command);
    }
}

impl Drop for AndroidMapRenderer {
    fn drop(&mut self) {
        let _ = self.commands.send(RenderCommand::Shutdown);
        let _ = self.tile_requests.send(TileLoaderRequest::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(tile_loader) = self.tile_loader.take() {
            let _ = tile_loader.join();
        }
    }
}

#[derive(Debug)]
enum RenderCommand {
    SurfaceCreated {
        window: NativeWindow,
    },
    SurfaceDestroyed,
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
    TileLoaded {
        id: TileId,
        tile: Option<LoadedTile>,
    },
    Shutdown,
}

enum TileLoaderRequest {
    Load(TileId),
    Shutdown,
}

#[derive(Debug, Clone)]
struct LoadedTile {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct TexturedVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

struct UploadedTile {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct TileDrawRef<'a> {
    bind_group: &'a wgpu::BindGroup,
    uv_left: f32,
    uv_right: f32,
    uv_top: f32,
    uv_bottom: f32,
}

struct RendererWorker {
    state: RenderState,
    surface_width_px: u32,
    surface_height_px: u32,
    loaded_tiles: SizedLruCache<LoadedTile>,
    pending_tile_loads: HashSet<TileId>,
    tile_requests: Sender<TileLoaderRequest>,
    gpu: Option<GpuSurface>,
    telemetry: TileTelemetry,
    cache_limits: TileCacheLimits,
    last_visible_zoom: Option<u8>,
}

struct GpuSurface {
    surface: wgpu::Surface<'static>,
    _instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    tile_bind_group_layout: wgpu::BindGroupLayout,
    tile_sampler: wgpu::Sampler,
    quad_pipeline: wgpu::RenderPipeline,
    uploaded_tiles: SizedLruCache<UploadedTile>,
    config: Option<wgpu::SurfaceConfiguration>,
    _native_window: NativeWindow,
}

impl RendererWorker {
    fn new(state: RenderState, tile_requests: Sender<TileLoaderRequest>) -> Self {
        Self {
            state,
            surface_width_px: 0,
            surface_height_px: 0,
            loaded_tiles: SizedLruCache::new(TileCacheLimits::default().ram_bytes),
            pending_tile_loads: HashSet::new(),
            tile_requests,
            gpu: None,
            telemetry: TileTelemetry::default(),
            cache_limits: TileCacheLimits::default(),
            last_visible_zoom: None,
        }
    }

    fn run(mut self, receiver: Receiver<RenderCommand>) {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(_) => return,
        };

        while let Ok(command) = receiver.recv() {
            match command {
                RenderCommand::SurfaceCreated { window } => {
                    self.create_gpu_surface(window, &runtime);
                    self.render_frame();
                }
                RenderCommand::SurfaceDestroyed => self.gpu = None,
                RenderCommand::Resize {
                    width_px,
                    height_px,
                    density,
                } => {
                    self.surface_width_px = width_px;
                    self.surface_height_px = height_px;
                    if let Some(gpu) = self.gpu.as_mut() {
                        gpu.configure(width_px, height_px);
                    }
                    if let Ok(viewport) = RenderViewport::new(width_px, height_px, density) {
                        let _ = self.state.set_viewport(viewport);
                        self.render_frame();
                    }
                }
                RenderCommand::SetCamera(camera) => {
                    let _ = self.state.set_camera(camera);
                    self.render_frame();
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
                        self.render_frame();
                    }
                }
                RenderCommand::RemoveLayer(id) => {
                    if let Ok(id) = LayerId::new(id) {
                        self.state.layers_mut().remove(&id);
                        self.render_frame();
                    }
                }
                RenderCommand::SetLayerVisible { id, visible } => {
                    if let Ok(id) = LayerId::new(id) {
                        let _ = self.state.layers_mut().set_visible(&id, visible);
                        self.render_frame();
                    }
                }
                RenderCommand::SetLayerOpacity { id, opacity } => {
                    if let Ok(id) = LayerId::new(id) {
                        let _ = self.state.layers_mut().set_opacity(&id, opacity);
                        self.render_frame();
                    }
                }
                RenderCommand::TileLoaded { id, tile } => {
                    self.pending_tile_loads.remove(&id);
                    if let Some(tile) = tile {
                        let size_bytes = tile.rgba.len();
                        self.loaded_tiles.insert(id, tile, size_bytes);
                        self.telemetry.decoded_hit += 1;
                    } else {
                        self.telemetry.decoded_miss += 1;
                    }
                    self.render_frame();
                }
                RenderCommand::Shutdown => break,
            }
        }
    }

    fn create_gpu_surface(&mut self, window: NativeWindow, runtime: &tokio::runtime::Runtime) {
        self.gpu = None;

        let mut gpu = match GpuSurface::new(window, runtime) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("failed to create Android wgpu surface: {error}");
                return;
            }
        };

        if self.surface_width_px > 0 && self.surface_height_px > 0 {
            gpu.configure(self.surface_width_px, self.surface_height_px);
        }

        self.gpu = Some(gpu);
    }

    fn render_frame(&mut self) {
        let visible_tiles = if self.has_visible_tile_layer() {
            self.state.visible_tiles().ok()
        } else {
            None
        };

        if let Some(visible_tiles) = visible_tiles.as_ref() {
            self.evict_far_zoom_layers(visible_tiles);
            self.ensure_visible_tiles(visible_tiles);
            self.enforce_cache_limits(visible_tiles);
        }

        if let Some(gpu) = self.gpu.as_mut() {
            gpu.render(visible_tiles.as_deref());
        }
    }

    fn ensure_visible_tiles(&mut self, visible_tiles: &[osm_renderer::VisibleTile]) {
        for visible_tile in visible_tiles {
            let tile_id = visible_tile.id;

            if !self.loaded_tiles.contains_key(&tile_id)
                && !self.pending_tile_loads.contains(&tile_id)
            {
                self.telemetry.decoded_miss += 1;
                self.pending_tile_loads.insert(tile_id);
                let _ = self.tile_requests.send(TileLoaderRequest::Load(tile_id));
            } else if self.loaded_tiles.contains_key(&tile_id) {
                self.telemetry.decoded_hit += 1;
            }

            let tile_data = self.loaded_tiles.get_cloned(&tile_id);
            if let (Some(gpu), Some(tile_data)) = (self.gpu.as_mut(), tile_data) {
                if !gpu.uploaded_tiles.contains_key(&tile_id) {
                    self.telemetry.gpu_miss += 1;
                    gpu.upload_tile(tile_id, &tile_data, &mut self.telemetry);
                } else {
                    self.telemetry.gpu_hit += 1;
                }
            }
        }
    }

    fn has_visible_tile_layer(&self) -> bool {
        self.state.layers().layers().iter().any(|layer| {
            matches!(layer, MapLayer::Tile(_))
                && layer.common().visible
                && layer.common().opacity > 0.0
        })
    }

    fn enforce_cache_limits(&mut self, visible_tiles: &[osm_renderer::VisibleTile]) {
        let visible_ids: HashSet<TileId> = visible_tiles.iter().map(|tile| tile.id).collect();
        while self.loaded_tiles.current_size_bytes > self.loaded_tiles.size_limit_bytes {
            if self.loaded_tiles.evict_one(&visible_ids).is_none() {
                break;
            }
            self.telemetry.decoded_evictions += 1;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            while gpu.uploaded_tiles.current_size_bytes > gpu.uploaded_tiles.size_limit_bytes {
                if gpu.uploaded_tiles.evict_one(&visible_ids).is_none() {
                    break;
                }
                self.telemetry.gpu_evictions += 1;
            }
        }
    }

    fn evict_far_zoom_layers(&mut self, visible_tiles: &[osm_renderer::VisibleTile]) {
        let Some(current_zoom) = visible_tiles.first().map(|tile| tile.id.z) else {
            return;
        };
        let changed = self
            .last_visible_zoom
            .map(|previous| previous != current_zoom)
            .unwrap_or(false);
        self.last_visible_zoom = Some(current_zoom);
        if !changed {
            return;
        }
        let max_distance = self.cache_limits.max_zoom_distance_to_keep;
        let stale_loaded: Vec<TileId> = self
            .loaded_tiles
            .entries
            .keys()
            .copied()
            .filter(|tile_id| tile_id.z.abs_diff(current_zoom) > max_distance)
            .collect();
        for tile_id in stale_loaded {
            if self.loaded_tiles.remove(&tile_id).is_some() {
                self.telemetry.decoded_evictions += 1;
            }
        }
        if let Some(gpu) = self.gpu.as_mut() {
            let stale_gpu: Vec<TileId> = gpu
                .uploaded_tiles
                .entries
                .keys()
                .copied()
                .filter(|tile_id| tile_id.z.abs_diff(current_zoom) > max_distance)
                .collect();
            for tile_id in stale_gpu {
                if gpu.uploaded_tiles.remove(&tile_id).is_some() {
                    self.telemetry.gpu_evictions += 1;
                }
            }
        }
    }
}

fn tile_loader_loop(
    source: CachedTileSource<HttpTileSource>,
    commands: Sender<RenderCommand>,
    receiver: Receiver<TileLoaderRequest>,
) {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    while let Ok(request) = receiver.recv() {
        match request {
            TileLoaderRequest::Load(tile_id) => {
                let tile = runtime
                    .block_on(source.load_tile(tile_id))
                    .ok()
                    .and_then(|bytes| image::load_from_memory(&bytes).ok())
                    .map(|image| {
                        let rgba = image.to_rgba8();
                        let (width, height) = image.dimensions();
                        LoadedTile {
                            width,
                            height,
                            rgba: rgba.into_raw(),
                        }
                    });
                let _ = commands.send(RenderCommand::TileLoaded { id: tile_id, tile });
            }
            TileLoaderRequest::Shutdown => break,
        }
    }
}

impl GpuSurface {
    fn new(native_window: NativeWindow, runtime: &tokio::runtime::Runtime) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let raw_window_handle =
            wgpu::rwh::AndroidNdkWindowHandle::new(native_window.raw_handle()).into();
        let raw_display_handle = wgpu::rwh::AndroidDisplayHandle::new().into();
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle),
                raw_window_handle,
            })
        }
        .map_err(|error| error.to_string())?;
        let adapter = runtime
            .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            }))
            .map_err(|error| error.to_string())?;
        let (device, queue) = runtime
            .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("osm-map-renderer-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))
            .map_err(|error| error.to_string())?;
        let surface_format = surface
            .get_capabilities(&adapter)
            .formats
            .first()
            .copied()
            .ok_or_else(|| "surface reported no supported formats".to_owned())?;
        let tile_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("osm-map-renderer-tile-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("osm-map-renderer-tile-pipeline-layout"),
            bind_group_layouts: &[Some(&tile_bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("osm-map-renderer-tile-shader"),
            source: wgpu::ShaderSource::Wgsl(TILE_SHADER.into()),
        });
        let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("osm-map-renderer-tile-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("osm-map-renderer-tile-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TexturedVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &TEXTURED_VERTEX_ATTRIBUTES,
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            surface,
            _instance: instance,
            adapter,
            device,
            queue,
            tile_bind_group_layout,
            tile_sampler,
            quad_pipeline,
            uploaded_tiles: SizedLruCache::new(TileCacheLimits::default().gpu_bytes),
            config: None,
            _native_window: native_window,
        })
    }

    fn configure(&mut self, width_px: u32, height_px: u32) {
        if width_px == 0 || height_px == 0 {
            return;
        }

        let Some(config) = self
            .surface
            .get_default_config(&self.adapter, width_px, height_px)
        else {
            eprintln!("failed to choose Android wgpu surface configuration");
            return;
        };

        self.surface.configure(&self.device, &config);
        self.config = Some(config);
    }

    fn upload_tile(&mut self, tile_id: TileId, tile: &LoadedTile, telemetry: &mut TileTelemetry) {
        let upload_started = std::time::Instant::now();
        let size = wgpu::Extent3d {
            width: tile.width,
            height: tile.height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("osm-map-renderer-tile-texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &tile.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(tile.width * 4),
                rows_per_image: Some(tile.height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("osm-map-renderer-tile-bind-group"),
            layout: &self.tile_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                },
            ],
        });

        let texture_size_bytes = tile.width as usize * tile.height as usize * 4;
        self.uploaded_tiles.insert(
            tile_id,
            UploadedTile {
                _texture: texture,
                bind_group,
            },
            texture_size_bytes,
        );
        telemetry.record_upload(upload_started.elapsed());
    }

    fn render(&mut self, visible_tiles: Option<&[osm_renderer::VisibleTile]>) {
        if self.config.is_none() {
            return;
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                if let Some(config) = self.config.clone() {
                    self.surface.configure(&self.device, &config);
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("Android wgpu surface returned a validation error");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("osm-map-renderer-clear-encoder"),
            });
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("osm-map-renderer-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        if let Some(visible_tiles) = visible_tiles {
            let mut batch_vertices = Vec::new();
            let mut batch_draws = Vec::new();

            for visible_tile in visible_tiles {
                let Some(tile_draw) = self.resolve_tile_draw(visible_tile.id) else {
                    continue;
                };
                let vertex_start =
                    batch_vertices.len() as u64 * std::mem::size_of::<TexturedVertex>() as u64;
                let vertices = tile_vertices(
                    visible_tile.screen_x_px,
                    visible_tile.screen_y_px,
                    visible_tile.size_px,
                    tile_draw.uv_left,
                    tile_draw.uv_right,
                    tile_draw.uv_top,
                    tile_draw.uv_bottom,
                    self.config
                        .as_ref()
                        .expect("surface configuration should exist"),
                );
                batch_vertices.extend_from_slice(&vertices);
                batch_draws.push((vertex_start, tile_draw.bind_group));
            }

            if !batch_vertices.is_empty() {
                let vertex_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("osm-map-renderer-tile-vertex-buffer"),
                            contents: bytes_of(&batch_vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("osm-map-renderer-tile-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                render_pass.set_pipeline(&self.quad_pipeline);

                for (vertex_start, bind_group) in batch_draws {
                    let vertex_end =
                        vertex_start + (6 * std::mem::size_of::<TexturedVertex>()) as u64;
                    render_pass.set_bind_group(0, bind_group, &[]);
                    render_pass.set_vertex_buffer(0, vertex_buffer.slice(vertex_start..vertex_end));
                    render_pass.draw(0..6, 0..1);
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }

    fn resolve_tile_draw(&self, tile_id: TileId) -> Option<TileDrawRef<'_>> {
        if let Some(uploaded_tile) = self.uploaded_tiles.peek(&tile_id) {
            return Some(TileDrawRef {
                bind_group: &uploaded_tile.bind_group,
                uv_left: 0.0,
                uv_right: 1.0,
                uv_top: 0.0,
                uv_bottom: 1.0,
            });
        }

        let mut ancestor = tile_id;
        let mut scale_divisor = 1_u32;
        let mut child_offset_x = 0_u32;
        let mut child_offset_y = 0_u32;

        while ancestor.z > 0 {
            child_offset_x += (ancestor.x & 1) * scale_divisor;
            child_offset_y += (ancestor.y & 1) * scale_divisor;
            scale_divisor *= 2;
            ancestor = TileId {
                z: ancestor.z - 1,
                x: ancestor.x / 2,
                y: ancestor.y / 2,
            };

            if let Some(uploaded_tile) = self.uploaded_tiles.peek(&ancestor) {
                let factor = scale_divisor as f32;
                let uv_left = child_offset_x as f32 / factor;
                let uv_right = (child_offset_x + 1) as f32 / factor;
                let uv_top = child_offset_y as f32 / factor;
                let uv_bottom = (child_offset_y + 1) as f32 / factor;
                return Some(TileDrawRef {
                    bind_group: &uploaded_tile.bind_group,
                    uv_left,
                    uv_right,
                    uv_top,
                    uv_bottom,
                });
            }
        }

        None
    }
}

fn tile_vertices(
    screen_x_px: f32,
    screen_y_px: f32,
    size_px: f32,
    uv_left: f32,
    uv_right: f32,
    uv_top: f32,
    uv_bottom: f32,
    config: &wgpu::SurfaceConfiguration,
) -> [TexturedVertex; 6] {
    let width = config.width as f32;
    let height = config.height as f32;
    let left = screen_x_px / width * 2.0 - 1.0;
    let right = (screen_x_px + size_px) / width * 2.0 - 1.0;
    let top = 1.0 - screen_y_px / height * 2.0;
    let bottom = 1.0 - (screen_y_px + size_px) / height * 2.0;

    [
        TexturedVertex {
            position: [left, bottom],
            uv: [uv_left, uv_bottom],
        },
        TexturedVertex {
            position: [right, bottom],
            uv: [uv_right, uv_bottom],
        },
        TexturedVertex {
            position: [right, top],
            uv: [uv_right, uv_top],
        },
        TexturedVertex {
            position: [left, bottom],
            uv: [uv_left, uv_bottom],
        },
        TexturedVertex {
            position: [right, top],
            uv: [uv_right, uv_top],
        },
        TexturedVertex {
            position: [left, top],
            uv: [uv_left, uv_top],
        },
    ]
}

fn bytes_of<T>(slice: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
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

#[cfg(feature = "mobile")]
fn renderer_from_engine_handle(engine_handle: jlong) -> Option<Arc<OsmTileEngine>> {
    if engine_handle == 0 {
        return None;
    }

    let handle = unsafe { uniffi::ffi::Handle::from_raw(engine_handle as u64) }?;
    Some(unsafe { handle.into_arc::<OsmTileEngine>() })
}

fn native_create_renderer(
    mut env: JNIEnv<'_>,
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

#[cfg(feature = "mobile")]
fn native_create_renderer_from_engine(engine_handle: jlong) -> jlong {
    let Some(engine) = renderer_from_engine_handle(engine_handle) else {
        return 0;
    };

    match AndroidMapRenderer::from_engine(engine) {
        Ok(renderer) => Box::into_raw(Box::new(renderer)) as jlong,
        Err(_) => 0,
    }
}

fn native_surface_created(env: JNIEnv<'_>, ptr: jlong, surface: JObject<'_>) {
    let Some(window) = NativeWindow::from_surface(&env, &surface) else {
        return;
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::SurfaceCreated { window });
    }
}

fn native_surface_changed(ptr: jlong, width_px: jint, height_px: jint, density: jfloat) {
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

fn native_set_camera(
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

fn native_surface_destroyed(ptr: jlong) {
    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::SurfaceDestroyed);
    }
}

fn native_destroy_renderer(ptr: jlong) {
    take_renderer_from_ptr(ptr);
}

fn native_add_tile_layer(
    mut env: JNIEnv<'_>,
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

fn native_remove_layer(mut env: JNIEnv<'_>, ptr: jlong, layer_id: JString<'_>) {
    let Some(layer_id) = java_string(&mut env, &layer_id) else {
        return;
    };

    if let Some(renderer) = renderer_from_ptr(ptr) {
        renderer.send(RenderCommand::RemoveLayer(layer_id));
    }
}

fn native_set_layer_visible(
    mut env: JNIEnv<'_>,
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

fn native_set_layer_opacity(
    mut env: JNIEnv<'_>,
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

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeCreateRenderer(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    tile_url_template: JString<'_>,
    cache_dir: JString<'_>,
) -> jlong {
    native_create_renderer(env, tile_url_template, cache_dir)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSurfaceCreated(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    surface: JObject<'_>,
) {
    native_surface_created(env, ptr, surface);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSurfaceChanged(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    width_px: jint,
    height_px: jint,
    density: jfloat,
) {
    native_surface_changed(ptr, width_px, height_px, density);
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
    native_set_camera(ptr, center_lat, center_lon, zoom, bearing, pitch);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSurfaceDestroyed(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
) {
    native_surface_destroyed(ptr);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeDestroyRenderer(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
) {
    native_destroy_renderer(ptr);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeAddTileLayer(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    url_template: JString<'_>,
    z_index: jint,
    opacity: jfloat,
) {
    native_add_tile_layer(env, ptr, layer_id, url_template, z_index, opacity);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeRemoveLayer(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
) {
    native_remove_layer(env, ptr, layer_id);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSetLayerVisible(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    visible: jboolean,
) {
    native_set_layer_visible(env, ptr, layer_id, visible);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapView_nativeSetLayerOpacity(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    opacity: jfloat,
) {
    native_set_layer_opacity(env, ptr, layer_id, opacity);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeCreateRenderer(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    tile_url_template: JString<'_>,
    cache_dir: JString<'_>,
) -> jlong {
    native_create_renderer(env, tile_url_template, cache_dir)
}

#[cfg(feature = "mobile")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeCreateRendererFromEngine(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    engine_handle: jlong,
) -> jlong {
    native_create_renderer_from_engine(engine_handle)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeSurfaceCreated(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    surface: JObject<'_>,
) {
    native_surface_created(env, ptr, surface);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeSurfaceChanged(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    width_px: jint,
    height_px: jint,
    density: jfloat,
) {
    native_surface_changed(ptr, width_px, height_px, density);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeSetCamera(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    center_lat: jdouble,
    center_lon: jdouble,
    zoom: jdouble,
    bearing: jdouble,
    pitch: jdouble,
) {
    native_set_camera(ptr, center_lat, center_lon, zoom, bearing, pitch);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeSurfaceDestroyed(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
) {
    native_surface_destroyed(ptr);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeDestroyRenderer(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
) {
    native_destroy_renderer(ptr);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeAddTileLayer(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    url_template: JString<'_>,
    z_index: jint,
    opacity: jfloat,
) {
    native_add_tile_layer(env, ptr, layer_id, url_template, z_index, opacity);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeRemoveLayer(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
) {
    native_remove_layer(env, ptr, layer_id);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeSetLayerVisible(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    visible: jboolean,
) {
    native_set_layer_visible(env, ptr, layer_id, visible);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_yegor_cheprasov_osmtileengine_OsmMapController_nativeSetLayerOpacity(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    ptr: jlong,
    layer_id: JString<'_>,
    opacity: jfloat,
) {
    native_set_layer_opacity(env, ptr, layer_id, opacity);
}
