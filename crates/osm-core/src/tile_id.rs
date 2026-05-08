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

    pub fn parent(&self, levels: u32) -> Option<Self> {
        let shift = levels.min(self.z);
        if shift == 0 {
            return Some(*self);
        }
        Self::new(self.z - shift, self.x >> shift, self.y >> shift).ok()
    }

    pub fn children(&self) -> impl Iterator<Item = Self> {
        let z = self.z.saturating_add(1);
        let base_x = self.x * 2;
        let base_y = self.y * 2;
        (0..2)
            .flat_map(move |dy| (0..2).map(move |dx| Self::new(z, base_x + dx, base_y + dy)))
            .filter_map(Result::ok)
    }

    pub fn overscale_to(self, overscaled_z: u32) -> OverscaledTileId {
        OverscaledTileId {
            overscaled_z,
            z: self.z,
            x: self.x,
            y: self.y,
            wrap: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverscaledTileId {
    pub overscaled_z: u32,
    pub z: u32,
    pub x: u32,
    pub y: u32,
    pub wrap: i32,
}

impl OverscaledTileId {
    pub fn new(overscaled_z: u32, z: u32, x: u32, y: u32, wrap: i32) -> Result<Self, TileError> {
        let canonical = TileId::new(z, x, y)?;
        if overscaled_z < z {
            return Err(TileError::InvalidZoom {
                z: overscaled_z,
                max: z,
            });
        }
        Ok(Self {
            overscaled_z,
            z: canonical.z,
            x: canonical.x,
            y: canonical.y,
            wrap,
        })
    }

    pub fn from_canonical(id: TileId, wrap: i32) -> Self {
        Self {
            overscaled_z: id.z,
            z: id.z,
            x: id.x,
            y: id.y,
            wrap,
        }
    }

    pub fn canonical(&self) -> TileId {
        TileId {
            z: self.z,
            x: self.x,
            y: self.y,
        }
    }

    pub fn overscale_factor(&self) -> f64 {
        if self.overscaled_z == self.z {
            return 1.0;
        }
        2_f64.powi((self.overscaled_z - self.z) as i32)
    }

    pub fn wrap_x(self, delta: i32, _limit: i32) -> Self {
        Self {
            wrap: self.wrap + delta,
            ..self
        }
    }

    pub fn parent(&self, levels: u32) -> Option<Self> {
        let shift = levels.min(self.z);
        if shift == 0 {
            return Some(*self);
        }
        Self::new(
            self.overscaled_z.saturating_sub(shift),
            self.z - shift,
            self.x >> shift,
            self.y >> shift,
            self.wrap,
        )
        .ok()
    }
}

impl std::fmt::Display for OverscaledTileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{} (render@{}, wrap={})",
            self.z, self.x, self.y, self.overscaled_z, self.wrap
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileState {
    Loading,
    Loaded,
    Complete,
    Error,
}

impl TileState {
    pub fn is_renderable(&self) -> bool {
        matches!(self, Self::Loaded | Self::Complete)
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded | Self::Complete)
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FadeState {
    Loaded,
    NeedsFirstPlacement,
    NeedsSecondPlacement,
    CanRemove,
}

impl FadeState {
    pub fn is_fading(&self) -> bool {
        matches!(self, Self::NeedsFirstPlacement | Self::NeedsSecondPlacement)
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

    #[test]
    fn tile_parent_returns_ancestor() {
        let tile = TileId::new(4, 8, 8).unwrap();
        assert_eq!(tile.parent(1).unwrap(), TileId::new(3, 4, 4).unwrap());
        assert_eq!(tile.parent(2).unwrap(), TileId::new(2, 2, 2).unwrap());
        assert_eq!(tile.parent(4).unwrap(), TileId::new(0, 0, 0).unwrap());
        assert_eq!(tile.parent(0).unwrap(), tile);
    }

    #[test]
    fn tile_children_returns_four_subtiles() {
        let tile = TileId::new(2, 1, 1).unwrap();
        let children: Vec<_> = tile.children().collect();
        assert_eq!(children.len(), 4);
        assert!(children.contains(&TileId::new(3, 2, 2).unwrap()));
        assert!(children.contains(&TileId::new(3, 3, 2).unwrap()));
        assert!(children.contains(&TileId::new(3, 2, 3).unwrap()));
        assert!(children.contains(&TileId::new(3, 3, 3).unwrap()));
    }

    #[test]
    fn overscaled_tile_id_from_canonical() {
        let canonical = TileId::new(5, 10, 12).unwrap();
        let overscaled = OverscaledTileId::from_canonical(canonical, 1);
        assert_eq!(overscaled.canonical(), canonical);
        assert_eq!(overscaled.overscaled_z, 5);
        assert_eq!(overscaled.wrap, 1);
    }

    #[test]
    fn overscaled_tile_id_rejects_overscaled_below_source() {
        assert!(OverscaledTileId::new(3, 5, 10, 12, 0).is_err());
    }

    #[test]
    fn overscale_factor_is_power_of_two() {
        let id = TileId::new(3, 2, 2).unwrap().overscale_to(5);
        assert!((id.overscale_factor() - 4.0).abs() < f64::EPSILON);

        let same = TileId::new(3, 2, 2).unwrap().overscale_to(3);
        assert!((same.overscale_factor() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn overscaled_parent_computes_correctly() {
        let overscaled = OverscaledTileId::new(6, 4, 10, 12, 1).unwrap();
        let parent = overscaled.parent(2).unwrap();
        assert_eq!(parent.overscaled_z, 4);
        assert_eq!(parent.z, 2);
        assert_eq!(parent.x, 2);
        assert_eq!(parent.y, 3);
        assert_eq!(parent.wrap, 1);
    }

    #[test]
    fn tile_state_renderable_checks() {
        assert!(!TileState::Loading.is_renderable());
        assert!(TileState::Loaded.is_renderable());
        assert!(TileState::Complete.is_renderable());
        assert!(!TileState::Error.is_renderable());
    }

    #[test]
    fn fade_state_is_fading_checks() {
        assert!(!FadeState::Loaded.is_fading());
        assert!(FadeState::NeedsFirstPlacement.is_fading());
        assert!(FadeState::NeedsSecondPlacement.is_fading());
        assert!(!FadeState::CanRemove.is_fading());
    }
}
