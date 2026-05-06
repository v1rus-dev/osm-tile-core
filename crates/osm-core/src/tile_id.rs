use crate::TileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    pub const MAX_ZOOM: u32 = 30;

    pub fn new(z: u32, x: u32, y: u32) -> Result<Self, TileError> {
        Self { z, x, y }.validate()
    }

    pub fn validate(self) -> Result<Self, TileError> {
        if self.z > Self::MAX_ZOOM {
            return Err(TileError::InvalidZoom {
                z: self.z,
                max: Self::MAX_ZOOM,
            });
        }

        let limit = 1_u32 << self.z;

        if self.x >= limit || self.y >= limit {
            return Err(TileError::InvalidTileCoordinate {
                z: self.z,
                x: self.x,
                y: self.y,
                limit,
            });
        }

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_tile_ids() {
        assert_eq!(TileId::new(0, 0, 0).unwrap(), TileId { z: 0, x: 0, y: 0 });
        assert_eq!(TileId::new(1, 1, 1).unwrap(), TileId { z: 1, x: 1, y: 1 });
    }

    #[test]
    fn rejects_coordinates_outside_zoom_range() {
        assert!(matches!(
            TileId::new(1, 2, 0),
            Err(TileError::InvalidTileCoordinate { .. })
        ));
        assert!(matches!(
            TileId::new(1, 0, 2),
            Err(TileError::InvalidTileCoordinate { .. })
        ));
    }

    #[test]
    fn rejects_zoom_above_supported_range() {
        assert!(matches!(
            TileId::new(31, 0, 0),
            Err(TileError::InvalidZoom { .. })
        ));
    }
}
