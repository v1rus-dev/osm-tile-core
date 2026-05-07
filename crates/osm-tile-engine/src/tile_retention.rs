use std::collections::HashSet;

use osm_core::TileId;

pub(crate) fn should_evict_for_zoom(
    tile_id: TileId,
    current_zoom: u32,
    max_zoom_distance_to_keep: u32,
    protected_tile_ids: &HashSet<TileId>,
) -> bool {
    !protected_tile_ids.contains(&tile_id)
        && tile_id.z.abs_diff(current_zoom) > max_zoom_distance_to_keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(z: u32, x: u32, y: u32) -> TileId {
        TileId::new(z, x, y).unwrap()
    }

    #[test]
    fn protected_fallback_ancestor_is_not_evicted_on_zoom_jump() {
        let fallback = tile(2, 1, 1);
        let protected = HashSet::from([fallback]);

        assert!(!should_evict_for_zoom(fallback, 8, 2, &protected));
    }

    #[test]
    fn unprotected_far_zoom_tile_is_evicted() {
        assert!(should_evict_for_zoom(tile(2, 1, 1), 8, 2, &HashSet::new()));
    }
}
