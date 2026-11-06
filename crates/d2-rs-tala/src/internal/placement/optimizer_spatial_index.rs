// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! optimizer_spatial_index.go: source-ordered snapshot, interval tree and
//! generational exact-top-left occupancy. Guard callbacks borrow the live owner.
// The included optimizer_spacing.rs keeps several statements per line.
#![allow(clippy::possible_missing_else)]
use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::owner_points::SharedOwnerPoint;
use super::shared_topology::{OwnerArenaId, OwnerGraphId};
use super::{ArenaGraph, NodeId, Point};
// Indexed APIs can read an externally shared proposal or a fresh local point.
// isPointOccupied's newly-created proposal never escapes into graph state.
pub(super) trait SpatialPoint {
    fn get(&self) -> Point;
}
impl SpatialPoint for SharedOwnerPoint {
    fn get(&self) -> Point {
        SharedOwnerPoint::get(self)
    }
}
impl SpatialPoint for Point {
    fn get(&self) -> Point {
        *self
    }
}

/// Hasher for packed cell keys and exact coordinate-bit pairs. The shared
/// IdHasher's single multiply leaves the low (bucket) bits a function of the
/// last word's low bits alone, so grid-shaped keys would chain; a folded
/// 128-bit multiply mixes every input bit into the bucket index.
#[derive(Default, Clone, Copy, Debug)]
pub(super) struct CellKeyHasher(u64);
impl std::hash::Hasher for CellKeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }
    fn write_u64(&mut self, value: u64) {
        let product = u128::from(self.0 ^ value) * 0x9e37_79b9_7f4a_7c15_u128;
        self.0 = (product as u64) ^ ((product >> 64) as u64);
    }
}
pub(super) type CellKeyBuild = std::hash::BuildHasherDefault<CellKeyHasher>;
/// sized_optimizer.go's `checked map[geo.Point]struct{}` of tried offsets,
/// keyed by normalized coordinate bits. Membership and size only: nothing
/// iterates it, so its order never reaches results.
pub(super) type CheckedOffsets = std::collections::HashSet<(u64, u64), CellKeyBuild>;

include!("optimizer_spacing.rs");
include!("optimizer_candidates.rs");

#[derive(Clone, Debug)]
struct Entry {
    graph_index: usize,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}
#[derive(Clone, Debug)]
struct Occupied {
    point: Point,
    graph_index: usize,
    generation: u64,
}
#[derive(Debug, Default)]
pub(super) struct OptimizerSpatialIndex {
    graph: Option<(OwnerArenaId, OwnerGraphId)>,
    node_count: usize,
    occupancy_usable: bool,
    spatial_usable: bool,
    entries: Vec<Entry>,
    // Go reslicing retains unused capacity slots. Keep full backing values
    // separately from visible length; a larger allocation starts with zeros.
    max_right: Vec<f64>,
    max_right_len: usize,
    candidates: CandidateSlice,
    // Lookup-only, like Go's map: iteration order never reaches results.
    occupied: std::collections::HashMap<(u64, u64), Occupied, CellKeyBuild>,
    generation: u64,
    // A doesOverlap full-scan header copy kept for the next call. An equal
    // live header reuses it instead of retaining the backing again.
    retained_members: Option<super::container_slices::SharedNodeSlice>,
    spacing_edges: SpacingEdges,
    // isPointOccupied's fresh one-node exception slice. It never escapes
    // doesOverlap, which only reads it, so a slice for the same node is
    // indistinguishable from a newly allocated one.
    single_exception: Option<(NodeId, super::container_slices::SharedNodeSlice)>,
}
fn invalid(guard: &OptimizationWorkGuard<'_>, message: &str) -> OptimizationError {
    OptimizationError::Invalid(format!("TALA {} {message}", guard.location()))
}
fn step(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.step_with_owner(owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}
fn finish(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.finish_with_owner(owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.finish()
    }
}
fn sort_charge(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
    n: usize,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.add_sort_with_owner(n, owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.add_sort(n)
    }
}
fn key(point: Point) -> (u64, u64) {
    let bits = |v: f64| if v == 0. { 0 } else { v.to_bits() };
    (bits(point.x), bits(point.y))
}
impl OptimizerSpatialIndex {
    pub(super) fn rebuild(
        &mut self,
        owner: &mut ArenaGraph,
        receiver: OwnerGraphId,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        let header = owner
            .owner_graph_member_header(receiver)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        if header.len() > 10000 {
            return Err(invalid(
                guard,
                "spatial index node count exceeds limit 10000",
            ));
        }
        self.graph = Some((
            owner.topology_arena_id().expect("bound spatial owner"),
            receiver,
        ));
        self.node_count = header.len();
        self.occupancy_usable = true;
        self.spatial_usable = header.len() >= 96;
        self.entries.clear();
        self.candidates.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 || self.occupied.len() > (4 * header.len()).max(64) {
            self.occupied.clear();
            self.generation = 1;
        }
        for i in 0..header.len() {
            let node = header.get(i).expect("retained spatial member slot");
            step(owner, guard)?;
            let node = node.ok_or_else(|| invalid(guard, "found a nil graph node"))?;
            let Some(point) = owner.position(node) else {
                continue;
            };
            if !point.x.is_finite() || !point.y.is_finite() {
                self.occupancy_usable = false;
                self.spatial_usable = false;
                continue;
            }
            let point_key = key(point);
            if self
                .occupied
                .get(&point_key)
                .is_none_or(|e| e.generation != self.generation)
            {
                self.occupied.insert(
                    point_key,
                    Occupied {
                        point,
                        graph_index: i,
                        generation: self.generation,
                    },
                );
            }
            if !self.spatial_usable {
                continue;
            }
            let size = owner.nodes[node.0 as usize].rect.size;
            let right = point.x + size.width;
            let bottom = point.y + size.height;
            if size.width < 0.
                || size.height < 0.
                || !size.width.is_finite()
                || !size.height.is_finite()
                || !right.is_finite()
                || !bottom.is_finite()
            {
                self.spatial_usable = false;
                continue;
            }
            self.entries.push(Entry {
                graph_index: i,
                left: point.x,
                top: point.y,
                right,
                bottom,
            });
        }
        if !self.spatial_usable || self.entries.is_empty() {
            return finish(owner, guard);
        }
        sort_charge(owner, guard, self.entries.len())?;
        self.entries.sort_by(|a, b| {
            a.left
                .partial_cmp(&b.left)
                .expect("finite indexed coordinate")
                .then_with(|| a.graph_index.cmp(&b.graph_index))
        });
        let required = 4 * self.entries.len();
        if self.max_right.len() < required {
            self.max_right = vec![0.; required];
        }
        self.max_right_len = required;
        self.build_max_right(1, 0, self.entries.len());
        finish(owner, guard)
    }
    fn build_max_right(&mut self, index: usize, low: usize, high: usize) -> f64 {
        let value = if high - low == 1 {
            self.entries[low].right
        } else {
            let middle = low + (high - low) / 2;
            let a = self.build_max_right(index * 2, low, middle);
            let b = self.build_max_right(index * 2 + 1, middle, high);
            super::owner_median::source_math_max(a, b)
        };
        self.max_right[index] = value;
        value
    }
    fn query(
        &mut self,
        owner: &mut ArenaGraph,
        bounds: [f64; 4],
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> (CandidateSlice, Result<(), OptimizationError>) {
        self.candidates.clear();
        if !self.spatial_usable || self.entries.is_empty() {
            return (self.candidates.clone(), finish(owner, guard));
        }
        if let Err(error) = self.query_tree(owner, 1, 0, self.entries.len(), bounds, guard) {
            return (CandidateSlice::default(), Err(error));
        }
        if let Err(error) = sort_charge(owner, guard, self.candidates.len()) {
            return (CandidateSlice::default(), Err(error));
        }
        self.candidates.sort_unstable();
        // Go evaluates the returned candidate header before Finish. A final
        // context error retains that return value instead of returning nil.
        (self.candidates.clone(), finish(owner, guard))
    }
    fn query_tree(
        &mut self,
        owner: &mut ArenaGraph,
        index: usize,
        low: usize,
        high: usize,
        bounds: [f64; 4],
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        step(owner, guard)?;
        let [left, top, right, bottom] = bounds;
        if low >= high || self.max_right[index] < left || self.entries[low].left > right {
            return Ok(());
        }
        if high - low == 1 {
            let e = &self.entries[low];
            if e.left <= right && e.right >= left && e.top <= bottom && e.bottom >= top {
                self.candidates.push(e.graph_index);
            }
            return Ok(());
        }
        let middle = low + (high - low) / 2;
        self.query_tree(owner, index * 2, low, middle, bounds, guard)?;
        self.query_tree(owner, index * 2 + 1, middle, high, bounds, guard)
    }
    pub(super) fn is_occupied(
        &mut self,
        owner: &mut ArenaGraph,
        receiver: OwnerGraphId,
        point: &dyn SpatialPoint,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> (Option<NodeId>, bool, Result<(), OptimizationError>) {
        let header = match owner.owner_graph_members_read(receiver) {
            Ok(header) => header,
            Err(error) => {
                return (
                    None,
                    false,
                    Err(OptimizationError::Invalid(error.to_string())),
                );
            }
        };
        let graph = owner.topology_arena_id().map(|arena| (arena, receiver));
        if self.graph != graph
            || self.node_count != header.len()
            || !self.occupancy_usable
            || !point.get().x.is_finite()
            || !point.get().y.is_finite()
        {
            // A fallback range crosses Step callbacks: retain its original
            // backing, then release the field borrow before entering the loop.
            let retained = header.clone();
            drop(header);
            let header = retained;
            if header.len() > 10000 {
                return (
                    None,
                    false,
                    Err(invalid(guard, "occupancy node count exceeds limit 10000")),
                );
            }
            for i in 0..header.len() {
                let node = header.get(i).expect("retained occupied member");
                if let Err(error) = step(owner, guard) {
                    return (None, false, Err(error));
                }
                let Some(node) = node else {
                    return (None, false, Err(invalid(guard, "found a nil graph node")));
                };
                if owner.position(node) == Some(point.get()) {
                    return (Some(node), true, Ok(()));
                }
            }
            return (None, false, Ok(()));
        }
        drop(header);
        if let Err(error) = step(owner, guard) {
            return (None, false, Err(error));
        }
        let Some(entry) = self
            .occupied
            .get(&key(point.get()))
            .filter(|e| e.generation == self.generation)
        else {
            return (None, false, finish(owner, guard));
        };
        // Read the current Graph.Nodes after Step; capture its returned node
        // before Finish. A final error does not erase node/found return values.
        let current = match owner.owner_graph_members_read(receiver) {
            Ok(header) => header,
            Err(error) => {
                return (
                    None,
                    false,
                    Err(OptimizationError::Invalid(error.to_string())),
                );
            }
        };
        let Some(node) = current.get(entry.graph_index) else {
            return (
                None,
                false,
                Err(invalid(guard, "spatial occupancy index is stale")),
            );
        };
        drop(current);
        (node, true, finish(owner, guard))
    }
    #[cfg(test)]
    fn state(&self) -> serde_json::Value {
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let entries = self
            .entries
            .iter()
            .map(|e| {
                serde_json::json!([
                    e.graph_index,
                    bits(e.left),
                    bits(e.top),
                    bits(e.right),
                    bits(e.bottom)
                ])
            })
            .collect::<Vec<_>>();
        let mut occupied = self
            .occupied
            .values()
            .map(|e| {
                format!(
                    "{}:{}:{}:{}",
                    bits(e.point.x),
                    bits(e.point.y),
                    e.graph_index,
                    e.generation
                )
            })
            .collect::<Vec<_>>();
        occupied.sort();
        serde_json::json!({"graph_bound":self.graph.is_some(),"count":self.node_count,"occupancy":self.occupancy_usable,"spatial":self.spatial_usable,"generation":self.generation,"entries":entries,"tree":self.max_right[..self.max_right_len].iter().copied().map(bits).collect::<Vec<_>>(),"occupied":occupied,"candidates":self.candidates.values()})
    }
}
include!("optimizer_overlap.rs");
include!("optimizer_search.rs");
include!("optimizer_can_move.rs");
