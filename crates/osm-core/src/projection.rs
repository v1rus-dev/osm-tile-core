use crate::{CoreError, GeoPoint, TileId};

pub const TILE_SIZE_PX: f64 = 256.0;
pub const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_78;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapProjection {
    WebMercator,
}

impl MapProjection {
    pub fn project_to_world_pixels(
        self,
        point: GeoPoint,
        zoom: u32,
    ) -> Result<(f64, f64), CoreError> {
        point.validate()?;
        validate_zoom(zoom)?;

        match self {
            Self::WebMercator => Ok(project_web_mercator_to_world_pixels(point, zoom)),
        }
    }
}

fn project_web_mercator_to_world_pixels(point: GeoPoint, zoom: u32) -> (f64, f64) {
    let lat = point.lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let world_size = TILE_SIZE_PX * 2_f64.powi(zoom as i32);
    let x = (point.lon + 180.0) / 360.0 * world_size;
    let lat_rad = lat.to_radians();
    let y = (1.0 - ((lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI)) / 2.0
        * world_size;

    (x, y)
}

fn validate_zoom(zoom: u32) -> Result<(), CoreError> {
    if zoom > TileId::MAX_ZOOM {
        return Err(CoreError::InvalidZoom {
            z: zoom,
            max: TileId::MAX_ZOOM,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_origin_to_middle_of_world_at_zero_zoom() {
        let point = GeoPoint::new(0.0, 0.0).unwrap();
        let (x, y) = MapProjection::WebMercator
            .project_to_world_pixels(point, 0)
            .unwrap();

        assert!((x - 128.0).abs() < f64::EPSILON);
        assert!((y - 128.0).abs() < f64::EPSILON);
    }

    #[test]
    fn rejects_projection_zoom_above_supported_range() {
        let point = GeoPoint::new(0.0, 0.0).unwrap();

        assert!(matches!(
            MapProjection::WebMercator.project_to_world_pixels(point, TileId::MAX_ZOOM + 1),
            Err(CoreError::InvalidZoom { .. })
        ));
    }
}
