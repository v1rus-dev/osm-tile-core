use thiserror::Error;

use osm_core::CoreError;

#[derive(Debug, Error)]
pub enum TileLoadError {
    #[error("invalid zoom {z}; max supported zoom is {max}")]
    InvalidZoom { z: u32, max: u32 },

    #[error("invalid continuous zoom {zoom}; expected value in 0.0..={max}")]
    InvalidContinuousZoom { zoom: f64, max: u32 },

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

    #[error("invalid tile URL template; expected placeholders {{z}}, {{x}}, and {{y}}")]
    InvalidTemplate,

    #[error("invalid cache path")]
    InvalidCachePath,

    #[error("cache I/O error: {0}")]
    CacheIo(#[source] std::io::Error),

    #[error("tile server returned HTTP status {0}")]
    HttpStatus(reqwest::StatusCode),

    #[error("network request failed: {0}")]
    Network(#[from] reqwest::Error),
}

impl From<CoreError> for TileLoadError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::InvalidZoom { z, max } => Self::InvalidZoom { z, max },
            CoreError::InvalidContinuousZoom { zoom, max } => {
                Self::InvalidContinuousZoom { zoom, max }
            }
            CoreError::InvalidTileCoordinate { z, x, y, limit } => {
                Self::InvalidTileCoordinate { z, x, y, limit }
            }
            CoreError::InvalidLatitude { lat } => Self::InvalidLatitude { lat },
            CoreError::InvalidLongitude { lon } => Self::InvalidLongitude { lon },
            CoreError::InvalidBounds {
                south,
                west,
                north,
                east,
            } => Self::InvalidBounds {
                south,
                west,
                north,
                east,
            },
        }
    }
}
