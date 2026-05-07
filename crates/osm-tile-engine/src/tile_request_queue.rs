use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use osm_core::TileId;
use osm_renderer::TilePlanPriority;

const MAX_NON_FALLBACK_ZOOM_DISTANCE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TileRequestMetadata {
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TileRequestLane {
    ChildPrefetch = 0,
    PeripheryCurrentZoom = 1,
    LookAheadCurrentZoom = 2,
    FallbackParent = 3,
    VisibleCurrentZoom = 4,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TileLoadRequest {
    pub(crate) id: TileId,
    pub(crate) metadata: TileRequestMetadata,
    pub(crate) lane: TileRequestLane,
    pub(crate) plan_priority: TilePlanPriority,
    pub(crate) priority: f32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PrioritizedTileRequest(pub(crate) TileLoadRequest);

impl PartialEq for PrioritizedTileRequest {
    fn eq(&self, other: &Self) -> bool {
        same_tile_request(self.0, other.0)
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
            .lane
            .cmp(&other.0.lane)
            .then_with(|| {
                // TilePlanPriority is ordered with smaller values being more urgent.
                other.0.plan_priority.cmp(&self.0.plan_priority)
            })
            .then_with(|| self.0.priority.total_cmp(&other.0.priority))
            .then_with(|| self.0.metadata.generation.cmp(&other.0.metadata.generation))
            .then_with(|| self.0.id.z.cmp(&other.0.id.z))
            .then_with(|| self.0.id.y.cmp(&other.0.id.y))
            .then_with(|| self.0.id.x.cmp(&other.0.id.x))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TileRequestSchedulerStats {
    pub(crate) queued_stale_skipped: u64,
    pub(crate) queued_active_deferred: u64,
}

#[derive(Debug, Default)]
pub(crate) struct TileRequestScheduler {
    queued: BinaryHeap<PrioritizedTileRequest>,
    queued_best: HashMap<TileId, TileLoadRequest>,
    retained_tiles: HashSet<TileId>,
    current_generation: u64,
    current_zoom: Option<u32>,
    stats: TileRequestSchedulerStats,
}

impl TileRequestScheduler {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn queue(&mut self, request: TileLoadRequest) {
        if request.metadata.generation < self.current_generation {
            self.stats.queued_stale_skipped += 1;
            return;
        }
        queue_tile_request(&mut self.queued, &mut self.queued_best, request);
    }

    pub(crate) fn retain_plan(
        &mut self,
        generation: u64,
        current_zoom: u32,
        tile_ids: HashSet<TileId>,
    ) -> usize {
        self.current_generation = generation;
        self.current_zoom = Some(current_zoom);
        self.retained_tiles = tile_ids;

        let previous_len = self.queued_best.len();
        prune_queued_requests(&mut self.queued_best, &self.retained_tiles);
        let pruned = previous_len.saturating_sub(self.queued_best.len());
        rebuild_queued_requests(&mut self.queued, &self.queued_best);
        pruned
    }

    pub(crate) fn pop_next(&mut self, active_fetches: &HashSet<TileId>) -> Option<TileLoadRequest> {
        loop {
            let request = self.queued.pop()?.0;
            if self
                .queued_best
                .get(&request.id)
                .map(|best| !same_tile_request(*best, request))
                .unwrap_or(true)
            {
                self.stats.queued_stale_skipped += 1;
                continue;
            }

            if !self.is_request_retained(request) || self.is_request_stale(request) {
                self.queued_best.remove(&request.id);
                self.stats.queued_stale_skipped += 1;
                continue;
            }

            if active_fetches.contains(&request.id) {
                self.stats.queued_active_deferred += 1;
                continue;
            }

            self.queued_best.remove(&request.id);
            return Some(request);
        }
    }

    pub(crate) fn requeue_best_for_tile(&mut self, id: TileId) -> bool {
        let Some(request) = self.queued_best.get(&id).copied() else {
            return false;
        };
        self.queued.push(PrioritizedTileRequest(request));
        true
    }

    pub(crate) fn rebuild_if_fragmented(&mut self) {
        if self.queued.len() > self.queued_best.len().saturating_mul(4).saturating_add(32) {
            rebuild_queued_requests(&mut self.queued, &self.queued_best);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    pub(crate) fn drain_stats(&mut self) -> TileRequestSchedulerStats {
        let stats = self.stats;
        self.stats = TileRequestSchedulerStats::default();
        stats
    }

    fn is_request_retained(&self, request: TileLoadRequest) -> bool {
        self.current_zoom.is_none()
            || (self.retained_tiles.contains(&request.id)
                && (request.lane == TileRequestLane::FallbackParent
                    || request
                        .id
                        .z
                        .abs_diff(self.current_zoom.unwrap_or(request.id.z))
                        <= MAX_NON_FALLBACK_ZOOM_DISTANCE))
    }

    fn is_request_stale(&self, request: TileLoadRequest) -> bool {
        request.metadata.generation < self.current_generation
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
        && left.lane == right.lane
        && left.plan_priority == right.plan_priority
        && left.priority.to_bits() == right.priority.to_bits()
}

fn should_replace_queued_tile(existing: TileLoadRequest, candidate: TileLoadRequest) -> bool {
    candidate.lane > existing.lane
        || (candidate.lane == existing.lane && candidate.plan_priority < existing.plan_priority)
        || (candidate.lane == existing.lane
            && candidate.plan_priority == existing.plan_priority
            && candidate.priority > existing.priority)
        || (candidate.lane == existing.lane
            && candidate.plan_priority == existing.plan_priority
            && candidate.priority == existing.priority
            && candidate.metadata.generation > existing.metadata.generation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(z: u32, x: u32, y: u32) -> TileId {
        TileId::new(z, x, y).unwrap()
    }

    fn request(id: TileId, generation: u64, priority: f32) -> TileLoadRequest {
        lane_request(
            id,
            generation,
            priority,
            TileRequestLane::VisibleCurrentZoom,
            TilePlanPriority::Visible,
        )
    }

    fn lane_request(
        id: TileId,
        generation: u64,
        priority: f32,
        lane: TileRequestLane,
        plan_priority: TilePlanPriority,
    ) -> TileLoadRequest {
        TileLoadRequest {
            id,
            metadata: TileRequestMetadata { generation },
            lane,
            plan_priority,
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

    #[test]
    fn scheduler_prefers_visible_current_zoom_over_child_prefetch() {
        let visible = tile(4, 8, 8);
        let child = tile(5, 16, 16);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.retain_plan(1, 4, HashSet::from([visible, child]));

        scheduler.queue(lane_request(
            child,
            1,
            50_000.0,
            TileRequestLane::ChildPrefetch,
            TilePlanPriority::Child,
        ));
        scheduler.queue(lane_request(
            visible,
            1,
            1.0,
            TileRequestLane::VisibleCurrentZoom,
            TilePlanPriority::Visible,
        ));

        assert_eq!(scheduler.pop_next(&HashSet::new()).unwrap().id, visible);
    }

    #[test]
    fn scheduler_prefers_fallback_parent_over_periphery() {
        let fallback = tile(2, 2, 2);
        let periphery = tile(4, 9, 8);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.retain_plan(1, 4, HashSet::from([fallback, periphery]));

        scheduler.queue(lane_request(
            periphery,
            1,
            50_000.0,
            TileRequestLane::PeripheryCurrentZoom,
            TilePlanPriority::Periphery,
        ));
        scheduler.queue(lane_request(
            fallback,
            1,
            1.0,
            TileRequestLane::FallbackParent,
            TilePlanPriority::Fallback,
        ));

        assert_eq!(scheduler.pop_next(&HashSet::new()).unwrap().id, fallback);
    }

    #[test]
    fn scheduler_rebuilds_fragmented_queue_and_reports_empty() {
        let id = tile(4, 8, 8);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.retain_plan(40, 4, HashSet::from([id]));

        for priority in 1..=40 {
            scheduler.queue(lane_request(
                id,
                40,
                priority as f32,
                TileRequestLane::LookAheadCurrentZoom,
                TilePlanPriority::LookAhead,
            ));
        }

        scheduler.rebuild_if_fragmented();

        assert!(!scheduler.is_empty());
        assert_eq!(
            scheduler
                .pop_next(&HashSet::new())
                .unwrap()
                .metadata
                .generation,
            40
        );
        assert!(scheduler.is_empty());
    }

    #[test]
    fn scheduler_retain_plan_prunes_requests_outside_current_plan() {
        let retained = tile(4, 8, 8);
        let stale = tile(4, 9, 8);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.queue(request(retained, 1, 5_000.0));
        scheduler.queue(request(stale, 1, 5_000.0));

        let pruned = scheduler.retain_plan(1, 4, HashSet::from([retained]));

        assert_eq!(pruned, 1);
        assert_eq!(scheduler.pop_next(&HashSet::new()).unwrap().id, retained);
        assert!(scheduler.pop_next(&HashSet::new()).is_none());
    }

    #[test]
    fn scheduler_skips_stale_generation_after_retain_plan() {
        let id = tile(4, 8, 8);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.queue(request(id, 1, 5_000.0));
        scheduler.retain_plan(2, 4, HashSet::from([id]));

        assert!(scheduler.pop_next(&HashSet::new()).is_none());
        assert_eq!(scheduler.drain_stats().queued_stale_skipped, 1);
    }

    #[test]
    fn scheduler_keeps_newer_active_tile_request_for_later() {
        let id = tile(4, 8, 8);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.retain_plan(1, 4, HashSet::from([id]));
        scheduler.queue(request(id, 1, 4_000.0));
        let first = scheduler.pop_next(&HashSet::new()).unwrap();
        assert_eq!(first.id, id);

        scheduler.retain_plan(2, 4, HashSet::from([id]));
        scheduler.queue(request(id, 2, 5_000.0));
        assert!(scheduler.pop_next(&HashSet::from([id])).is_none());
        assert_eq!(scheduler.drain_stats().queued_active_deferred, 1);

        scheduler.requeue_best_for_tile(id);
        assert!(same_tile_request(
            scheduler.pop_next(&HashSet::new()).unwrap(),
            request(id, 2, 5_000.0)
        ));
    }

    #[test]
    fn scheduler_prefers_newer_generation_for_equal_lane_and_priority() {
        let older = tile(4, 8, 8);
        let newer = tile(4, 9, 8);
        let mut scheduler = TileRequestScheduler::new();
        scheduler.retain_plan(2, 4, HashSet::from([older, newer]));
        scheduler.queue(request(older, 1, 5_000.0));
        scheduler.queue(request(newer, 2, 5_000.0));

        assert_eq!(scheduler.pop_next(&HashSet::new()).unwrap().id, newer);
    }
}
