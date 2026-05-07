use crate::{CoreError, TileId, geo::validate_latitude};

pub const MIN_ZOOM: u32 = 0;
pub const MAX_ZOOM: u32 = TileId::MAX_ZOOM;
pub const OSM_STANDARD_MAX_ZOOM: u32 = 19;
pub const TILE_SIZE_PX: f64 = 256.0;
pub const EQUATOR_M_PER_PIXEL_Z0: f64 = 156_543.033_928_040_97;

pub fn validate_zoom_level(zoom: u32) -> Result<(), CoreError> {
    if zoom > MAX_ZOOM {
        return Err(CoreError::InvalidZoom {
            z: zoom,
            max: MAX_ZOOM,
        });
    }

    Ok(())
}

pub fn validate_continuous_zoom(zoom: f64) -> Result<(), CoreError> {
    if !zoom.is_finite() || zoom < MIN_ZOOM as f64 || zoom > MAX_ZOOM as f64 {
        return Err(CoreError::InvalidContinuousZoom {
            zoom,
            max: MAX_ZOOM,
        });
    }

    Ok(())
}

pub fn lower_tile_zoom(zoom: f64) -> Result<u32, CoreError> {
    validate_continuous_zoom(zoom)?;
    Ok(zoom.floor() as u32)
}

pub fn nearest_zoom_level(zoom: f64) -> Result<u32, CoreError> {
    validate_continuous_zoom(zoom)?;
    Ok((zoom + 0.5).floor().clamp(MIN_ZOOM as f64, MAX_ZOOM as f64) as u32)
}

pub fn zoom_fraction(zoom: f64) -> Result<f64, CoreError> {
    validate_continuous_zoom(zoom)?;
    Ok(zoom.fract())
}

pub fn world_size_px(zoom: f64) -> Result<f64, CoreError> {
    validate_continuous_zoom(zoom)?;
    Ok(TILE_SIZE_PX * 2_f64.powf(zoom))
}

pub fn world_size_px_at_level(zoom: u32) -> Result<f64, CoreError> {
    validate_zoom_level(zoom)?;
    Ok(TILE_SIZE_PX * 2_f64.powi(zoom as i32))
}

pub fn tile_count_per_axis(zoom: u32) -> Result<u32, CoreError> {
    validate_zoom_level(zoom)?;
    Ok(1_u32 << zoom)
}

pub fn tile_count(zoom: u32) -> Result<u64, CoreError> {
    let axis = u64::from(tile_count_per_axis(zoom)?);
    Ok(axis * axis)
}

pub fn meters_per_pixel(latitude: f64, zoom: f64) -> Result<f64, CoreError> {
    validate_latitude(latitude)?;
    validate_continuous_zoom(zoom)?;

    Ok(EQUATOR_M_PER_PIXEL_Z0 * latitude.to_radians().cos().abs() / 2_f64.powf(zoom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_integer_and_continuous_zoom_bounds() {
        validate_zoom_level(MIN_ZOOM).unwrap();
        validate_zoom_level(MAX_ZOOM).unwrap();
        validate_continuous_zoom(0.0).unwrap();
        validate_continuous_zoom(MAX_ZOOM as f64).unwrap();

        assert!(matches!(
            validate_zoom_level(MAX_ZOOM + 1),
            Err(CoreError::InvalidZoom { .. })
        ));
        assert!(matches!(
            validate_continuous_zoom(-0.1),
            Err(CoreError::InvalidContinuousZoom { .. })
        ));
        assert!(matches!(
            validate_continuous_zoom(f64::NAN),
            Err(CoreError::InvalidContinuousZoom { .. })
        ));
    }

    #[test]
    fn derives_lower_nearest_and_fractional_zoom_parts() {
        assert_eq!(lower_tile_zoom(3.0).unwrap(), 3);
        assert_eq!(lower_tile_zoom(3.75).unwrap(), 3);
        assert_eq!(nearest_zoom_level(3.49).unwrap(), 3);
        assert_eq!(nearest_zoom_level(3.5).unwrap(), 4);
        assert_eq!(nearest_zoom_level(MAX_ZOOM as f64).unwrap(), MAX_ZOOM);
        assert!((zoom_fraction(3.75).unwrap() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn calculates_world_and_tile_counts() {
        assert_eq!(world_size_px_at_level(0).unwrap(), TILE_SIZE_PX);
        assert_eq!(world_size_px_at_level(2).unwrap(), TILE_SIZE_PX * 4.0);
        assert_eq!(tile_count_per_axis(0).unwrap(), 1);
        assert_eq!(tile_count_per_axis(3).unwrap(), 8);
        assert_eq!(tile_count(3).unwrap(), 64);
    }

    #[test]
    fn calculates_meters_per_pixel_by_latitude() {
        let equator_z0 = meters_per_pixel(0.0, 0.0).unwrap();
        let equator_z1 = meters_per_pixel(0.0, 1.0).unwrap();
        let latitude_60_z0 = meters_per_pixel(60.0, 0.0).unwrap();

        assert!((equator_z0 - EQUATOR_M_PER_PIXEL_Z0).abs() < 0.000_001);
        assert!((equator_z1 - EQUATOR_M_PER_PIXEL_Z0 / 2.0).abs() < 0.000_001);
        assert!((latitude_60_z0 - EQUATOR_M_PER_PIXEL_Z0 / 2.0).abs() < 0.000_001);
    }
}
