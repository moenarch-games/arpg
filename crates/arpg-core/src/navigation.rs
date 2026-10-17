//! Room-local pursuit navigation (#65, #110).
//!
//! ARPG owns the tactical path; Rapier's character controller moves the body along it. A room is
//! rasterized into cells whose body-sized footprint is clear of every fixed obstacle.
//!
//! The grid's topology (room bounds, obstacles, cell centres) is whole world units. Bodies
//! and targets are sampled at their authoritative `f32` positions ([`Point`]), never at a
//! rounded view of them (#182); the geometry on them is computed in `f64`, where the
//! differences and squares of `f32` coordinates are exact.
//!
//! Two planners search that grid:
//!
//! - [`RoomGrid::find_path`], the exact per-tick planner: the shared `graph-kernels` A*
//!   from the body's cell to a cell within reach of the exact target position with a
//!   clear strike line. It is the reference the retained planner is checked against.
//! - [`TargetField`], the retained planner: exact distances to the goal cells of the
//!   target's *cell*, and the canonical route that descends them. See the type's docs.

use std::{cmp::Reverse, collections::BinaryHeap};

use graph_kernels::astar;

/// Cell edge in world units.
pub(crate) const NAV_CELL_SIZE: i32 = 20;
const STRAIGHT_COST: u64 = 10;
const DIAGONAL_COST: u64 = 14;
/// How much longer than the exact planner's route a retained route may be (#110). A
/// retained route that cannot be shown to be within this bound is not used.
pub(crate) const ROUTE_SLACK: u64 = 2 * STRAIGHT_COST;

/// An axis-aligned XZ rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Rect {
    pub(crate) min_x: i32,
    pub(crate) max_x: i32,
    pub(crate) min_z: i32,
    pub(crate) max_z: i32,
}

impl Rect {
    pub(crate) fn centered(x: i32, z: i32, half_x: i32, half_z: i32) -> Self {
        Self {
            min_x: x - half_x,
            max_x: x + half_x,
            min_z: z - half_z,
            max_z: z + half_z,
        }
    }

    /// Strict overlap: rectangles that only touch do not overlap.
    fn overlaps(self, other: Self) -> bool {
        self.min_x < other.max_x
            && other.min_x < self.max_x
            && self.min_z < other.max_z
            && other.min_z < self.max_z
    }

    fn corners(self) -> [(i32, i32); 4] {
        [
            (self.min_x, self.min_z),
            (self.max_x, self.min_z),
            (self.min_x, self.max_z),
            (self.max_x, self.max_z),
        ]
    }

    #[cfg(test)]
    fn contains(self, (x, z): (i32, i32)) -> bool {
        self.min_x <= x && x <= self.max_x && self.min_z <= z && z <= self.max_z
    }
}

/// Whether every segment from `apex` to a point of `area` stays clear of `obstacle`.
///
/// Those segments fill the convex hull of `apex` and `area`, so this is a separating-axis
/// test of that hull against the obstacle. The obstacle is closed and grown by one unit,
/// so touching or grazing it counts as blocked.
pub(crate) fn fan_is_clear(apex: (i32, i32), area: Rect, obstacle: Rect) -> bool {
    let obstacle = Rect {
        min_x: obstacle.min_x - 1,
        max_x: obstacle.max_x + 1,
        min_z: obstacle.min_z - 1,
        max_z: obstacle.max_z + 1,
    };
    let hull = [
        apex,
        (area.min_x, area.min_z),
        (area.max_x, area.min_z),
        (area.min_x, area.max_z),
        (area.max_x, area.max_z),
    ];
    // The hull's edges run along the axes or from the apex to a corner; the obstacle's
    // edges run along the axes. Extra axes never make the test wrong.
    let axes = [(1_i64, 0_i64), (0, 1)].into_iter().chain(
        area.corners()
            .into_iter()
            .map(|(x, z)| (-i64::from(z - apex.1), i64::from(x - apex.0))),
    );
    let project = |points: &[(i32, i32)], (ax, az): (i64, i64)| {
        points
            .iter()
            .fold((i64::MAX, i64::MIN), |(low, high), &(x, z)| {
                let value = i64::from(x) * ax + i64::from(z) * az;
                (low.min(value), high.max(value))
            })
    };
    axes.filter(|&axis| axis != (0, 0)).any(|axis| {
        let (hull_low, hull_high) = project(&hull, axis);
        let (low, high) = project(&obstacle.corners(), axis);
        hull_high < low || high < hull_low
    })
}

pub(crate) type Cell = (i32, i32);

/// A position on the plane in world units: a body's or target's authoritative `f32`
/// position, or a whole-unit layout point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f32,
    pub(crate) z: f32,
}

impl From<(f32, f32)> for Point {
    fn from((x, z): (f32, f32)) -> Self {
        Self { x, z }
    }
}

impl From<(i32, i32)> for Point {
    fn from((x, z): (i32, i32)) -> Self {
        // Layout coordinates lie far inside the range `f32` represents exactly.
        Self {
            x: x as f32,
            z: z as f32,
        }
    }
}

impl Rect {
    /// Whether `point` lies in the closed rectangle.
    fn contains_point(self, point: Point) -> bool {
        f64::from(self.min_x) <= f64::from(point.x)
            && f64::from(point.x) <= f64::from(self.max_x)
            && f64::from(self.min_z) <= f64::from(point.z)
            && f64::from(point.z) <= f64::from(self.max_z)
    }
}

/// Passability of one room for a body with the given XZ clearance (half extent).
#[derive(Clone)]
pub(crate) struct RoomGrid {
    bounds: Rect,
    columns: i32,
    rows: i32,
    free: Vec<bool>,
}

impl RoomGrid {
    pub(crate) fn new(bounds: Rect, obstacles: &[Rect], clearance: i32) -> Self {
        let columns = ((bounds.max_x - bounds.min_x) / NAV_CELL_SIZE).max(1);
        let rows = ((bounds.max_z - bounds.min_z) / NAV_CELL_SIZE).max(1);
        let mut grid = Self {
            bounds,
            columns,
            rows,
            free: Vec::with_capacity(usize::try_from(columns * rows).unwrap_or(0)),
        };
        for row in 0..rows {
            for column in 0..columns {
                let (x, z) = grid.cell_center((column, row));
                let footprint = Rect::centered(x, z, clearance, clearance);
                let inside = footprint.min_x >= bounds.min_x
                    && footprint.max_x <= bounds.max_x
                    && footprint.min_z >= bounds.min_z
                    && footprint.max_z <= bounds.max_z;
                grid.free.push(
                    inside
                        && !obstacles
                            .iter()
                            .any(|obstacle| footprint.overlaps(*obstacle)),
                );
            }
        }
        grid
    }

    fn cell_center(&self, (column, row): Cell) -> (i32, i32) {
        (
            self.bounds.min_x + column * NAV_CELL_SIZE + NAV_CELL_SIZE / 2,
            self.bounds.min_z + row * NAV_CELL_SIZE + NAV_CELL_SIZE / 2,
        )
    }

    /// The cell whose half-open span holds `point`, clamped into the grid.
    fn cell_at(&self, point: impl Into<Point>) -> Cell {
        let point = point.into();
        let index = |value: f32, min: i32, count: i32| {
            // `as` saturates, and the clamp keeps any position inside the grid.
            let cell = ((f64::from(value) - f64::from(min)) / f64::from(NAV_CELL_SIZE)).floor();
            (cell as i32).clamp(0, count - 1)
        };
        (
            index(point.x, self.bounds.min_x, self.columns),
            index(point.z, self.bounds.min_z, self.rows),
        )
    }

    fn is_free(&self, (column, row): Cell) -> bool {
        column >= 0
            && row >= 0
            && column < self.columns
            && row < self.rows
            && self.free[usize::try_from(row * self.columns + column).unwrap_or(usize::MAX)]
    }

    /// Whether every cell the straight segment passes through is free, including both
    /// neighbours where it crosses exactly through a cell corner.
    pub(crate) fn segment_is_free(&self, from: impl Into<Point>, to: impl Into<Point>) -> bool {
        let (from, to) = (from.into(), to.into());
        let (mut column, mut row) = self.cell_at(from);
        let end = self.cell_at(to);
        if !self.is_free((column, row)) {
            return false;
        }
        let x0 = f64::from(from.x) - f64::from(self.bounds.min_x);
        let z0 = f64::from(from.z) - f64::from(self.bounds.min_z);
        let dx = f64::from(to.x) - f64::from(from.x);
        let dz = f64::from(to.z) - f64::from(from.z);
        let step = |delta: f64| i32::from(delta > 0.0) - i32::from(delta < 0.0);
        let (step_x, step_z) = (step(dx), step(dz));
        let size = f64::from(NAV_CELL_SIZE);
        for _ in 0..=(self.columns + self.rows) {
            if (column, row) == end {
                return true;
            }
            // Compare the parameters of the next x and z boundary crossings.
            let boundary = |cell: i32, step: i32| f64::from(cell + i32::from(step > 0)) * size;
            let to_x = if step_x == 0 {
                f64::INFINITY
            } else {
                (boundary(column, step_x) - x0).abs() * dz.abs()
            };
            let to_z = if step_z == 0 {
                f64::INFINITY
            } else {
                (boundary(row, step_z) - z0).abs() * dx.abs()
            };
            if to_x < to_z {
                column += step_x;
            } else if to_z < to_x {
                row += step_z;
            } else {
                if !self.is_free((column + step_x, row)) || !self.is_free((column, row + step_z)) {
                    return false;
                }
                column += step_x;
                row += step_z;
            }
            if !self.is_free((column, row)) {
                return false;
            }
        }
        false
    }

    /// The free cell holding `point`, or the first free neighbour in a fixed order: a body
    /// resting against an obstacle can sit in a cell whose centre is too close to it.
    fn free_cell_near(&self, point: Point) -> Option<Cell> {
        let cell = self.cell_at(point);
        std::iter::once(cell)
            .chain(neighbour_offsets().map(|(dx, dz, _)| (cell.0 + dx, cell.1 + dz)))
            .find(|&cell| self.is_free(cell))
    }

    /// Cell centres from `start` to a free cell within `reach` of `target` from which
    /// `clear_line` holds. `None` when no such cell is reachable. The line test is the
    /// caller's: the target itself may stand where a monster body does not fit.
    /// `searched` counts A* expansions.
    pub(crate) fn find_path(
        &self,
        start: impl Into<Point>,
        target: impl Into<Point>,
        reach: i64,
        mut clear_line: impl FnMut((i32, i32)) -> bool,
        searched: &mut u64,
    ) -> Option<Vec<(i32, i32)>> {
        let target = target.into();
        let start = self.free_cell_near(start.into())?;
        let target_cell = self.cell_at(target);
        let reach_sq = range_sq(reach);
        let reach_cost = u64::try_from(reach).unwrap_or(0) * STRAIGHT_COST
            / u64::from(NAV_CELL_SIZE.unsigned_abs());
        let path = astar(
            start,
            |&cell| {
                let centre = self.cell_center(cell);
                point_distance_sq(centre, target) <= reach_sq && clear_line(centre)
            },
            |&cell| {
                *searched += 1;
                self.moves(cell).collect::<Vec<_>>()
            },
            |&cell| octile_cost(cell, target_cell).saturating_sub(reach_cost),
        )?;
        Some(
            path.nodes
                .into_iter()
                .map(|cell| self.cell_center(cell))
                .collect(),
        )
    }

    /// Cell centres from `start` to the nearest free cell whose centre lies `near..=far`
    /// from `threat` and from which `clear_line` holds (#108), through cells no nearer to
    /// `threat` than the start cell: a retreat backs away and never passes the threat.
    /// `None` when no such cell is reachable that way. `searched` counts A* expansions.
    pub(crate) fn find_retreat(
        &self,
        start: impl Into<Point>,
        threat: impl Into<Point>,
        (near, far): (i64, i64),
        mut clear_line: impl FnMut((i32, i32)) -> bool,
        searched: &mut u64,
    ) -> Option<Vec<(i32, i32)>> {
        let threat = threat.into();
        let start = self.free_cell_near(start.into())?;
        let from_threat_sq = |cell: Cell| point_distance_sq(self.cell_center(cell), threat);
        let floor_sq = from_threat_sq(start);
        let band = range_sq(near)..=range_sq(far);
        let path = astar(
            start,
            |&cell| band.contains(&from_threat_sq(cell)) && clear_line(self.cell_center(cell)),
            |&cell| {
                *searched += 1;
                self.moves(cell)
                    .filter(|&(next, _)| from_threat_sq(next) >= floor_sq)
                    .collect::<Vec<_>>()
            },
            // Backing off by d units takes a route of at least d units, and no step costs
            // less than 0.49 per unit (a diagonal: 14 per 20√2).
            |&cell| {
                let short = near as f64 - from_threat_sq(cell).sqrt();
                // Saturating: a cell beyond `near` needs no more backing off.
                (short.max(0.0) * 0.49).floor() as u64
            },
        )?;
        Some(
            path.nodes
                .into_iter()
                .map(|cell| self.cell_center(cell))
                .collect(),
        )
    }
}

impl RoomGrid {
    /// Allowed moves from `cell`, in the fixed neighbour order. Moves are symmetric.
    fn moves(&self, cell: Cell) -> impl Iterator<Item = (Cell, u64)> + '_ {
        neighbour_offsets()
            .filter(move |&(dx, dz, _)| {
                // No corner cutting: a diagonal needs both orthogonal cells free.
                self.is_free((cell.0 + dx, cell.1 + dz))
                    && (dx == 0
                        || dz == 0
                        || (self.is_free((cell.0 + dx, cell.1))
                            && self.is_free((cell.0, cell.1 + dz))))
            })
            .map(move |(dx, dz, cost)| ((cell.0 + dx, cell.1 + dz), cost))
    }

    fn index(&self, (column, row): Cell) -> usize {
        usize::try_from(row * self.columns + column).expect("cells lie inside the grid")
    }

    fn cell_count(&self) -> usize {
        usize::try_from(self.columns * self.rows).unwrap_or(0)
    }

    /// The closed span holding every position inside the room that `cell_at` maps to
    /// `cell`; the last column and row absorb the remainder of the room. Neighbouring
    /// spans share their boundary.
    fn cell_area(&self, (column, row): Cell) -> Rect {
        let low = |min: i32, index: i32| min + index * NAV_CELL_SIZE;
        let high = |min: i32, max: i32, index: i32, count: i32| {
            if index == count - 1 {
                max
            } else {
                low(min, index) + NAV_CELL_SIZE
            }
        };
        Rect {
            min_x: low(self.bounds.min_x, column),
            max_x: high(self.bounds.min_x, self.bounds.max_x, column, self.columns),
            min_z: low(self.bounds.min_z, row),
            max_z: high(self.bounds.min_z, self.bounds.max_z, row, self.rows),
        }
    }

    /// The cell holding `target`, or `None` when the target is outside the room's cells
    /// (then only the exact planner applies).
    pub(crate) fn target_cell(&self, target: impl Into<Point>) -> Option<Cell> {
        let target = target.into();
        let cell = self.cell_at(target);
        self.cell_area(cell).contains_point(target).then_some(cell)
    }

    /// The retained planner's field for a target anywhere in `target_cell`, whose goals are
    /// the *strict* goal cells: free cells whose centre is within `reach` of every point of
    /// the target's cell and whose straight lines to every such point clear `obstacles`
    /// (see [`fan_is_clear`]). With `obstacles` the fixed bodies the strike line tests, a
    /// strict goal is a goal of [`Self::find_path`] for every target position in the cell.
    pub(crate) fn target_field(
        &self,
        target_cell: Cell,
        reach: i64,
        obstacles: &[Rect],
    ) -> TargetField {
        let area = self.cell_area(target_cell);
        let span = goal_span(reach);
        let near = Rect {
            min_x: area.min_x - (span + 1) * NAV_CELL_SIZE,
            max_x: area.max_x + (span + 1) * NAV_CELL_SIZE,
            min_z: area.min_z - (span + 1) * NAV_CELL_SIZE,
            max_z: area.max_z + (span + 1) * NAV_CELL_SIZE,
        };
        let nearby = obstacles
            .iter()
            .copied()
            .filter(|obstacle| obstacle.overlaps(near))
            .collect::<Vec<_>>();
        let strict = self.distances(target_cell, reach, |centre| {
            area.corners()
                .iter()
                .all(|&corner| distance_sq(centre, corner) <= reach * reach)
                && nearby
                    .iter()
                    .all(|&obstacle| fan_is_clear(centre, area, obstacle))
        });
        TargetField {
            target_cell,
            reach,
            covered: vec![false; strict.distance.len()],
            strict,
            loose: None,
        }
    }

    /// Unknown distances to the free cells near `target_cell` whose centre passes `is_goal`.
    fn distances(
        &self,
        target_cell: Cell,
        reach: i64,
        mut is_goal: impl FnMut((i32, i32)) -> bool,
    ) -> Distances {
        let span = goal_span(reach);
        let mut goals = Vec::new();
        let mut goal = vec![false; self.cell_count()];
        for row in (target_cell.1 - span).max(0)..=(target_cell.1 + span).min(self.rows - 1) {
            for column in
                (target_cell.0 - span).max(0)..=(target_cell.0 + span).min(self.columns - 1)
            {
                let cell = (column, row);
                if self.is_free(cell) && is_goal(self.cell_center(cell)) {
                    goal[self.index(cell)] = true;
                    goals.push(cell);
                }
            }
        }
        Distances {
            distance: vec![u64::MAX; goal.len()],
            goal,
            goals,
            complete: false,
        }
    }

    /// The retained planner's route from `start` (see [`TargetField`]), searching only for
    /// distances the field does not know yet.
    pub(crate) fn field_route(
        &self,
        field: &mut TargetField,
        start: impl Into<Point>,
        work: &mut FieldWork,
    ) -> FieldRoute {
        let Some(start) = self.free_cell_near(start.into()) else {
            return FieldRoute::NoStartCell;
        };
        let index = self.index(start);
        if field.strict.goal[index] {
            return FieldRoute::AtGoal;
        }
        if !field.strict.complete && !field.covered[index] {
            work.searches += 1;
            self.search(&mut field.strict, start, true, &mut work.expansions);
        }
        let distance = field.strict.distance[index];
        let route = if distance == u64::MAX {
            None
        } else {
            // A descent always exists from a searched or covered cell; should that ever fail,
            // the exact planner decides instead of a route that depends on the cache.
            let Some(route) = self.descend(&field.strict, start) else {
                debug_assert!(false, "no canonical descent from a known cell");
                return FieldRoute::Unknown;
            };
            for &cell in &route {
                field.covered[self.index(cell)] = true;
            }
            Some(route)
        };
        // The exact planner's goals for any target position in the cell are loose goals, so
        // the loose distance bounds its route cost from below. The octile cost to the
        // nearest loose goal bounds that in turn and often settles the question unsearched.
        let within_slack = |bound: u64| distance <= bound.saturating_add(ROUTE_SLACK);
        if route.is_some() && within_slack(self.loose_octile_bound(field, start)) {
            return FieldRoute::path(self, route);
        }
        let loose = field
            .loose
            .get_or_insert_with(|| self.loose_distances(field.target_cell, field.reach));
        if !loose.complete && loose.distance[index] == u64::MAX {
            work.searches += 1;
            self.search(loose, start, false, &mut work.expansions);
        }
        match (route, loose.distance[index]) {
            (_, u64::MAX) => FieldRoute::NoRoute,
            (None, _) => FieldRoute::Unknown,
            (Some(route), bound) if within_slack(bound) => FieldRoute::path(self, Some(route)),
            (Some(_), _) => FieldRoute::Unknown,
        }
    }

    /// Free cells within `reach` of some point of the target's cell: the exact planner's
    /// goals for every target position in it are among them.
    fn loose_distances(&self, target_cell: Cell, reach: i64) -> Distances {
        let area = self.cell_area(target_cell);
        self.distances(target_cell, reach, |(x, z)| {
            let nearest = (
                x.clamp(area.min_x, area.max_x),
                z.clamp(area.min_z, area.max_z),
            );
            distance_sq((x, z), nearest) <= reach * reach
        })
    }

    /// The octile cost, around no obstacle, from `start` to the nearest loose goal cell.
    fn loose_octile_bound(&self, field: &TargetField, start: Cell) -> u64 {
        let area = self.cell_area(field.target_cell);
        let span = goal_span(field.reach);
        let (target_column, target_row) = field.target_cell;
        let mut bound = u64::MAX;
        for row in (target_row - span).max(0)..=(target_row + span).min(self.rows - 1) {
            for column in
                (target_column - span).max(0)..=(target_column + span).min(self.columns - 1)
            {
                let cell = (column, row);
                let (x, z) = self.cell_center(cell);
                let nearest = (
                    x.clamp(area.min_x, area.max_x),
                    z.clamp(area.min_z, area.max_z),
                );
                if distance_sq((x, z), nearest) <= field.reach * field.reach && self.is_free(cell) {
                    bound = bound.min(octile_cost(start, cell));
                }
            }
        }
        bound
    }

    /// Multi-source A* from the goal cells toward `start`, with the octile distance to
    /// `start` as a consistent heuristic, so every closed cell's distance is exact.
    ///
    /// With `all_shortest`, the search continues past `start` until every cell whose
    /// estimate does not exceed the start's distance is closed: every cell on *any*
    /// shortest route from the start, and from each cell of such a route, is then known,
    /// which the canonical descent needs.
    fn search(
        &self,
        distances: &mut Distances,
        start: Cell,
        all_shortest: bool,
        expansions: &mut u64,
    ) {
        let mut tentative = vec![u64::MAX; distances.goal.len()];
        let mut closed = vec![false; distances.goal.len()];
        // Min-heap on (estimate, cost descending, cell): deeper cells first among equals.
        let mut open = BinaryHeap::new();
        for &cell in &distances.goals {
            tentative[self.index(cell)] = 0;
            open.push(Reverse((octile_cost(cell, start), u64::MAX, cell)));
        }
        let start_index = self.index(start);
        let mut limit = None;
        while let Some(Reverse((estimate, inverted_cost, cell))) = open.pop() {
            let index = self.index(cell);
            let cost = u64::MAX - inverted_cost;
            if closed[index] || cost != tentative[index] {
                continue;
            }
            if limit.is_some_and(|limit| estimate > limit) {
                return;
            }
            closed[index] = true;
            distances.distance[index] = cost;
            *expansions += 1;
            if index == start_index {
                if !all_shortest {
                    return;
                }
                limit = Some(cost);
            }
            for (next, step) in self.moves(cell) {
                let next_index = self.index(next);
                let next_cost = cost + step;
                if !closed[next_index] && next_cost < tentative[next_index] {
                    tentative[next_index] = next_cost;
                    open.push(Reverse((
                        next_cost + octile_cost(next, start),
                        u64::MAX - next_cost,
                        next,
                    )));
                }
            }
        }
        // Exhausted: every cell that can reach a goal cell is closed.
        distances.complete = true;
    }

    /// From `start`, the first move in neighbour order that stays on a shortest route, until
    /// a goal cell.
    fn descend(&self, distances: &Distances, start: Cell) -> Option<Vec<Cell>> {
        let mut route = vec![start];
        let mut cell = start;
        loop {
            let distance = distances.distance[self.index(cell)];
            if distance == 0 {
                return Some(route);
            }
            cell = self
                .moves(cell)
                .find(|&(next, step)| {
                    let next = distances.distance[self.index(next)];
                    next != u64::MAX && next + step == distance
                })?
                .0;
            route.push(cell);
        }
    }
}

/// Cells around the target's cell that can hold a goal: a centre `span + 1` cells away is
/// more than `reach` from every point of the target's cell, even a widened outer one.
fn goal_span(reach: i64) -> i32 {
    i32::try_from(reach).unwrap_or(i32::MAX - 2 * NAV_CELL_SIZE) / NAV_CELL_SIZE + 2
}

fn distance_sq(from: (i32, i32), to: (i32, i32)) -> i64 {
    let dx = i64::from(to.0 - from.0);
    let dz = i64::from(to.1 - from.1);
    dx * dx + dz * dz
}

/// Squared planar distance from a layout point to a position.
fn point_distance_sq(from: (i32, i32), to: Point) -> f64 {
    let dx = f64::from(to.x) - f64::from(from.0);
    let dz = f64::from(to.z) - f64::from(from.1);
    dx * dx + dz * dz
}

/// A whole-unit range, squared, for comparison with `f64` squared distances.
fn range_sq(range: i64) -> f64 {
    let range = range as f64;
    range * range
}

#[cfg(test)]
impl TargetField {
    /// Centres of the strict goal cells, and the closed span of the target cell.
    pub(crate) fn strict_goal_centres(&self, grid: &RoomGrid) -> (Vec<(i32, i32)>, Rect) {
        (
            self.strict
                .goals
                .iter()
                .map(|&cell| grid.cell_center(cell))
                .collect(),
            grid.cell_area(self.target_cell),
        )
    }
}

/// Distances to a set of goal cells, filled in by searches. Every known distance is exact.
#[derive(Clone, Debug)]
struct Distances {
    goal: Vec<bool>,
    goals: Vec<Cell>,
    /// Exact distance to the nearest goal cell, `u64::MAX` while unknown.
    distance: Vec<u64>,
    /// Every cell that can reach a goal cell has a known distance.
    complete: bool,
}

/// Navigation toward one target cell, retained across ticks (#110).
///
/// The route from a cell is the *canonical descent* to the strict goal cells: from each
/// cell, the first move in the fixed neighbour order whose next cell is exactly one step
/// nearer to a strict goal. It is a shortest route to them and a pure function of the
/// room's fixed bodies, the target's cell and the start cell. The field only decides how
/// much has to be searched: the route from any cell of an earlier route is that route's
/// suffix, so a body walking its route needs no further search. Dropping a field, at any
/// tick or across a save and load, changes work counts but never a result.
#[derive(Clone, Debug)]
pub(crate) struct TargetField {
    target_cell: Cell,
    reach: i64,
    strict: Distances,
    /// Cells of an earlier route: every cell on any shortest route from them is known.
    covered: Vec<bool>,
    /// Distances to the loose goal cells, built when the octile bound is not enough.
    loose: Option<Distances>,
}

/// Work done by [`RoomGrid::field_route`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FieldWork {
    pub(crate) searches: u64,
    pub(crate) expansions: u64,
}

/// The retained planner's answer for one start (see [`RoomGrid::field_route`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FieldRoute {
    /// No free cell at or beside the start: the exact planner finds no route either.
    NoStartCell,
    /// No cell within reach of the target's cell is reachable: the exact planner finds no
    /// route either.
    NoRoute,
    /// The start cell is a strict goal: the exact planner finishes the approach.
    AtGoal,
    /// No strict goal is reachable, or the route to one may cost more than
    /// [`ROUTE_SLACK`] above the exact planner's: the exact planner decides.
    Unknown,
    /// Cell centres from the start cell to a strict goal, costing at most [`ROUTE_SLACK`]
    /// more than the exact planner's route.
    Path(Vec<(i32, i32)>),
}

impl FieldRoute {
    fn path(grid: &RoomGrid, route: Option<Vec<Cell>>) -> Self {
        Self::Path(
            route
                .expect("a route within slack exists")
                .into_iter()
                .map(|cell| grid.cell_center(cell))
                .collect(),
        )
    }
}

/// Cost of a route of adjacent cell centres, in the planners' step costs.
#[cfg(test)]
pub(crate) fn route_cost(route: &[(i32, i32)]) -> u64 {
    route
        .windows(2)
        .map(|step| {
            if step[0].0 != step[1].0 && step[0].1 != step[1].1 {
                DIAGONAL_COST
            } else {
                STRAIGHT_COST
            }
        })
        .sum()
}

fn neighbour_offsets() -> impl Iterator<Item = (i32, i32, u64)> {
    [
        (1, 0, STRAIGHT_COST),
        (-1, 0, STRAIGHT_COST),
        (0, 1, STRAIGHT_COST),
        (0, -1, STRAIGHT_COST),
        (1, 1, DIAGONAL_COST),
        (1, -1, DIAGONAL_COST),
        (-1, 1, DIAGONAL_COST),
        (-1, -1, DIAGONAL_COST),
    ]
    .into_iter()
}

fn octile_cost(from: Cell, to: Cell) -> u64 {
    let dx = u64::from((from.0 - to.0).unsigned_abs());
    let dz = u64::from((from.1 - to.1).unsigned_abs());
    STRAIGHT_COST * dx.max(dz) + (DIAGONAL_COST - STRAIGHT_COST) * dx.min(dz)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: Rect = Rect {
        min_x: 0,
        max_x: 400,
        min_z: 0,
        max_z: 400,
    };

    /// Independent reference: breadth-first reachability over the same free cells.
    fn reachable_cells(grid: &RoomGrid, start: Cell) -> usize {
        let mut seen = vec![start];
        let mut frontier = vec![start];
        while let Some(cell) = frontier.pop() {
            for (dx, dz, _) in neighbour_offsets().filter(|(dx, dz, _)| *dx == 0 || *dz == 0) {
                let next = (cell.0 + dx, cell.1 + dz);
                if grid.is_free(next) && !seen.contains(&next) {
                    seen.push(next);
                    frontier.push(next);
                }
            }
        }
        seen.len()
    }

    #[test]
    fn clearance_blocks_cells_whose_footprint_touches_walls_or_obstacles() {
        let pillar = Rect::centered(200, 200, 20, 60);
        let grid = RoomGrid::new(ROOM, &[pillar], 30);

        assert!(!grid.is_free(grid.cell_at((10, 200))));
        assert!(!grid.is_free(grid.cell_at((200, 200))));
        assert!(!grid.is_free(grid.cell_at((235, 200))));
        assert!(grid.is_free(grid.cell_at((260, 200))));
        assert!(grid.is_free(grid.cell_at((60, 60))));
    }

    /// Bodies and targets are sampled at their `f32` positions (#182): 19.6 lies in the
    /// first cell, where a rounded view (20) would have put it in the second.
    #[test]
    fn grid_sampling_uses_exact_positions() {
        let grid = RoomGrid::new(ROOM, &[], 30);
        assert_eq!(grid.cell_at((19.6_f32, 0.4_f32)), (0, 0));
        assert_eq!(grid.cell_at((20.0_f32, 0.4_f32)), (1, 0));
        assert_eq!(grid.target_cell((19.6_f32, 39.9_f32)), Some((0, 1)));
        // Outside the room's span there is no target cell, even within rounding of it.
        assert_eq!(grid.target_cell((-0.4_f32, 100.0_f32)), None);
        assert_eq!(grid.target_cell((400.4_f32, 100.0_f32)), None);
        assert_eq!(grid.target_cell((400.0_f32, 100.0_f32)), Some((19, 5)));
        // The goal test measures from the cell centre to the exact target position: the
        // first cell's centre (10, 10) is exactly 50 units from (40, 50), and about 50.3
        // from (40, 50.4), which a rounded view would put back at (40, 50).
        let grid = RoomGrid::new(ROOM, &[], 5);
        let mut searched = 0;
        let reaches = |target: (f32, f32), searched: &mut u64| {
            grid.find_path((10, 10), target, 50, |centre| centre == (10, 10), searched)
                .is_some()
        };
        assert!(reaches((40.0, 50.0), &mut searched));
        assert!(!reaches((40.0, 50.4), &mut searched));
    }

    #[test]
    fn path_goes_around_an_obstacle_and_ends_with_a_clear_line_in_reach() {
        let wall = Rect::centered(200, 160, 20, 160);
        let grid = RoomGrid::new(ROOM, &[wall], 30);
        let mut searched = 0;

        let path = grid
            .find_path(
                (100, 100),
                (300, 100),
                60,
                |point| grid.segment_is_free(point, (300, 100)),
                &mut searched,
            )
            .expect("the target is reachable around the wall");

        assert!(searched > 0);
        assert!(
            path.iter().any(|&(_, z)| z > 320),
            "the path rounds the wall end"
        );
        let &(x, z) = path.last().unwrap();
        assert!(i64::from((300 - x).pow(2) + (100 - z).pow(2)) <= 60 * 60);
        assert!(grid.segment_is_free((x, z), (300, 100)));
        assert!(
            path.windows(2)
                .all(|step| grid.segment_is_free(step[0], step[1]))
        );
    }

    #[test]
    fn enclosed_targets_are_unreachable_like_the_reference_search_says() {
        let walls = [
            Rect::centered(300, 200, 10, 120),
            Rect::centered(200, 330, 110, 10),
            Rect::centered(200, 70, 110, 10),
        ];
        let open = RoomGrid::new(ROOM, &walls, 30);
        let mut closed_walls = walls.to_vec();
        closed_walls.push(Rect::centered(100, 200, 10, 120));
        let closed = RoomGrid::new(ROOM, &closed_walls, 30);
        let mut searched = 0;

        assert!(
            open.find_path(
                (30, 30),
                (200, 200),
                40,
                |point| open.segment_is_free(point, (200, 200)),
                &mut searched
            )
            .is_some()
        );
        assert!(
            closed
                .find_path(
                    (30, 30),
                    (200, 200),
                    40,
                    |point| closed.segment_is_free(point, (200, 200)),
                    &mut searched
                )
                .is_none()
        );
        let start = closed.cell_at((30, 30));
        let target = closed.cell_at((200, 200));
        assert!(closed.is_free(target));
        assert!(reachable_cells(&closed, start) < reachable_cells(&open, start));
    }

    #[test]
    fn identical_inputs_give_identical_paths() {
        let pillar = Rect::centered(200, 200, 20, 60);
        let grid = RoomGrid::new(ROOM, &[pillar], 30);
        let mut searched = 0;
        let first = grid.find_path(
            (60, 200),
            (340, 200),
            50,
            |point| grid.segment_is_free(point, (340, 200)),
            &mut searched,
        );
        let second = grid.find_path(
            (60, 200),
            (340, 200),
            50,
            |point| grid.segment_is_free(point, (340, 200)),
            &mut searched,
        );

        assert!(first.is_some());
        assert_eq!(first, second);
    }

    /// Deterministic xorshift for the randomized comparisons.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, bound: i32) -> i32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            i32::try_from((self.0 >> 33) % u64::from(bound.unsigned_abs())).unwrap()
        }
    }

    const GAME_CLEARANCE: i32 = 30 + NAV_CELL_SIZE;
    const GOAL_REACH: i64 = 160;

    fn random_room(rng: &mut Rng) -> (RoomGrid, Vec<Rect>) {
        let bounds = Rect {
            min_x: -400,
            max_x: 400 + 20 * rng.below(10),
            min_z: -300,
            max_z: 300 + 20 * rng.below(5),
        };
        let obstacles = (0..3 + rng.below(6))
            .map(|_| {
                Rect::centered(
                    bounds.min_x + rng.below(bounds.max_x - bounds.min_x),
                    bounds.min_z + rng.below(bounds.max_z - bounds.min_z),
                    10 + rng.below(60),
                    10 + rng.below(60),
                )
            })
            .collect::<Vec<_>>();
        (RoomGrid::new(bounds, &obstacles, GAME_CLEARANCE), obstacles)
    }

    /// A point where a player-sized body (half extent 25) stands clear of every obstacle.
    fn random_point(rng: &mut Rng, grid: &RoomGrid, obstacles: &[Rect]) -> (i32, i32) {
        loop {
            let point = (
                grid.bounds.min_x + 25 + rng.below(grid.bounds.max_x - grid.bounds.min_x - 50),
                grid.bounds.min_z + 25 + rng.below(grid.bounds.max_z - grid.bounds.min_z - 50),
            );
            if stands_clear(point, obstacles) {
                return point;
            }
        }
    }

    fn stands_clear(point: (i32, i32), obstacles: &[Rect]) -> bool {
        let body = Rect::centered(point.0, point.1, 25, 25);
        obstacles.iter().all(|obstacle| !body.overlaps(*obstacle))
    }

    /// The exact planner as the game calls it, with the strike line as an exact segment test.
    fn reference_path(
        grid: &RoomGrid,
        obstacles: &[Rect],
        start: (i32, i32),
        target: (i32, i32),
        expansions: &mut u64,
    ) -> Option<Vec<(i32, i32)>> {
        let point = Rect::centered(target.0, target.1, 0, 0);
        grid.find_path(
            start,
            target,
            GOAL_REACH,
            |centre| {
                obstacles
                    .iter()
                    .all(|&obstacle| fan_is_clear(centre, point, obstacle))
            },
            expansions,
        )
    }

    #[test]
    fn fans_clear_only_when_every_segment_misses_the_obstacle() {
        let pillar = Rect::centered(100, 0, 10, 10);
        let cell = Rect {
            min_x: 200,
            max_x: 219,
            min_z: -9,
            max_z: 10,
        };
        assert!(!fan_is_clear((0, 0), cell, pillar));
        assert!(fan_is_clear((0, 200), cell, pillar));
        // Grazing a corner counts as blocked.
        assert!(!fan_is_clear(
            (0, 11),
            Rect::centered(200, 11, 0, 0),
            pillar
        ));
        assert!(fan_is_clear((0, 12), Rect::centered(200, 12, 0, 0), pillar));
    }

    /// The acceptance comparison of #110: retained routes against freshly built fields, and
    /// against the per-tick exact planner for reachability and cost, while targets move
    /// across cells and bodies walk their routes or are knocked off them.
    #[test]
    fn retained_routes_match_fresh_fields_and_the_per_tick_reference() {
        let mut rng = Rng(0x0110_5EED);
        let (mut queries, mut routed, mut kinds) = (0_u64, 0_u64, [0_u64; 4]);
        let (mut max_excess, mut total_excess) = (0_u64, 0_u64);
        let (mut retained_work, mut reference_expansions) = (FieldWork::default(), 0_u64);
        for _ in 0..120 {
            let (grid, obstacles) = random_room(&mut rng);
            let mut target = random_point(&mut rng, &grid, &obstacles);
            let mut start = random_point(&mut rng, &grid, &obstacles);
            let mut fields = std::collections::BTreeMap::new();
            for _ in 0..40 {
                match rng.below(10) {
                    // Knocked off its route.
                    0 => start = random_point(&mut rng, &grid, &obstacles),
                    // The target walks, often into another cell.
                    1..=4 => {
                        let step = (target.0 + rng.below(29) - 14, target.1 + rng.below(29) - 14);
                        if grid.bounds.contains(step) && stands_clear(step, &obstacles) {
                            target = step;
                        }
                    }
                    _ => {}
                }
                let mut exact_expansions = 0;
                let reference =
                    reference_path(&grid, &obstacles, start, target, &mut exact_expansions);
                reference_expansions += exact_expansions;
                let Some(cell) = grid.target_cell(target) else {
                    continue;
                };
                queries += 1;
                let field = fields
                    .entry(cell)
                    .or_insert_with(|| grid.target_field(cell, GOAL_REACH, &obstacles));
                let route = grid.field_route(field, start, &mut retained_work);
                let mut fresh = grid.target_field(cell, GOAL_REACH, &obstacles);
                let fresh = grid.field_route(&mut fresh, start, &mut FieldWork::default());
                assert_eq!(route, fresh, "a retained field never changes a route");

                if matches!(route, FieldRoute::AtGoal | FieldRoute::Unknown) {
                    // The policy runs the exact planner instead.
                    retained_work.expansions += exact_expansions;
                }
                match &route {
                    FieldRoute::NoStartCell | FieldRoute::NoRoute => {
                        kinds[3] += 1;
                        assert!(
                            reference.is_none(),
                            "the exact planner finds no route either"
                        );
                    }
                    FieldRoute::AtGoal => {
                        // Arrived: the exact planner finishes, then the target runs off.
                        kinds[0] += 1;
                        target = random_point(&mut rng, &grid, &obstacles);
                    }
                    FieldRoute::Unknown => kinds[1 + usize::from(reference.is_some())] += 1,
                    FieldRoute::Path(path) => {
                        routed += 1;
                        let reference = reference
                            .as_ref()
                            .expect("a retained route implies the reference reaches too");
                        assert!(
                            path.windows(2).all(|step| {
                                let from = grid.cell_at(step[0]);
                                let to = grid.cell_at(step[1]);
                                grid.moves(from).any(|(next, _)| next == to)
                            }),
                            "routes take allowed moves"
                        );
                        // Its end is a goal of the exact planner for this target position.
                        let &(x, z) = path.last().unwrap();
                        let (dx, dz) = (i64::from(target.0 - x), i64::from(target.1 - z));
                        assert!(dx * dx + dz * dz <= GOAL_REACH * GOAL_REACH);
                        let point = Rect::centered(target.0, target.1, 0, 0);
                        assert!(obstacles.iter().all(|&o| fan_is_clear((x, z), point, o)));
                        let (cost, best) = (route_cost(path), route_cost(reference));
                        assert!(cost >= best, "the reference is a shortest route");
                        max_excess = max_excess.max(cost - best);
                        total_excess += cost - best;
                        // Walk on: the body reaches the next cell centre.
                        start = path.get(1).copied().unwrap_or(start);
                    }
                }
            }
        }
        eprintln!(
            "retained routes: {queries} queries, {routed} routed, [at goal, unknown and \
             unreachable, unknown and reachable, no route] {kinds:?}, excess cost max {max_excess} total {total_excess}; {} searches, {} \
             expansions (with exact plans) vs {reference_expansions} per-query reference \
             expansions",
            retained_work.searches, retained_work.expansions
        );
        assert!(routed > queries / 2, "most queries follow a retained route");
        // The target cell's goals are the exact goals for its worst-placed point: at most
        // a few cells farther than for the exact target position.
        assert!(max_excess <= ROUTE_SLACK, "excess {max_excess}");
        assert!(retained_work.searches < routed);
        assert!(retained_work.expansions < reference_expansions);
    }
}
