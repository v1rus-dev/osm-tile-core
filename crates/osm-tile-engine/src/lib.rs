#[cfg(feature = "mobile")]
uniffi::setup_scaffolding!();

#[cfg(feature = "mobile")]
pub mod mobile;

#[cfg(feature = "mobile")]
pub use mobile::{
    MobileMarker, MobileMarkerCluster, MobileMarkerRenderItem, MobileRenderItemType,
    MobileViewport, OsmTileEngine, OsmTileEngineError,
};

#[cfg(feature = "android-renderer")]
pub mod android_renderer;

#[cfg(any(feature = "android-renderer", test))]
mod tile_request_queue;

#[cfg(any(feature = "android-renderer", test))]
mod tile_retention;
