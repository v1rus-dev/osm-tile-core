#[cfg(feature = "mobile")]
uniffi::setup_scaffolding!();

pub mod cache;
pub mod error;
pub mod geo;
pub mod map_state;
pub mod marker;
#[cfg(feature = "mobile")]
pub mod mobile;
pub mod source;
pub mod tile_id;

pub use cache::FileTileCache;
pub use error::TileError;
pub use geo::{GeoBounds, GeoPoint, Viewport};
pub use map_state::MapState;
pub use marker::{Marker, MarkerCluster, MarkerId, MarkerRenderItem};
#[cfg(feature = "mobile")]
pub use mobile::{
    MobileMarker, MobileMarkerCluster, MobileMarkerRenderItem, MobileRenderItemType,
    MobileViewport, OsmTileCore, OsmTileCoreError,
};
pub use source::{CachedTileSource, HttpTileSource, TileSource};
pub use tile_id::TileId;
