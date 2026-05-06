use osm_core::{GeoPoint, MapProjection, TileId};

use crate::RenderError;

pub const TILE_SIZE_PX: f64 = 256.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapCamera {
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: f64,
    pub bearing: f64,
    pub pitch: f64,
}

impl MapCamera {
    pub fn new(center_lat: f64, center_lon: f64, zoom: f64) -> Result<Self, RenderError> {
        Self {
            center_lat,
            center_lon,
            zoom,
            bearing: 0.0,
            pitch: 0.0,
        }
        .validate()
    }

    pub fn validate(self) -> Result<Self, RenderError> {
        if !self.zoom.is_finite() || self.zoom < 0.0 || self.zoom > TileId::MAX_ZOOM as f64 {
            return Err(RenderError::InvalidCameraZoom {
                zoom: self.zoom,
                max: TileId::MAX_ZOOM,
            });
        }
        if !self.bearing.is_finite() {
            return Err(RenderError::InvalidCameraAngle {
                name: "bearing",
                value: self.bearing,
            });
        }
        if !self.pitch.is_finite() || !(0.0..=85.0).contains(&self.pitch) {
            return Err(RenderError::InvalidCameraAngle {
                name: "pitch",
                value: self.pitch,
            });
        }

        GeoPoint::new(self.center_lat, self.center_lon)?;
        Ok(self)
    }

    pub fn tile_zoom(self) -> u32 {
        self.zoom.floor().clamp(0.0, TileId::MAX_ZOOM as f64) as u32
    }
}

impl Default for MapCamera {
    fn default() -> Self {
        Self {
            center_lat: 0.0,
            center_lon: 0.0,
            zoom: 0.0,
            bearing: 0.0,
            pitch: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderViewport {
    pub width_px: u32,
    pub height_px: u32,
    pub density: f64,
}

impl RenderViewport {
    pub fn new(width_px: u32, height_px: u32, density: f64) -> Result<Self, RenderError> {
        Self {
            width_px,
            height_px,
            density,
        }
        .validate()
    }

    pub fn validate(self) -> Result<Self, RenderError> {
        if self.width_px == 0
            || self.height_px == 0
            || !self.density.is_finite()
            || self.density <= 0.0
        {
            return Err(RenderError::InvalidRenderViewport {
                width_px: self.width_px,
                height_px: self.height_px,
                density: self.density,
            });
        }

        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisibleTile {
    pub id: TileId,
}

pub fn visible_tiles(
    camera: MapCamera,
    viewport: RenderViewport,
) -> Result<Vec<VisibleTile>, RenderError> {
    let camera = camera.validate()?;
    let viewport = viewport.validate()?;
    let zoom = camera.tile_zoom();
    let scale = 2_f64.powf(camera.zoom - zoom as f64);
    let center = GeoPoint::new(camera.center_lat, camera.center_lon)?;
    let (center_x, center_y) = MapProjection::WebMercator.project_to_world_pixels(center, zoom)?;
    let half_width_world_px = viewport.width_px as f64 / (2.0 * scale);
    let half_height_world_px = viewport.height_px as f64 / (2.0 * scale);

    let min_tile_x = ((center_x - half_width_world_px) / TILE_SIZE_PX).floor() as i64;
    let max_tile_x = ((center_x + half_width_world_px) / TILE_SIZE_PX).floor() as i64;
    let min_tile_y = ((center_y - half_height_world_px) / TILE_SIZE_PX).floor() as i64;
    let max_tile_y = ((center_y + half_height_world_px) / TILE_SIZE_PX).floor() as i64;
    let limit = 1_i64 << zoom;
    let mut tiles = Vec::new();

    for y in min_tile_y..=max_tile_y {
        if y < 0 || y >= limit {
            continue;
        }

        for x in min_tile_x..=max_tile_x {
            let wrapped_x = x.rem_euclid(limit);
            let id = TileId::new(zoom, wrapped_x as u32, y as u32)?;
            if !tiles.iter().any(|tile: &VisibleTile| tile.id == id) {
                tiles.push(VisibleTile { id });
            }
        }
    }

    tiles.sort_by_key(|tile| (tile.id.z, tile.id.y, tile.id.x));
    Ok(tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_tiles_selects_tiles_around_camera() {
        let camera = MapCamera::new(0.0, 0.0, 1.0).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport)
            .unwrap()
            .into_iter()
            .map(|tile| tile.id)
            .collect::<Vec<_>>();

        assert_eq!(tiles.len(), 4);
        assert!(tiles.contains(&TileId::new(1, 0, 0).unwrap()));
        assert!(tiles.contains(&TileId::new(1, 1, 0).unwrap()));
        assert!(tiles.contains(&TileId::new(1, 0, 1).unwrap()));
        assert!(tiles.contains(&TileId::new(1, 1, 1).unwrap()));
    }

    #[test]
    fn visible_tiles_wraps_x_at_antimeridian() {
        let camera = MapCamera::new(0.0, 179.9, 2.0).unwrap();
        let viewport = RenderViewport::new(512, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport).unwrap();

        assert!(tiles.iter().any(|tile| tile.id.x == 0));
        assert!(tiles.iter().any(|tile| tile.id.x == 3));
    }

    #[test]
    fn fractional_zoom_uses_lower_tile_zoom_and_scaled_viewport() {
        let camera = MapCamera::new(0.0, 0.0, 3.5).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport).unwrap();

        assert!(tiles.iter().all(|tile| tile.id.z == 3));
        assert!(!tiles.is_empty());
    }

    #[test]
    fn rejects_invalid_viewport_and_camera() {
        assert!(matches!(
            RenderViewport::new(0, 256, 1.0),
            Err(RenderError::InvalidRenderViewport { .. })
        ));
        assert!(matches!(
            MapCamera::new(0.0, 0.0, TileId::MAX_ZOOM as f64 + 1.0),
            Err(RenderError::InvalidCameraZoom { .. })
        ));
    }
}
