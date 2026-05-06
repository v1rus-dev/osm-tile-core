use crate::{TileError, TileId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

impl GeoPoint {
    pub fn new(lat: f64, lon: f64) -> Result<Self, TileError> {
        Self { lat, lon }.validate()
    }

    pub fn validate(self) -> Result<Self, TileError> {
        validate_latitude(self.lat)?;
        validate_longitude(self.lon)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoBounds {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl GeoBounds {
    pub fn new(south: f64, west: f64, north: f64, east: f64) -> Result<Self, TileError> {
        Self {
            south,
            west,
            north,
            east,
        }
        .validate()
    }

    pub fn validate(self) -> Result<Self, TileError> {
        validate_latitude(self.south)?;
        validate_latitude(self.north)?;
        validate_longitude(self.west)?;
        validate_longitude(self.east)?;

        if self.south > self.north {
            return Err(TileError::InvalidBounds {
                south: self.south,
                west: self.west,
                north: self.north,
                east: self.east,
            });
        }

        Ok(self)
    }

    pub fn contains(&self, point: GeoPoint) -> bool {
        if point.lat < self.south || point.lat > self.north {
            return false;
        }

        if self.crosses_antimeridian() {
            point.lon >= self.west || point.lon <= self.east
        } else {
            point.lon >= self.west && point.lon <= self.east
        }
    }

    pub fn crosses_antimeridian(&self) -> bool {
        self.west > self.east
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub bounds: GeoBounds,
    pub zoom: u32,
}

impl Viewport {
    pub fn new(bounds: GeoBounds, zoom: u32) -> Result<Self, TileError> {
        Self { bounds, zoom }.validate()
    }

    pub fn validate(self) -> Result<Self, TileError> {
        self.bounds.validate()?;

        if self.zoom > TileId::MAX_ZOOM {
            return Err(TileError::InvalidZoom {
                z: self.zoom,
                max: TileId::MAX_ZOOM,
            });
        }

        Ok(self)
    }
}

pub fn validate_latitude(lat: f64) -> Result<(), TileError> {
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        return Err(TileError::InvalidLatitude { lat });
    }

    Ok(())
}

pub fn validate_longitude(lon: f64) -> Result<(), TileError> {
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        return Err(TileError::InvalidLongitude { lon });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_geo_point() {
        assert_eq!(
            GeoPoint::new(53.9023, 27.5619).unwrap(),
            GeoPoint {
                lat: 53.9023,
                lon: 27.5619
            }
        );
        assert!(matches!(
            GeoPoint::new(91.0, 27.0),
            Err(TileError::InvalidLatitude { .. })
        ));
        assert!(matches!(
            GeoPoint::new(53.0, 181.0),
            Err(TileError::InvalidLongitude { .. })
        ));
    }

    #[test]
    fn validates_geo_bounds_and_viewport() {
        let bounds = GeoBounds::new(50.0, 20.0, 55.0, 30.0).unwrap();
        assert_eq!(Viewport::new(bounds, 12).unwrap().zoom, 12);

        assert!(matches!(
            GeoBounds::new(55.0, 20.0, 50.0, 30.0),
            Err(TileError::InvalidBounds { .. })
        ));
        assert!(matches!(
            Viewport::new(bounds, TileId::MAX_ZOOM + 1),
            Err(TileError::InvalidZoom { .. })
        ));
    }

    #[test]
    fn contains_point_inside_regular_bounds() {
        let bounds = GeoBounds::new(50.0, 20.0, 55.0, 30.0).unwrap();

        assert!(bounds.contains(GeoPoint::new(53.0, 25.0).unwrap()));
        assert!(!bounds.contains(GeoPoint::new(53.0, 31.0).unwrap()));
    }

    #[test]
    fn contains_point_inside_antimeridian_bounds() {
        let bounds = GeoBounds::new(-10.0, 170.0, 10.0, -170.0).unwrap();

        assert!(bounds.crosses_antimeridian());
        assert!(bounds.contains(GeoPoint::new(0.0, 175.0).unwrap()));
        assert!(bounds.contains(GeoPoint::new(0.0, -175.0).unwrap()));
        assert!(!bounds.contains(GeoPoint::new(0.0, 0.0).unwrap()));
    }
}
