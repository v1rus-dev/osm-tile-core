use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use thiserror::Error;

use osm_core::{CoreError, GeoBounds, TileId, Viewport};
use osm_loader::{CachedTileSource, FileTileCache, HttpTileSource, TileLoadError, TileSource};
use osm_renderer::{MapState, Marker, MarkerCluster, MarkerId, MarkerRenderItem, RenderError};

/// The currently visible map rectangle and zoom level.
///
/// Pass this from Android or iOS whenever the map camera changes enough that
/// visible markers or clusters should be recalculated.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MobileViewport {
    /// Southern latitude of the visible map area.
    pub south: f64,
    /// Western longitude of the visible map area.
    ///
    /// If `west > east`, the bounds cross the antimeridian.
    pub west: f64,
    /// Northern latitude of the visible map area.
    pub north: f64,
    /// Eastern longitude of the visible map area.
    pub east: f64,
    /// Integer map zoom. The supported range is `0..=30`.
    pub zoom: i64,
}

/// A point marker known to the Rust core.
///
/// The UI still decides how the marker looks. Rust stores coordinates, kind,
/// and zoom visibility so it can return only the markers relevant to the
/// current viewport.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MobileMarker {
    /// Stable marker id. Use the same id when updating a marker with `upsertMarkers`.
    pub id: i64,
    /// Marker latitude in `-90..=90`.
    pub lat: f64,
    /// Marker longitude in `-180..=180`.
    pub lon: f64,
    /// Domain-specific marker category, for example `cafe`, `hotel`, or `vehicle`.
    pub kind: String,
    /// First zoom level where this marker should be visible.
    pub min_zoom: i64,
    /// Last zoom level where this marker should be visible.
    pub max_zoom: i64,
}

/// A cluster returned by Rust for rendering on the current map.
///
/// The cluster coordinate is the average coordinate of all markers in the
/// cluster. The mobile UI decides which icon, text, and interaction to use.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MobileMarkerCluster {
    /// Stable cluster id for the current zoom/grid cell.
    pub id: String,
    /// Cluster latitude.
    pub lat: f64,
    /// Cluster longitude.
    pub lon: f64,
    /// Number of markers represented by this cluster.
    pub count: i64,
    /// Marker ids included in the cluster.
    pub marker_ids: Vec<i64>,
}

/// Identifies which optional payload is present in `MobileMarkerRenderItem`.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum MobileRenderItemType {
    /// `marker` is present and `cluster` is empty.
    Marker,
    /// `cluster` is present and `marker` is empty.
    Cluster,
}

/// A render item returned by `clusteredMarkers` or `clusteredAll`.
///
/// UniFFI records are simpler for Kotlin and Swift when the shape is stable, so
/// this type uses `item_type` plus optional `marker`/`cluster` payloads.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MobileMarkerRenderItem {
    /// Tells the UI which payload field to read.
    pub item_type: MobileRenderItemType,
    /// Present when `item_type == Marker`.
    pub marker: Option<MobileMarker>,
    /// Present when `item_type == Cluster`.
    pub cluster: Option<MobileMarkerCluster>,
}

/// Error type exposed to Kotlin and Swift callers.
///
/// The `details` field is safe to display in logs and developer diagnostics.
#[derive(Debug, Error, uniffi::Error)]
pub enum OsmTileEngineError {
    /// The caller passed invalid coordinates, zoom, ids, or URL template.
    #[error("invalid input: {details}")]
    InvalidInput { details: String },

    /// The tile cache could not read or write local files.
    #[error("cache error: {details}")]
    Cache { details: String },

    /// The tile server request failed or returned a non-success HTTP status.
    #[error("network error: {details}")]
    Network { details: String },

    /// The map state is not ready for the requested operation.
    #[error("state error: {details}")]
    State { details: String },
}

/// Main entry point for Android and iOS apps.
///
/// Create one instance per map screen or per application. The object keeps a
/// cache-first tile source plus an in-memory marker store. `loadTile` is
/// synchronous in v1 and blocks on an internal Tokio runtime, so call it from a
/// background thread, coroutine dispatcher, or Swift task instead of the UI
/// thread.
#[derive(uniffi::Object)]
pub struct OsmTileEngine {
    source: CachedTileSource<HttpTileSource>,
    map_state: Mutex<MapState>,
    runtime: tokio::runtime::Runtime,
}

#[uniffi::export]
impl OsmTileEngine {
    /// Creates a tile engine with cache-first behavior.
    ///
    /// `tile_url_template` must contain `{z}`, `{x}`, and `{y}` placeholders,
    /// for example `http://10.0.2.2:8080/tile/{z}/{x}/{y}.png` on the Android
    /// emulator. `cache_dir` should be inside the app-private storage directory.
    #[uniffi::constructor]
    pub fn new(
        tile_url_template: String,
        cache_dir: String,
    ) -> Result<Arc<Self>, OsmTileEngineError> {
        let http_source = HttpTileSource::new(tile_url_template)?;
        let cache = FileTileCache::new(PathBuf::from(cache_dir));
        let source = CachedTileSource::new(http_source, cache);
        let runtime =
            tokio::runtime::Runtime::new().map_err(|error| OsmTileEngineError::State {
                details: format!("failed to create async runtime: {error}"),
            })?;

        Ok(Arc::new(Self {
            source,
            map_state: Mutex::new(MapState::new()),
            runtime,
        }))
    }

    /// Loads a map tile as raw image bytes.
    ///
    /// The method first checks the local offline cache. If the tile is missing,
    /// it downloads the tile from the configured tile server, saves it to cache,
    /// and returns the image bytes. Do not call this from the UI thread.
    pub fn load_tile(&self, z: i64, x: i64, y: i64) -> Result<Vec<u8>, OsmTileEngineError> {
        let tile_id = TileId::new(
            i64_to_u32(z, "z")?,
            i64_to_u32(x, "x")?,
            i64_to_u32(y, "y")?,
        )?;

        self.runtime
            .block_on(self.source.load_tile(tile_id))
            .map_err(OsmTileEngineError::from)
    }

    /// Updates the current map viewport used by `visibleMarkers` and `clusteredMarkers`.
    pub fn set_viewport(&self, viewport: MobileViewport) -> Result<(), OsmTileEngineError> {
        let viewport = viewport.try_into_viewport()?;
        let mut state = self.lock_map_state()?;
        state.set_viewport(viewport)?;
        Ok(())
    }

    /// Replaces all markers currently loaded into Rust.
    ///
    /// Use this when the app has a complete offline marker set or wants to reset
    /// the working set after a new server query.
    pub fn replace_markers(&self, markers: Vec<MobileMarker>) -> Result<(), OsmTileEngineError> {
        let markers = markers
            .into_iter()
            .map(MobileMarker::try_into_marker)
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.lock_map_state()?;
        state.replace_markers(markers)?;
        Ok(())
    }

    /// Adds or updates markers by id.
    ///
    /// This is useful when the app loads markers page-by-page or bbox-by-bbox
    /// from a backend. If the same id appears more than once, the last marker wins.
    pub fn upsert_markers(&self, markers: Vec<MobileMarker>) -> Result<(), OsmTileEngineError> {
        let markers = markers
            .into_iter()
            .map(MobileMarker::try_into_marker)
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.lock_map_state()?;
        state.upsert_markers(markers)?;
        Ok(())
    }

    /// Removes one marker from the Rust marker store.
    pub fn remove_marker(&self, id: i64) -> Result<(), OsmTileEngineError> {
        let id = i64_to_marker_id(id)?;
        let mut state = self.lock_map_state()?;
        state.remove_marker(id);
        Ok(())
    }

    /// Clears all markers from the Rust marker store.
    pub fn clear_markers(&self) -> Result<(), OsmTileEngineError> {
        let mut state = self.lock_map_state()?;
        state.clear_markers();
        Ok(())
    }

    /// Returns markers visible in the current viewport.
    ///
    /// The result is filtered by bbox and marker zoom range, then sorted by id
    /// for stable rendering.
    pub fn visible_markers(&self) -> Result<Vec<MobileMarker>, OsmTileEngineError> {
        let state = self.lock_map_state()?;
        state
            .visible_markers()?
            .into_iter()
            .map(MobileMarker::try_from_marker)
            .collect()
    }

    /// Returns marker and cluster render items visible in the current viewport.
    ///
    /// Rust performs simple grid-based clustering in WebMercator pixel space.
    /// The UI should render markers and clusters using its own icons and views.
    pub fn clustered_markers(&self) -> Result<Vec<MobileMarkerRenderItem>, OsmTileEngineError> {
        let state = self.lock_map_state()?;
        state
            .clustered_markers()?
            .into_iter()
            .map(MobileMarkerRenderItem::try_from_render_item)
            .collect()
    }

    /// Clusters every marker loaded into Rust for the given zoom.
    ///
    /// This ignores the current viewport but still respects each marker's zoom
    /// visibility range.
    pub fn clustered_all(
        &self,
        zoom: i64,
    ) -> Result<Vec<MobileMarkerRenderItem>, OsmTileEngineError> {
        let zoom = i64_to_u32(zoom, "zoom")?;
        let state = self.lock_map_state()?;
        state
            .clustered_all(zoom)?
            .into_iter()
            .map(MobileMarkerRenderItem::try_from_render_item)
            .collect()
    }
}

impl OsmTileEngine {
    fn lock_map_state(&self) -> Result<std::sync::MutexGuard<'_, MapState>, OsmTileEngineError> {
        self.map_state
            .lock()
            .map_err(|_| OsmTileEngineError::State {
                details: "marker state lock is poisoned".to_owned(),
            })
    }

    pub(crate) fn shared_source(&self) -> CachedTileSource<HttpTileSource> {
        self.source.clone()
    }

    pub(crate) fn tile_url_template(&self) -> String {
        self.source.source().url_template().to_owned()
    }
}

impl MobileViewport {
    fn try_into_viewport(self) -> Result<Viewport, OsmTileEngineError> {
        let bounds = GeoBounds::new(self.south, self.west, self.north, self.east)?;
        Ok(Viewport::new(bounds, i64_to_u32(self.zoom, "zoom")?)?)
    }
}

impl MobileMarker {
    fn try_into_marker(self) -> Result<Marker, OsmTileEngineError> {
        Ok(Marker::new(
            i64_to_marker_id(self.id)?,
            self.lat,
            self.lon,
            self.kind,
            i64_to_u32(self.min_zoom, "min_zoom")?,
            i64_to_u32(self.max_zoom, "max_zoom")?,
        )?)
    }

    fn try_from_marker(marker: Marker) -> Result<Self, OsmTileEngineError> {
        Ok(Self {
            id: marker_id_to_i64(marker.id)?,
            lat: marker.lat,
            lon: marker.lon,
            kind: marker.kind,
            min_zoom: u32_to_i64(marker.min_zoom),
            max_zoom: u32_to_i64(marker.max_zoom),
        })
    }
}

impl MobileMarkerCluster {
    fn try_from_cluster(cluster: MarkerCluster) -> Result<Self, OsmTileEngineError> {
        Ok(Self {
            id: cluster.id,
            lat: cluster.lat,
            lon: cluster.lon,
            count: usize_to_i64(cluster.count)?,
            marker_ids: cluster
                .marker_ids
                .into_iter()
                .map(marker_id_to_i64)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl MobileMarkerRenderItem {
    fn try_from_render_item(item: MarkerRenderItem) -> Result<Self, OsmTileEngineError> {
        match item {
            MarkerRenderItem::Marker(marker) => Ok(Self {
                item_type: MobileRenderItemType::Marker,
                marker: Some(MobileMarker::try_from_marker(marker)?),
                cluster: None,
            }),
            MarkerRenderItem::Cluster(cluster) => Ok(Self {
                item_type: MobileRenderItemType::Cluster,
                marker: None,
                cluster: Some(MobileMarkerCluster::try_from_cluster(cluster)?),
            }),
        }
    }
}

impl From<CoreError> for OsmTileEngineError {
    fn from(error: CoreError) -> Self {
        Self::InvalidInput {
            details: error.to_string(),
        }
    }
}

impl From<TileLoadError> for OsmTileEngineError {
    fn from(error: TileLoadError) -> Self {
        match error {
            TileLoadError::CacheIo(_) | TileLoadError::InvalidCachePath => Self::Cache {
                details: error.to_string(),
            },
            TileLoadError::Network(_) | TileLoadError::HttpStatus(_) => Self::Network {
                details: error.to_string(),
            },
            _ => Self::InvalidInput {
                details: error.to_string(),
            },
        }
    }
}

impl From<RenderError> for OsmTileEngineError {
    fn from(error: RenderError) -> Self {
        match error {
            RenderError::MissingViewport => Self::State {
                details: error.to_string(),
            },
            _ => Self::InvalidInput {
                details: error.to_string(),
            },
        }
    }
}

fn i64_to_u32(value: i64, name: &str) -> Result<u32, OsmTileEngineError> {
    if value < 0 || value > u32::MAX as i64 {
        return Err(OsmTileEngineError::InvalidInput {
            details: format!("{name} must be in 0..={}", u32::MAX),
        });
    }

    Ok(value as u32)
}

fn i64_to_marker_id(value: i64) -> Result<MarkerId, OsmTileEngineError> {
    if value < 0 {
        return Err(OsmTileEngineError::InvalidInput {
            details: "marker id must be non-negative".to_owned(),
        });
    }

    Ok(value as MarkerId)
}

fn marker_id_to_i64(value: MarkerId) -> Result<i64, OsmTileEngineError> {
    if value > i64::MAX as MarkerId {
        return Err(OsmTileEngineError::InvalidInput {
            details: format!("marker id {value} does not fit into i64"),
        });
    }

    Ok(value as i64)
}

fn u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

fn usize_to_i64(value: usize) -> Result<i64, OsmTileEngineError> {
    if value > i64::MAX as usize {
        return Err(OsmTileEngineError::InvalidInput {
            details: format!("value {value} does not fit into i64"),
        });
    }

    Ok(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Arc<OsmTileEngine> {
        OsmTileEngine::new(
            "http://localhost:8080/tile/{z}/{x}/{y}.png".to_owned(),
            std::env::temp_dir()
                .join("osm-tile-engine-mobile-tests")
                .display()
                .to_string(),
        )
        .unwrap()
    }

    fn marker(id: i64, lat: f64, lon: f64, min_zoom: i64, max_zoom: i64) -> MobileMarker {
        MobileMarker {
            id,
            lat,
            lon,
            kind: "poi".to_owned(),
            min_zoom,
            max_zoom,
        }
    }

    #[test]
    fn mobile_facade_filters_visible_markers() {
        let core = core();

        core.set_viewport(MobileViewport {
            south: 53.0,
            west: 27.0,
            north: 54.5,
            east: 28.5,
            zoom: 12,
        })
        .unwrap();
        core.replace_markers(vec![
            marker(2, 53.9, 27.56, 0, 18),
            marker(1, 53.91, 27.57, 0, 18),
            marker(3, 52.0, 27.56, 0, 18),
            marker(4, 53.92, 27.58, 13, 18),
        ])
        .unwrap();

        let ids = core
            .visible_markers()
            .unwrap()
            .into_iter()
            .map(|marker| marker.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn mobile_facade_returns_cluster_items() {
        let core = core();

        core.set_viewport(MobileViewport {
            south: 53.0,
            west: 27.0,
            north: 54.5,
            east: 28.5,
            zoom: 14,
        })
        .unwrap();
        core.replace_markers(vec![
            marker(1, 53.9000, 27.5600, 0, 18),
            marker(2, 53.9001, 27.5601, 0, 18),
        ])
        .unwrap();

        let items = core.clustered_markers().unwrap();

        assert!(items.iter().any(|item| {
            matches!(item.item_type, MobileRenderItemType::Cluster)
                && item
                    .cluster
                    .as_ref()
                    .is_some_and(|cluster| cluster.count == 2 && cluster.marker_ids == vec![1, 2])
        }));
    }

    #[test]
    fn mobile_facade_validates_signed_inputs() {
        let core = core();

        assert!(matches!(
            core.remove_marker(-1),
            Err(OsmTileEngineError::InvalidInput { .. })
        ));
        assert!(matches!(
            core.load_tile(-1, 0, 0),
            Err(OsmTileEngineError::InvalidInput { .. })
        ));
    }
}
