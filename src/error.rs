use thiserror::Error;

#[derive(Debug, Error)]
pub enum TileError {
    #[error("invalid zoom {z}; max supported zoom is {max}")]
    InvalidZoom { z: u32, max: u32 },

    #[error(
        "invalid tile coordinate for z={z}: x={x}, y={y}; coordinates must be less than {limit}"
    )]
    InvalidTileCoordinate { z: u32, x: u32, y: u32, limit: u32 },

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
