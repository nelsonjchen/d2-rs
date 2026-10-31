// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! sized_optimizer.go715–887: canonical guarded candidate generation.
use crate::internal::limits::{
    OptimizationError, OptimizationResourceLimitDetail, OptimizationWorkGuard,
};
use crate::internal::optimizer_spatial_index::SpatialPoint;
use crate::internal::{ArenaGraph, NodeId, Point};
/// sized_optimizer.go's `seen` is a Go map used only for membership; its
/// iteration order never reaches candidates or output.
pub(super) type SeenCells =
    std::collections::HashSet<u64, crate::internal::optimizer_spatial_index::CellKeyBuild>;
#[derive(Default)]
pub(super) struct PlacementScratch {
    pub(super) seen: Option<SeenCells>,
    pub(super) points: Option<Vec<Point>>,
}
fn step(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.step_with_owner(owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}
fn add_work(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
    n: u64,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.add_with_owner(n, owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.add(n)
    }
}
fn sort_work(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
    n: usize,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.add_sort_with_owner(n, owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.add_sort(n)
    }
}
fn invalid(guard: &OptimizationWorkGuard<'_>, message: &str) -> OptimizationError {
    OptimizationError::Invalid(format!("TALA {} {message}", guard.location()))
}
fn candidate_limit() -> OptimizationError {
    OptimizationError::ResourceLimit {
        location: "LocalOptimize".into(),
        detail: OptimizationResourceLimitDetail::PlacementCandidateCount(1_000_000),
    }
}
impl PlacementScratch {
    fn push(&mut self, point: Point) {
        let points = self.points.as_mut().expect("allocated candidate points");
        if points.len() == points.capacity() {
            let cap = crate::internal::container_slices::go127_slice_growth_capacity(
                points.capacity(),
                points.len() + 1,
                16,
                false,
            )
            .expect("bounded candidate capacity");
            let mut replacement = Vec::with_capacity(cap);
            replacement.extend(points.iter().copied());
            *points = replacement;
        }
        points.push(point);
    }
    fn add(
        &mut self,
        owner: &mut ArenaGraph,
        median: &dyn SpatialPoint,
        offset: Point,
        cell: f64,
        origin: Option<Point>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        step(owner, guard)?;
        let p = median.get();
        let point = Point {
            x: ((p.x + offset.x) / cell).round() * cell,
            y: ((p.y + offset.y) / cell).round() * cell,
        };
        if origin.is_some_and(|o| point.x < o.x || point.y < o.y) {
            return Ok(());
        }
        let ix = crate::internal::loop_offsets::loop_spacing_units(point.x / cell) as u32;
        let iy = crate::internal::loop_offsets::loop_spacing_units(point.y / cell) as u32;
        let key = ((ix as u64) << 32) | iy as u64;
        if self.seen.as_ref().unwrap().contains(&key) {
            return Ok(());
        }
        if self.points.as_ref().unwrap().len() >= 1_000_000 {
            return Err(candidate_limit());
        }
        self.seen.as_mut().unwrap().insert(key);
        self.push(point);
        Ok(())
    }
    fn around(
        &mut self,
        owner: &mut ArenaGraph,
        node: NodeId,
        median: &dyn SpatialPoint,
        x: f64,
        y: f64,
        minimize: bool,
        cell: f64,
        origin: Option<Point>,
        guard: &mut OptimizationWorkGuard<'_>,
        failure: &mut Option<OptimizationError>,
    ) {
        let size = owner.nodes[node.0 as usize].rect.size;
        let graph = owner
            .topology_node_owner(node)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let increment = owner
            .owner_graph_cell_size(graph)
            .expect("bound candidate graph");
        let x = x * increment;
        let y = y * increment;
        if failure.is_some() {
            return;
        }
        if let Err(error) = self.add(owner, median, Point { x, y }, cell, origin, guard) {
            *failure = Some(error);
            return;
        }
        if !minimize {
            return;
        }
        let mut current_x = x - size.width;
        while current_x <= x {
            let mut current_y = y - size.height;
            while current_y <= y {
                if current_x != x || current_y != y {
                    if let Err(error) = self.add(
                        owner,
                        median,
                        Point {
                            x: current_x,
                            y: current_y,
                        },
                        cell,
                        origin,
                        guard,
                    ) {
                        *failure = Some(error);
                        return;
                    }
                }
                current_y += increment;
            }
            current_x += increment;
        }
    }
    pub(super) fn generate(
        &mut self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        median: Option<&dyn SpatialPoint>,
        minimum: f64,
        minimize: bool,
        cell: f64,
        origin: Option<Point>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        let (Some(node), Some(median)) = (node, median) else {
            return Err(invalid(
                guard,
                "placement generation requires a positioned node and median",
            ));
        };
        if owner.position(node).is_none() {
            return Err(invalid(
                guard,
                "placement generation requires a positioned node and median",
            ));
        }
        if !(0.0..=100.).contains(&minimum) || !minimum.is_finite() {
            return Err(invalid(
                guard,
                "placement distance must be finite and within [0, 100]",
            ));
        }
        if cell <= 0. || !cell.is_finite() {
            return Err(invalid(
                guard,
                "placement generation requires a finite positive cell size",
            ));
        }
        let retained = self.seen.get_or_insert_with(SeenCells::default).len();
        self.points.get_or_insert_with(|| Vec::with_capacity(128));
        // One charged step per retained key, each followed by its removal in
        // ascending key order. A failed step leaves exactly the keys that
        // ordered removal had not reached yet.
        for removed in 0..retained {
            if let Err(error) = step(owner, guard) {
                let seen = self.seen.as_mut().unwrap();
                let mut keys = seen.iter().copied().collect::<Vec<_>>();
                keys.sort_unstable();
                for key in &keys[..removed] {
                    seen.remove(key);
                }
                return Err(error);
            }
        }
        self.seen.as_mut().unwrap().clear();
        self.points.as_mut().unwrap().clear();
        let dist = minimum + 1.;
        let mut x = dist;
        while x >= -dist {
            step(owner, guard)?;
            let max_y = dist - x.abs();
            let mut y = 0.;
            while y <= max_y {
                step(owner, guard)?;
                let mut failure = None;
                if y != 0. {
                    self.around(
                        owner,
                        node,
                        median,
                        x,
                        -y,
                        minimize,
                        cell,
                        origin,
                        guard,
                        &mut failure,
                    );
                }
                self.around(
                    owner,
                    node,
                    median,
                    x,
                    y,
                    minimize,
                    cell,
                    origin,
                    guard,
                    &mut failure,
                );
                if let Some(error) = failure {
                    return Err(error);
                }
                y += 1.;
            }
            x -= 1.;
        }
        let data = owner
            .owner_node_requirement_map(node)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        if !data.is_nil() {
            if data.len() > 1_000_000 {
                return Err(invalid(
                    guard,
                    "long-distance neighbor references exceed limit 1000000",
                ));
            }
            let mut neighbors = Vec::with_capacity(data.len());
            let mut cursor = data.live_cursor();
            while let Some(key) = cursor.next() {
                step(owner, guard)?;
                neighbors.push(key);
            }
            sort_work(owner, guard, neighbors.len())?;
            neighbors.sort_unstable_by_key(|n| n.map_or(0, |n| owner.nodes[n.0 as usize].tala_id));
            let c = crate::internal::loop_offsets::loop_spacing_units(cell) as f64;
            for adjacent in neighbors {
                step(owner, guard)?;
                let r = data.get(adjacent);
                let Some(adjacent) = adjacent else {
                    return Err(invalid(
                        guard,
                        "found an unpositioned long-distance neighbor",
                    ));
                };
                let Some(position) = owner.position(adjacent) else {
                    return Err(invalid(
                        guard,
                        "found an unpositioned long-distance neighbor",
                    ));
                };
                let p = median.get();
                let ax = position.x - p.x;
                let ay = position.y - p.y;
                if r.max_width > c as i64 {
                    let width = owner.nodes[node.0 as usize].rect.size.width;
                    self.add(
                        owner,
                        median,
                        Point {
                            x: ((ax - width - r.max_width as f64) / c).floor() * c,
                            y: ay,
                        },
                        cell,
                        origin,
                        guard,
                    )?;
                    let width = owner.nodes[adjacent.0 as usize].rect.size.width;
                    self.add(
                        owner,
                        median,
                        Point {
                            x: ((ax + width + r.max_width as f64) / c).ceil() * c,
                            y: ay,
                        },
                        cell,
                        origin,
                        guard,
                    )?;
                }
                if r.max_height > c as i64 {
                    let height = owner.nodes[node.0 as usize].rect.size.height;
                    self.add(
                        owner,
                        median,
                        Point {
                            x: ax,
                            y: ((ay - height - r.max_height as f64) / c).floor() * c,
                        },
                        cell,
                        origin,
                        guard,
                    )?;
                    let height = owner.nodes[adjacent.0 as usize].rect.size.height;
                    self.add(
                        owner,
                        median,
                        Point {
                            x: ax,
                            y: ((ay + height + r.max_height as f64) / c).ceil() * c,
                        },
                        cell,
                        origin,
                        guard,
                    )?;
                }
            }
        }
        if owner.has_herd_assignment(node) {
            if self.points.as_ref().unwrap().len() >= 1_000_000 {
                return Err(candidate_limit());
            }
            let point = owner
                .position(node)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            self.push(point);
        }
        add_work(owner, guard, self.points.as_ref().unwrap().len() as u64)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;
    struct Counter {
        polls: Cell<u64>,
        at: u64,
    }
    impl Context for Counter {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let p = self.polls.get() + 1;
            self.polls.set(p);
            (self.at > 0 && p >= self.at).then_some(ContextError::Cancelled)
        }
    }
    fn encoded(points: &[Point]) -> Vec<[String; 2]> {
        points
            .iter()
            .map(|p| {
                [
                    format!("{:016x}", p.x.to_bits()),
                    format!("{:016x}", p.y.to_bits()),
                ]
            })
            .collect()
    }
}
