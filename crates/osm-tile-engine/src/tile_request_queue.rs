use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use osm_core::TileId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TileRequestMetadata {
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TileLoadRequest {
    pub(crate) id: TileId,
    pub(crate) metadata: TileRequestMetadata,
    pub(crate) priority: f32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PrioritizedTileRequest(pub(crate) TileLoadRequest);

impl PartialEq for PrioritizedTileRequest {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for PrioritizedTileRequest {}

impl PartialOrd for PrioritizedTileRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedTileRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .priority
            .total_cmp(&other.0.priority)
            .then_with(|| self.0.metadata.generation.cmp(&other.0.metadata.generation))
    }
}

pub(crate) fn queue_tile_request(
    queued: &mut BinaryHeap<PrioritizedTileRequest>,
    queued_best: &mut HashMap<TileId, TileLoadRequest>,
    request: TileLoadRequest,
) {
    let should_queue = queued_best
        .get(&request.id)
        .map(|existing| should_replace_queued_tile(*existing, request))
        .unwrap_or(true);
    if should_queue {
        queued_best.insert(request.id, request);
        queued.push(PrioritizedTileRequest(request));
    }
}

pub(crate) fn prune_queued_requests(
    queued_best: &mut HashMap<TileId, TileLoadRequest>,
    retained_tiles: &HashSet<TileId>,
) {
    queued_best.retain(|tile_id, _| retained_tiles.contains(tile_id));
}

pub(crate) fn rebuild_queued_requests(
    queued: &mut BinaryHeap<PrioritizedTileRequest>,
    queued_best: &HashMap<TileId, TileLoadRequest>,
) {
    queued.clear();
    queued.extend(queued_best.values().copied().map(PrioritizedTileRequest));
}

pub(crate) fn pending_metadata_matches(
    pending_metadata: &HashMap<TileId, TileRequestMetadata>,
    id: TileId,
    metadata: TileRequestMetadata,
) -> bool {
    pending_metadata
        .get(&id)
        .map(|pending| *pending == metadata)
        .unwrap_or(false)
}

pub(crate) fn same_tile_request(left: TileLoadRequest, right: TileLoadRequest) -> bool {
    left.id == right.id
        && left.metadata == right.metadata
        && left.priority.to_bits() == right.priority.to_bits()
}

fn should_replace_queued_tile(existing: TileLoadRequest, candidate: TileLoadRequest) -> bool {
    candidate.priority > existing.priority
        || (candidate.priority == existing.priority
            && candidate.metadata.generation > existing.metadata.generation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(z: u32, x: u32, y: u32) -> TileId {
        TileId::new(z, x, y).unwrap()
    }

    fn request(id: TileId, generation: u64, priority: f32) -> TileLoadRequest {
        TileLoadRequest {
            id,
            metadata: TileRequestMetadata { generation },
            priority,
        }
    }

    #[test]
    fn queue_tile_request_keeps_best_request_per_tile() {
        let id = tile(4, 8, 8);
        let mut queued = BinaryHeap::new();
        let mut queued_best = HashMap::new();

        queue_tile_request(&mut queued, &mut queued_best, request(id, 1, 1_000.0));
        queue_tile_request(&mut queued, &mut queued_best, request(id, 1, 5_000.0));
        queue_tile_request(&mut queued, &mut queued_best, request(id, 2, 3_000.0));

        assert!(same_tile_request(
            *queued_best.get(&id).unwrap(),
            request(id, 1, 5_000.0)
        ));
    }

    #[test]
    fn queue_tile_request_replaces_equal_priority_with_newer_generation() {
        let id = tile(4, 8, 8);
        let mut queued = BinaryHeap::new();
        let mut queued_best = HashMap::new();

        queue_tile_request(&mut queued, &mut queued_best, request(id, 1, 5_000.0));
        queue_tile_request(&mut queued, &mut queued_best, request(id, 2, 5_000.0));

        assert!(same_tile_request(
            *queued_best.get(&id).unwrap(),
            request(id, 2, 5_000.0)
        ));
    }

    #[test]
    fn prune_queued_requests_removes_tiles_outside_current_plan() {
        let retained = tile(4, 8, 8);
        let stale = tile(4, 9, 8);
        let mut queued_best = HashMap::new();
        queued_best.insert(retained, request(retained, 1, 5_000.0));
        queued_best.insert(stale, request(stale, 1, 5_000.0));

        prune_queued_requests(&mut queued_best, &HashSet::from([retained]));

        assert!(queued_best.contains_key(&retained));
        assert!(!queued_best.contains_key(&stale));
    }

    #[test]
    fn rebuild_queued_requests_drops_stale_heap_entries() {
        let retained = tile(4, 8, 8);
        let stale = tile(4, 9, 8);
        let mut queued = BinaryHeap::new();
        let mut queued_best = HashMap::new();

        queue_tile_request(&mut queued, &mut queued_best, request(retained, 1, 5_000.0));
        queue_tile_request(&mut queued, &mut queued_best, request(stale, 1, 4_000.0));
        prune_queued_requests(&mut queued_best, &HashSet::from([retained]));
        rebuild_queued_requests(&mut queued, &queued_best);

        assert_eq!(queued.len(), 1);
        assert_eq!(queued.pop().unwrap().0.id, retained);
    }

    #[test]
    fn old_result_does_not_match_newer_pending_metadata() {
        let id = tile(4, 8, 8);
        let mut pending_metadata = HashMap::new();
        pending_metadata.insert(id, TileRequestMetadata { generation: 2 });

        assert!(!pending_metadata_matches(
            &pending_metadata,
            id,
            TileRequestMetadata { generation: 1 }
        ));
        assert!(pending_metadata_matches(
            &pending_metadata,
            id,
            TileRequestMetadata { generation: 2 }
        ));
    }
}
