use std::collections::HashMap;

use osm_core::{FadeState, OverscaledTileId, TileId, TileState};

const DEFAULT_MAX_MEMORY_TILES: usize = 512;

#[derive(Debug, Clone)]
pub struct TileEntry {
    pub overscaled_id: OverscaledTileId,
    pub state: TileState,
    pub fade_state: FadeState,
    pub data: Option<Vec<u8>>,
    pub last_used_generation: u64,
}

impl TileEntry {
    pub fn new(overscaled_id: OverscaledTileId) -> Self {
        Self {
            overscaled_id,
            state: TileState::Loading,
            fade_state: FadeState::Loaded,
            data: None,
            last_used_generation: 0,
        }
    }

    pub fn mark_loaded(&mut self, data: Vec<u8>) {
        self.data = Some(data);
        self.state = TileState::Loaded;
        self.fade_state = FadeState::NeedsFirstPlacement;
    }

    pub fn mark_complete(&mut self) {
        self.state = TileState::Complete;
        if self.fade_state == FadeState::NeedsFirstPlacement {
            self.fade_state = FadeState::NeedsSecondPlacement;
        }
    }

    pub fn mark_error(&mut self) {
        self.state = TileState::Error;
        self.data = None;
    }

    pub fn mark_rendered_ideal(&mut self) {
        if self.fade_state == FadeState::NeedsSecondPlacement {
            self.fade_state = FadeState::CanRemove;
        }
    }

    pub fn mark_rendered_previously(&mut self) {
        if self.fade_state == FadeState::NeedsFirstPlacement {
            self.fade_state = FadeState::NeedsSecondPlacement;
        }
    }

    pub fn performed_fade_placement(&mut self) {
        if self.fade_state == FadeState::NeedsSecondPlacement {
            self.fade_state = FadeState::CanRemove;
        }
    }

    pub fn data(&self) -> Option<&[u8]> {
        self.data.as_deref()
    }

    pub fn is_renderable(&self) -> bool {
        self.state.is_renderable() && self.data.is_some()
    }
}

#[derive(Debug, Default)]
pub struct TileStore {
    tiles: HashMap<TileId, TileEntry>,
    current_generation: u64,
    max_tiles: usize,
}

impl TileStore {
    pub fn new(max_tiles: usize) -> Self {
        Self {
            tiles: HashMap::with_capacity(max_tiles),
            current_generation: 0,
            max_tiles,
        }
    }

    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_MAX_MEMORY_TILES)
    }

    pub fn get(&self, id: TileId) -> Option<&TileEntry> {
        self.tiles.get(&id)
    }

    pub fn get_mut(&mut self, id: TileId) -> Option<&mut TileEntry> {
        self.tiles.get_mut(&id)
    }

    pub fn insert(&mut self, overscaled_id: OverscaledTileId) -> &mut TileEntry {
        let entry = self
            .tiles
            .entry(overscaled_id.canonical())
            .or_insert_with(|| {
                let mut e = TileEntry::new(overscaled_id);
                e.last_used_generation = self.current_generation;
                e
            });
        entry.last_used_generation = self.current_generation;
        if entry.overscaled_id != overscaled_id {
            entry.overscaled_id = overscaled_id;
        }
        entry
    }

    pub fn mark_loading(&mut self, id: TileId) {
        if let Some(entry) = self.tiles.get_mut(&id) {
            entry.state = TileState::Loading;
            entry.last_used_generation = self.current_generation;
        }
    }

    pub fn mark_loaded(&mut self, id: TileId, data: Vec<u8>) {
        if let Some(entry) = self.tiles.get_mut(&id) {
            entry.mark_loaded(data);
        }
    }

    pub fn mark_complete(&mut self, id: TileId) {
        if let Some(entry) = self.tiles.get_mut(&id) {
            entry.mark_complete();
        }
    }

    pub fn mark_error(&mut self, id: TileId) {
        if let Some(entry) = self.tiles.get_mut(&id) {
            entry.mark_error();
        }
    }

    pub fn remove(&mut self, id: TileId) -> Option<TileEntry> {
        self.tiles.remove(&id)
    }

    pub fn contains(&self, id: TileId) -> bool {
        self.tiles.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn retain_renderable(&mut self, ids: impl Iterator<Item = TileId>) {
        let retained: HashMap<TileId, ()> = ids.map(|id| (id, ())).collect();
        self.tiles
            .retain(|id, entry| retained.contains_key(id) || entry.state.is_loading());
    }

    pub fn evict_oldest(&mut self) {
        if self.tiles.len() <= self.max_tiles {
            return;
        }

        let mut entries: Vec<_> = self
            .tiles
            .iter()
            .filter(|(_, entry)| entry.state.is_complete() || entry.state.is_error())
            .map(|(id, entry)| (*id, entry.last_used_generation))
            .collect();
        entries.sort_by_key(|&(_, generation)| generation);

        let to_remove = self.tiles.len() - self.max_tiles;
        for (id, _) in entries.into_iter().take(to_remove) {
            self.tiles.remove(&id);
        }
    }

    pub fn advance_generation(&mut self) {
        self.current_generation += 1;
    }

    pub fn current_generation(&self) -> u64 {
        self.current_generation
    }

    pub fn renderable_tile_ids(&self) -> impl Iterator<Item = TileId> + '_ {
        self.tiles
            .iter()
            .filter(|(_, entry)| entry.is_renderable())
            .map(|(id, _)| *id)
    }

    pub fn loading_tile_ids(&self) -> impl Iterator<Item = TileId> + '_ {
        self.tiles
            .iter()
            .filter(|(_, entry)| entry.state.is_loading())
            .map(|(id, _)| *id)
    }

    pub fn fallback_tile_for(&self, id: TileId, max_levels: u32) -> Option<TileId> {
        for level in 1..=max_levels {
            if let Some(parent) = id.parent(level)
                && let Some(entry) = self.tiles.get(&parent)
                && entry.is_renderable()
            {
                return Some(parent);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(z: u32, x: u32, y: u32) -> TileId {
        TileId::new(z, x, y).unwrap()
    }

    fn overscaled(z: u32, x: u32, y: u32) -> OverscaledTileId {
        OverscaledTileId::from_canonical(tile(z, x, y), 0)
    }

    #[test]
    fn tile_entry_transitions_through_states() {
        let mut entry = TileEntry::new(overscaled(4, 8, 8));
        assert!(entry.state.is_loading());
        assert!(!entry.is_renderable());

        entry.mark_loaded(b"tile data".to_vec());
        assert!(entry.state.is_loaded());
        assert!(entry.is_renderable());
        assert!(entry.fade_state.is_fading());

        entry.mark_complete();
        assert!(entry.state.is_complete());
        assert!(entry.fade_state.is_fading());

        entry.mark_rendered_ideal();
        assert!(!entry.fade_state.is_fading());
    }

    #[test]
    fn tile_entry_error_clears_data() {
        let mut entry = TileEntry::new(overscaled(4, 8, 8));
        entry.mark_loaded(b"data".to_vec());
        entry.mark_error();
        assert!(entry.state.is_error());
        assert!(entry.data().is_none());
        assert!(!entry.is_renderable());
    }

    #[test]
    fn store_tracks_tile_states() {
        let mut store = TileStore::with_default_capacity();

        store.insert(overscaled(4, 8, 8));
        assert!(store.contains(tile(4, 8, 8)));
        assert!(store.get(tile(4, 8, 8)).unwrap().state.is_loading());

        store.mark_loaded(tile(4, 8, 8), b"data".to_vec());
        assert!(store.get(tile(4, 8, 8)).unwrap().state.is_loaded());

        store.mark_complete(tile(4, 8, 8));
        assert!(store.get(tile(4, 8, 8)).unwrap().state.is_complete());
    }

    #[test]
    fn store_finds_fallback_parent() {
        let mut store = TileStore::with_default_capacity();

        store.insert(overscaled(2, 2, 2));
        store.mark_loaded(tile(2, 2, 2), b"parent".to_vec());
        store.mark_complete(tile(2, 2, 2));

        store.insert(overscaled(4, 8, 8));
        store.mark_loading(tile(4, 8, 8));

        let fallback = store.fallback_tile_for(tile(4, 8, 8), 4);
        assert_eq!(fallback, Some(tile(2, 2, 2)));
    }

    #[test]
    fn store_retains_loading_tiles_during_cleanup() {
        let mut store = TileStore::new(2);

        store.insert(overscaled(4, 8, 8));
        store.mark_loaded(tile(4, 8, 8), b"a".to_vec());
        store.mark_complete(tile(4, 8, 8));

        store.insert(overscaled(4, 9, 9));
        store.mark_loading(tile(4, 9, 9));

        store.retain_renderable([tile(4, 8, 8)].into_iter());
        assert!(store.contains(tile(4, 8, 8)));
        assert!(store.contains(tile(4, 9, 9)));
    }

    #[test]
    fn store_evicts_oldest_when_over_capacity() {
        let mut store = TileStore::new(2);

        store.insert(overscaled(4, 8, 8));
        store.mark_loaded(tile(4, 8, 8), b"a".to_vec());
        store.mark_complete(tile(4, 8, 8));

        store.advance_generation();

        store.insert(overscaled(4, 9, 9));
        store.mark_loaded(tile(4, 9, 9), b"b".to_vec());
        store.mark_complete(tile(4, 9, 9));

        store.advance_generation();

        store.insert(overscaled(4, 10, 10));
        store.mark_loaded(tile(4, 10, 10), b"c".to_vec());
        store.mark_complete(tile(4, 10, 10));

        store.evict_oldest();
        assert!(!store.contains(tile(4, 8, 8)));
        assert!(store.contains(tile(4, 9, 9)));
        assert!(store.contains(tile(4, 10, 10)));
    }

    #[test]
    fn store_returns_renderable_ids() {
        let mut store = TileStore::with_default_capacity();

        store.insert(overscaled(4, 8, 8));
        store.mark_loaded(tile(4, 8, 8), b"a".to_vec());
        store.mark_complete(tile(4, 8, 8));

        store.insert(overscaled(4, 9, 9));
        store.mark_loading(tile(4, 9, 9));

        let renderable: Vec<_> = store.renderable_tile_ids().collect();
        assert_eq!(renderable, vec![tile(4, 8, 8)]);
    }
}
