use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
#[cfg(feature = "mobile")]
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
    loaded_tiles: HashMap<TileId, LoadedTile>,
    pending_tile_loads: HashSet<TileId>,
    pending_tile_priorities: HashMap<TileId, TilePriority>,
    tile_requests: Sender<TileLoaderRequest>,
    gpu: Option<GpuSurface>,
    camera_motion: CameraMotion,
    frame_timing: FrameTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TilePriority {
    P0Visible = 0,
    P1LookAhead = 1,
    P2Periphery = 2,
}

#[derive(Debug, Clone, Copy)]
struct CameraMotion {
    last_camera: Option<MapCamera>,
    last_update: Option<Instant>,
    velocity_tiles_per_sec: (f64, f64),
}

#[derive(Debug, Clone, Copy)]
struct FrameTiming {
    last_frame: Option<Instant>,
    frame_interval: Duration,
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
    uploaded_tiles: HashMap<TileId, UploadedTile>,
    config: Option<wgpu::SurfaceConfiguration>,
    _native_window: NativeWindow,
}

impl RendererWorker {
    fn new(state: RenderState, tile_requests: Sender<TileLoaderRequest>) -> Self {
        Self {
            state,
            surface_width_px: 0,
            surface_height_px: 0,
            loaded_tiles: HashMap::new(),
            pending_tile_loads: HashSet::new(),
            pending_tile_priorities: HashMap::new(),
            tile_requests,
            gpu: None,
            camera_motion: CameraMotion {
                last_camera: None,
                last_update: None,
                velocity_tiles_per_sec: (0.0, 0.0),
            },
            frame_timing: FrameTiming {
                last_frame: None,
                frame_interval: Duration::from_millis(16),
            },
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
                    self.update_camera_motion(camera);
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
                        self.loaded_tiles.insert(id, tile);
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
        let now = Instant::now();
        if let Some(prev) = self.frame_timing.last_frame {
            self.frame_timing.frame_interval = now.saturating_duration_since(prev);
        }
        self.frame_timing.last_frame = Some(now);
        let visible_tiles = if self.has_visible_tile_layer() {
            self.state.visible_tiles().ok()
        } else {
            None
        };

        if let Some(visible_tiles) = visible_tiles.as_ref() {
            self.ensure_visible_tiles(visible_tiles);
        }

        if let Some(gpu) = self.gpu.as_mut() {
            gpu.render(visible_tiles.as_deref());
        }
    }

    fn ensure_visible_tiles(&mut self, visible_tiles: &[osm_renderer::VisibleTile]) {
        let request_plan = self.build_tile_request_plan(visible_tiles);
        self.pending_tile_loads
            .retain(|tile_id| request_plan.contains_key(tile_id));
        self.pending_tile_priorities
            .retain(|tile_id, _| request_plan.contains_key(tile_id));

        let mut ordered_requests = request_plan
            .iter()
            .filter(|(tile_id, _)| {
                !self.loaded_tiles.contains_key(tile_id)
                    && !self.pending_tile_loads.contains(tile_id)
            })
            .map(|(tile_id, priority)| (*tile_id, *priority))
            .collect::<Vec<_>>();
        ordered_requests.sort_by_key(|(_, priority)| *priority);

        let max_inflight = self.prefetch_budget();
        let available_slots = max_inflight.saturating_sub(self.pending_tile_loads.len());
        for (tile_id, priority) in ordered_requests.into_iter().take(available_slots) {
            self.pending_tile_loads.insert(tile_id);
            self.pending_tile_priorities.insert(tile_id, priority);
            let _ = self.tile_requests.send(TileLoaderRequest::Load(tile_id));
        }

        for visible_tile in visible_tiles {
            let tile_id = visible_tile.id;
            if let (Some(gpu), Some(tile_data)) =
                (self.gpu.as_mut(), self.loaded_tiles.get(&tile_id).cloned())
            {
                if !gpu.uploaded_tiles.contains_key(&tile_id) {
                    gpu.upload_tile(tile_id, &tile_data);
                }
            }
        }
    }

    fn update_camera_motion(&mut self, camera: MapCamera) {
        let now = Instant::now();
        if let (Some(last_camera), Some(last_update)) = (
            self.camera_motion.last_camera,
            self.camera_motion.last_update,
        ) {
            let dt = now.saturating_duration_since(last_update).as_secs_f64();
            if dt > 0.0 {
                let zoom = camera.tile_zoom();
                let prev = osm_core::GeoPoint::new(last_camera.center_lat, last_camera.center_lon);
                let curr = osm_core::GeoPoint::new(camera.center_lat, camera.center_lon);
                if let (Ok(prev), Ok(curr)) = (prev, curr) {
                    if let (Ok((px, py)), Ok((cx, cy))) = (
                        osm_core::MapProjection::WebMercator.project_to_world_pixels(prev, zoom),
                        osm_core::MapProjection::WebMercator.project_to_world_pixels(curr, zoom),
                    ) {
                        let dx_tiles = (cx - px) / osm_renderer::TILE_SIZE_PX;
                        let dy_tiles = (cy - py) / osm_renderer::TILE_SIZE_PX;
                        self.camera_motion.velocity_tiles_per_sec = (dx_tiles / dt, dy_tiles / dt);
                    }
                }
            }
        }
        self.camera_motion.last_camera = Some(camera);
        self.camera_motion.last_update = Some(now);
    }

    fn build_tile_request_plan(
        &self,
        visible_tiles: &[osm_renderer::VisibleTile],
    ) -> HashMap<TileId, TilePriority> {
        let mut plan = HashMap::new();
        if visible_tiles.is_empty() {
            return plan;
        }
        let zoom = visible_tiles[0].id.z;
        let limit = 1_i64 << zoom;
        let min_x = visible_tiles
            .iter()
            .map(|t| t.id.x as i64)
            .min()
            .unwrap_or(0);
        let max_x = visible_tiles
            .iter()
            .map(|t| t.id.x as i64)
            .max()
            .unwrap_or(0);
        let min_y = visible_tiles
            .iter()
            .map(|t| t.id.y as i64)
            .min()
            .unwrap_or(0);
        let max_y = visible_tiles
            .iter()
            .map(|t| t.id.y as i64)
            .max()
            .unwrap_or(0);
        for tile in visible_tiles {
            plan.insert(tile.id, TilePriority::P0Visible);
        }
        let rings = self.dynamic_prefetch_rings();
        for ring in 1..=rings {
            for y in (min_y - ring as i64)..=(max_y + ring as i64) {
                if y < 0 || y >= limit {
                    continue;
                }
                for x in (min_x - ring as i64)..=(max_x + ring as i64) {
                    let border = x == min_x - ring as i64
                        || x == max_x + ring as i64
                        || y == min_y - ring as i64
                        || y == max_y + ring as i64;
                    if !border {
                        continue;
                    }
                    let wrapped_x = x.rem_euclid(limit);
                    if let Ok(id) = TileId::new(zoom, wrapped_x as u32, y as u32) {
                        plan.entry(id).or_insert(TilePriority::P2Periphery);
                    }
                }
            }
        }
        for id in self.look_ahead_tiles(zoom, min_x, max_x, min_y, max_y) {
            if !matches!(plan.get(&id), Some(TilePriority::P0Visible)) {
                plan.insert(id, TilePriority::P1LookAhead);
            }
        }
        plan
    }

    fn dynamic_prefetch_rings(&self) -> u32 {
        let speed = self
            .camera_motion
            .velocity_tiles_per_sec
            .0
            .hypot(self.camera_motion.velocity_tiles_per_sec.1);
        if speed > 3.5 {
            3
        } else if speed > 1.5 {
            2
        } else {
            1
        }
    }

    fn look_ahead_tiles(
        &self,
        zoom: u32,
        min_x: i64,
        max_x: i64,
        min_y: i64,
        max_y: i64,
    ) -> Vec<TileId> {
        let limit = 1_i64 << zoom;
        let (vx, vy) = self.camera_motion.velocity_tiles_per_sec;
        let speed = vx.hypot(vy);
        if speed < 0.25 {
            return Vec::new();
        }
        let dir_x = vx / speed;
        let dir_y = vy / speed;
        let depth = (speed.ceil() as i64).clamp(1, 3);
        let half_width = 1_i64;
        let mut tiles = Vec::new();
        for step in 1..=depth {
            for lateral in -half_width..=half_width {
                let ahead_x = ((min_x + max_x) / 2)
                    + (dir_x * step as f64).round() as i64
                    + (dir_y * lateral as f64).round() as i64;
                let ahead_y = ((min_y + max_y) / 2) + (dir_y * step as f64).round() as i64
                    - (dir_x * lateral as f64).round() as i64;
                if ahead_y < 0 || ahead_y >= limit {
                    continue;
                }
                let wrapped_x = ahead_x.rem_euclid(limit);
                if let Ok(id) = TileId::new(zoom, wrapped_x as u32, ahead_y as u32) {
                    tiles.push(id);
                }
            }
        }
        tiles
    }

    fn prefetch_budget(&self) -> usize {
        let ms = self.frame_timing.frame_interval.as_millis();
        if ms > 40 {
            4
        } else if ms > 24 {
            8
        } else {
            12
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
            uploaded_tiles: HashMap::new(),
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

    fn upload_tile(&mut self, tile_id: TileId, tile: &LoadedTile) {
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

        self.uploaded_tiles.insert(
            tile_id,
            UploadedTile {
                _texture: texture,
                bind_group,
            },
        );
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
        if let Some(uploaded_tile) = self.uploaded_tiles.get(&tile_id) {
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

            if let Some(uploaded_tile) = self.uploaded_tiles.get(&ancestor) {
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
