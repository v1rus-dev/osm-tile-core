use std::collections::HashMap;

use osm_core::{MAX_ZOOM, TileId, tile_count_per_axis};

use crate::VisibleTile;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TilePlanOptions {
    pub moving: bool,
    pub velocity_tiles_per_sec: (f64, f64),
    pub parent_fallback_levels: u32,
    pub offscreen_buffer_multiplier: f64,
    pub idle_child_prefetch_levels: u32,
    pub zoom_fraction: f64,
    pub moving_child_prefetch_threshold: f64,
    pub look_ahead_min_speed_tiles_per_sec: f64,
    pub look_ahead_max_depth_tiles: u32,
    pub look_ahead_half_width_tiles: i64,
}

impl Default for TilePlanOptions {
    fn default() -> Self {
        Self {
            moving: false,
            velocity_tiles_per_sec: (0.0, 0.0),
            parent_fallback_levels: MAX_ZOOM,
            offscreen_buffer_multiplier: 1.5,
            idle_child_prefetch_levels: 1,
            zoom_fraction: 0.0,
            moving_child_prefetch_threshold: 0.65,
            look_ahead_min_speed_tiles_per_sec: 0.25,
            look_ahead_max_depth_tiles: 3,
            look_ahead_half_width_tiles: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlannedTile {
    pub id: TileId,
    pub priority: TilePlanPriority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileLoadPlan {
    tiles: Vec<PlannedTile>,
}

impl TileLoadPlan {
    pub fn tiles(&self) -> &[PlannedTile] {
        &self.tiles
    }

    pub fn contains(&self, id: TileId) -> bool {
        self.tiles.iter().any(|tile| tile.id == id)
    }

    pub fn priority_for(&self, id: TileId) -> Option<TilePlanPriority> {
        self.tiles
            .iter()
            .find(|tile| tile.id == id)
            .map(|tile| tile.priority)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TilePlanPriority {
    Visible = 0,
    Fallback = 1,
    LookAhead = 2,
    Periphery = 3,
    Child = 4,
}

pub fn plan_tile_loads(visible_tiles: &[VisibleTile], options: TilePlanOptions) -> TileLoadPlan {
    let mut planned = HashMap::<TileId, TilePlanPriority>::new();
    let Some(bounds) = TileBounds::from_visible_tiles(visible_tiles) else {
        return TileLoadPlan { tiles: Vec::new() };
    };

    for tile in visible_tiles {
        insert_best(&mut planned, tile.id, TilePlanPriority::Visible);
    }

    add_parent_fallbacks(&mut planned, visible_tiles, options.parent_fallback_levels);
    add_periphery(&mut planned, bounds, options.offscreen_buffer_multiplier);
    add_look_ahead(&mut planned, bounds, options);
    if !options.moving {
        add_idle_children(
            &mut planned,
            visible_tiles,
            options.idle_child_prefetch_levels,
        );
    } else if options.zoom_fraction >= options.moving_child_prefetch_threshold {
        add_children(&mut planned, visible_tiles, 1, TilePlanPriority::LookAhead);
    }

    let mut tiles = planned
        .into_iter()
        .map(|(id, priority)| PlannedTile { id, priority })
        .collect::<Vec<_>>();
    tiles.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.z.cmp(&right.id.z))
            .then_with(|| left.id.y.cmp(&right.id.y))
            .then_with(|| left.id.x.cmp(&right.id.x))
    });

    TileLoadPlan { tiles }
}

#[derive(Debug, Clone, Copy)]
struct TileBounds {
    zoom: u32,
    min_x: i64,
    max_x: i64,
    min_y: i64,
    max_y: i64,
    limit: i64,
}

impl TileBounds {
    fn from_visible_tiles(visible_tiles: &[VisibleTile]) -> Option<Self> {
        let zoom = visible_tiles.first()?.id.z;
        let limit = i64::from(tile_count_per_axis(zoom).ok()?);
        let base_x = visible_tiles.first()?.id.x as i64;
        let mut min_x = i64::MAX;
        let mut max_x = i64::MIN;
        let mut min_y = i64::MAX;
        let mut max_y = i64::MIN;

        for tile in visible_tiles.iter().filter(|tile| tile.id.z == zoom) {
            let x = unwrap_x(tile.id.x as i64, base_x, limit);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(tile.id.y as i64);
            max_y = max_y.max(tile.id.y as i64);
        }

        if min_x == i64::MAX {
            return None;
        }

        Some(Self {
            zoom,
            min_x,
            max_x,
            min_y,
            max_y,
            limit,
        })
    }

    fn width_tiles(self) -> i64 {
        self.max_x - self.min_x + 1
    }

    fn height_tiles(self) -> i64 {
        self.max_y - self.min_y + 1
    }
}

fn insert_best(
    planned: &mut HashMap<TileId, TilePlanPriority>,
    id: TileId,
    priority: TilePlanPriority,
) {
    planned
        .entry(id)
        .and_modify(|existing| {
            if priority < *existing {
                *existing = priority;
            }
        })
        .or_insert(priority);
}

fn add_parent_fallbacks(
    planned: &mut HashMap<TileId, TilePlanPriority>,
    visible_tiles: &[VisibleTile],
    levels: u32,
) {
    if levels == 0 {
        return;
    }

    for tile in visible_tiles {
        for level in 1..=levels.min(tile.id.z) {
            let shift = level;
            if let Ok(parent) =
                TileId::new(tile.id.z - shift, tile.id.x >> shift, tile.id.y >> shift)
            {
                insert_best(planned, parent, TilePlanPriority::Fallback);
            }
        }
    }
}

fn add_periphery(
    planned: &mut HashMap<TileId, TilePlanPriority>,
    bounds: TileBounds,
    buffer_multiplier: f64,
) {
    let multiplier = if buffer_multiplier.is_finite() {
        buffer_multiplier.max(0.0)
    } else {
        0.0
    };
    let margin_x = ((bounds.width_tiles() as f64) * multiplier).ceil() as i64;
    let margin_y = ((bounds.height_tiles() as f64) * multiplier).ceil() as i64;

    if margin_x == 0 && margin_y == 0 {
        return;
    }

    for y in (bounds.min_y - margin_y)..=(bounds.max_y + margin_y) {
        if y < 0 || y >= bounds.limit {
            continue;
        }
        for x in (bounds.min_x - margin_x)..=(bounds.max_x + margin_x) {
            if (bounds.min_x..=bounds.max_x).contains(&x)
                && (bounds.min_y..=bounds.max_y).contains(&y)
            {
                continue;
            }

            if let Some(id) = tile_at(bounds.zoom, x, y) {
                insert_best(planned, id, TilePlanPriority::Periphery);
            }
        }
    }
}

fn add_look_ahead(
    planned: &mut HashMap<TileId, TilePlanPriority>,
    bounds: TileBounds,
    options: TilePlanOptions,
) {
    let (vx, vy) = options.velocity_tiles_per_sec;
    let speed = vx.hypot(vy);
    if speed < options.look_ahead_min_speed_tiles_per_sec {
        return;
    }

    let dir_x = vx / speed;
    let dir_y = vy / speed;
    let depth = (speed.ceil() as i64).clamp(1, options.look_ahead_max_depth_tiles as i64);
    let half_width = options.look_ahead_half_width_tiles.max(0);
    let center_x = (bounds.min_x + bounds.max_x) / 2;
    let center_y = (bounds.min_y + bounds.max_y) / 2;

    for step in 1..=depth {
        for lateral in -half_width..=half_width {
            let ahead_x = center_x
                + (dir_x * step as f64).round() as i64
                + (dir_y * lateral as f64).round() as i64;
            let ahead_y = center_y + (dir_y * step as f64).round() as i64
                - (dir_x * lateral as f64).round() as i64;
            if ahead_y < 0 || ahead_y >= bounds.limit {
                continue;
            }

            if let Some(id) = tile_at(bounds.zoom, ahead_x, ahead_y) {
                insert_best(planned, id, TilePlanPriority::LookAhead);
            }
        }
    }
}

fn add_idle_children(
    planned: &mut HashMap<TileId, TilePlanPriority>,
    visible_tiles: &[VisibleTile],
    levels: u32,
) {
    add_children(planned, visible_tiles, levels, TilePlanPriority::Child);
}

fn add_children(
    planned: &mut HashMap<TileId, TilePlanPriority>,
    visible_tiles: &[VisibleTile],
    levels: u32,
    priority: TilePlanPriority,
) {
    if levels == 0 {
        return;
    }

    for tile in visible_tiles {
        for level in 1..=levels {
            let Some(child_zoom) = tile.id.z.checked_add(level) else {
                continue;
            };
            if child_zoom > MAX_ZOOM {
                continue;
            }
            let scale = 1_u32 << level;
            let base_x = tile.id.x * scale;
            let base_y = tile.id.y * scale;
            for child_y in base_y..(base_y + scale) {
                for child_x in base_x..(base_x + scale) {
                    if let Ok(child) = TileId::new(child_zoom, child_x, child_y) {
                        insert_best(planned, child, priority);
                    }
                }
            }
        }
    }
}

fn tile_at(zoom: u32, x: i64, y: i64) -> Option<TileId> {
    let limit = i64::from(tile_count_per_axis(zoom).ok()?);
    if y < 0 || y >= limit {
        return None;
    }
    TileId::new(zoom, x.rem_euclid(limit) as u32, y as u32).ok()
}

fn unwrap_x(x: i64, base_x: i64, limit: i64) -> i64 {
    [x - limit, x, x + limit]
        .into_iter()
        .min_by_key(|candidate| (candidate - base_x).abs())
        .unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(z: u32, x: u32, y: u32) -> TileId {
        TileId::new(z, x, y).unwrap()
    }

    fn visible(id: TileId) -> VisibleTile {
        VisibleTile {
            id,
            screen_x_px: 0.0,
            screen_y_px: 0.0,
            size_px: 256.0,
        }
    }

    #[test]
    fn planner_keeps_visible_tiles_at_top_priority() {
        let plan = plan_tile_loads(&[visible(tile(3, 4, 4))], TilePlanOptions::default());

        assert_eq!(
            plan.priority_for(tile(3, 4, 4)),
            Some(TilePlanPriority::Visible)
        );
    }

    #[test]
    fn planner_adds_adaptive_periphery_buffer() {
        let plan = plan_tile_loads(
            &[
                visible(tile(5, 10, 10)),
                visible(tile(5, 11, 10)),
                visible(tile(5, 12, 10)),
                visible(tile(5, 10, 11)),
                visible(tile(5, 11, 11)),
                visible(tile(5, 12, 11)),
                visible(tile(5, 10, 12)),
                visible(tile(5, 11, 12)),
                visible(tile(5, 12, 12)),
                visible(tile(5, 10, 13)),
                visible(tile(5, 11, 13)),
                visible(tile(5, 12, 13)),
                visible(tile(5, 10, 14)),
                visible(tile(5, 11, 14)),
                visible(tile(5, 12, 14)),
                visible(tile(5, 10, 15)),
                visible(tile(5, 11, 15)),
                visible(tile(5, 12, 15)),
            ],
            TilePlanOptions::default(),
        );

        assert_eq!(
            plan.priority_for(tile(5, 5, 10)),
            Some(TilePlanPriority::Periphery)
        );
        assert_eq!(
            plan.priority_for(tile(5, 17, 10)),
            Some(TilePlanPriority::Periphery)
        );
        assert_eq!(
            plan.priority_for(tile(5, 10, 1)),
            Some(TilePlanPriority::Periphery)
        );
        assert_eq!(
            plan.priority_for(tile(5, 10, 24)),
            Some(TilePlanPriority::Periphery)
        );
        assert_eq!(plan.priority_for(tile(5, 4, 10)), None);
        assert_eq!(plan.priority_for(tile(5, 10, 0)), None);
    }

    #[test]
    fn planner_adds_look_ahead_from_velocity() {
        let plan = plan_tile_loads(
            &[visible(tile(4, 8, 8))],
            TilePlanOptions {
                moving: true,
                velocity_tiles_per_sec: (2.0, 0.0),
                ..TilePlanOptions::default()
            },
        );

        assert_eq!(
            plan.priority_for(tile(4, 9, 8)),
            Some(TilePlanPriority::LookAhead)
        );
    }

    #[test]
    fn planner_adds_parent_fallbacks_and_idle_children() {
        let plan = plan_tile_loads(&[visible(tile(4, 8, 8))], TilePlanOptions::default());

        assert_eq!(
            plan.priority_for(tile(3, 4, 4)),
            Some(TilePlanPriority::Fallback)
        );
        assert_eq!(
            plan.priority_for(tile(2, 2, 2)),
            Some(TilePlanPriority::Fallback)
        );
        assert_eq!(
            plan.priority_for(tile(1, 1, 1)),
            Some(TilePlanPriority::Fallback)
        );
        assert_eq!(
            plan.priority_for(tile(0, 0, 0)),
            Some(TilePlanPriority::Fallback)
        );
        assert_eq!(
            plan.priority_for(tile(5, 16, 16)),
            Some(TilePlanPriority::Child)
        );
    }

    #[test]
    fn planner_keeps_parent_fallbacks_higher_than_prefetch_tiles() {
        let plan = plan_tile_loads(
            &[visible(tile(4, 8, 8))],
            TilePlanOptions {
                moving: true,
                zoom_fraction: 0.9,
                ..TilePlanOptions::default()
            },
        );

        assert!(
            plan.priority_for(tile(3, 4, 4)).unwrap() < plan.priority_for(tile(5, 16, 16)).unwrap()
        );
        assert!(
            plan.priority_for(tile(3, 4, 4)).unwrap() < plan.priority_for(tile(4, 7, 8)).unwrap()
        );
    }

    #[test]
    fn planner_skips_children_while_moving() {
        let plan = plan_tile_loads(
            &[visible(tile(4, 8, 8))],
            TilePlanOptions {
                moving: true,
                ..TilePlanOptions::default()
            },
        );

        assert_eq!(plan.priority_for(tile(5, 16, 16)), None);
    }

    #[test]
    fn planner_prefetches_next_zoom_children_near_zoom_transition() {
        let plan = plan_tile_loads(
            &[visible(tile(4, 8, 8))],
            TilePlanOptions {
                moving: true,
                zoom_fraction: 0.8,
                ..TilePlanOptions::default()
            },
        );

        assert_eq!(
            plan.priority_for(tile(5, 16, 16)),
            Some(TilePlanPriority::LookAhead)
        );
    }

    #[test]
    fn planner_wraps_x_and_clamps_y_at_edges() {
        let plan = plan_tile_loads(
            &[visible(tile(2, 3, 1)), visible(tile(2, 0, 1))],
            TilePlanOptions::default(),
        );

        assert!(plan.contains(tile(2, 3, 1)));
        assert!(plan.contains(tile(2, 0, 1)));
        assert!(plan.contains(tile(2, 2, 1)));
        assert!(plan.contains(tile(2, 1, 1)));
        assert!(plan.tiles().iter().all(|planned| planned.id.y < 4));
    }

    #[test]
    fn planner_stays_inside_zoom_range() {
        let plan = plan_tile_loads(
            &[visible(tile(TileId::MAX_ZOOM, 0, 0))],
            TilePlanOptions::default(),
        );

        assert!(
            plan.tiles()
                .iter()
                .all(|planned| planned.id.z <= TileId::MAX_ZOOM)
        );
    }
}
