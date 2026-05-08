pub mod error;
pub mod geo;
pub mod projection;
pub mod tile_id;
pub mod zoom;

pub use error::CoreError;
pub use geo::{GeoBounds, GeoPoint, Viewport, validate_latitude, validate_longitude};
pub use projection::{MapProjection, WEB_MERCATOR_MAX_LAT};
pub use tile_id::{FadeState, OverscaledTileId, TileId, TileState};
pub use zoom::{
    EQUATOR_M_PER_PIXEL_Z0, MAX_ZOOM, MIN_ZOOM, OSM_STANDARD_MAX_ZOOM, TILE_SIZE_PX,
    lower_tile_zoom, meters_per_pixel, nearest_zoom_level, tile_count, tile_count_per_axis,
    validate_continuous_zoom, validate_zoom_level, world_size_px, world_size_px_at_level,
    zoom_fraction,
};

pub type BoundingBox = GeoBounds;
pub type TileError = CoreError;
