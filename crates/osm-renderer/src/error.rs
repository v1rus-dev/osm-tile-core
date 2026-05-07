use thiserror::Error;

use osm_core::CoreError;

#[derive(Debug, Error)]
pub enum RenderError {
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

    #[error("invalid marker zoom range: min_zoom={min_zoom}, max_zoom={max_zoom}")]
    InvalidMarkerZoomRange { min_zoom: u32, max_zoom: u32 },

    #[error("invalid cluster radius {radius}; expected a positive finite value")]
    InvalidClusterRadius { radius: f64 },

    #[error("invalid camera zoom {zoom}; max supported zoom is {max}")]
    InvalidCameraZoom { zoom: f64, max: u32 },

    #[error("invalid camera {name} angle {value}")]
    InvalidCameraAngle { name: &'static str, value: f64 },

    #[error("invalid render viewport: width={width_px}, height={height_px}, density={density}")]
    InvalidRenderViewport {
        width_px: u32,
        height_px: u32,
        density: f64,
    },

    #[error("render viewport is not set")]
    MissingRenderViewport,

    #[error("invalid layer id")]
    InvalidLayerId,

    #[error("invalid layer opacity {opacity}; expected 0.0..=1.0")]
    InvalidLayerOpacity { opacity: f32 },

    #[error("invalid layer zoom range: min_zoom={min_zoom}, max_zoom={max_zoom}")]
    InvalidLayerZoomRange { min_zoom: u32, max_zoom: u32 },

    #[error("missing layer {id}")]
    MissingLayer { id: String },

    #[error("map viewport is not set")]
    MissingViewport,
}

impl From<CoreError> for RenderError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::InvalidZoom { z, max } => Self::InvalidZoom { z, max },
            CoreError::InvalidContinuousZoom { zoom, max } => Self::InvalidCameraZoom { zoom, max },
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
