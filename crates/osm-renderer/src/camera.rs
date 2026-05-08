use osm_core::{
    GeoPoint, MAX_ZOOM, MIN_ZOOM, MapProjection, OverscaledTileId, TILE_SIZE_PX, TileId,
    tile_count_per_axis,
};

use crate::RenderError;

const CAMERA_ZOOM_OVERSHOOT: f64 = 1.0;

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
        let min_zoom = MIN_ZOOM as f64 - CAMERA_ZOOM_OVERSHOOT;
        let max_zoom = MAX_ZOOM as f64 + CAMERA_ZOOM_OVERSHOOT;
        if !self.zoom.is_finite() || self.zoom < min_zoom || self.zoom > max_zoom {
            return Err(RenderError::InvalidCameraZoom {
                zoom: self.zoom,
                max: MAX_ZOOM,
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

    pub fn lower_tile_zoom(self) -> u32 {
        self.zoom.floor().clamp(MIN_ZOOM as f64, MAX_ZOOM as f64) as u32
    }

    pub fn tile_zoom(self) -> u32 {
        self.lower_tile_zoom()
    }

    pub fn zoom_fraction(self) -> f64 {
        (self.zoom - self.lower_tile_zoom() as f64).clamp(0.0, 1.0)
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleTile {
    pub id: TileId,
    pub overscaled_id: OverscaledTileId,
    pub screen_x_px: f32,
    pub screen_y_px: f32,
    pub size_px: f32,
}

pub fn visible_tiles(
    camera: MapCamera,
    viewport: RenderViewport,
) -> Result<Vec<VisibleTile>, RenderError> {
    let camera = camera.validate()?;
    let viewport = viewport.validate()?;
    let zoom = camera.lower_tile_zoom();
    let scale = 2_f64.powf(camera.zoom - zoom as f64);
    let center = GeoPoint::new(camera.center_lat, camera.center_lon)?;
    let (center_x, center_y) = MapProjection::WebMercator.project_to_world_pixels(center, zoom)?;
    let half_width_world_px = viewport.width_px as f64 / (2.0 * scale);
    let half_height_world_px = viewport.height_px as f64 / (2.0 * scale);

    let min_tile_x = ((center_x - half_width_world_px) / TILE_SIZE_PX).floor() as i64;
    let max_tile_x = ((center_x + half_width_world_px) / TILE_SIZE_PX).floor() as i64;
    let min_tile_y = ((center_y - half_height_world_px) / TILE_SIZE_PX).floor() as i64;
    let max_tile_y = ((center_y + half_height_world_px) / TILE_SIZE_PX).floor() as i64;
    let limit = i64::from(tile_count_per_axis(zoom)?);
    let mut tiles = Vec::new();
    let half_width_screen_px = viewport.width_px as f64 / 2.0;
    let half_height_screen_px = viewport.height_px as f64 / 2.0;
    let tile_size_screen_px = (TILE_SIZE_PX * scale) as f32;

    for y in min_tile_y..=max_tile_y {
        if y < 0 || y >= limit {
            continue;
        }

        for x in min_tile_x..=max_tile_x {
            let wrapped_x = x.rem_euclid(limit);
            let wrap = (x / limit) as i32;
            let id = TileId::new(zoom, wrapped_x as u32, y as u32)?;
            let overscaled_id = OverscaledTileId::from_canonical(id, wrap);
            let tile_world_left_px = x as f64 * TILE_SIZE_PX;
            let tile_world_top_px = y as f64 * TILE_SIZE_PX;
            let screen_x_px =
                ((tile_world_left_px - center_x) * scale + half_width_screen_px) as f32;
            let screen_y_px =
                ((tile_world_top_px - center_y) * scale + half_height_screen_px) as f32;
            tiles.push(VisibleTile {
                id,
                overscaled_id,
                screen_x_px,
                screen_y_px,
                size_px: tile_size_screen_px,
            });
        }
    }

    tiles.sort_by(|left, right| {
        left.screen_y_px
            .total_cmp(&right.screen_y_px)
            .then_with(|| left.screen_x_px.total_cmp(&right.screen_x_px))
    });
    Ok(tiles)
}

pub fn position_tile(
    camera: MapCamera,
    viewport: RenderViewport,
    tile_id: TileId,
) -> Result<VisibleTile, RenderError> {
    let camera = camera.validate()?;
    let viewport = viewport.validate()?;
    let tile_id = tile_id.validate()?;
    let scale = 2_f64.powf(camera.zoom - tile_id.z as f64);
    let center = GeoPoint::new(camera.center_lat, camera.center_lon)?;
    let (center_x, center_y) =
        MapProjection::WebMercator.project_to_world_pixels(center, tile_id.z)?;
    let limit = i64::from(tile_count_per_axis(tile_id.z)?);
    let center_tile_x = center_x / TILE_SIZE_PX;
    let base_x = tile_id.x as i64;
    let unwrapped_x = [base_x - limit, base_x, base_x + limit]
        .into_iter()
        .min_by(|left, right| {
            ((*left as f64) - center_tile_x)
                .abs()
                .total_cmp(&((*right as f64) - center_tile_x).abs())
        })
        .unwrap_or(base_x);
    let wrap = ((unwrapped_x - base_x) / limit) as i32;
    let overscaled_id = OverscaledTileId::from_canonical(tile_id, wrap);
    let tile_world_left_px = unwrapped_x as f64 * TILE_SIZE_PX;
    let tile_world_top_px = tile_id.y as f64 * TILE_SIZE_PX;
    let half_width_screen_px = viewport.width_px as f64 / 2.0;
    let half_height_screen_px = viewport.height_px as f64 / 2.0;

    Ok(VisibleTile {
        id: tile_id,
        overscaled_id,
        screen_x_px: ((tile_world_left_px - center_x) * scale + half_width_screen_px) as f32,
        screen_y_px: ((tile_world_top_px - center_y) * scale + half_height_screen_px) as f32,
        size_px: (TILE_SIZE_PX * scale) as f32,
    })
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

        assert_eq!(camera.lower_tile_zoom(), 3);
        assert!(tiles.iter().all(|tile| tile.id.z == 3));
        assert!(!tiles.is_empty());
    }

    #[test]
    fn transient_zoom_below_min_uses_min_tile_zoom_with_smaller_scale() {
        let camera = MapCamera::new(0.0, 0.0, -0.25).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport).unwrap();

        assert_eq!(camera.lower_tile_zoom(), MIN_ZOOM);
        assert!(tiles.iter().all(|tile| tile.id.z == MIN_ZOOM));
        assert!(tiles.iter().all(|tile| tile.size_px < 256.0));
    }

    #[test]
    fn transient_zoom_above_max_uses_max_tile_zoom_with_larger_scale() {
        let camera = MapCamera::new(0.0, 0.0, MAX_ZOOM as f64 + 0.25).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport).unwrap();

        assert_eq!(camera.lower_tile_zoom(), MAX_ZOOM);
        assert!(tiles.iter().all(|tile| tile.id.z == MAX_ZOOM));
        assert!(tiles.iter().all(|tile| tile.size_px > 256.0));
    }

    #[test]
    fn position_tile_projects_any_zoom_level() {
        let camera = MapCamera::new(0.0, 0.0, 3.5).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tile = position_tile(camera, viewport, TileId::new(2, 2, 2).unwrap()).unwrap();

        assert_eq!(tile.id, TileId::new(2, 2, 2).unwrap());
        assert!(tile.size_px > 256.0);
    }

    #[test]
    fn rejects_invalid_viewport_and_camera() {
        assert!(matches!(
            RenderViewport::new(0, 256, 1.0),
            Err(RenderError::InvalidRenderViewport { .. })
        ));
        assert!(matches!(
            MapCamera::new(0.0, 0.0, TileId::MAX_ZOOM as f64 + 2.0),
            Err(RenderError::InvalidCameraZoom { .. })
        ));
        assert!(matches!(
            MapCamera::new(0.0, 0.0, MIN_ZOOM as f64 - 2.0),
            Err(RenderError::InvalidCameraZoom { .. })
        ));
    }

    #[test]
    fn visible_tiles_include_overscaled_ids() {
        let camera = MapCamera::new(0.0, 0.0, 2.0).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport).unwrap();

        for tile in &tiles {
            assert_eq!(tile.overscaled_id.canonical(), tile.id);
            assert_eq!(tile.overscaled_id.overscaled_z, tile.id.z);
        }
    }

    #[test]
    fn visible_tiles_wrap_produces_correct_wrap_count() {
        let camera = MapCamera::new(0.0, 179.9, 2.0).unwrap();
        let viewport = RenderViewport::new(1024, 256, 1.0).unwrap();
        let tiles = visible_tiles(camera, viewport).unwrap();

        let wraps: Vec<_> = tiles.iter().map(|t| t.overscaled_id.wrap).collect();
        assert!(wraps.iter().any(|&w| w != 0));
    }

    #[test]
    fn position_tile_includes_overscaled_id_with_wrap() {
        let camera = MapCamera::new(0.0, 0.0, 3.5).unwrap();
        let viewport = RenderViewport::new(256, 256, 1.0).unwrap();
        let tile = position_tile(camera, viewport, TileId::new(2, 2, 2).unwrap()).unwrap();

        assert_eq!(tile.overscaled_id.canonical(), tile.id);
        assert!(tile.size_px > 256.0);
    }

    #[test]
    fn camera_zoom_fraction_returns_correct_value() {
        let camera = MapCamera::new(0.0, 0.0, 3.75).unwrap();
        assert!((camera.zoom_fraction() - 0.75).abs() < f64::EPSILON);

        let whole = MapCamera::new(0.0, 0.0, 5.0).unwrap();
        assert!((whole.zoom_fraction() - 0.0).abs() < f64::EPSILON);
    }
}
