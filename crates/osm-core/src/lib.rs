pub mod error;
pub mod geo;
pub mod projection;
pub mod tile_id;

pub use error::CoreError;
pub use geo::{GeoBounds, GeoPoint, Viewport, validate_latitude, validate_longitude};
pub use projection::{MapProjection, TILE_SIZE_PX, WEB_MERCATOR_MAX_LAT};
pub use tile_id::TileId;

pub type BoundingBox = GeoBounds;
pub type TileError = CoreError;
