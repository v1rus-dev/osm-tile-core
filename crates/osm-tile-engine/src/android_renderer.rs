use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr::NonNull;
#[cfg(feature = "mobile")]
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(feature = "mobile")]
use crate::mobile::OsmTileEngine;
use image::GenericImageView;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong};
use osm_core::TileId;
use osm_loader::{CachedTileSource, FileTileCache, HttpTileSource, TileSource};
use osm_renderer::{LayerId, MapCamera, MapLayer, RenderState, RenderViewport, TileLayer};

const DEFAULT_TILE_LAYER_ID: &str = "base";
const LOG_TAG: &str = "OsmTileEngine";
const ANDROID_LOG_DEBUG: c_int = 3;
const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_WARN: c_int = 5;
const ANDROID_LOG_ERROR: c_int = 6;
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
const TARGET_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const CAMERA_IDLE_PREFETCH_DELAY: Duration = Duration::from_millis(250);
const TELEMETRY_LOG_INTERVAL: Duration = Duration::from_secs(1);
const MOVING_UPLOADS_PER_FRAME: usize = 1;
const IDLE_UPLOADS_PER_FRAME: usize = 2;

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
    gpu_upload_deferred: u64,
    upload_count: u64,
    upload_time_total_ns: u128,
    fetch_count: u64,
    fetch_time_total_ns: u128,
    decode_count: u64,
    decode_time_total_ns: u128,
    render_count: u64,
    render_time_total_ns: u128,
    render_visible_tiles: u64,
    render_drawn_tiles: u64,
    render_missing_tiles: u64,
    camera_updates_received: u64,
    camera_updates_applied: u64,
    camera_updates_coalesced: u64,
    tile_requests_enqueued: u64,
    last_log: Option<Instant>,
}

impl TileTelemetry {
    fn avg_decode_ms(&self) -> f64 {
        if self.decode_count == 0 {
            return 0.0;
        }
        self.decode_time_total_ns as f64 / self.decode_count as f64 / 1_000_000.0
    }

    fn record_fetch(&mut self, elapsed: std::time::Duration) {
        self.fetch_count += 1;
        self.fetch_time_total_ns += elapsed.as_nanos();
    }

    fn record_decode(&mut self, elapsed: std::time::Duration) {
        self.decode_count += 1;
        self.decode_time_total_ns += elapsed.as_nanos();
    }

    fn record_upload(&mut self, elapsed: std::time::Duration) {
        self.upload_count += 1;
        self.upload_time_total_ns += elapsed.as_nanos();
    }

    fn record_render(&mut self, elapsed: Duration, stats: RenderStats) {
        self.render_count += 1;
        self.render_time_total_ns += elapsed.as_nanos();
        self.render_visible_tiles += stats.visible_tiles as u64;
        self.render_drawn_tiles += stats.drawn_tiles as u64;
        self.render_missing_tiles += stats.missing_tiles as u64;
    }

    fn avg_fetch_ms(&self) -> f64 {
        avg_ms(self.fetch_time_total_ns, self.fetch_count)
    }

    fn avg_upload_ms(&self) -> f64 {
        avg_ms(self.upload_time_total_ns, self.upload_count)
    }

    fn avg_render_ms(&self) -> f64 {
        avg_ms(self.render_time_total_ns, self.render_count)
    }
}

fn avg_ms(total_ns: u128, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total_ns as f64 / count as f64 / 1_000_000.0
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

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

fn log_android(priority: c_int, message: impl AsRef<str>) {
    let Ok(tag) = CString::new(LOG_TAG) else {
        return;
    };
    let text = message.as_ref().replace('\0', "\\0");
    let Ok(text) = CString::new(text) else {
        return;
    };

    unsafe {
        __android_log_write(priority, tag.as_ptr(), text.as_ptr());
    }
}

fn log_debug(message: impl AsRef<str>) {
    log_android(ANDROID_LOG_DEBUG, message);
}

fn log_info(message: impl AsRef<str>) {
    log_android(ANDROID_LOG_INFO, message);
}

fn log_warn(message: impl AsRef<str>) {
    log_android(ANDROID_LOG_WARN, message);
}

fn log_error(message: impl AsRef<str>) {
    log_android(ANDROID_LOG_ERROR, message);
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
        log_info(format!(
            "creating Android renderer url_template={} cache_dir={}",
            tile_url_template, cache_dir
        ));
        let http_source = HttpTileSource::new(tile_url_template.clone()).map_err(|error| {
            let message = error.to_string();
            log_error(format!("failed to create HTTP tile source: {message}"));
            message
        })?;
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
                log_info("tile loader thread started");
                tile_loader_loop(
                    tile_loader_source,
                    tile_loader_commands,
                    tile_request_receiver,
                )
            })
            .map_err(|error| {
                let message = error.to_string();
                log_error(format!("failed to spawn tile loader thread: {message}"));
                message
            })?;
        let worker_tile_requests = tile_requests.clone();
        let worker = thread::Builder::new()
            .name("osm-map-renderer".to_owned())
            .spawn(move || {
                log_info("renderer worker thread started");
                RendererWorker::new(state, worker_tile_requests).run(receiver)
            })
            .map_err(|error| {
                let message = error.to_string();
                log_error(format!("failed to spawn renderer worker thread: {message}"));
                message
            })?;

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
        if self.commands.send(command).is_err() {
            log_warn("failed to send renderer command: renderer worker is gone");
        }
    }
}

impl Drop for AndroidMapRenderer {
    fn drop(&mut self) {
        log_info("dropping Android renderer");
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
        error: Option<String>,
        fetch_elapsed: Duration,
        decode_elapsed: Duration,
    },
    Shutdown,
}

enum TileLoaderRequest {
    Load(TileLoadRequest),
    Shutdown,
}

#[derive(Debug)]
struct FetchedTile {
    request: TileLoadRequest,
    bytes: Result<Vec<u8>, String>,
    fetch_elapsed: Duration,
}

#[derive(Debug)]
struct DecodedTileResult {
    id: TileId,
    tile: Option<LoadedTile>,
    error: Option<String>,
    fetch_elapsed: Duration,
    decode_elapsed: Duration,
}

#[derive(Debug, Clone, Copy)]
struct TileRequestMetadata {
    generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct TileLoadRequest {
    id: TileId,
    metadata: TileRequestMetadata,
    priority: f32,
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

const RAW_RGBA_CACHE_MAGIC: &[u8; 8] = b"OSMRGBA1";

fn decode_tile_bytes(bytes: Vec<u8>) -> Result<LoadedTile, String> {
    if bytes.len() > 16 && &bytes[..8] == RAW_RGBA_CACHE_MAGIC {
        let width = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| "raw RGBA tile width header is invalid".to_owned())?,
        );
        let height = u32::from_le_bytes(
            bytes[12..16]
                .try_into()
                .map_err(|_| "raw RGBA tile height header is invalid".to_owned())?,
        );
        let rgba = bytes[16..].to_vec();
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "raw RGBA tile dimensions overflow".to_owned())?
            as usize;
        if rgba.len() == expected {
            return Ok(LoadedTile {
                width,
                height,
                rgba,
            });
        }
        return Err(format!(
            "raw RGBA tile has {} bytes, expected {} for {}x{}",
            rgba.len(),
            expected,
            width,
            height
        ));
    }

    let image = image::load_from_memory(&bytes).map_err(|error| {
        format!(
            "failed to decode tile bytes: {error}; len={} first_bytes={}",
            bytes.len(),
            first_bytes_hex(&bytes)
        )
    })?;
    let (width, height) = image.dimensions();
    Ok(LoadedTile {
        width,
        height,
        rgba: image.to_rgba8().into_raw(),
    })
}

fn first_bytes_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
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

#[derive(Debug, Default, Clone, Copy)]
struct RenderStats {
    visible_tiles: usize,
    drawn_tiles: usize,
    missing_tiles: usize,
}

struct RendererWorker {
    state: RenderState,
    surface_width_px: u32,
    surface_height_px: u32,
    loaded_tiles: SizedLruCache<LoadedTile>,
    pending_tile_loads: HashSet<TileId>,
    pending_metadata: HashMap<TileId, TileRequestMetadata>,
    request_generation: u64,
    last_camera_center: Option<(f64, f64)>,
    camera_velocity_hint: (f32, f32),
    pending_tile_priorities: HashMap<TileId, TilePriority>,
    tile_requests: Sender<TileLoaderRequest>,
    gpu: Option<GpuSurface>,
    camera_motion: CameraMotion,
    frame_timing: FrameTiming,
    telemetry: TileTelemetry,
    cache_limits: TileCacheLimits,
    last_visible_zoom: Option<u32>,
    pending_camera: Option<MapCamera>,
    needs_render: bool,
    render_in_flight: bool,
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
    tile_vertex_buffer: wgpu::Buffer,
    tile_vertex_buffer_capacity_bytes: u64,
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
            pending_metadata: HashMap::new(),
            request_generation: 0,
            last_camera_center: None,
            camera_velocity_hint: (0.0, 0.0),
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
                frame_interval: TARGET_FRAME_INTERVAL,
            },
            telemetry: TileTelemetry::default(),
            cache_limits: TileCacheLimits::default(),
            last_visible_zoom: None,
            pending_camera: None,
            needs_render: false,
            render_in_flight: false,
        }
    }

    fn run(mut self, receiver: Receiver<RenderCommand>) {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                log_error(format!("failed to create renderer async runtime: {error}"));
                return;
            }
        };

        loop {
            let timeout = self.time_until_next_frame();
            let recv_result = match timeout {
                Some(timeout) => receiver.recv_timeout(timeout),
                None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };

            match recv_result {
                Ok(command) => match command {
                    RenderCommand::SurfaceCreated { window } => {
                        log_info("surface created");
                        self.create_gpu_surface(window, &runtime);
                        self.mark_needs_render();
                    }
                    RenderCommand::SurfaceDestroyed => {
                        log_info("surface destroyed");
                        self.gpu = None;
                    }
                    RenderCommand::Resize {
                        width_px,
                        height_px,
                        density,
                    } => {
                        log_info(format!(
                            "resize surface width={} height={} density={:.2}",
                            width_px, height_px, density
                        ));
                        self.surface_width_px = width_px;
                        self.surface_height_px = height_px;
                        if let Some(gpu) = self.gpu.as_mut() {
                            gpu.configure(width_px, height_px);
                        }
                        match RenderViewport::new(width_px, height_px, density) {
                            Ok(viewport) => match self.state.set_viewport(viewport) {
                                Ok(()) => self.mark_needs_render(),
                                Err(error) => {
                                    log_warn(format!("failed to set render viewport: {error}"))
                                }
                            },
                            Err(error) => {
                                log_warn(format!("invalid render viewport from surface: {error}"))
                            }
                        }
                    }
                    RenderCommand::SetCamera(camera) => {
                        self.telemetry.camera_updates_received += 1;
                        if self.pending_camera.replace(camera).is_some() {
                            self.telemetry.camera_updates_coalesced += 1;
                        }
                        self.mark_needs_render();
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
                            match self
                                .state
                                .layers_mut()
                                .add_or_replace(MapLayer::Tile(layer))
                            {
                                Ok(()) => {
                                    log_info("tile layer added or replaced");
                                    self.mark_needs_render();
                                }
                                Err(error) => {
                                    log_warn(format!("failed to add tile layer: {error}"))
                                }
                            }
                        }
                    }
                    RenderCommand::RemoveLayer(id) => {
                        if let Ok(id) = LayerId::new(id) {
                            self.state.layers_mut().remove(&id);
                            self.mark_needs_render();
                        }
                    }
                    RenderCommand::SetLayerVisible { id, visible } => {
                        if let Ok(id) = LayerId::new(id) {
                            match self.state.layers_mut().set_visible(&id, visible) {
                                Ok(()) => self.mark_needs_render(),
                                Err(error) => {
                                    log_warn(format!("failed to set layer visibility: {error}"))
                                }
                            }
                        }
                    }
                    RenderCommand::SetLayerOpacity { id, opacity } => {
                        if let Ok(id) = LayerId::new(id) {
                            match self.state.layers_mut().set_opacity(&id, opacity) {
                                Ok(()) => self.mark_needs_render(),
                                Err(error) => {
                                    log_warn(format!("failed to set layer opacity: {error}"))
                                }
                            }
                        }
                    }
                    RenderCommand::TileLoaded {
                        id,
                        tile,
                        error,
                        fetch_elapsed,
                        decode_elapsed,
                    } => {
                        let still_relevant = self.pending_metadata.contains_key(&id);
                        self.pending_tile_loads.remove(&id);
                        if still_relevant {
                            self.telemetry.record_fetch(fetch_elapsed);
                            self.telemetry.record_decode(decode_elapsed);
                            self.pending_metadata.remove(&id);
                            if let Some(error) = error {
                                log_warn(format!(
                                    "tile z={} x={} y={} failed: {}",
                                    id.z, id.x, id.y, error
                                ));
                            }
                            if let Some(tile) = tile {
                                let size_bytes = tile.rgba.len();
                                self.loaded_tiles.insert(id, tile, size_bytes);
                                self.telemetry.decoded_hit += 1;
                            } else {
                                self.telemetry.decoded_miss += 1;
                            }
                            self.mark_needs_render();
                        }
                    }
                    RenderCommand::Shutdown => {
                        log_info("renderer worker shutting down");
                        break;
                    }
                },
                Err(RecvTimeoutError::Timeout) => {
                    self.render_if_due();
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }

            self.render_if_due();
        }
    }

    fn mark_needs_render(&mut self) {
        self.needs_render = true;
    }

    fn time_until_next_frame(&self) -> Option<Duration> {
        if !self.needs_render || self.render_in_flight {
            return None;
        }

        if let Some(last_frame) = self.frame_timing.last_frame {
            let elapsed = last_frame.elapsed();
            Some(self.frame_timing.frame_interval.saturating_sub(elapsed))
        } else {
            Some(Duration::ZERO)
        }
    }

    fn render_if_due(&mut self) {
        if !self.needs_render || self.render_in_flight {
            return;
        }

        if let Some(last_frame) = self.frame_timing.last_frame {
            let elapsed = last_frame.elapsed();
            if elapsed < self.frame_timing.frame_interval {
                return;
            }
        }

        self.render_in_flight = true;
        self.needs_render = false;
        self.apply_pending_camera();
        self.render_frame();
        self.render_in_flight = false;
    }

    fn apply_pending_camera(&mut self) {
        let Some(camera) = self.pending_camera.take() else {
            return;
        };

        self.update_camera_velocity_hint(camera);
        match self.state.set_camera(camera) {
            Ok(()) => {
                self.request_generation = self.request_generation.saturating_add(1);
                self.update_camera_motion(camera);
                self.telemetry.camera_updates_applied += 1;
            }
            Err(error) => log_warn(format!("failed to set camera: {error}")),
        }
    }

    fn create_gpu_surface(&mut self, window: NativeWindow, runtime: &tokio::runtime::Runtime) {
        self.gpu = None;

        let mut gpu = match GpuSurface::new(window, runtime) {
            Ok(gpu) => gpu,
            Err(error) => {
                log_error(format!("failed to create Android wgpu surface: {error}"));
                return;
            }
        };
        log_info("Android wgpu surface created");

        if self.surface_width_px > 0 && self.surface_height_px > 0 {
            gpu.configure(self.surface_width_px, self.surface_height_px);
        }

        self.gpu = Some(gpu);
    }

    fn render_frame(&mut self) {
        let render_started = Instant::now();
        self.frame_timing.last_frame = Some(render_started);
        let visible_tiles = if self.has_visible_tile_layer() {
            match self.state.visible_tiles() {
                Ok(tiles) => Some(tiles),
                Err(error) => {
                    log_warn(format!("failed to calculate visible tiles: {error}"));
                    None
                }
            }
        } else {
            log_debug("render frame skipped tile planning: no visible tile layer");
            None
        };

        if let Some(visible_tiles) = visible_tiles.as_ref() {
            self.evict_far_zoom_layers(visible_tiles);
            self.ensure_visible_tiles(visible_tiles);
            self.enforce_cache_limits(visible_tiles);
        }

        let mut stats = visible_tiles
            .as_ref()
            .map(|tiles| RenderStats {
                visible_tiles: tiles.len(),
                ..Default::default()
            })
            .unwrap_or_default();
        if let Some(gpu) = self.gpu.as_mut() {
            if let Some(render_stats) = gpu.render(visible_tiles.as_deref()) {
                stats = render_stats;
            }
        }
        self.telemetry
            .record_render(render_started.elapsed(), stats);
        self.log_telemetry_if_due();
    }

    fn ensure_visible_tiles(&mut self, visible_tiles: &[osm_renderer::VisibleTile]) {
        let request_plan = self.build_tile_request_plan(visible_tiles);
        self.pending_tile_loads
            .retain(|tile_id| request_plan.contains_key(tile_id));
        self.pending_metadata
            .retain(|tile_id, _| request_plan.contains_key(tile_id));
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
        let mut enqueued = 0_usize;
        for (tile_id, priority) in ordered_requests.into_iter().take(available_slots) {
            let metadata = TileRequestMetadata {
                generation: self.request_generation,
            };
            self.pending_tile_loads.insert(tile_id);
            self.pending_tile_priorities.insert(tile_id, priority);
            self.pending_metadata.insert(tile_id, metadata);
            match self
                .tile_requests
                .send(TileLoaderRequest::Load(TileLoadRequest {
                    id: tile_id,
                    metadata,
                    priority: self.tile_request_priority(tile_id, priority, visible_tiles),
                })) {
                Ok(()) => enqueued += 1,
                Err(_) => log_warn(format!(
                    "failed to enqueue tile z={} x={} y={}: tile loader is gone",
                    tile_id.z, tile_id.x, tile_id.y
                )),
            }
        }
        self.telemetry.tile_requests_enqueued += enqueued as u64;

        let upload_budget = self.upload_budget_per_frame();
        let mut uploads_this_frame = 0_usize;
        let mut deferred_uploads = 0_usize;
        for visible_tile in visible_tiles {
            let tile_id = visible_tile.id;
            let loaded = self.loaded_tiles.contains_key(&tile_id);
            if loaded {
                self.telemetry.decoded_hit += 1;
            }

            let needs_upload = loaded
                && self
                    .gpu
                    .as_ref()
                    .map(|gpu| !gpu.uploaded_tiles.contains_key(&tile_id))
                    .unwrap_or(false);
            if !needs_upload {
                if loaded && self.gpu.is_some() {
                    self.telemetry.gpu_hit += 1;
                }
                continue;
            }

            self.telemetry.gpu_miss += 1;
            if uploads_this_frame >= upload_budget {
                deferred_uploads += 1;
                continue;
            }

            let Some(tile_data) = self.loaded_tiles.get_cloned(&tile_id) else {
                continue;
            };
            if let Some(gpu) = self.gpu.as_mut() {
                if !gpu.uploaded_tiles.contains_key(&tile_id) {
                    gpu.upload_tile(tile_id, &tile_data, &mut self.telemetry);
                    uploads_this_frame += 1;
                }
            }
        }

        if deferred_uploads > 0 {
            self.telemetry.gpu_upload_deferred += deferred_uploads as u64;
            self.mark_needs_render();
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
                        let dx_tiles = (cx - px) / osm_core::TILE_SIZE_PX;
                        let dy_tiles = (cy - py) / osm_core::TILE_SIZE_PX;
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
        if self.camera_is_moving() {
            return plan;
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
        if self.camera_is_moving() {
            return 4;
        }

        let render_ms = self.telemetry.avg_render_ms();
        let decode_ms = self.telemetry.avg_decode_ms();
        if render_ms > 24.0 || decode_ms > 14.0 {
            4
        } else if render_ms > 16.0 || decode_ms > 8.0 {
            8
        } else {
            12
        }
    }

    fn upload_budget_per_frame(&self) -> usize {
        if self.camera_is_moving() {
            MOVING_UPLOADS_PER_FRAME
        } else {
            IDLE_UPLOADS_PER_FRAME
        }
    }

    fn camera_is_moving(&self) -> bool {
        self.pending_camera.is_some()
            || self
                .camera_motion
                .last_update
                .map(|updated| updated.elapsed() < CAMERA_IDLE_PREFETCH_DELAY)
                .unwrap_or(false)
    }

    fn log_telemetry_if_due(&mut self) {
        let now = Instant::now();
        if let Some(last_log) = self.telemetry.last_log {
            if now.saturating_duration_since(last_log) < TELEMETRY_LOG_INTERVAL {
                return;
            }
        }
        self.telemetry.last_log = Some(now);

        log_debug(format!(
            "renderer telemetry frames={} avg_render={:.1}ms avg_fetch={:.1}ms avg_decode={:.1}ms avg_upload={:.1}ms visible={} drawn={} missing={} uploads={} deferred={} tile_requests={} camera_recv={} applied={} coalesced={}",
            self.telemetry.render_count,
            self.telemetry.avg_render_ms(),
            self.telemetry.avg_fetch_ms(),
            self.telemetry.avg_decode_ms(),
            self.telemetry.avg_upload_ms(),
            self.telemetry.render_visible_tiles,
            self.telemetry.render_drawn_tiles,
            self.telemetry.render_missing_tiles,
            self.telemetry.upload_count,
            self.telemetry.gpu_upload_deferred,
            self.telemetry.tile_requests_enqueued,
            self.telemetry.camera_updates_received,
            self.telemetry.camera_updates_applied,
            self.telemetry.camera_updates_coalesced,
        ));
    }

    fn tile_request_priority(
        &self,
        tile_id: TileId,
        priority: TilePriority,
        visible_tiles: &[osm_renderer::VisibleTile],
    ) -> f32 {
        let tier = match priority {
            TilePriority::P0Visible => 3_000.0,
            TilePriority::P1LookAhead => 2_000.0,
            TilePriority::P2Periphery => 1_000.0,
        };
        let screen_priority = visible_tiles
            .iter()
            .find(|tile| tile.id == tile_id)
            .map(|tile| self.tile_priority(tile))
            .unwrap_or(0.0);
        tier + screen_priority
    }

    fn has_visible_tile_layer(&self) -> bool {
        self.state.layers().layers().iter().any(|layer| {
            matches!(layer, MapLayer::Tile(_))
                && layer.common().visible
                && layer.common().opacity > 0.0
        })
    }

    fn update_camera_velocity_hint(&mut self, camera: MapCamera) {
        let current = (camera.center_lat, camera.center_lon);
        if let Some(previous) = self.last_camera_center {
            let dx = (current.1 - previous.1) as f32;
            let dy = (current.0 - previous.0) as f32;
            let magnitude = (dx * dx + dy * dy).sqrt();
            self.camera_velocity_hint = if magnitude > 0.0 {
                (dx / magnitude, dy / magnitude)
            } else {
                (0.0, 0.0)
            };
        }
        self.last_camera_center = Some(current);
    }

    fn tile_priority(&self, visible_tile: &osm_renderer::VisibleTile) -> f32 {
        let viewport_center_x = self.surface_width_px as f32 * 0.5;
        let viewport_center_y = self.surface_height_px as f32 * 0.5;
        let tile_center_x = visible_tile.screen_x_px + visible_tile.size_px * 0.5;
        let tile_center_y = visible_tile.screen_y_px + visible_tile.size_px * 0.5;
        let to_tile_x = tile_center_x - viewport_center_x;
        let to_tile_y = tile_center_y - viewport_center_y;
        let distance = (to_tile_x * to_tile_x + to_tile_y * to_tile_y).sqrt();
        let direction_bonus = (to_tile_x * self.camera_velocity_hint.0
            + to_tile_y * self.camera_velocity_hint.1)
            / visible_tile.size_px.max(1.0);
        -distance + direction_bonus * 32.0
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
        let max_distance = u32::from(self.cache_limits.max_zoom_distance_to_keep);
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
    const MAX_CONCURRENT_FETCHES: usize = 4;
    const DECODE_QUEUE_BOUND: usize = 32;

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            log_error(format!(
                "failed to create tile loader async runtime: {error}"
            ));
            return;
        }
    };

    let decode_workers = std::thread::available_parallelism()
        .map(|cpus| usize::min(2, usize::max(1, cpus.get() / 2)))
        .unwrap_or(1);
    let (decode_tx, decode_rx) = mpsc::sync_channel::<FetchedTile>(DECODE_QUEUE_BOUND);
    let (decoded_tx, decoded_rx) = mpsc::channel::<DecodedTileResult>();

    let decode_rx = std::sync::Arc::new(std::sync::Mutex::new(decode_rx));
    let mut decode_threads = Vec::with_capacity(decode_workers);
    for _ in 0..decode_workers {
        let rx = decode_rx.clone();
        let tx = decoded_tx.clone();
        decode_threads.push(std::thread::spawn(move || {
            loop {
                let fetched = {
                    let guard = rx.lock().expect("decode queue lock poisoned");
                    guard.recv()
                };
                let Ok(fetched) = fetched else { break };
                let decode_started = Instant::now();
                let (tile, error) = match fetched.bytes {
                    Ok(bytes) => match decode_tile_bytes(bytes) {
                        Ok(tile) => (Some(tile), None),
                        Err(error) => (None, Some(error)),
                    },
                    Err(error) => (None, Some(error)),
                };
                let _ = tx.send(DecodedTileResult {
                    id: fetched.request.id,
                    tile,
                    error,
                    fetch_elapsed: fetched.fetch_elapsed,
                    decode_elapsed: decode_started.elapsed(),
                });
            }
        }));
    }

    let mut queued: BinaryHeap<PrioritizedTileRequest> = BinaryHeap::new();
    let mut in_flight = HashSet::new();
    let mut in_flight_fetches = tokio::task::JoinSet::new();
    let mut shutting_down = false;

    loop {
        while in_flight_fetches.len() < MAX_CONCURRENT_FETCHES {
            let Some(request) = queued.pop() else { break };
            let request = request.0;
            if request.metadata.generation
                != newest_generation(&queued, request.metadata.generation)
            {
                in_flight.remove(&request.id);
                continue;
            }
            let source = source.clone();
            in_flight_fetches.spawn_on(
                async move {
                    let fetch_started = Instant::now();
                    let cache_path = source.cache().tile_path(request.id);
                    let remote_url = source.source().tile_url(request.id);
                    let bytes = source
                        .load_tile(request.id)
                        .await
                        .map_err(|error| error.to_string());
                    if let Err(error) = &bytes {
                        log_warn(format!(
                            "tile z={} x={} y={} fetch failed in {}ms cache_path={} remote_url={}: {}",
                            request.id.z,
                            request.id.x,
                            request.id.y,
                            fetch_started.elapsed().as_millis(),
                            cache_path.display(),
                            remote_url,
                            error
                        ));
                    }
                    FetchedTile {
                        request,
                        bytes,
                        fetch_elapsed: fetch_started.elapsed(),
                    }
                },
                runtime.handle(),
            );
        }

        while let Ok(request) = receiver.recv_timeout(Duration::from_millis(2)) {
            match request {
                TileLoaderRequest::Load(request) => {
                    if in_flight.insert(request.id) {
                        queued.push(PrioritizedTileRequest(request));
                    }
                }
                TileLoaderRequest::Shutdown => {
                    log_info("tile loader shutdown requested");
                    shutting_down = true;
                }
            }
        }

        if let Ok(Some(join_result)) = runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(2), in_flight_fetches.join_next()).await
        }) {
            if let Ok(fetched) = join_result {
                if decode_tx.send(fetched).is_err() {
                    log_warn("decode queue disconnected");
                    break;
                }
            } else {
                log_warn("tile fetch task failed to join");
            }
        }

        loop {
            match decoded_rx.try_recv() {
                Ok(result) => {
                    in_flight.remove(&result.id);
                    if result.tile.is_none() {
                        let cache = source.cache().clone();
                        if let Err(error) = runtime.block_on(cache.remove(result.id)) {
                            log_warn(format!(
                                "failed to remove undecodable cached tile z={} x={} y={}: {}",
                                result.id.z, result.id.x, result.id.y, error
                            ));
                        }
                    }
                    if commands
                        .send(RenderCommand::TileLoaded {
                            id: result.id,
                            tile: result.tile,
                            error: result.error,
                            fetch_elapsed: result.fetch_elapsed,
                            decode_elapsed: result.decode_elapsed,
                        })
                        .is_err()
                    {
                        log_warn("failed to send decoded tile to renderer worker");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if shutting_down
            && queued.is_empty()
            && in_flight_fetches.is_empty()
            && in_flight.is_empty()
        {
            break;
        }
    }

    drop(decode_tx);
    for handle in decode_threads {
        let _ = handle.join();
    }
    log_info("tile loader thread stopped");
}

#[derive(Debug, Clone, Copy)]
struct PrioritizedTileRequest(TileLoadRequest);

impl PartialEq for PrioritizedTileRequest {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for PrioritizedTileRequest {}

impl PartialOrd for PrioritizedTileRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedTileRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .priority
            .total_cmp(&other.0.priority)
            .then_with(|| self.0.metadata.generation.cmp(&other.0.metadata.generation))
    }
}

fn newest_generation(queue: &BinaryHeap<PrioritizedTileRequest>, fallback: u64) -> u64 {
    queue
        .iter()
        .map(|entry| entry.0.metadata.generation)
        .max()
        .map(|generation| generation.max(fallback))
        .unwrap_or(fallback)
}

impl GpuSurface {
    fn new(native_window: NativeWindow, runtime: &tokio::runtime::Runtime) -> Result<Self, String> {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = wgpu::Backends::GL;
        let instance = wgpu::Instance::new(instance_descriptor);
        log_info("using OpenGL backend for Android wgpu surface");
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
        let tile_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("osm-map-renderer-tile-vertex-buffer"),
            size: 1,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            tile_vertex_buffer,
            tile_vertex_buffer_capacity_bytes: 1,
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
            log_error("failed to choose Android wgpu surface configuration");
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

    fn render(
        &mut self,
        visible_tiles: Option<&[osm_renderer::VisibleTile]>,
    ) -> Option<RenderStats> {
        if self.config.is_none() {
            return None;
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                if let Some(config) = self.config.clone() {
                    self.surface.configure(&self.device, &config);
                }
                return None;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return None;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log_error("Android wgpu surface returned a validation error");
                return None;
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

        let mut stats = visible_tiles
            .map(|tiles| RenderStats {
                visible_tiles: tiles.len(),
                ..Default::default()
            })
            .unwrap_or_default();
        if let Some(visible_tiles) = visible_tiles {
            let mut batch_vertices = Vec::new();
            let mut batch_draws = Vec::new();
            let mut missing_tiles = 0_usize;
            self.ensure_tile_vertex_buffer_capacity(
                (visible_tiles.len() * 6 * std::mem::size_of::<TexturedVertex>()) as u64,
            );

            for visible_tile in visible_tiles {
                let Some(tile_draw) = self.resolve_tile_draw(visible_tile.id) else {
                    missing_tiles += 1;
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
                stats.drawn_tiles = batch_draws.len();
                stats.missing_tiles = missing_tiles;
                self.write_tile_vertices(&batch_vertices);
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
                    render_pass.set_vertex_buffer(
                        0,
                        self.tile_vertex_buffer.slice(vertex_start..vertex_end),
                    );
                    render_pass.draw(0..6, 0..1);
                }
            } else {
                stats.missing_tiles = missing_tiles;
            }
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Some(stats)
    }

    fn ensure_tile_vertex_buffer_capacity(&mut self, required_bytes: u64) {
        if required_bytes > self.tile_vertex_buffer_capacity_bytes {
            let capacity = required_bytes.next_power_of_two();
            self.tile_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("osm-map-renderer-tile-vertex-buffer"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.tile_vertex_buffer_capacity_bytes = capacity;
        }
    }

    fn write_tile_vertices(&self, vertices: &[TexturedVertex]) {
        let vertex_bytes = bytes_of(vertices);
        self.queue
            .write_buffer(&self.tile_vertex_buffer, 0, vertex_bytes);
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


mod jni_bridge;
