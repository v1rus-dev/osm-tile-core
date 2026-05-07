use super::*;

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
