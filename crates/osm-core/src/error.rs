use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid zoom {z}; max supported zoom is {max}")]
    InvalidZoom { z: u32, max: u32 },

    #[error(
        "invalid tile coordinate for z={z}: x={x}, y={y}; coordinates must be less than {limit}"
    )]
    InvalidTileCoordinate { z: u32, x: u32, y: u32, limit: u32 },

    #[error("invalid latitude {lat}; expected value in -90..=90")]
    InvalidLatitude { lat: f64 },

    #[error("invalid longitude {lon}; expected value in -180..=180")]
    InvalidLongitude { lon: f64 },

    #[error("invalid geo bounds: south={south}, west={west}, north={north}, east={east}")]
    InvalidBounds {
        south: f64,
        west: f64,
        north: f64,
        east: f64,
    },
}
