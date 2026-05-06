use std::collections::HashMap;

use crate::{
    GeoPoint, Marker, MarkerCluster, MarkerId, MarkerRenderItem, TileError, TileId, Viewport,
};

pub const DEFAULT_CLUSTER_RADIUS_PX: f64 = 64.0;
const TILE_SIZE_PX: f64 = 256.0;
const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_78;

#[derive(Debug, Clone)]
pub struct MapState {
    viewport: Option<Viewport>,
    markers: HashMap<MarkerId, Marker>,
    cluster_radius_px: f64,
}

impl Default for MapState {
    fn default() -> Self {
        Self::new()
    }
}

impl MapState {
    pub fn new() -> Self {
        Self {
            viewport: None,
            markers: HashMap::new(),
            cluster_radius_px: DEFAULT_CLUSTER_RADIUS_PX,
        }
    }

    pub fn viewport(&self) -> Option<Viewport> {
        self.viewport
    }

    pub fn marker_count(&self) -> usize {
        self.markers.len()
    }

    pub fn cluster_radius_px(&self) -> f64 {
        self.cluster_radius_px
    }

    pub fn set_viewport(&mut self, viewport: Viewport) -> Result<(), TileError> {
        self.viewport = Some(viewport.validate()?);
        Ok(())
    }

    pub fn set_cluster_radius_px(&mut self, radius_px: f64) -> Result<(), TileError> {
        validate_cluster_radius(radius_px)?;
        self.cluster_radius_px = radius_px;
        Ok(())
    }

    pub fn replace_markers(
        &mut self,
        markers: impl IntoIterator<Item = Marker>,
    ) -> Result<(), TileError> {
        let mut next_markers = HashMap::new();

        for marker in markers {
            let marker = marker.validate()?;
            next_markers.insert(marker.id, marker);
        }

        self.markers = next_markers;
        Ok(())
    }

    pub fn upsert_markers(
        &mut self,
        markers: impl IntoIterator<Item = Marker>,
    ) -> Result<(), TileError> {
        let markers = markers
            .into_iter()
            .map(Marker::validate)
            .collect::<Result<Vec<_>, _>>()?;

        for marker in markers {
            self.markers.insert(marker.id, marker);
        }

        Ok(())
    }

    pub fn remove_marker(&mut self, id: MarkerId) -> Option<Marker> {
        self.markers.remove(&id)
    }

    pub fn clear_markers(&mut self) {
        self.markers.clear();
    }

    pub fn visible_markers(&self) -> Result<Vec<Marker>, TileError> {
        let viewport = self.viewport.ok_or(TileError::MissingViewport)?;

        Ok(self.visible_markers_for_viewport(viewport))
    }

    pub fn clustered_markers(&self) -> Result<Vec<MarkerRenderItem>, TileError> {
        let viewport = self.viewport.ok_or(TileError::MissingViewport)?;
        let markers = self.visible_markers_for_viewport(viewport);

        cluster_markers(markers, viewport.zoom, self.cluster_radius_px)
    }

    pub fn clustered_all(&self, zoom: u32) -> Result<Vec<MarkerRenderItem>, TileError> {
        validate_zoom(zoom)?;
        let markers = self.markers_visible_at_zoom(zoom);

        cluster_markers(markers, zoom, self.cluster_radius_px)
    }

    pub fn clustered_markers_with_radius(
        &self,
        radius_px: f64,
    ) -> Result<Vec<MarkerRenderItem>, TileError> {
        validate_cluster_radius(radius_px)?;
        let viewport = self.viewport.ok_or(TileError::MissingViewport)?;
        let markers = self.visible_markers_for_viewport(viewport);

        cluster_markers(markers, viewport.zoom, radius_px)
    }

    pub fn clustered_all_with_radius(
        &self,
        zoom: u32,
        radius_px: f64,
    ) -> Result<Vec<MarkerRenderItem>, TileError> {
        validate_zoom(zoom)?;
        validate_cluster_radius(radius_px)?;
        let markers = self.markers_visible_at_zoom(zoom);

        cluster_markers(markers, zoom, radius_px)
    }

    fn visible_markers_for_viewport(&self, viewport: Viewport) -> Vec<Marker> {
        let mut markers = self
            .markers
            .values()
            .filter(|marker| marker.is_visible_at_zoom(viewport.zoom))
            .filter(|marker| viewport.bounds.contains(marker.point()))
            .cloned()
            .collect::<Vec<_>>();

        markers.sort_by_key(|marker| marker.id);
        markers
    }

    fn markers_visible_at_zoom(&self, zoom: u32) -> Vec<Marker> {
        let mut markers = self
            .markers
            .values()
            .filter(|marker| marker.is_visible_at_zoom(zoom))
            .cloned()
            .collect::<Vec<_>>();

        markers.sort_by_key(|marker| marker.id);
        markers
    }
}

fn cluster_markers(
    markers: Vec<Marker>,
    zoom: u32,
    radius_px: f64,
) -> Result<Vec<MarkerRenderItem>, TileError> {
    validate_zoom(zoom)?;
    validate_cluster_radius(radius_px)?;

    let mut cells = HashMap::<(i64, i64), Vec<Marker>>::new();

    for marker in markers {
        let (world_x, world_y) = project_to_world_pixels(marker.point(), zoom);
        let cell = (
            (world_x / radius_px).floor() as i64,
            (world_y / radius_px).floor() as i64,
        );

        cells.entry(cell).or_default().push(marker);
    }

    let mut cells = cells.into_iter().collect::<Vec<_>>();
    cells.sort_by_key(|(_, markers)| markers.iter().map(|marker| marker.id).min().unwrap_or(0));

    let mut items = Vec::with_capacity(cells.len());

    for ((cell_x, cell_y), mut markers) in cells {
        markers.sort_by_key(|marker| marker.id);

        if markers.len() == 1 {
            items.push(MarkerRenderItem::Marker(markers.remove(0)));
            continue;
        }

        let count = markers.len();
        let marker_ids = markers.iter().map(|marker| marker.id).collect::<Vec<_>>();
        let lat = markers.iter().map(|marker| marker.lat).sum::<f64>() / count as f64;
        let lon = markers.iter().map(|marker| marker.lon).sum::<f64>() / count as f64;

        items.push(MarkerRenderItem::Cluster(MarkerCluster {
            id: format!("cluster:{zoom}:{cell_x}:{cell_y}"),
            lat,
            lon,
            count,
            marker_ids,
        }));
    }

    Ok(items)
}

fn project_to_world_pixels(point: GeoPoint, zoom: u32) -> (f64, f64) {
    let lat = point.lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let world_size = TILE_SIZE_PX * 2_f64.powi(zoom as i32);
    let x = (point.lon + 180.0) / 360.0 * world_size;
    let lat_rad = lat.to_radians();
    let y = (1.0 - ((lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI)) / 2.0
        * world_size;

    (x, y)
}

fn validate_zoom(zoom: u32) -> Result<(), TileError> {
    if zoom > TileId::MAX_ZOOM {
        return Err(TileError::InvalidZoom {
            z: zoom,
            max: TileId::MAX_ZOOM,
        });
    }

    Ok(())
}

fn validate_cluster_radius(radius: f64) -> Result<(), TileError> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(TileError::InvalidClusterRadius { radius });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{GeoBounds, Viewport};

    use super::*;

    fn marker(id: MarkerId, lat: f64, lon: f64, min_zoom: u32, max_zoom: u32) -> Marker {
        Marker::new(id, lat, lon, "poi", min_zoom, max_zoom).unwrap()
    }

    fn minsk_viewport(zoom: u32) -> Viewport {
        Viewport::new(GeoBounds::new(53.0, 27.0, 54.5, 28.5).unwrap(), zoom).unwrap()
    }

    #[test]
    fn manages_marker_store_operations() {
        let mut state = MapState::new();

        state
            .replace_markers(vec![
                marker(1, 53.9, 27.5, 0, 18),
                marker(1, 54.0, 27.6, 0, 18),
            ])
            .unwrap();
        assert_eq!(state.marker_count(), 1);

        state
            .upsert_markers(vec![marker(2, 53.91, 27.51, 0, 18)])
            .unwrap();
        assert_eq!(state.marker_count(), 2);

        assert_eq!(state.remove_marker(1).unwrap().id, 1);
        assert_eq!(state.marker_count(), 1);

        state.clear_markers();
        assert_eq!(state.marker_count(), 0);
    }

    #[test]
    fn visible_markers_require_viewport() {
        let state = MapState::new();

        assert!(matches!(
            state.visible_markers(),
            Err(TileError::MissingViewport)
        ));
        assert!(matches!(
            state.clustered_markers(),
            Err(TileError::MissingViewport)
        ));
    }

    #[test]
    fn visible_markers_filter_by_bounds_zoom_and_sort_by_id() {
        let mut state = MapState::new();
        state.set_viewport(minsk_viewport(12)).unwrap();
        state
            .replace_markers(vec![
                marker(3, 53.92, 27.56, 0, 18),
                marker(1, 53.91, 27.55, 0, 18),
                marker(2, 52.0, 27.55, 0, 18),
                marker(4, 53.93, 27.57, 13, 18),
            ])
            .unwrap();

        let ids = state
            .visible_markers()
            .unwrap()
            .into_iter()
            .map(|marker| marker.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn clustered_markers_cluster_visible_markers_only() {
        let mut state = MapState::new();
        state.set_viewport(minsk_viewport(14)).unwrap();
        state.set_cluster_radius_px(128.0).unwrap();
        state
            .replace_markers(vec![
                marker(1, 53.9000, 27.5600, 0, 18),
                marker(2, 53.9001, 27.5601, 0, 18),
                marker(3, 53.0, 28.4, 0, 18),
                marker(4, 52.0, 27.5, 0, 18),
            ])
            .unwrap();

        let items = state.clustered_markers().unwrap();

        assert!(items.iter().any(|item| matches!(
            item,
            MarkerRenderItem::Cluster(cluster)
                if cluster.count == 2 && cluster.marker_ids == vec![1, 2]
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            MarkerRenderItem::Marker(marker) if marker.id == 3
        )));
        assert!(!items.iter().any(|item| matches!(
            item,
            MarkerRenderItem::Marker(marker) if marker.id == 4
        )));
    }

    #[test]
    fn clustered_all_ignores_viewport_but_respects_zoom() {
        let mut state = MapState::new();
        state.set_viewport(minsk_viewport(14)).unwrap();
        state.set_cluster_radius_px(128.0).unwrap();
        state
            .replace_markers(vec![
                marker(1, 53.9000, 27.5600, 0, 18),
                marker(2, 53.9001, 27.5601, 0, 18),
                marker(3, 52.0, 27.5, 0, 18),
                marker(4, 53.9, 27.56, 15, 18),
            ])
            .unwrap();

        let items = state.clustered_all(14).unwrap();
        let visible_ids = items
            .iter()
            .flat_map(|item| match item {
                MarkerRenderItem::Marker(marker) => vec![marker.id],
                MarkerRenderItem::Cluster(cluster) => cluster.marker_ids.clone(),
            })
            .collect::<Vec<_>>();

        assert!(visible_ids.contains(&3));
        assert!(!visible_ids.contains(&4));
    }

    #[test]
    fn rejects_invalid_cluster_radius() {
        let mut state = MapState::new();

        assert!(matches!(
            state.set_cluster_radius_px(0.0),
            Err(TileError::InvalidClusterRadius { .. })
        ));
    }
}
