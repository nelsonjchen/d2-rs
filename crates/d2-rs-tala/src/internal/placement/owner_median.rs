// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Go math.Max behavior: Copyright 2010, 2016 The Go Authors.
// BSD-3-Clause; see THIRD_PARTY_NOTICES.md.

//! Source optimization_guard.go53–70,95–167,329–429 and
//! sizeless_optimizer.go226–265. The complete neighbor collector supplies the
//! live median calculation, fixed-origin lookup and actual caller RNG tail.

use super::container_slices::SharedNodeSlice;
use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::owner_placement::{
    OwnerPlacementError, OwnerPlacementPoint, optimizer_owner_fixed_origin, owner_owning_container,
};
use super::owner_sizeless_constructor::OwnerSizelessOptimizer;
use super::{ArenaGraph, NodeId, OwnerTopologyError, OwnerTreeId, Point};
use std::collections::BTreeSet;

fn invalid(guard: &OptimizationWorkGuard<'_>, text: &str) -> OwnerPlacementError {
    OptimizationError::Invalid(format!("TALA {} {text}", guard.location())).into()
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

fn sort_charge(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
    length: usize,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.add_sort_with_owner(length, owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.add_sort(length)
    }
}

pub(super) fn optimizer_owner_ordered_nears(
    owner: &mut ArenaGraph,
    node: Option<NodeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<SharedNodeSlice, OwnerPlacementError> {
    let node = node.ok_or_else(|| invalid(guard, "cannot order nears for a nil node"))?;
    let live = owner
        .topology_node(node)
        .ok_or(OwnerTopologyError::UnknownNode(node))?;
    if live.nears.len() > 10_000 {
        return Err(invalid(guard, "near count exceeds limit 10000"));
    }
    let mut cursor = live.nears.live_cursor();
    let mut nears = SharedNodeSlice::with_len_cap(0, live.nears.len())
        .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
    for near in &mut cursor {
        step(owner, guard)?;
        let Some(near) = near else {
            // Aggregate DebugID metadata is not synthesized from member IDs.
            if owner.topology_source_input_node(node).is_none() {
                return Err(OwnerPlacementError::UnboundDebugId(node));
            }
            let name = &owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?
                .external_id;
            return Err(invalid(guard, &format!("found a nil near on {name}")));
        };
        nears = nears
            .append_node(near)
            .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
    }
    sort_charge(owner, guard, nears.len())?;
    let mut ordered: Vec<_> = nears.require_nodes().collect();
    super::go_sort::sort_by(&mut ordered, |left, right| {
        owner.nodes[left.0 as usize].tala_id < owner.nodes[right.0 as usize].tala_id
    });
    for (index, node) in ordered.into_iter().enumerate() {
        nears
            .write_capacity_slot(index, Some(node))
            .expect("ordered Near slot");
    }
    Ok(nears)
}

pub(super) fn optimizer_owner_tree_root(
    owner: &mut ArenaGraph,
    mut tree: Option<OwnerTreeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<Option<OwnerTreeId>, OwnerPlacementError> {
    let mut seen = BTreeSet::new();
    while let Some(current) = tree {
        let record = owner
            .owner_tree(current)
            .ok_or(OwnerTopologyError::UnknownTree(current))?;
        if record.fields().parent.is_none() {
            break;
        }
        step(owner, guard)?;
        if seen.contains(&current) {
            return Err(invalid(guard, "found a cycle in tree ancestry"));
        }
        if seen.len() >= 10_000 {
            return Err(invalid(guard, "tree ancestry exceeds node limit 10000"));
        }
        seen.insert(current);
        tree = owner
            .owner_tree(current)
            .ok_or(OwnerTopologyError::UnknownTree(current))?
            .fields()
            .parent;
    }
    Ok(tree)
}

pub(super) fn append_owner_tree_nears(
    owner: &mut ArenaGraph,
    mut nears: SharedNodeSlice,
    tree: Option<OwnerTreeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<SharedNodeSlice, OwnerPlacementError> {
    let Some(tree) = tree else { return Ok(nears) };
    let mut stack = vec![tree];
    let mut seen = BTreeSet::from([tree]);
    while !stack.is_empty() {
        step(owner, guard)?;
        let current = stack.pop().expect("nonempty tree traversal");
        let node = owner
            .owner_tree(current)
            .ok_or(OwnerTopologyError::UnknownTree(current))?
            .fields()
            .node;
        let Some(node) = node else {
            return Err(invalid(guard, "found a tree entry without a node"));
        };
        let ordered = optimizer_owner_ordered_nears(owner, Some(node), guard)?;
        if nears.len() > 1_000_000usize.saturating_sub(ordered.len()) {
            return Err(invalid(
                guard,
                "optimizer node references exceed limit 1000000",
            ));
        }
        nears = nears
            .append(&ordered.visible_slots())
            .map_err(|error| OwnerTopologyError::ContainerSlice(error.to_string()))?;
        let children = owner
            .owner_tree(current)
            .ok_or(OwnerTopologyError::UnknownTree(current))?
            .fields()
            .children;
        if children.len() > 10_000 {
            return Err(invalid(guard, "tree child references exceed limit 10000"));
        }
        for index in (0..children.len()).rev() {
            let child = children.get(index).expect("retained backward child slot");
            step(owner, guard)?;
            let Some(child) = child else {
                return Err(invalid(guard, "found a nil tree child"));
            };
            if seen.contains(&child) {
                return Err(invalid(guard, "found a cycle or shared child in a tree"));
            }
            if seen.len() >= 10_000 {
                return Err(invalid(guard, "tree exceeds node limit 10000"));
            }
            seen.insert(child);
            stack.push(child);
        }
    }
    Ok(nears)
}

fn live_coordinate(owner: &ArenaGraph, node: NodeId, x: bool, sizes: bool) -> f64 {
    let point = owner
        .position(node)
        .expect("runtime error: invalid memory address or nil pointer dereference");
    let node = owner.topology_node(node).expect("validated median node");
    if x {
        if sizes {
            point.x + node.rect.size.width / 2.0
        } else {
            point.x
        }
    } else if sizes {
        point.y + node.rect.size.height / 2.0
    } else {
        point.y
    }
}

pub(super) fn source_math_max(a: f64, b: f64) -> f64 {
    if a == f64::INFINITY || b == f64::INFINITY {
        return f64::INFINITY;
    }
    // Go 1.27 dim_arm64.s uses FMAX with y as its first input. Signaling
    // NaNs take priority, then y's quiet payload, then x's. The scalar and
    // amd64 implementations instead return Go's canonical NaN.
    #[cfg(target_arch = "aarch64")]
    {
        let quiet = 1u64 << 51;
        if a.is_nan() && a.to_bits() & quiet == 0 {
            return f64::from_bits(a.to_bits() | quiet);
        }
        if b.is_nan() && b.to_bits() & quiet == 0 {
            return f64::from_bits(b.to_bits() | quiet);
        }
        if b.is_nan() {
            return b;
        }
        if a.is_nan() {
            return a;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    if a.is_nan() || b.is_nan() {
        return f64::from_bits(0x7ff8000000000001);
    }
    if a == 0.0 && a == b {
        if a.is_sign_negative() { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
}

pub(super) fn optimizer_owner_median(
    owner: &mut ArenaGraph,
    nodes: &SharedNodeSlice,
    sizes: bool,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<Point, OwnerPlacementError> {
    if nodes.is_empty() {
        return Err(invalid(
            guard,
            "cannot compute a median without positioned neighbors",
        ));
    }
    if nodes.len() > 1_000_000 {
        return Err(invalid(guard, "median inputs exceed limit 1000000"));
    }
    if nodes.len() == 1 {
        let node = nodes.get(0).expect("singleton median slot");
        step(owner, guard)?;
        let node = node.ok_or_else(|| {
            invalid(
                guard,
                "cannot compute a median with an unpositioned neighbor",
            )
        })?;
        owner
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        if owner.position(node).is_none() {
            return Err(invalid(
                guard,
                "cannot compute a median with an unpositioned neighbor",
            ));
        }
        sort_charge(owner, guard, 1)?;
        sort_charge(owner, guard, 1)?;
        if sizes {
            let graph = owner
                .topology_node_owner(node)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            let cell = owner.owner_graph_cell_size(graph)?;
            if cell <= 0.0 || !cell.is_finite() {
                return Err(invalid(guard, "requires a finite positive cell size"));
            }
            return Ok(Point {
                x: live_coordinate(owner, node, true, true) / cell,
                y: live_coordinate(owner, node, false, true) / cell,
            });
        }
        return Ok(Point {
            x: live_coordinate(owner, node, true, false) + 0.5,
            y: live_coordinate(owner, node, false, false) + 0.5,
        });
    }
    let mut by_x = Vec::with_capacity(nodes.len());
    let mut by_y = Vec::with_capacity(nodes.len());
    for index in 0..nodes.len() {
        let node = nodes.get(index).expect("retained median node slot");
        step(owner, guard)?;
        let node = node.ok_or_else(|| {
            invalid(
                guard,
                "cannot compute a median with an unpositioned neighbor",
            )
        })?;
        owner
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        if owner.position(node).is_none() {
            return Err(invalid(
                guard,
                "cannot compute a median with an unpositioned neighbor",
            ));
        }
        by_x.push(node);
        by_y.push(node);
    }
    sort_charge(owner, guard, nodes.len())?;
    if nodes.len() > 1 {
        super::go_sort::sort_by(&mut by_x, |left, right| {
            let a = live_coordinate(owner, *left, true, sizes);
            let b = live_coordinate(owner, *right, true, sizes);
            if a == b {
                owner.nodes[left.0 as usize].tala_id < owner.nodes[right.0 as usize].tala_id
            } else {
                a < b
            }
        });
    }
    sort_charge(owner, guard, nodes.len())?;
    if nodes.len() > 1 {
        super::go_sort::sort_by(&mut by_y, |left, right| {
            let a = live_coordinate(owner, *left, false, sizes);
            let b = live_coordinate(owner, *right, false, sizes);
            if a == b {
                owner.nodes[left.0 as usize].tala_id < owner.nodes[right.0 as usize].tala_id
            } else {
                a < b
            }
        });
    }
    let middle = nodes.len() / 2;
    let mut result = Point {
        x: live_coordinate(owner, by_x[middle], true, sizes),
        y: live_coordinate(owner, by_y[middle], false, sizes),
    };
    if !sizes {
        result.x += 0.5;
        result.y += 0.5;
    }
    if nodes.len().is_multiple_of(2) {
        if sizes {
            // Preserve Go's left-associative additions before dividing.
            let x = &owner.nodes[by_x[middle - 1].0 as usize];
            let y = &owner.nodes[by_y[middle - 1].0 as usize];
            result.x = (result.x
                + owner
                    .position(by_x[middle - 1])
                    .expect("nil median point")
                    .x
                + x.rect.size.width / 2.0)
                / 2.0;
            result.y = (result.y
                + owner
                    .position(by_y[middle - 1])
                    .expect("nil median point")
                    .y
                + y.rect.size.height / 2.0)
                / 2.0;
        } else {
            result.x = (result.x + live_coordinate(owner, by_x[middle - 1], true, false)) / 2.0;
            result.y = (result.y + live_coordinate(owner, by_y[middle - 1], false, false)) / 2.0;
        }
    }
    if sizes {
        // The caller's original first slot is read again after both sorts,
        // unlike the singleton's pre-Step local pointer.
        let first = nodes
            .get(0)
            .expect("first median slot")
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let graph = owner
            .topology_node_owner(first)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let cell = owner.owner_graph_cell_size(graph)?;
        if cell <= 0.0 || !cell.is_finite() {
            return Err(invalid(guard, "requires a finite positive cell size"));
        }
        result.x /= cell;
        result.y /= cell;
    }
    Ok(result)
}

impl OwnerSizelessOptimizer {
    pub(super) fn median_point_guarded(
        &self,
        owner: &mut ArenaGraph,
        node: NodeId,
        temperature: f64,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<OwnerPlacementPoint, OwnerPlacementError> {
        let abductions = owner.allocate_owner_abduction_header(&[], 0, false)?;
        let neighbors = super::owner_adjacents::optimizer_owner_adjacents(
            owner,
            Some(node),
            &abductions,
            guard,
        )?;
        self.median_point_from_neighbors_guarded(owner, node, &neighbors, temperature, guard)
    }
    /// Shared tail for the complete adjacency path and explicit-neighbor calls.
    pub(super) fn median_point_from_neighbors_guarded(
        &self,
        owner: &mut ArenaGraph,
        node: NodeId,
        neighbors: &SharedNodeSlice,
        temperature: f64,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<OwnerPlacementPoint, OwnerPlacementError> {
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_MEDIAN_OPERANDS") {
            let incident = owner.owner_node_incident_header(node)?;
            let edges = incident
                .iter_slots()
                .map(|edge| {
                    let edge = edge.expect("median diagnostic incident");
                    let from = owner
                        .owner_edge_endpoint(edge, false)
                        .expect("median diagnostic from");
                    let to = owner
                        .owner_edge_endpoint(edge, true)
                        .expect("median diagnostic to");
                    let describe = |id: Option<NodeId>| {
                        id.map(|id| {
                    serde_json::json!({
                        "id": id.0,
                        "name": crate::internal::pipeline::trace_node_external_id(owner, id),
                        "position": owner.position(id),
                        "container": owner.nodes[id.0 as usize].container.map(|id| id.0),
                    })
                })
                    };
                    serde_json::json!({"edge":edge.0,"from":describe(from),"to":describe(to)})
                })
                .collect::<Vec<_>>();
            eprintln!(
                "{}",
                serde_json::json!({
                    "event":"owner-median-operands",
                    "node":crate::internal::pipeline::trace_node_external_id(owner,node),
                    "node_id":node.0,
                    "position":owner.position(node),
                    "neighbors":neighbors.iter_slots().map(|id| id.map(|id| id.0)).collect::<Vec<_>>(),
                    "edges":edges,
                })
            );
        }
        let mut median = optimizer_owner_median(owner, neighbors, false, guard)?;
        let receiver = self.selected_graph();
        let cell = owner.owner_graph_cell_size(receiver)?;
        let container = owner_owning_container(owner, node)?;
        let origin = optimizer_owner_fixed_origin(owner, Some(receiver), container, guard)?;
        if let Some(origin) = origin {
            if cell <= 0.0 || !cell.is_finite() {
                return Err(invalid(
                    guard,
                    "fixed-origin placement requires a finite positive cell size",
                ));
            }
            if median.x < origin.x / cell {
                median.x = origin.x / cell;
                median.x += temperature;
            }
            if median.y < origin.y / cell {
                median.y = origin.y / cell;
                median.y += temperature;
            }
        }
        let rng = self
            .rng()
            .expect("runtime error: invalid memory address or nil pointer dereference");
        median.x += -temperature + rng.stream().borrow_mut().float64() * (2.0 * temperature);
        median.y += -temperature + rng.stream().borrow_mut().float64() * (2.0 * temperature);
        if let Some(origin) = origin {
            // Go math.Max: +Inf wins over NaN; NaN is returned otherwise;
            // positive zero wins over negative zero.
            median.x = source_math_max(median.x, origin.x / cell);
            median.y = source_math_max(median.y, origin.y / cell);
        }
        Ok(OwnerPlacementPoint::new(Point {
            x: median.x.round(),
            y: median.y.round(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::go_rng::GoRng;
    use super::super::limits::{Context, ContextError};
    use super::super::node_near_map::NodeNearMap;
    use super::super::owner_sizeless_constructor::OwnerOptimizerRng;
    use super::super::owner_tree_map::OwnerTreeSlice;
    use super::*;
    use crate::{Graph, Node, Size};
    #[cfg(feature = "diagnostic-traces")]
    use std::cell::Cell;

    struct Quiet;
    impl Context for Quiet {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }

    fn fixture() -> (ArenaGraph, [NodeId; 4]) {
        let mut input = Graph::default();
        let nodes = ["A", "B", "C", "D"].map(|id| {
            input.add_node(Node::new(
                id,
                Size {
                    width: 10.0,
                    height: 20.0,
                },
            ))
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.cell_size = 10.0;
        for (index, node) in nodes.iter().enumerate() {
            owner.set_position(
                *node,
                Point {
                    x: index as f64 * 2.0,
                    y: index as f64 * -3.0,
                },
            );
        }
        owner.initialize_shared_topology();
        (owner, nodes)
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn owner_median_actual_source_controls() {
        struct Observe {
            polls: Cell<usize>,
            name: &'static str,
            nodes: [NodeId; 4],
            input: SharedNodeSlice,
            root: OwnerTreeId,
        }
        impl Context for Observe {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                None
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                let poll = self.polls.get();
                if poll == 2 {
                    match self.name {
                        "median-singleton-cell-priority" => {
                            owner.nodes[self.nodes[0].0 as usize].position = None;
                            owner.cell_size = 0.0;
                        }
                        "median-singleton-slot" => {
                            self.input
                                .write_capacity_slot(0, Some(self.nodes[1]))
                                .unwrap();
                            owner.set_position(self.nodes[0], Point { x: 100.0, y: 200.0 });
                        }
                        "median-future-slot" => {
                            self.input
                                .write_capacity_slot(2, Some(self.nodes[3]))
                                .unwrap();
                        }
                        "median-sort-live" => {
                            owner.set_position(self.nodes[0], Point { x: 1000.0, y: 0.0 });
                            owner.cell_size = 20.0;
                        }
                        "median-cancel" => return Some(ContextError::Cancelled),
                        "tree-parent-live" => {
                            let mut f = owner.owner_tree(self.root).unwrap().fields();
                            f.parent = None;
                            owner.set_owner_tree_fields(self.root, f).unwrap();
                        }
                        _ => {}
                    }
                }
                if poll == 3 && self.name == "tree-child-live" {
                    let mut f = owner.owner_tree(self.root).unwrap().fields();
                    f.children.write_capacity_slot(0, None);
                    f.children = OwnerTreeSlice::nil();
                    owner.set_owner_tree_fields(self.root, f).unwrap();
                }
                None
            }
        }
        for name in [
            "median-empty",
            "median-singleton",
            "median-even",
            "median-odd",
            "median-sizes-foreign",
            "median-nil",
            "median-unpositioned",
            "median-invalid-cell",
            "median-long",
            "median-long-nan",
            "median-large-even",
            "median-singleton-cell-priority",
            "median-singleton-slot",
            "median-future-slot",
            "median-sort-live",
            "median-cancel",
            "tree-order",
            "tree-shared-child",
            "tree-parent-cycle",
            "tree-parent-live",
            "tree-child-live",
            "ordered-nil-near",
            "math-max",
        ] {
            let (mut owner, nodes) = fixture();
            for (index, node) in nodes.iter().enumerate() {
                owner.nodes[node.0 as usize].tala_id = index as i64 + 1;
            }
            let mut slots: Vec<_> = nodes[..2].iter().copied().map(Some).collect();
            let mut sizes = false;
            let mut prefix = 0;
            if name == "median-empty" {
                slots.clear();
            }
            if matches!(
                name,
                "median-singleton"
                    | "median-invalid-cell"
                    | "median-singleton-cell-priority"
                    | "median-singleton-slot"
            ) {
                slots.truncate(1);
            }
            if matches!(
                name,
                "median-odd" | "median-future-slot" | "median-sort-live"
            ) {
                slots = nodes[..3].iter().copied().map(Some).collect();
            }
            if name == "median-sizes-foreign" {
                sizes = true;
                let foreign = owner.create_owner_graph_view(vec![], vec![]).unwrap();
                owner.set_owner_graph_cell_size(foreign, 20.0).unwrap();
                owner
                    .set_owner_node_graph_reference(nodes[0], Some(foreign))
                    .unwrap();
            }
            if name == "median-nil" {
                slots[1] = None;
            }
            if name == "median-unpositioned" {
                owner.nodes[nodes[1].0 as usize].position = None;
            }
            if name == "median-invalid-cell" {
                sizes = true;
                owner.cell_size = 0.0;
            }
            if matches!(name, "median-long" | "median-long-nan") {
                slots = (0..65).map(|i| Some(nodes[i % 4])).collect();
                if name == "median-long-nan" {
                    owner.set_position(
                        nodes[2],
                        Point {
                            x: f64::from_bits(0x7ff8000000000001),
                            y: -6.0,
                        },
                    );
                }
            }
            if name == "median-large-even" {
                sizes = true;
                owner.set_position(nodes[0], Point { x: 1e16, y: 0.0 });
                owner.set_position(nodes[1], Point { x: -1e16, y: -3.0 });
                owner.nodes[nodes[0].0 as usize].rect.size.width = 3.0;
                owner.nodes[nodes[1].0 as usize].rect.size.width = 7.0;
            }
            if name == "median-singleton-cell-priority" {
                sizes = true;
            }
            if matches!(
                name,
                "median-singleton-slot" | "median-cancel" | "tree-parent-live"
            ) {
                prefix = 63;
            }
            if matches!(name, "median-future-slot" | "tree-child-live") {
                prefix = 62;
            }
            if name == "median-sort-live" {
                prefix = 56;
                sizes = true;
            }
            let input = SharedNodeSlice::from_slots(&slots).unwrap();
            let root = owner.new_owner_tree(Some(nodes[0])).unwrap();
            let a = owner.new_owner_tree(Some(nodes[1])).unwrap();
            let b = owner.new_owner_tree(Some(nodes[2])).unwrap();
            if matches!(name, "tree-order" | "tree-shared-child") {
                let mut f = owner.owner_tree(root).unwrap().fields();
                f.children = OwnerTreeSlice::from_slots(
                    &[
                        Some(a),
                        Some(if name == "tree-shared-child" { a } else { b }),
                    ],
                    2,
                )
                .unwrap();
                owner.set_owner_tree_fields(root, f).unwrap();
                for (node, near) in [
                    (nodes[0], nodes[3]),
                    (nodes[1], nodes[2]),
                    (nodes[2], nodes[1]),
                ] {
                    owner.nodes[node.0 as usize].nears = NodeNearMap::from_raw_keys([Some(near)]);
                }
            }
            if matches!(name, "tree-parent-cycle" | "tree-parent-live") {
                let mut f = owner.owner_tree(root).unwrap().fields();
                f.parent = Some(a);
                owner.set_owner_tree_fields(root, f).unwrap();
                if name == "tree-parent-cycle" {
                    let mut f = owner.owner_tree(a).unwrap().fields();
                    f.parent = Some(root);
                    owner.set_owner_tree_fields(a, f).unwrap();
                }
            }
            if name == "tree-child-live" {
                let mut f = owner.owner_tree(root).unwrap().fields();
                f.children = OwnerTreeSlice::from_slots(&[Some(a)], 1).unwrap();
                owner.set_owner_tree_fields(root, f).unwrap();
            }
            if name == "ordered-nil-near" {
                owner.nodes[nodes[0].0 as usize].nears = NodeNearMap::from_raw_keys([None]);
            }
            let context = Observe {
                polls: Cell::new(0),
                name,
                nodes,
                input: input.clone(),
                root,
            };
            let mut guard = OptimizationWorkGuard::new_with_owner(
                Some(&context),
                "control",
                100000,
                &mut owner,
            )
            .unwrap();
            if prefix != 0 {
                guard.add_with_owner(prefix, &mut owner).unwrap();
            }
            let mut row = serde_json::json!({"case":name,"point_bits":null,"near_ids":null,"near_nil":null,"near_capacity":null,"root":null,"max_bits":null});
            let error = if name == "math-max" {
                let pairs = [
                    (0x7ff8000000000007, 0x4000000000000000),
                    (0x7ff8000000000007, 0x7ff8000000000009),
                    (0, 0x7ff8000000000009),
                    (0x7ff0000000000000, 0x7ff8000000000009),
                    (0x8000000000000000, 0),
                    (0x7ff0000000000001, 0x7ff8000000000009),
                ];
                row["max_bits"] = serde_json::json!(
                    pairs
                        .iter()
                        .map(|(a, b)| format!(
                            "{:016x}",
                            source_math_max(f64::from_bits(*a), f64::from_bits(*b)).to_bits()
                        ))
                        .collect::<Vec<_>>()
                );
                None
            } else if matches!(name, "tree-parent-cycle" | "tree-parent-live") {
                match optimizer_owner_tree_root(&mut owner, Some(root), &mut guard) {
                    Ok(result) => {
                        row["root"] = serde_json::json!(
                            [root, a, b].iter().position(|tree| Some(*tree) == result)
                        );
                        None
                    }
                    Err(error) => Some(error.to_string()),
                }
            } else if matches!(
                name,
                "tree-order" | "tree-shared-child" | "tree-child-live" | "ordered-nil-near"
            ) {
                let result = if name == "ordered-nil-near" {
                    optimizer_owner_ordered_nears(&mut owner, Some(nodes[0]), &mut guard)
                } else {
                    append_owner_tree_nears(
                        &mut owner,
                        SharedNodeSlice::nil(),
                        Some(root),
                        &mut guard,
                    )
                };
                match result {
                    Ok(result) => {
                        row["near_nil"] = serde_json::json!(result.is_nil());
                        row["near_capacity"] = serde_json::json!(result.capacity());
                        if !result.is_nil() {
                            row["near_ids"] = serde_json::json!(
                                result
                                    .require_nodes()
                                    .map(|node| owner.nodes[node.0 as usize].tala_id)
                                    .collect::<Vec<_>>()
                            );
                        }
                        None
                    }
                    Err(error) => {
                        row["near_nil"] = serde_json::json!(true);
                        row["near_capacity"] = serde_json::json!(0);
                        Some(error.to_string())
                    }
                }
            } else {
                match optimizer_owner_median(&mut owner, &input, sizes, &mut guard) {
                    Ok(point) => {
                        row["point_bits"] = serde_json::json!([
                            format!("{:016x}", point.x.to_bits()),
                            format!("{:016x}", point.y.to_bits())
                        ]);
                        None
                    }
                    Err(error) => Some(error.to_string()),
                }
            };
            row["used"] = serde_json::json!(guard.used());
            row["polls"] = serde_json::json!(context.polls.get());
            row["error"] = serde_json::json!(error);
            println!("OWNER_MEDIAN_CONTROL {row}");
        }
    }

    #[test]
    fn median_keeps_even_quarter_cell_bias_sizes_and_original_first_graph_cell() {
        let (mut owner, nodes) = fixture();
        let slice = SharedNodeSlice::from_nodes(&nodes[..2]).unwrap();
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "median", 100).unwrap();
        assert_eq!(
            optimizer_owner_median(&mut owner, &slice, false, &mut guard).unwrap(),
            Point { x: 1.25, y: -1.25 }
        );
        assert_eq!(guard.used(), 6);
        let foreign = owner.create_owner_graph_view(vec![], vec![]).unwrap();
        owner.set_owner_graph_cell_size(foreign, 20.0).unwrap();
        owner
            .set_owner_node_graph_reference(nodes[0], Some(foreign))
            .unwrap();
        assert_eq!(
            optimizer_owner_median(&mut owner, &slice, true, &mut guard).unwrap(),
            Point { x: 0.3, y: 0.425 }
        );
        assert_eq!(guard.used(), 12);
    }

    #[test]
    fn median_rejects_empty_nil_and_unpositioned_inputs_with_actual_work_priority() {
        let (mut owner, nodes) = fixture();
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "median", 100).unwrap();
        let empty = SharedNodeSlice::nil();
        assert_eq!(
            optimizer_owner_median(&mut owner, &empty, false, &mut guard)
                .unwrap_err()
                .to_string(),
            "TALA median cannot compute a median without positioned neighbors"
        );
        let nil = SharedNodeSlice::from_slots(&[None]).unwrap();
        assert!(optimizer_owner_median(&mut owner, &nil, false, &mut guard).is_err());
        assert_eq!(guard.used(), 1);
        owner.nodes[nodes[0].0 as usize].position = None;
        let unpositioned = SharedNodeSlice::from_nodes(&nodes[..1]).unwrap();
        assert!(optimizer_owner_median(&mut owner, &unpositioned, false, &mut guard).is_err());
        assert_eq!(guard.used(), 2);
        let mut limited = OptimizationWorkGuard::new(Some(&Quiet), "median", 0).unwrap();
        assert!(matches!(
            optimizer_owner_median(&mut owner, &nil, false, &mut limited),
            Err(OwnerPlacementError::Optimization(
                OptimizationError::ResourceLimit { .. }
            ))
        ));
        assert_eq!(limited.used(), 0);
    }

    #[test]
    fn median_tail_uses_selected_graph_and_consumes_the_actual_caller_rng_twice() {
        let (mut owner, nodes) = fixture();
        let rng = OwnerOptimizerRng::new(GoRng::new_untraced(1729));
        let mut expected = rng.stream().borrow().clone_without_trace();
        let optimizer = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(super::super::OwnerGraphId(0)),
            Some(rng.clone()),
            Some(&Quiet),
        )
        .unwrap();
        let neighbors = SharedNodeSlice::from_nodes(&nodes[1..2]).unwrap();
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "median", 100).unwrap();
        let x = 2.5 + (-2.0 + expected.float64() * 4.0);
        let y = -2.5 + (-2.0 + expected.float64() * 4.0);
        let result = optimizer
            .median_point_from_neighbors_guarded(&mut owner, nodes[0], &neighbors, 2.0, &mut guard)
            .unwrap();
        assert_eq!(
            result.get(),
            Point {
                x: x.round(),
                y: y.round()
            }
        );
        assert_eq!(rng.stream().borrow_mut().int63(), expected.int63());
        assert_eq!(guard.used(), 5);
    }

    #[test]
    fn tree_fields_and_overlapping_child_headers_are_live_and_numerical_copies_detach_together() {
        let (mut owner, nodes) = fixture();
        let parent = owner.new_owner_tree(Some(nodes[0])).unwrap();
        let a = owner.new_owner_tree(Some(nodes[1])).unwrap();
        let b = owner.new_owner_tree(Some(nodes[2])).unwrap();
        let children = OwnerTreeSlice::from_slots(&[Some(a), None], 3).unwrap();
        let mut fields = owner.owner_tree(parent).unwrap().fields();
        fields.children = children.clone();
        owner.set_owner_tree_fields(parent, fields).unwrap();
        let mut fields = owner.owner_tree(a).unwrap().fields();
        fields.children = children.reslice(1, 2).unwrap();
        owner.set_owner_tree_fields(a, fields).unwrap();
        let fork = owner.clone();
        let fork_parent = fork.owner_tree(parent).unwrap().fields().children;
        let fork_a = fork.owner_tree(a).unwrap().fields().children;
        assert_ne!(fork_parent.data_address(), children.data_address());
        assert_eq!(fork_parent.slot_address(1), fork_a.slot_address(0));
        assert!(fork_a.write_capacity_slot(0, Some(b)));
        assert_eq!(fork_parent.get(1), Some(Some(b)));
        assert_eq!(children.get(1), Some(None));
        let retained = owner.owner_tree(parent).unwrap().fields().children;
        let mut fields = owner.owner_tree(parent).unwrap().fields();
        fields.children = OwnerTreeSlice::nil();
        owner.set_owner_tree_fields(parent, fields).unwrap();
        children.write_capacity_slot(1, Some(b));
        assert_eq!(retained.get(1), Some(Some(b)));
        assert!(owner.owner_tree(parent).unwrap().fields().children.is_nil());
    }

    #[test]
    fn tree_nears_follow_backward_push_forward_visit_and_report_shared_children() {
        let (mut owner, nodes) = fixture();
        let parent = owner.new_owner_tree(Some(nodes[0])).unwrap();
        let a = owner.new_owner_tree(Some(nodes[1])).unwrap();
        let b = owner.new_owner_tree(Some(nodes[2])).unwrap();
        for (node, near) in [
            (nodes[0], nodes[3]),
            (nodes[1], nodes[2]),
            (nodes[2], nodes[1]),
        ] {
            owner.nodes[node.0 as usize].nears = NodeNearMap::from_raw_keys([Some(near)]);
        }
        let mut fields = owner.owner_tree(parent).unwrap().fields();
        fields.children = OwnerTreeSlice::from_slots(&[Some(a), Some(b)], 2).unwrap();
        owner.set_owner_tree_fields(parent, fields).unwrap();
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "tree", 100).unwrap();
        let nears =
            append_owner_tree_nears(&mut owner, SharedNodeSlice::nil(), Some(parent), &mut guard)
                .unwrap();
        assert_eq!(
            nears.visible_slots(),
            vec![Some(nodes[3]), Some(nodes[2]), Some(nodes[1])]
        );
        assert_eq!(guard.used(), 8);
        let mut fields = owner.owner_tree(parent).unwrap().fields();
        fields.children = OwnerTreeSlice::from_slots(&[Some(a), Some(a)], 2).unwrap();
        owner.set_owner_tree_fields(parent, fields).unwrap();
        assert_eq!(
            append_owner_tree_nears(&mut owner, SharedNodeSlice::nil(), Some(parent), &mut guard)
                .unwrap_err()
                .to_string(),
            "TALA tree found a cycle or shared child in a tree"
        );
        let mut fields = owner.owner_tree(a).unwrap().fields();
        fields.parent = Some(b);
        owner.set_owner_tree_fields(a, fields).unwrap();
        let mut fields = owner.owner_tree(b).unwrap().fields();
        fields.parent = Some(a);
        owner.set_owner_tree_fields(b, fields).unwrap();
        assert_eq!(
            optimizer_owner_tree_root(&mut owner, Some(a), &mut guard)
                .unwrap_err()
                .to_string(),
            "TALA tree found a cycle in tree ancestry"
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn singleton_sort_polls_read_live_fields_and_validate_cell_before_coordinate_dereference() {
        struct Mutate {
            polls: Cell<usize>,
            node: NodeId,
        }
        impl Context for Mutate {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 2 {
                    owner.nodes[self.node.0 as usize].position = None;
                    owner.cell_size = 0.0;
                }
                None
            }
        }
        let (mut owner, nodes) = fixture();
        let context = Mutate {
            polls: Cell::new(0),
            node: nodes[0],
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "median", 100, &mut owner)
                .unwrap();
        let nodes = SharedNodeSlice::from_nodes(&nodes[..1]).unwrap();
        assert_eq!(
            optimizer_owner_median(&mut owner, &nodes, true, &mut guard)
                .unwrap_err()
                .to_string(),
            "TALA median requires a finite positive cell size"
        );
        assert_eq!((guard.used(), context.polls.get()), (1, 3));
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn tree_root_reads_parent_after_poll_and_child_loop_retains_original_header() {
        struct Mutate {
            polls: Cell<usize>,
            root: OwnerTreeId,
            when: usize,
        }
        impl Context for Mutate {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == self.when {
                    let mut fields = owner.owner_tree(self.root).unwrap().fields();
                    fields.children.write_capacity_slot(0, None);
                    fields.children = OwnerTreeSlice::nil();
                    fields.parent = None;
                    owner.set_owner_tree_fields(self.root, fields).unwrap();
                }
                None
            }
        }
        let (mut owner, nodes) = fixture();
        let root = owner.new_owner_tree(Some(nodes[0])).unwrap();
        let child = owner.new_owner_tree(Some(nodes[1])).unwrap();
        let mut fields = owner.owner_tree(root).unwrap().fields();
        fields.children = OwnerTreeSlice::from_slots(&[Some(child)], 1).unwrap();
        fields.parent = Some(child);
        owner.set_owner_tree_fields(root, fields).unwrap();
        let context = Mutate {
            polls: Cell::new(0),
            root,
            when: 2,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "tree", 100, &mut owner).unwrap();
        guard.add_with_owner(63, &mut owner).unwrap();
        assert_eq!(
            optimizer_owner_tree_root(&mut owner, Some(root), &mut guard).unwrap(),
            None
        );
        assert_eq!(guard.used(), 64);
        // Restore a child header, then trigger the callback at the child's
        // Step: its already loaded pointer must survive the nil slot edit.
        let mut fields = owner.owner_tree(root).unwrap().fields();
        fields.children = OwnerTreeSlice::from_slots(&[Some(child)], 1).unwrap();
        owner.set_owner_tree_fields(root, fields).unwrap();
        let context = Mutate {
            polls: Cell::new(0),
            root,
            when: 3,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "tree", 100, &mut owner).unwrap();
        guard.add_with_owner(62, &mut owner).unwrap();
        append_owner_tree_nears(&mut owner, SharedNodeSlice::nil(), Some(root), &mut guard)
            .unwrap();
        assert_eq!((guard.used(), context.polls.get()), (65, 4));
    }
}
