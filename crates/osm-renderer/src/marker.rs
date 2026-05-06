use osm_core::{GeoPoint, TileId, validate_latitude, validate_longitude};

use crate::TileError;

pub type MarkerId = u64;

#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub id: MarkerId,
    pub lat: f64,
    pub lon: f64,
    pub kind: String,
    pub min_zoom: u32,
    pub max_zoom: u32,
}

impl Marker {
    pub fn new(
        id: MarkerId,
        lat: f64,
        lon: f64,
        kind: impl Into<String>,
        min_zoom: u32,
        max_zoom: u32,
    ) -> Result<Self, TileError> {
        Self {
            id,
            lat,
            lon,
            kind: kind.into(),
            min_zoom,
            max_zoom,
        }
        .validate()
    }

    pub fn validate(self) -> Result<Self, TileError> {
        validate_latitude(self.lat)?;
        validate_longitude(self.lon)?;
        validate_marker_zoom(self.min_zoom)?;
        validate_marker_zoom(self.max_zoom)?;

        if self.min_zoom > self.max_zoom {
            return Err(TileError::InvalidMarkerZoomRange {
                min_zoom: self.min_zoom,
                max_zoom: self.max_zoom,
            });
        }

        Ok(self)
    }

    pub fn point(&self) -> GeoPoint {
        GeoPoint {
            lat: self.lat,
            lon: self.lon,
        }
    }

    pub fn is_visible_at_zoom(&self, zoom: u32) -> bool {
        self.min_zoom <= zoom && zoom <= self.max_zoom
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarkerRenderItem {
    Marker(Marker),
    Cluster(MarkerCluster),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkerCluster {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub count: usize,
    pub marker_ids: Vec<MarkerId>,
}

fn validate_marker_zoom(zoom: u32) -> Result<(), TileError> {
    if zoom > TileId::MAX_ZOOM {
        return Err(TileError::InvalidZoom {
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
    fn validates_marker_fields() {
        let marker = Marker::new(1, 53.9023, 27.5619, "cafe", 10, 18).unwrap();

        assert_eq!(marker.id, 1);
        assert!(marker.is_visible_at_zoom(12));
        assert!(!marker.is_visible_at_zoom(9));
    }

    #[test]
    fn rejects_invalid_marker_zoom_range() {
        assert!(matches!(
            Marker::new(1, 53.0, 27.0, "cafe", 18, 10),
            Err(TileError::InvalidMarkerZoomRange { .. })
        ));
        assert!(matches!(
            Marker::new(1, 53.0, 27.0, "cafe", 0, TileId::MAX_ZOOM + 1),
            Err(TileError::InvalidZoom { .. })
        ));
    }
}
