// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Size-aware placement on the graph's pixel-scaled cell lattice.
//!
//! This stage refines the dimension-independent placement using rendered box
//! sizes, route obstructions, symmetry, alignment, and fixed-position rules.

use super::go_rng::GoRng;
use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::{
    ArenaGraph, CONTAINER_PADDING, NodeId, OwnerScoreSession, OwnerTopologyError, Point, Rect,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[path = "sized_generation.rs"]
mod generation;
#[path = "sized_median.rs"]
mod median;
#[path = "sized_movement_adapter.rs"]
pub(super) mod movement_adapter;
#[path = "sized_point_shuffle.rs"]
mod point_shuffle;
#[path = "sized_rollback.rs"]
mod rollback;
use super::optimizer_scoring_work as scoring_work;
#[path = "sized_setup.rs"]
mod setup;

fn scoring_error(error: super::limits::Error) -> OptimizationError {
    match error {
        super::limits::Error::Context { location, cause } => {
            OptimizationError::Context { location, cause }
        }
        error => OptimizationError::Invalid(error.to_string()),
    }
}

const PRECISION: f64 = 0.0001;

pub(super) fn trace_sized_graph_scope(graph: &ArenaGraph) -> bool {
    let Some(ids) = crate::internal::trace_env_value("TALA_TRACE_SIZED_STATE_IDS") else {
        return false;
    };
    if ids == "all" {
        return true;
    }
    if let Some(target) = ids
        .strip_prefix("contains:")
        .and_then(|target| target.trim().parse::<i64>().ok())
    {
        return graph.nodes.iter().any(|node| {
            node.tala_id == target || node.external_id.parse::<i64>().ok() == Some(target)
        });
    }
    let wanted = ids
        .split(',')
        .filter_map(|id| id.trim().parse::<i64>().ok())
        .collect::<BTreeSet<_>>();
    wanted.len() == graph.nodes.len()
        && graph.nodes.iter().all(|node| {
            wanted.contains(&node.tala_id)
                || node
                    .external_id
                    .parse::<i64>()
                    .is_ok_and(|external_id| wanted.contains(&external_id))
        })
}

fn trace_sized_node_debug_id(external_id: &str, tala_id: i64) -> String {
    if external_id.is_empty() {
        tala_id.to_string()
    } else {
        external_id.to_owned()
    }
}

#[cfg(feature = "diagnostic-traces")]
fn trace_sized_optimizer_order(
    phase: &str,
    temperature: f64,
    nodes: &[NodeId],
    graph: &ArenaGraph,
) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_SIZED_CALLS")
        || !trace_sized_graph_scope(graph)
    {
        return;
    }
    let event = serde_json::json!({
        "event": "sized-optimize",
        "phase": phase,
        "temperature_bits": format!("{:016x}", temperature.to_bits()),
        "nodes": nodes.iter().map(|node| {
            let node = &graph.nodes[node.0 as usize];
            trace_sized_node_debug_id(&node.external_id, node.tala_id)
        }).collect::<Vec<_>>(),
    });
    eprintln!("{event}");
}

#[cfg(feature = "diagnostic-traces")]
fn trace_sized_optimizer_result(temperature: f64, changed: bool, graph: &ArenaGraph) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_ZERO_RESULT")
        || temperature != 0.0
        || !trace_sized_graph_scope(graph)
    {
        return;
    }
    let event = serde_json::json!({
        "event": "sized-zero-result",
        "changed": changed,
    });
    eprintln!("{event}");
}

#[cfg(not(feature = "diagnostic-traces"))]
fn trace_sized_optimizer_result(_temperature: f64, _changed: bool, _graph: &ArenaGraph) {}

#[cfg(not(feature = "diagnostic-traces"))]
fn trace_sized_optimizer_order(
    _phase: &str,
    _temperature: f64,
    _nodes: &[NodeId],
    _graph: &ArenaGraph,
) {
}

// The comparison boundary is open; changing it alters candidate tie-breaking.
fn sized_precision_compare(left: f64, right: f64) -> std::cmp::Ordering {
    if (left - right).abs() < PRECISION {
        std::cmp::Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

#[cfg(feature = "diagnostic-traces")]
fn trace_sized_external_positions(graph: &ArenaGraph) -> BTreeMap<String, serde_json::Value> {
    let wanted = crate::internal::trace_env_value("TALA_TRACE_SIZED_EXTERNAL_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|id| id.trim().parse::<i64>().ok())
        .collect::<BTreeSet<_>>();
    graph
        .transaction_external_containers
        .iter()
        .filter(|node| wanted.contains(&node.tala_id))
        .filter_map(|node| {
            let position = node.position?;
            Some((
                node.tala_id.to_string(),
                serde_json::json!({
                    "x_bits": format!("{:016x}", position.x.to_bits()),
                    "y_bits": format!("{:016x}", position.y.to_bits()),
                }),
            ))
        })
        .collect()
}

#[cfg(not(feature = "diagnostic-traces"))]
#[inline(always)]
fn trace_sized_external_positions(_graph: &ArenaGraph) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::new()
}

fn trace_sized_optimizer_scope(nodes: &[NodeId], graph: &ArenaGraph) -> bool {
    let Some(ids) = crate::internal::trace_env_value("TALA_TRACE_SIZED_STATE_IDS") else {
        return false;
    };
    if let Some(target) = ids
        .strip_prefix("contains:")
        .and_then(|target| target.trim().parse::<i64>().ok())
    {
        return nodes
            .iter()
            .any(|node| graph.nodes[node.0 as usize].tala_id == target);
    }
    let wanted = ids
        .split(',')
        .filter_map(|id| id.trim().parse::<i64>().ok())
        .collect::<BTreeSet<_>>();
    wanted.len() == nodes.len()
        && nodes
            .iter()
            .all(|node| wanted.contains(&graph.nodes[node.0 as usize].tala_id))
}

fn edges_without_hub_spokes(
    graph: &ArenaGraph,
    node: NodeId,
    spokes: &[NodeId],
    old_edges: &[super::EdgeId],
) -> Vec<super::EdgeId> {
    // Match Go's `retryWithoutHubSpokes`: for each spoke, mark only the first
    // matching edge in the original adjacency slice. Parallel edges to the
    // same spoke therefore retain their remaining copies.
    let mut removed = vec![false; old_edges.len()];
    for spoke in spokes {
        for (index, edge_id) in old_edges.iter().copied().enumerate() {
            if removed[index] {
                continue;
            }
            let edge = &graph.edge_records[edge_id.0 as usize];
            let adjacent = if edge.from == node {
                edge.to
            } else {
                edge.from
            };
            if adjacent == *spoke {
                removed[index] = true;
                break;
            }
        }
    }
    old_edges
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, edge)| (!removed[index]).then_some(edge))
        .collect()
}

impl ArenaGraph {
    /// Recovered `Node.isAdjacentTo(candidate, true)` swap gate.
    pub(super) fn sized_swap_adjacent(&self, node: NodeId, candidate: NodeId) -> bool {
        self.active_box_distance_to(node, candidate) <= self.cell_size
    }

    /// `Node.distanceTo` observes the active Graph node.  In the flattened
    /// arena a cluster/sequence carrier is represented by a stable member, so
    /// using that member's raw box here admits swaps that the distinct Go
    /// synthetic vessel would reject.
    fn active_box_distance_to(&self, a: NodeId, b: NodeId) -> f64 {
        let Some(a_position) = self.active_node_position(a) else {
            return f64::INFINITY;
        };
        let Some(b_position) = self.active_node_position(b) else {
            return f64::INFINITY;
        };
        let a_size = self.active_node_size(a);
        let b_size = self.active_node_size(b);
        let dx = if b_position.x + b_size.width < a_position.x {
            a_position.x - (b_position.x + b_size.width)
        } else if a_position.x + a_size.width < b_position.x {
            b_position.x - (a_position.x + a_size.width)
        } else {
            0.0
        };
        let dy = if b_position.y + b_size.height < a_position.y {
            a_position.y - (b_position.y + b_size.height)
        } else if a_position.y + a_size.height < b_position.y {
            b_position.y - (a_position.y + a_size.height)
        } else {
            0.0
        };
        if dx != 0.0 && dy != 0.0 {
            dy.mul_add(dy, dx * dx).sqrt()
        } else {
            dx + dy
        }
    }
}

pub(super) struct SizedOptimizer<'a> {
    graph: &'a mut ArenaGraph,
    nodes: Vec<NodeId>,
    components: Vec<Vec<NodeId>>,
    component_index: Vec<Option<usize>>,
    edge_abduction_nodes: Option<BTreeSet<NodeId>>,
    rng: GoRng,
    cell_size: f64,
    fixed_origin: Option<Point>,
    obstacles: Vec<Rect>,
    mutation_scratch: super::owner_score_session::OwnerOptimizerMutationSnapshot,
    spatial_index: super::optimizer_spatial_index::OptimizerSpatialIndex,
    placement_scratch: generation::PlacementScratch,
}

impl<'a> SizedOptimizer<'a> {
    pub(super) fn graph(&self) -> &ArenaGraph {
        self.graph
    }

    pub(super) fn graph_mut(&mut self) -> &mut ArenaGraph {
        self.graph
    }

    pub(super) fn new(
        graph: &'a mut ArenaGraph,
        rng: GoRng,
        edge_abduction_nodes: Option<BTreeSet<NodeId>>,
    ) -> Self {
        // The recovered NewSizedOptimizer only derives its neighbor metadata
        // from the post-compaction graph.  Its constructor does not publish or
        // otherwise rewrite shared nested-member geometry; that lifecycle
        // boundary is handled by the owning placement scope before this call.
        graph.refresh_node_order_membership();
        let cell_size = graph.cell_size;
        let components = graph.optimizable_components();
        // Unlike NewSizelessOptimizer, the recovered sized optimizer shuffles
        // every `g.Nodes` index and skips fixed/non-optimizable nodes only
        // inside its loop. Keeping those skipped indices is observable because
        // Go's Shuffle advances the shared RNG stream.
        let nodes = graph.sized_optimizer_nodes();
        if crate::internal::trace_env_enabled("TALA_TRACE_HUB_OPT") {
            eprint!("HUB_OPT_RUST nodes={}", nodes.len());
            for (hub, spokes) in &graph.hubs {
                eprint!(" {}=[", graph.active_node_tala_id(hub));
                for spoke in spokes {
                    eprint!(" {}", graph.active_node_tala_id(spoke));
                }
                eprint!("]");
            }
            eprintln!();
        }
        let mut component_index = vec![None; graph.nodes.len()];
        for (index, component) in components.iter().enumerate() {
            for node in component {
                component_index[node.0 as usize] = Some(index);
            }
        }
        // sized_optimizer.go182–222: independent full integer requirements.
        // Assign only if a qualifying parallel-edge neighbor exists; otherwise
        // retain a prior constructor's map, as in the source node field.
        for node in nodes.iter().copied() {
            let abducted_edges = graph
                .sized_edge_abductions
                .iter()
                .filter(|abduction| {
                    (abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                        || abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            == Some(node))
                        && (abduction.originally_from.is_some()
                            || abduction.originally_to.is_some())
                })
                .filter_map(|abduction| abduction.edge)
                .collect::<BTreeSet<_>>();
            let mut requirements =
                BTreeMap::<NodeId, super::model::LongDistanceNeighborRequirements>::new();
            for edge_id in graph.nodes[node.0 as usize].edges.read().iter().copied() {
                if abducted_edges.contains(&edge_id) {
                    continue;
                }
                let edge = &graph.edge_records[edge_id.0 as usize];
                let adjacent = if edge.from == node {
                    edge.to
                } else {
                    edge.from
                };
                let requirement = requirements.entry(adjacent).or_default();
                requirement.edge_count = requirement.edge_count.wrapping_add(1);
                let width = super::loop_offsets::loop_spacing_units(edge.min_width);
                let height = super::loop_offsets::loop_spacing_units(edge.min_height);
                if width > requirement.max_width {
                    requirement.max_width = width;
                }
                if height > requirement.max_height {
                    requirement.max_height = height;
                }
            }
            let cell_size_integer = super::loop_offsets::loop_spacing_units(cell_size);
            if requirements.values().any(|requirement| {
                requirement.edge_count >= 3
                    && (requirement.max_width > cell_size_integer
                        || requirement.max_height > cell_size_integer)
            }) {
                graph
                    .set_owner_node_requirement_map(
                        node,
                        super::node_requirements_map::NodeRequirementsMap::from_entries(
                            requirements
                                .into_iter()
                                .map(|(key, value)| (Some(key), value)),
                        ),
                    )
                    .expect("sized node requirement field");
            }
        }
        let fixed_origin =
            graph.container_fixed_origin(graph.nodes.first().and_then(|n| n.container));
        Self {
            graph,
            nodes,
            components,
            component_index,
            edge_abduction_nodes,
            rng,
            cell_size,
            fixed_origin,
            obstacles: Vec::new(),
            mutation_scratch: Default::default(),
            spatial_index: Default::default(),
            placement_scratch: Default::default(),
        }
    }

    pub(super) fn with_obstacles(mut self, obstacles: Vec<Rect>) -> Self {
        self.obstacles = obstacles;
        self
    }

    pub(super) fn into_rng(self) -> GoRng {
        self.rng
    }

    #[cfg(test)]
    pub(super) fn test_positions<const N: usize>(&self, nodes: [NodeId; N]) -> [Point; N] {
        nodes.map(|node| {
            self.graph
                .position(node)
                .expect("positioned optimizer node")
        })
    }

    #[cfg(test)]
    pub(super) fn test_checked_offset_reuse(&self, node: NodeId, median: Point) -> (bool, bool) {
        let mut checked = Some(Default::default());
        let first = self.find_unoccupied(node, median, 0.0, 0.0, false, &mut checked);
        let second = self.find_unoccupied(node, median, 0.0, 0.0, false, &mut checked);
        (first, second)
    }

    #[cfg(test)]
    pub(super) fn test_placements(
        &self,
        node: NodeId,
        median: Point,
        minimum: f64,
        minimizing_self: bool,
    ) -> Vec<Point> {
        self.placements(node, median, minimum, minimizing_self)
    }

    fn round_to_cell(&self, value: f64) -> f64 {
        (value / self.cell_size).round() * self.cell_size
    }

    fn same_component(&self, left: NodeId, right: NodeId) -> bool {
        self.component_index[left.0 as usize].is_some()
            && self.component_index[left.0 as usize] == self.component_index[right.0 as usize]
    }

    fn median_point(
        &mut self,
        node: NodeId,
        temperature: f64,
        protruding_children: &[super::ProjectedAdjacent],
    ) -> Point {
        self.median_point_from_base(node, temperature, protruding_children, None)
    }

    fn median_point_from_base(
        &mut self,
        node: NodeId,
        temperature: f64,
        protruding_children: &[super::ProjectedAdjacent],
        owner_median: Option<Point>,
    ) -> Point {
        let size = self.graph.nodes[node.0 as usize].rect.size;
        let mut median = owner_median.unwrap_or_else(|| self.graph.sized_median_to_neighbors(node));
        let base_median = median;
        let width = size.width / self.cell_size;
        let height = size.height / self.cell_size;

        // v0.9.0 medianPointGuarded searches siblings in the owning scope,
        // including the root scope, for their current fixed origin. A node
        // may not propose a median below it. The first clamp preserves the temperature
        // margin when the neighbor median is already outside the origin;
        // the final clamp below applies the hard lower bound after the
        // random perturbation and protruding-child correction.
        let fixed_origin = self
            .graph
            .container_fixed_origin(self.graph.nodes[node.0 as usize].container);
        if let Some(origin) = fixed_origin {
            let origin_x = origin.x / self.cell_size;
            let origin_y = origin.y / self.cell_size;
            if median.x < origin_x {
                median.x = origin_x + temperature * width;
            }
            if median.y < origin_y {
                median.y = origin_y + temperature * height;
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        let pre_jitter = median;
        let random_x = self.rng.float64();
        let random_y = self.rng.float64();
        // Go permits its compiler to fuse the random multiply and addition
        // in `(-temp*size) + random*(2*temp*size)`. Preserve that rounded
        // operation with mul_add; separate multiply/add changes candidate
        // geometry by a few ULPs and can change later acceptance decisions.
        median.x += random_x.mul_add(2.0 * temperature * width, -(temperature * width));
        median.y += random_y.mul_add(2.0 * temperature * height, -(temperature * height));
        #[cfg(feature = "diagnostic-traces")]
        let post_jitter = median;
        let projected_children_median = self
            .graph
            .sized_projected_children_median(protruding_children);
        if let Some(children_median) = projected_children_median {
            let node_position = self.graph.position(node).unwrap_or_default();
            median.x -= children_median.x - node_position.x / self.cell_size;
            median.y -= children_median.y - node_position.y / self.cell_size;
        }
        if let Some(origin) = fixed_origin {
            median.x = median.x.max(origin.x / self.cell_size);
            median.y = median.y.max(origin.y / self.cell_size);
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("D2RS_TRACE_MEDIAN_DETAIL_NODE")
            .as_deref()
            .is_some_and(|target| {
                target.split(',').any(|candidate| {
                    candidate == "all"
                        || candidate == self.graph.nodes[node.0 as usize].external_id
                        || candidate.parse::<i64>().ok()
                            == Some(self.graph.nodes[node.0 as usize].tala_id)
                })
            })
            && trace_sized_graph_scope(self.graph)
            && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
        {
            let (fixed_origin_x, fixed_origin_y) = fixed_origin
                .map(|origin| (origin.x, origin.y))
                .unwrap_or((f64::NAN, f64::NAN));
            let position = self.graph.position(node).unwrap_or_default();
            let graph_nodes = self
                .graph
                .nodes
                .iter()
                .map(|member| {
                    let (x_bits, y_bits) = member.position.map_or_else(
                        || ("nil".to_owned(), "nil".to_owned()),
                        |position| {
                            (
                                format!("{:016x}", position.x.to_bits()),
                                format!("{:016x}", position.y.to_bits()),
                            )
                        },
                    );
                    serde_json::json!({
                        "external_id": member.external_id,
                        "tala_id": member.tala_id.to_string(),
                        "x_bits": x_bits,
                        "y_bits": y_bits,
                        "width_bits": format!("{:016x}", member.rect.size.width.to_bits()),
                        "height_bits": format!("{:016x}", member.rect.size.height.to_bits()),
                    })
                })
                .collect::<Vec<_>>();
            let graph_edges = self
                .graph
                .member_edges()
                .map(|edge| {
                    serde_json::json!({
                        "source": self.graph.nodes[edge.from.0 as usize].external_id,
                        "target": self.graph.nodes[edge.to.0 as usize].external_id,
                    })
                })
                .collect::<Vec<_>>();
            let protruding_children_detail = protruding_children
                .iter()
                .map(|child| {
                    let owner_position = self.graph.position(child.owner);
                    let external_position =
                        self.graph.projected_external_position(child.tala_id);
                    let live_external_position =
                        self.graph.projected_live_external_position(child.tala_id);
                    let source_position = self
                        .graph
                        .projected_source_position(child.tala_id, &mut BTreeSet::new());
                    let position = external_position
                        .or_else(|| self.graph.projected_absolute_position(*child));
                    let center = position.map(|position| Point {
                        x: position.x + child.size.width * 0.5,
                        y: position.y + child.size.height * 0.5,
                    });
                    serde_json::json!({
                        "tala_id": child.tala_id.to_string(),
                        "owner_tala_id": self.graph.nodes[child.owner.0 as usize].tala_id.to_string(),
                        "container_tala_id": child.container_tala_id.map(|id| id.to_string()),
                        "owner_x_bits": owner_position.map(|point| format!("{:016x}", point.x.to_bits())),
                        "owner_y_bits": owner_position.map(|point| format!("{:016x}", point.y.to_bits())),
                        "offset_x_bits": format!("{:016x}", child.offset.x.to_bits()),
                        "offset_y_bits": format!("{:016x}", child.offset.y.to_bits()),
                        "x_bits": position.map(|point| format!("{:016x}", point.x.to_bits())),
                        "y_bits": position.map(|point| format!("{:016x}", point.y.to_bits())),
                        "width_bits": format!("{:016x}", child.size.width.to_bits()),
                        "height_bits": format!("{:016x}", child.size.height.to_bits()),
                        "center_x_bits": center.map(|point| format!("{:016x}", point.x.to_bits())),
                        "center_y_bits": center.map(|point| format!("{:016x}", point.y.to_bits())),
                        "external_x_bits": external_position.map(|point| format!("{:016x}", point.x.to_bits())),
                        "external_y_bits": external_position.map(|point| format!("{:016x}", point.y.to_bits())),
                        "live_external_x_bits": live_external_position.map(|point| format!("{:016x}", point.x.to_bits())),
                        "live_external_y_bits": live_external_position.map(|point| format!("{:016x}", point.y.to_bits())),
                        "source_x_bits": source_position.map(|point| format!("{:016x}", point.x.to_bits())),
                        "source_y_bits": source_position.map(|point| format!("{:016x}", point.y.to_bits())),
                        "cluster_member": child.cluster_member,
                    })
                })
                .collect::<Vec<_>>();
            let children_median_x_bits =
                projected_children_median.map(|point| format!("{:016x}", point.x.to_bits()));
            let children_median_y_bits =
                projected_children_median.map(|point| format!("{:016x}", point.y.to_bits()));
            let fixed_origin_x_bits = fixed_origin
                .map(|_| format!("{:016x}", fixed_origin_x.to_bits()))
                .unwrap_or_else(|| "none".to_owned());
            let fixed_origin_y_bits = fixed_origin
                .map(|_| format!("{:016x}", fixed_origin_y.to_bits()))
                .unwrap_or_else(|| "none".to_owned());
            let event = serde_json::json!({
                "event": "sized-median-detail",
                "node": self.graph.nodes[node.0 as usize].external_id,
                "temperature_bits": format!("{:016x}", temperature.to_bits()),
                "cell_size_bits": format!("{:016x}", self.cell_size.to_bits()),
                "width_bits": format!("{:016x}", width.to_bits()),
                "height_bits": format!("{:016x}", height.to_bits()),
                "base_median_x_bits": format!("{:016x}", base_median.x.to_bits()),
                "base_median_y_bits": format!("{:016x}", base_median.y.to_bits()),
                "fixed_origin_x_bits": fixed_origin_x_bits,
                "fixed_origin_y_bits": fixed_origin_y_bits,
                "pre_jitter_x_bits": format!("{:016x}", pre_jitter.x.to_bits()),
                "pre_jitter_y_bits": format!("{:016x}", pre_jitter.y.to_bits()),
                "random_x_bits": format!("{:016x}", random_x.to_bits()),
                "random_y_bits": format!("{:016x}", random_y.to_bits()),
                "post_jitter_x_bits": format!("{:016x}", post_jitter.x.to_bits()),
                "post_jitter_y_bits": format!("{:016x}", post_jitter.y.to_bits()),
                "final_x_bits": format!("{:016x}", median.x.to_bits()),
                "final_y_bits": format!("{:016x}", median.y.to_bits()),
                "top_left_x_bits": format!("{:016x}", position.x.to_bits()),
                "top_left_y_bits": format!("{:016x}", position.y.to_bits()),
                "protruding_children_n": protruding_children.len(),
                "children_median_x_bits": children_median_x_bits,
                "children_median_y_bits": children_median_y_bits,
                "protruding_children": protruding_children_detail,
                "graph_nodes": graph_nodes,
                "graph_edges": graph_edges,
            });
            eprintln!("{event}");
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_MEDIAN_DETAIL") {
            let ingestion_position = self
                .graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == 3_302_651_804)
                .and_then(|candidate| candidate.position);
            eprintln!(
                "MEDIAN_DETAIL_RUST node={} base={},{} temp={} random={},{} final={},{} ingestion={:?}",
                self.graph.nodes[node.0 as usize].tala_id,
                base_median.x,
                base_median.y,
                temperature,
                random_x,
                random_y,
                median.x,
                median.y,
                ingestion_position
            );
        }
        Point {
            x: (median.x * self.cell_size).round(),
            y: (median.y * self.cell_size).round(),
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_sized_candidate_occupancy(
        &self,
        node: NodeId,
        point: Point,
        occupied: bool,
        uses_unset_direction: bool,
    ) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_SIZED_OCCUPANCY")
            || !trace_sized_graph_scope(self.graph)
            || (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_some()
                && !crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
        {
            return;
        }
        let Some(target_node) = crate::internal::trace_env_value("D2RS_TRACE_SIZED_OCCUPANCY_NODE")
        else {
            return;
        };
        let moving = &self.graph.nodes[node.0 as usize];
        if target_node != "all"
            && target_node != moving.tala_id.to_string()
            && target_node != moving.external_id
        {
            return;
        }
        let Some(target_point) =
            crate::internal::trace_env_value("D2RS_TRACE_SIZED_OCCUPANCY_POINT")
        else {
            return;
        };
        if target_point != format!("{},{}", point.x, point.y) {
            return;
        }

        eprintln!(
            "SIZED_OCCUPANCY_RUST node={} point={},{} occupied={} unset_direction={} graph_nodes={}",
            moving.tala_id,
            point.x,
            point.y,
            occupied,
            uses_unset_direction,
            self.graph.nodes.len(),
        );
        for (graph_index, other) in self.graph.nodes.iter().enumerate() {
            if other.input_id == node {
                continue;
            }
            let Some(other_position) = other.position else {
                continue;
            };
            let moving_size = moving.rect.size;
            const MAX_SAFE_DELTA: f64 = 500.0;
            if point.x > other_position.x + other.rect.size.width + MAX_SAFE_DELTA
                || point.x + moving_size.width + MAX_SAFE_DELTA < other_position.x
                || point.y > other_position.y + other.rect.size.height + MAX_SAFE_DELTA
                || point.y + moving_size.height + MAX_SAFE_DELTA < other_position.y
            {
                continue;
            }
            let delta = self.graph.spacing_delta(node, other.input_id, point);
            let loop_delta = self
                .graph
                .spacing_delta_with_loops(node, other.input_id, point);
            let overlaps = |delta: f64| {
                point.x < other_position.x + other.rect.size.width + delta
                    && point.x + moving_size.width + delta > other_position.x
                    && point.y < other_position.y + other.rect.size.height + delta
                    && point.y + moving_size.height + delta > other_position.y
            };
            if overlaps(delta) || overlaps(loop_delta) {
                eprintln!(
                    "SIZED_OCCUPANCY_BLOCKER_RUST graph_index={} other={} external={} at={},{} size={},{} delta={} delta_bits={:016x} loop_delta={} loop_delta_bits={:016x} overlaps={} overlaps_with_loops={}",
                    graph_index,
                    other.tala_id,
                    other.external_id,
                    other_position.x,
                    other_position.y,
                    other.rect.size.width,
                    other.rect.size.height,
                    delta,
                    delta.to_bits(),
                    loop_delta,
                    loop_delta.to_bits(),
                    overlaps(delta),
                    overlaps(loop_delta),
                );
            }
        }
    }

    fn point_is_occupied(&self, node: NodeId, point: Point) -> bool {
        if self
            .fixed_origin
            .is_some_and(|origin| point.x < origin.x || point.y < origin.y)
        {
            return true;
        }
        // optimizerCanMove first performs Graph.isOccupied, which rejects an
        // exact top-left collision even when the boxes do not overlap under
        // the spacing predicate. Keep that identity check ahead of the
        // dimension-aware overlap scan.
        if self
            .graph
            .nodes
            .iter()
            .any(|other| other.input_id != node && other.position == Some(point))
        {
            return true;
        }
        if self.graph.container_direction_is_unset(node) {
            let occupied = self.graph.sized_point_overlaps(node, point);
            #[cfg(feature = "diagnostic-traces")]
            self.trace_sized_candidate_occupancy(node, point, occupied, true);
            if crate::internal::trace_env_enabled("TALA_TRACE_OCCUPANCY")
                && self.graph.nodes[node.0 as usize].tala_id == 2_699_771_459
                && point
                    == (Point {
                        x: 693.0,
                        y: 1386.0,
                    })
            {
                eprintln!(
                    "OCCUPANCY_RUST node={} point={},{} occupied={} unset=true",
                    self.graph.nodes[node.0 as usize].tala_id, point.x, point.y, occupied,
                );
                for other in &self.graph.nodes {
                    let Some(other_position) = other.position else {
                        continue;
                    };
                    if other.input_id == node {
                        continue;
                    }
                    let delta = self
                        .graph
                        .spacing_delta_with_loops(node, other.input_id, point);
                    let overlaps = point.x < other_position.x + other.rect.size.width + delta
                        && point.x + self.graph.nodes[node.0 as usize].rect.size.width + delta
                            > other_position.x
                        && point.y < other_position.y + other.rect.size.height + delta
                        && point.y + self.graph.nodes[node.0 as usize].rect.size.height + delta
                            > other_position.y;
                    if overlaps {
                        eprintln!(
                            "OCCUPANCY_BLOCKER_RUST other={} pos={},{} size={},{} delta={}",
                            other.tala_id,
                            other_position.x,
                            other_position.y,
                            other.rect.size.width,
                            other.rect.size.height,
                            delta,
                        );
                    }
                }
            }
            return occupied;
        }
        let occupied = self.graph.nodes.iter().any(|other| {
            // Graph.isOccupied/Graph.doesOverlap scan the active Graph.Nodes
            // slice directly.  This arena may retain multiple placement
            // components in one scoped graph, so do not silently narrow the
            // recovered global obstruction scan to the optimizer's local
            // connectivity partition.
            if other.input_id == node {
                return false;
            }
            let Some(other_position) = other.position else {
                return false;
            };
            let node_size = self.graph.nodes[node.0 as usize].rect.size;
            // `optimizerDoesOverlap` has a 500px broad-phase guard before
            // calling Node.DeltaTo.  Keep the same guard on the directed
            // container path; without it, distant nodes can still contribute
            // direction-dependent margins to a candidate even though Go
            // skips them entirely.
            const MAX_SAFE_DELTA: f64 = 500.0;
            if point.x > other_position.x + other.rect.size.width + MAX_SAFE_DELTA
                || point.x + node_size.width + MAX_SAFE_DELTA < other_position.x
                || point.y > other_position.y + other.rect.size.height + MAX_SAFE_DELTA
                || point.y + node_size.height + MAX_SAFE_DELTA < other_position.y
            {
                return false;
            }
            let delta = self.graph.spacing_delta(node, other.input_id, point);
            point.x < other_position.x + other.rect.size.width + delta
                && point.x + node_size.width + delta > other_position.x
                && point.y < other_position.y + other.rect.size.height + delta
                && point.y + node_size.height + delta > other_position.y
        });
        #[cfg(feature = "diagnostic-traces")]
        self.trace_sized_candidate_occupancy(node, point, occupied, false);
        occupied
    }

    /// Reproduce placement.optimizerCanMove for sized candidates.  The Go
    /// path performs an exact-top-left occupancy check and then scans every
    /// graph node with the 500px safe-delta guard before applying DeltaTo;
    /// the search-time overlap helper is intentionally narrower and cannot be
    /// used here without admitting candidates that Go rejects.
    fn can_move_to(&self, node: NodeId, point: Point) -> bool {
        !self.point_is_occupied(node, point)
    }

    fn search_point_is_occupied(&self, node: NodeId, point: Point) -> bool {
        // Recovered isPointOccupied first calls Graph.isOccupied, which treats
        // even the node's own current top-left as occupied. The subsequent
        // overlap probe is the part that excludes the moving node.
        self.graph.nodes.iter().any(|candidate| {
            self.same_component(node, candidate.input_id) && candidate.position == Some(point)
        }) || self.point_is_occupied(node, point)
    }

    fn find_unoccupied(
        &self,
        node: NodeId,
        median: Point,
        offset_x: f64,
        offset_y: f64,
        minimizing_self: bool,
        checked: &mut Option<super::optimizer_spatial_index::CheckedOffsets>,
    ) -> bool {
        let offset_key = |x: f64, y: f64| {
            let normalized_bits = |value: f64| {
                if value == 0.0 {
                    0.0_f64.to_bits()
                } else {
                    value.to_bits()
                }
            };
            (normalized_bits(x), normalized_bits(y))
        };
        let size = self.graph.nodes[node.0 as usize].rect.size;
        let direct = Point {
            x: self.round_to_cell(median.x + offset_x),
            y: self.round_to_cell(median.y + offset_y),
        };
        let skip_direct = checked
            .as_mut()
            .is_some_and(|checked| !checked.insert(offset_key(offset_x, offset_y)));
        if !skip_direct {
            let occupied = self.search_point_is_occupied(node, direct);
            if !occupied {
                return true;
            }
        }
        if !minimizing_self {
            return false;
        }
        let mut x = offset_x - size.width;
        while x <= offset_x {
            let mut y = offset_y - size.height;
            while y <= offset_y {
                let candidate = Point {
                    x: self.round_to_cell(median.x + x),
                    y: self.round_to_cell(median.y + y),
                };
                if x == offset_x && y == offset_y {
                    y += self.cell_size;
                    continue;
                }
                let already_checked = checked
                    .as_mut()
                    .is_some_and(|checked| !checked.insert(offset_key(x, y)));
                if !already_checked {
                    let occupied = self.search_point_is_occupied(node, candidate);
                    if !occupied {
                        return true;
                    }
                }
                y += self.cell_size;
            }
            x += self.cell_size;
        }
        false
    }

    fn closest_unoccupied_distance(
        &self,
        node: NodeId,
        median: Point,
        minimizing_self: bool,
        checked: &mut Option<super::optimizer_spatial_index::CheckedOffsets>,
    ) -> f64 {
        for current in 0..=100_i32 {
            let distance = f64::from(current) * self.cell_size;
            let mut y = 0.0;
            let mut x = distance;
            while x >= -distance {
                if y != 0.0 {
                    let unoccupied =
                        self.find_unoccupied(node, median, x, -y, minimizing_self, checked);
                    if unoccupied {
                        return f64::from(current);
                    }
                }
                let unoccupied = self.find_unoccupied(node, median, x, y, minimizing_self, checked);
                if unoccupied {
                    return f64::from(current);
                }
                if x > 0.0 {
                    y += self.cell_size;
                } else {
                    y -= self.cell_size;
                }
                x -= self.cell_size;
            }
        }
        0.0
    }

    fn append_placement(
        &self,
        output: &mut Vec<Point>,
        seen: &mut HashSet<(i64, i64)>,
        median: Point,
        offset_x: f64,
        offset_y: f64,
    ) {
        let point = Point {
            x: self.round_to_cell(median.x + offset_x),
            y: self.round_to_cell(median.y + offset_y),
        };
        if self
            .fixed_origin
            .is_some_and(|origin| point.x < origin.x || point.y < origin.y)
        {
            return;
        }
        let key = (
            (point.x / self.cell_size) as i64,
            (point.y / self.cell_size) as i64,
        );
        if seen.insert(key) {
            output.push(point);
        }
    }

    fn placements(
        &self,
        node: NodeId,
        median: Point,
        minimum: f64,
        minimizing_self: bool,
    ) -> Vec<Point> {
        let distance = minimum + 1.0;
        let size = self.graph.nodes[node.0 as usize].rect.size;
        let mut points = Vec::new();
        // Only the insertion result is observable: generated points retain
        // their original order in `points`, and the key set is never
        // iterated. A hash set avoids rebalancing a tree for every candidate.
        let mut seen = HashSet::new();
        let mut x = distance;
        while x >= -distance {
            let maximum_y = distance - x.abs();
            let mut y = 0.0;
            while y <= maximum_y {
                let signed_y_values = [-y, y];
                let signed_y_start = usize::from(y == 0.0);
                for &signed_y in &signed_y_values[signed_y_start..] {
                    let offset_x = x * self.cell_size;
                    let offset_y = signed_y * self.cell_size;
                    self.append_placement(&mut points, &mut seen, median, offset_x, offset_y);
                    if minimizing_self {
                        let mut self_x = offset_x - size.width;
                        while self_x <= offset_x {
                            let mut self_y = offset_y - size.height;
                            while self_y <= offset_y {
                                if self_x != offset_x || self_y != offset_y {
                                    self.append_placement(
                                        &mut points,
                                        &mut seen,
                                        median,
                                        self_x,
                                        self_y,
                                    );
                                }
                                self_y += self.cell_size;
                            }
                            self_x += self.cell_size;
                        }
                    }
                }
                y += 1.0;
            }
            x -= 1.0;
        }
        let data = self
            .graph
            .owner_node_requirement_map_read(node)
            .expect("sized node requirement field");
        if !data.is_nil() {
            let cell = (self.cell_size as i64) as f64;
            let node_size = self.graph.nodes[node.0 as usize].rect.size;
            let mut neighbors = data
                .raw_keys()
                .into_iter()
                .map(|key| key.expect("ordinary placement requires a nonnil neighbor"))
                .collect::<Vec<_>>();
            neighbors
                .sort_unstable_by_key(|adjacent| self.graph.nodes[adjacent.0 as usize].tala_id);
            for adjacent in neighbors {
                let requirement = data.get(Some(adjacent));
                let adjacent_position = self.graph.position(adjacent).unwrap();
                let adjacent_size = self.graph.nodes[adjacent.0 as usize].rect.size;
                let max_width = requirement.max_width as f64;
                let max_height = requirement.max_height as f64;
                let ax = adjacent_position.x - median.x;
                let ay = adjacent_position.y - median.y;
                if max_width > cell {
                    self.append_placement(
                        &mut points,
                        &mut seen,
                        median,
                        ((ax - node_size.width - max_width) / cell).floor() * cell,
                        ay,
                    );
                    self.append_placement(
                        &mut points,
                        &mut seen,
                        median,
                        ((ax + adjacent_size.width + max_width) / cell).ceil() * cell,
                        ay,
                    );
                }
                if max_height > cell {
                    self.append_placement(
                        &mut points,
                        &mut seen,
                        median,
                        ax,
                        ((ay - node_size.height - max_height) / cell).floor() * cell,
                    );
                    self.append_placement(
                        &mut points,
                        &mut seen,
                        median,
                        ax,
                        ((ay + adjacent_size.height + max_height) / cell).ceil() * cell,
                    );
                }
            }
        }
        if self.graph.has_herd_assignment(node) {
            // Recovered getPlacementPoints appends the current herd position
            // after the generated inventory without consulting its point set.
            // The duplicate is observable because the complete slice is then
            // shuffled and advances the shared optimizer RNG stream.
            points.push(self.graph.position(node).unwrap());
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_POINTS")
            && crate::internal::trace_env_value("TALA_TRACE_SIZED_POINTS")
                .as_deref()
                .is_some_and(|target| {
                    target == "all"
                        || target.parse::<i64>().ok()
                            == Some(self.graph.nodes[node.0 as usize].tala_id)
                })
        {
            eprint!(
                "SIZED_POINTS_RUST node={} median={},{} min={} dist={} minimizingSelf={} cell={} size={},{} current={},{} count={}",
                self.graph.nodes[node.0 as usize].tala_id,
                median.x,
                median.y,
                minimum,
                distance,
                minimizing_self,
                self.cell_size,
                size.width,
                size.height,
                self.graph.position(node).unwrap().x,
                self.graph.position(node).unwrap().y,
                points.len()
            );
            for point in &points {
                eprint!(" point={},{}", point.x, point.y);
            }
            eprintln!();
        }
        points
    }

    fn move_to_best(&mut self, node: NodeId, points: &[Point], must_improve: bool) -> bool {
        self.move_to_best_with_owner_nears(node, points, must_improve, None, None)
            .expect("unbound candidate scoring cannot fail")
    }

    fn candidate_edge_length(
        &self,
        node: NodeId,
        cache: Option<&[Vec<NodeId>]>,
        session: &mut Option<&mut OwnerScoreSession>,
        guard: Option<&mut OptimizationWorkGuard<'_>>,
        owner_authoritative: bool,
        scorer: Option<&mut super::scoring::SizedEdgeLengthScorer>,
        context: super::scoring_context::ScoreContext<'_>,
        frame_validated: Option<&mut bool>,
    ) -> Result<f64, OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        if scorer.is_some() {
            crate::internal::call_counts::count!(SizedCandidateScore);
        }
        if let Some(session) = session.as_deref_mut() {
            if !owner_authoritative {
                session
                    .update_current_member_positions(self.graph)
                    .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            }
            let canonical = session
                .canonical_node(node)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            if let Some(guard) = guard {
                let abductions = session
                    .abductions()
                    .map_or(0, |records| records.records.len());
                scoring_work::charge(
                    session.owner_mut(),
                    Some(canonical),
                    abductions,
                    true,
                    guard,
                )?;
            }
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_value("D2RS_TRACE_CANDIDATE_OWNER_NODE")
                .is_some_and(|target| target == self.graph.nodes[node.0 as usize].external_id)
            {
                let owner = session.owner();
                eprintln!(
                    "CANDIDATE_OWNER_RUST node={} point={:?} children={:?}",
                    self.graph.nodes[node.0 as usize].external_id,
                    self.graph.position(node),
                    owner
                        .container_node_order(Some(canonical))
                        .iter()
                        .map(|&child| (
                            owner.nodes[child.0 as usize].tala_id,
                            owner.position(child),
                            owner.nodes[child.0 as usize].rect.size,
                        ))
                        .collect::<Vec<_>>(),
                );
            }
            let score = |frame: &crate::internal::OwnerScoreFrame<'_>| {
                if let Some(scorer) = scorer {
                    scorer.score_owner_routes(
                        self.graph,
                        cache,
                        frame.owner(),
                        canonical,
                        Some(frame),
                        context,
                    )
                } else {
                    self.graph.sized_edge_length_with_owner_routes_checked(
                        node,
                        true,
                        cache,
                        frame.owner(),
                        canonical,
                        Some(frame),
                        context,
                    )
                }
            };
            match frame_validated {
                Some(validated) => session.with_frame_reusing_mapping_validation(validated, score),
                None => session.with_frame(score),
            }
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?
            .map_err(scoring_error)
        } else {
            Ok(self.graph.sized_edge_length_with_cache(node, true, cache))
        }
    }

    fn move_to_best_with_owner_nears(
        &mut self,
        node: NodeId,
        points: &[Point],
        must_improve: bool,
        owner_session: Option<&mut OwnerScoreSession>,
        owner_guard: Option<&mut OptimizationWorkGuard<'_>>,
    ) -> Result<bool, OptimizationError> {
        self.move_to_best_with_context(node, points, must_improve, owner_session, owner_guard, None)
    }
    fn move_to_best_with_context(
        &mut self,
        node: NodeId,
        points: &[Point],
        must_improve: bool,
        owner_session: Option<&mut OwnerScoreSession>,
        mut owner_guard: Option<&mut OptimizationWorkGuard<'_>>,
        scoring_context: Option<&dyn super::limits::Context>,
    ) -> Result<bool, OptimizationError> {
        if let Some(session) = owner_session {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.move_to_best_inner(
                    node,
                    points,
                    must_improve,
                    Some(session),
                    owner_guard.as_deref_mut(),
                    scoring_context,
                )
            }));
            movement_adapter::mirror(self.graph, session);
            match result {
                Ok(result) => result,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        } else {
            self.move_to_best_inner(
                node,
                points,
                must_improve,
                None,
                owner_guard,
                scoring_context,
            )
        }
    }
    fn move_to_best_inner(
        &mut self,
        node: NodeId,
        points: &[Point],
        must_improve: bool,
        mut owner_session: Option<&mut OwnerScoreSession>,
        mut owner_guard: Option<&mut OptimizationWorkGuard<'_>>,
        scoring_context: Option<&dyn super::limits::Context>,
    ) -> Result<bool, OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(SizedMove);
        let scoring_context = scoring_context.or_else(|| owner_guard.as_ref().map(|g| g.context()));
        let mut scorer = super::scoring::SizedEdgeLengthScorer::new(node, true);
        let score_context = scoring_context.map_or_else(
            super::scoring_context::ScoreContext::none,
            super::scoring_context::ScoreContext::new,
        );
        // Candidate movement changes geometry only (candidate_movement.go).
        // Both contexts must preserve topology before a mapping validated at
        // one score can be reused at the next. Custom contexts keep full
        // validation after every work charge.
        let reuse_frame_validation = score_context.stable_owner()
            && owner_guard
                .as_ref()
                .is_none_or(|guard| guard.context().stable_sized_owner());
        let mut frame_validated = false;
        if reuse_frame_validation {
            scorer.use_stable_owner_obstructions();
        }
        // Go sized_optimizer.go:959–976 scores and prunes on the live owner
        // before asking for symmetry. Ordinary edge scores need no
        // numerical transport; publish only candidates that reach the still
        // workspace-backed crossing/symmetry terms below.
        let defer_candidate_publication = owner_session.as_deref().is_some_and(|session| {
            movement_adapter::owner_edge_score_with_inert_external(
                self.graph,
                session,
                node,
                scoring_context.is_some_and(|context| context.stable_sized_owner()),
            )
        });
        if defer_candidate_publication {
            scorer.use_owner_carrier_positions();
        }
        let trace_current = self.graph.position(node).unwrap_or_default();
        let trace_scoped_score = trace_sized_optimizer_scope(&self.nodes, self.graph)
            && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_SCORE")
            && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
            && crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE").is_some_and(
                |target| {
                    target == "all"
                        || target.split(',').any(|target| {
                            target == self.graph.nodes[node.0 as usize].external_id
                                || target.parse::<i64>().ok()
                                    == Some(self.graph.nodes[node.0 as usize].tala_id)
                        })
                },
            );
        let trace_move_target = crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE");
        let trace_move_always = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_MOVE_ALWAYS");
        let trace_move = (crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE")
            || trace_move_always)
            && (crate::internal::trace_env_value("TALA_TRACE_SIZED_STATE_IDS").is_none()
                || trace_sized_optimizer_scope(&self.nodes, self.graph))
            && (!crate::internal::trace_env_enabled("TALA_TRACE_ROOT_ONLY")
                || self.graph.nodes.len() == 15)
            && (trace_move_target.as_deref() == Some("all")
                || trace_move_target.as_deref().is_some_and(|targets| {
                    targets.split(',').any(|target| {
                        target == self.graph.nodes[node.0 as usize].external_id
                            || target.parse::<i64>().ok()
                                == Some(self.graph.nodes[node.0 as usize].tala_id)
                    })
                }));
        if trace_move {
            eprint!(
                "SIZED_MOVE_RUST node={} begin current={},{} must={must_improve} descendants=",
                self.graph.nodes[node.0 as usize].tala_id, trace_current.x, trace_current.y
            );
            for descendant in self.graph.active_descendants(node) {
                eprint!("{},", self.graph.nodes[descendant.0 as usize].tala_id);
            }
            for point in points {
                eprint!(" point={},{}", point.x, point.y);
            }
            eprintln!();
        }
        let edge_count = if let Some(session) = owner_session.as_deref() {
            let canonical = session
                .canonical_node(node)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            session.owner().owner_node_incident_count(canonical)
        } else {
            self.graph.nodes[node.0 as usize].edges.len()
        };
        let symmetry_cost = self.cell_size * edge_count as f64;
        // Candidate movement changes boxes but not the Graph.Containers walk
        // that supplies edge obstructions. Materialize the ordered inventory
        // once per node, as the sized compactor already does, instead of
        // rebuilding container chains for every candidate point.
        let mut obstruction_cache = vec![Vec::new(); self.graph.edge_records.len()];
        for edge in self.graph.active_edge_ids(node) {
            let adjacent = self.graph.active_adjacent(node, edge);
            obstruction_cache[edge.0 as usize] = self
                .graph
                .edge_obstruction_nodes(node, adjacent)
                .into_iter()
                .filter(|obstruction| {
                    *obstruction != node
                        && *obstruction != adjacent
                        && !self.graph.is_descendant_of(node, *obstruction)
                        && !self.graph.is_descendant_of(adjacent, *obstruction)
                })
                .collect();
        }
        let mut score = |optimizer: &Self| -> Result<f64, OptimizationError> {
            // Keep the baseline operation order used by Go's mustImprove
            // path: score the edge, add column crossings, then subtract the
            // symmetry credit.  Writing this as one expression permits a
            // different reassociation/rounding on a few non-integral scores.
            let cache =
                (!crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_OBSTRUCTION_CACHE"))
                    .then_some(&obstruction_cache[..]);
            let graph = &optimizer.graph;
            let mut baseline = optimizer.candidate_edge_length(
                node,
                cache,
                &mut owner_session,
                owner_guard.as_deref_mut(),
                false,
                Some(&mut scorer),
                score_context,
                reuse_frame_validation.then_some(&mut frame_validated),
            )?;
            baseline += graph
                .column_to_column_crossing_cost_checked(node, false, score_context)
                .map_err(scoring_error)?;
            // Go 1.27's arm64 compiler lowers this subtraction to FMSUBD;
            // use the explicit fused form so the baseline keeps the same ULP
            // as the candidate totals.
            baseline = (-graph
                .sized_symmetry_checked(node, true, score_context)
                .map_err(scoring_error)?)
            .mul_add(symmetry_cost, baseline);
            Ok(baseline)
        };
        let mut least = if must_improve {
            score(self)?
        } else {
            f64::INFINITY
        };
        let current = if let Some(session) = owner_session.as_deref() {
            let canonical = session
                .canonical_node(node)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            session.owner().position(canonical)
        } else {
            self.graph.position(node)
        }
        .expect("runtime error: invalid memory address or nil pointer dereference");
        let mut best = current;
        let mut movement = if let Some(session) = owner_session.take() {
            Some(
                session.optimizer_candidate_movement(
                    node,
                    owner_guard
                        .as_deref_mut()
                        .expect("owner candidate capture guard"),
                )?,
            )
        } else {
            None
        };
        for point_index in 0..points.len() {
            Self::step_owner_work(
                movement.as_mut().map(|m| m.session()),
                owner_guard.as_deref_mut(),
            )?;
            let point = points[point_index];
            if self
                .fixed_origin
                .is_some_and(|origin| point.x < origin.x || point.y < origin.y)
            {
                continue;
            }
            let can_move = if point == current {
                true
            } else if let Some(session) = movement.as_mut().map(|m| m.session()) {
                let canonical = session
                    .canonical_node(node)
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
                let receiver = session.current_view();
                self.spatial_index.can_move(
                    session.owner_mut(),
                    receiver,
                    Some(canonical),
                    Some(&point),
                    owner_guard
                        .as_deref_mut()
                        .expect("owner candidate movement guard"),
                )?
            } else {
                self.can_move_to(node, point)
            };
            if !can_move {
                if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_MOVE_ALWAYS")
                    && crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE").is_some_and(
                        |target| {
                            target == "all"
                                || target.split(',').any(|target| {
                                    target == self.graph.nodes[node.0 as usize].external_id
                                        || target.parse::<i64>().ok()
                                            == Some(self.graph.nodes[node.0 as usize].tala_id)
                                })
                        },
                    )
                {
                    eprintln!("SIZED_MOVE_RUST candidate={},{} rejected", point.x, point.y);
                }
                continue;
            }
            if let Some(movement) = movement.as_mut() {
                movement.move_abs(
                    point,
                    owner_guard
                        .as_deref_mut()
                        .expect("owner trial movement guard"),
                )?;
                if !defer_candidate_publication {
                    movement_adapter::mirror(self.graph, movement.session());
                }
            } else {
                self.graph.move_active_node_abs_with_children(node, point);
            }
            // TALA's route-obstruction scan follows Graph.Containers order.
            // Its alternate-route state is accumulated during that scan, so
            // an x-sorted candidate list changes which alternates are tested.
            let cache =
                (!crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_OBSTRUCTION_CACHE"))
                    .then_some(&obstruction_cache[..]);
            let owner_authoritative = movement.is_some();
            let edge_length = self.candidate_edge_length(
                node,
                cache,
                &mut movement.as_mut().map(|m| m.session()),
                owner_guard.as_deref_mut(),
                owner_authoritative,
                Some(&mut scorer),
                score_context,
                reuse_frame_validation.then_some(&mut frame_validated),
            )?;
            #[cfg(feature = "diagnostic-traces")]
            {
                let scored_position = if let Some(movement) = movement.as_mut() {
                    let session = movement.session();
                    session
                        .owner()
                        .position(session.canonical_node(node).expect("scored owner node"))
                } else {
                    self.graph.position(node)
                };
                crate::internal::pipeline::trace_placement_score_graph(
                    "edge",
                    self.graph,
                    node,
                    scored_position.unwrap_or(point),
                    edge_length,
                    least,
                );
            }
            if trace_scoped_score {
                eprintln!(
                    "SIZED_SCORE_RUST phase=edge node={} point={},{} score={} least={}",
                    self.graph.nodes[node.0 as usize].tala_id, point.x, point.y, edge_length, least,
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_CONTAINER_SCORE")
                && self.graph.nodes[node.0 as usize].tala_id == 3_211_751_022
                && point.x == 594.0
                && (point.y == 297.0 || point.y == 396.0)
            {
                eprintln!(
                    "CONTAINER_SCORE_RUST point={},{} score={} raw_turn={}",
                    point.x,
                    point.y,
                    edge_length,
                    self.graph.raw_turn_cost()
                );
            }
            // TALA avoids the more expensive crossing/symmetry work when even
            // the strongest possible symmetry credit cannot beat the current
            // best score. This check applies regardless of the node's
            // direction; the Go oracle does not gate it on an unset
            // container direction.
            if least.is_finite()
                && sized_precision_compare(edge_length - symmetry_cost, least)
                    == std::cmp::Ordering::Greater
            {
                if trace_move {
                    eprintln!(
                        "SIZED_MOVE_RUST candidate={},{} pruned={} least={least}",
                        point.x,
                        point.y,
                        edge_length - symmetry_cost
                    );
                }
                continue;
            }
            if defer_candidate_publication {
                movement_adapter::mirror(
                    self.graph,
                    movement
                        .as_mut()
                        .expect("owner candidate movement")
                        .session(),
                );
            }
            let column_crossing = self
                .graph
                .column_to_column_crossing_cost_checked(node, false, score_context)
                .map_err(scoring_error)?;
            let symmetry = self
                .graph
                .sized_symmetry_checked(node, true, score_context)
                .map_err(scoring_error)?;
            let mut distance = edge_length + column_crossing;
            // Match Go's ARM64 FMSUBD lowering for `edgeLength -= symmetry * symmetryCost`:
            // one fused multiply-subtract rounding step.
            distance = (-symmetry).mul_add(symmetry_cost, distance);
            for obstacle in &self.obstacles {
                Self::step_owner_work(
                    movement.as_mut().map(|m| m.session()),
                    owner_guard.as_deref_mut(),
                )?;
                if obstacle.origin.x == -CONTAINER_PADDING
                    && obstacle.origin.y == -CONTAINER_PADDING
                {
                    continue;
                }
                let candidate_box = if let Some(movement) = movement.as_mut() {
                    let session = movement.session();
                    let canonical = session
                        .canonical_node(node)
                        .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
                    Rect {
                        origin: session.owner().position(canonical).expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        ),
                        size: session.owner().nodes[canonical.0 as usize].rect.size,
                    }
                } else {
                    Rect {
                        origin: self.graph.position(node).unwrap_or(point),
                        size: self.graph.active_node_size(node),
                    }
                };
                if candidate_box.overlaps(*obstacle) {
                    // Go 1.27 on arm64 contracts `edgeLength += 3 * TurnCost`
                    // into a fused multiply-add. Preserve that single rounding
                    // step for the obstacle penalty.
                    distance = self.graph.turn_cost_value().mul_add(3.0, distance);
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SCORE_COMPONENTS")
                && crate::internal::trace_env_value("TALA_TRACE_SCORE_NODE").is_some_and(|target| {
                    target == "all" || target == self.graph.nodes[node.0 as usize].external_id
                })
            {
                eprintln!(
                    "SCORE_COMPONENTS_RUST node={} point={},{} edge={} edge_bits=0x{:016x} column={} column_bits=0x{:016x} symmetry={} symmetry_bits=0x{:016x} symmetry_cost={} symmetry_cost_bits=0x{:016x} total={} total_bits=0x{:016x}",
                    self.graph.nodes[node.0 as usize].external_id,
                    point.x,
                    point.y,
                    edge_length,
                    edge_length.to_bits(),
                    column_crossing,
                    column_crossing.to_bits(),
                    symmetry,
                    symmetry.to_bits(),
                    symmetry_cost,
                    symmetry_cost.to_bits(),
                    distance,
                    distance.to_bits(),
                );
            }
            crate::internal::pipeline::trace_placement_score_graph(
                "total",
                self.graph,
                node,
                self.graph.position(node).unwrap_or(point),
                distance,
                least,
            );
            if trace_scoped_score {
                eprintln!(
                    "SIZED_SCORE_RUST phase=total node={} point={},{} score={} least={}",
                    self.graph.nodes[node.0 as usize].tala_id, point.x, point.y, distance, least,
                );
            }
            if trace_move {
                let herd = self.graph.herd_penalty(node);
                let common_uncle = self.graph.common_uncle_penalty(node, true);
                let herd_assignment = &self.graph.nodes[node.0 as usize].herd_assignment;
                let common_uncle_siblings =
                    self.graph.common_uncle_siblings.get(&node).map(|siblings| {
                        siblings
                            .iter()
                            .map(|sibling| self.graph.nodes[sibling.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    });
                eprintln!(
                    "SIZED_MOVE_RUST candidate={},{} edge={edge_length} column={column_crossing} base={} herd={herd} herdAssignment={herd_assignment:?} commonUncle={common_uncle} commonUncleSiblings={common_uncle_siblings:?} symmetry={symmetry} symmetryCost={symmetry_cost} score={distance}",
                    point.x,
                    point.y,
                    edge_length - herd - common_uncle,
                );
            }
            match sized_precision_compare(distance, least) {
                std::cmp::Ordering::Less => {
                    least = distance;
                    best = point;
                }
                std::cmp::Ordering::Equal if point == current => {
                    least = distance;
                    best = point;
                }
                _ => {}
            }
        }
        if let Some(context) = scoring_context {
            #[cfg(feature = "diagnostic-traces")]
            let cause = if let Some(movement) = movement.as_mut() {
                context.error_with_owner(movement.session().owner_mut())
            } else {
                context.error()
            };
            #[cfg(not(feature = "diagnostic-traces"))]
            let cause = context.error();
            if let Some(cause) = cause {
                return Err(OptimizationError::Context {
                    location: "EdgeLength".into(),
                    cause,
                });
            }
        }
        if movement.is_some() && least == f64::INFINITY {
            return Err(OptimizationError::Invalid(
                "sizedOptimizer: could not find any placement".into(),
            ));
        }
        if let Some(movement) = movement.as_mut() {
            movement.move_abs(best, owner_guard.expect("owner final movement guard"))?;
            movement_adapter::mirror(self.graph, movement.session());
        } else {
            self.graph.move_active_node_abs_with_children(node, best);
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_POSITIONS")
            && self
                .graph
                .nodes
                .iter()
                .any(|candidate| candidate.tala_id == 19)
            && self
                .graph
                .nodes
                .iter()
                .any(|candidate| candidate.tala_id == 20)
            && self
                .graph
                .nodes
                .iter()
                .any(|candidate| candidate.tala_id == 21)
        {
            eprintln!(
                "SIZED_POSITIONS_RUST moved={} best={},{} {:?}",
                self.graph.nodes[node.0 as usize].tala_id,
                best.x,
                best.y,
                self.graph
                    .nodes
                    .iter()
                    .filter(|candidate| matches!(candidate.tala_id, 19..=21))
                    .map(|candidate| (candidate.tala_id, candidate.position))
                    .collect::<Vec<_>>()
            );
        }
        if trace_move {
            eprintln!(
                "SIZED_MOVE_RUST selected={},{} score={least}",
                best.x, best.y
            );
        }
        if trace_scoped_score {
            eprintln!(
                "SIZED_SCORE_RUST selected node={} point={},{} moved={}",
                self.graph.nodes[node.0 as usize].tala_id,
                best.x,
                best.y,
                best != current,
            );
        }
        let changed = if let Some(movement) = movement.as_mut() {
            let session = movement.session();
            let canonical = session
                .canonical_node(node)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            session.owner().position(canonical) != Some(current)
        } else {
            best != current
        };
        if let Some(movement) = movement {
            movement.commit();
        }
        Ok(changed)
    }

    fn score(
        &self,
        node: NodeId,
        owner_session: &mut Option<&mut OwnerScoreSession>,
    ) -> Result<f64, OptimizationError> {
        let symmetry_cost = self.cell_size * self.graph.nodes[node.0 as usize].edges.len() as f64;
        let edge_length = self.candidate_edge_length(
            node,
            None,
            owner_session,
            None,
            false,
            None,
            super::scoring_context::ScoreContext::none(),
            None,
        )?;
        let crossing = self.graph.column_to_column_crossing_cost(node, false);
        let symmetry = self.graph.sized_symmetry(node, true);
        let mut score = edge_length + crossing;
        score = (-symmetry).mul_add(symmetry_cost, score);
        if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_COMPONENTS")
            && crate::internal::trace_env_value("TALA_TRACE_SWAP_NODE").is_some_and(|target| {
                target == "all"
                    || target.split(',').any(|id| {
                        id.parse::<i64>().ok() == Some(self.graph.nodes[node.0 as usize].tala_id)
                    })
            })
        {
            eprintln!(
                "SWAP_COMPONENTS_RUST node={} pos={:?} edge={} crossing={} symmetry={} symmetryCost={} total={}",
                self.graph.nodes[node.0 as usize].tala_id,
                self.graph.position(node),
                edge_length,
                crossing,
                symmetry,
                symmetry_cost,
                score,
            );
        }
        Ok(score)
    }

    fn overlaps_except(&self, node: NodeId, point: Point, ignored: &[NodeId]) -> bool {
        self.graph.nodes.iter().any(|other| {
            if other.input_id == node || ignored.contains(&other.input_id) {
                return false;
            }
            let Some(other_position) = other.position else {
                return false;
            };
            let node_size = self.graph.nodes[node.0 as usize].rect.size;
            // D2's optimizerDoesOverlap first applies a broad 500px
            // prefilter, then evaluates the directional DeltaTo predicate.
            // The prefilter is part of the observable behavior because it
            // avoids calculating DeltaTo for distant nodes.
            const MAX_SAFE_DELTA: f64 = 500.0;
            if point.x > other_position.x + other.rect.size.width + MAX_SAFE_DELTA
                || point.x + node_size.width + MAX_SAFE_DELTA < other_position.x
                || point.y > other_position.y + other.rect.size.height + MAX_SAFE_DELTA
                || point.y + node_size.height + MAX_SAFE_DELTA < other_position.y
            {
                return false;
            }
            let delta = if self.graph.container_direction_is_unset(node)
                && self.graph.container_direction_is_unset(other.input_id)
            {
                self.graph
                    .spacing_delta_with_loops(node, other.input_id, point)
            } else {
                self.graph.spacing_delta(node, other.input_id, point)
            };
            point.x < other_position.x + other.rect.size.width + delta
                && point.x + node_size.width + delta > other_position.x
                && point.y < other_position.y + other.rect.size.height + delta
                && point.y + node_size.height + delta > other_position.y
        })
    }

    fn best_swap_candidate(
        &mut self,
        node: NodeId,
        mut owner_session: Option<&mut OwnerScoreSession>,
    ) -> Result<Option<NodeId>, OptimizationError> {
        let current_score = self.score(node, &mut owner_session)?;
        // Recovered getBestSwapCandidate shuffles all `g.Nodes` indices, then
        // rejects self, fixed, tree, and non-adjacent candidates in the loop.
        // Prefiltering changes the shared RNG stream even when every rejected
        // candidate would be semantically unusable.
        let mut candidates = self.nodes.clone();
        if crate::internal::trace_env_enabled("TALA_TRACE_SHUFFLE_CALLS") {
            eprintln!(
                "SHUFFLE_CALL_RUST kind=swap node={} len={}",
                self.graph.nodes[node.0 as usize].tala_id,
                candidates.len()
            );
        }
        self.rng.shuffle(&mut candidates);
        if crate::internal::trace_env_enabled("TALA_TRACE_RNG_OPS") && self.graph.nodes.len() == 10
        {
            eprintln!(
                "RNG_OP_RUST kind=swap node={} len={}",
                self.graph.nodes[node.0 as usize].tala_id,
                candidates.len()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_ORDER") {
            eprint!(
                "SWAP_ORDER_RUST node={}",
                self.graph.nodes[node.0 as usize].tala_id
            );
            for candidate in &candidates {
                eprint!(" {}", self.graph.nodes[candidate.0 as usize].tala_id);
            }
            eprintln!();
        }
        let mut best = None;
        let mut best_score = f64::INFINITY;
        let trace_swap_target = crate::internal::trace_env_value("TALA_TRACE_SWAP_TARGET");
        let trace_swap_node = trace_swap_target.as_deref().is_some_and(|target| {
            target == "all"
                || target.parse::<i64>().ok() == Some(self.graph.nodes[node.0 as usize].tala_id)
        });
        for candidate in candidates {
            if candidate == node
                || self.graph.nodes[candidate.0 as usize]
                    .fixed_top_left
                    .is_some()
                || !self.graph.sized_swap_adjacent(node, candidate)
            {
                continue;
            }
            let node_position = self.graph.position(node).unwrap();
            let candidate_position = self.graph.position(candidate).unwrap();
            let trace_swap = trace_swap_node;
            if trace_swap {
                eprintln!(
                    "SWAP_TRIAL_RUST node={} candidate={} node_pos={},{} candidate_pos={},{} before203={:?}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    node_position.x,
                    node_position.y,
                    candidate_position.x,
                    candidate_position.y,
                    self.graph
                        .nodes
                        .iter()
                        .find(|n| n.tala_id == 2_039_569_393)
                        .and_then(|n| n.position),
                );
            }
            let pre_node_overlaps = self.overlaps_except(node, candidate_position, &[candidate]);
            let pre_candidate_overlaps = self.overlaps_except(candidate, node_position, &[node]);
            if trace_swap
                && self.graph.nodes[node.0 as usize].tala_id == 6_334_824_724_549_167_320
                && self.graph.nodes[candidate.0 as usize].tala_id == 2_039_569_393
            {
                eprintln!(
                    "SWAP_PRE_OVERLAP_RUST node={} candidate={} first={} second={} current={},{}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    pre_node_overlaps,
                    pre_candidate_overlaps,
                    node_position.x,
                    node_position.y
                );
            }
            if pre_node_overlaps || pre_candidate_overlaps {
                continue;
            }
            let candidate_score = self.score(candidate, &mut owner_session)?;
            if trace_swap
                && self.graph.nodes[node.0 as usize].tala_id == 6_334_824_724_549_167_320
                && self.graph.nodes[candidate.0 as usize].tala_id == 2_039_569_393
            {
                eprintln!(
                    "SWAP_SCORE_BEGIN_RUST node={} candidate={} current1={} current2={}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    current_score,
                    candidate_score
                );
            }
            self.graph
                .move_active_node_abs_with_children(node, candidate_position);
            self.graph
                .move_active_node_abs_with_children(candidate, node_position);
            if trace_swap && crate::internal::trace_env_enabled("TALA_TRACE_SWAP_MEMBERS") {
                let members = self
                    .graph
                    .transaction_external_aggregate_children
                    .get(&self.graph.nodes[node.0 as usize].tala_id)
                    .map(|children| {
                        children
                            .iter()
                            .map(|child| format!(" {}={:?}", child.tala_id, child.position))
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                eprintln!(
                    "SWAP_MEMBER_POS_RUST node={} candidate={} nodePos={:?} candidatePos={:?} members={}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    self.graph.position(node),
                    self.graph.position(candidate),
                    members
                );
            }
            let node_overlaps = self.overlaps_except(node, candidate_position, &[]);
            let candidate_overlaps = self.overlaps_except(candidate, node_position, &[]);
            if trace_swap
                && self.graph.nodes[node.0 as usize].tala_id == 6_334_824_724_549_167_320
                && self.graph.nodes[candidate.0 as usize].tala_id == 2_039_569_393
            {
                eprintln!(
                    "SWAP_OVERLAP_RUST node={} candidate={} first={} second={}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    node_overlaps,
                    candidate_overlaps
                );
            }
            if node_overlaps || candidate_overlaps {
                self.graph
                    .move_active_node_abs_with_children(node, node_position);
                self.graph
                    .move_active_node_abs_with_children(candidate, candidate_position);
                continue;
            }
            let swapped_node_score = self.score(node, &mut owner_session)?;
            let swapped_candidate_score = self.score(candidate, &mut owner_session)?;
            if trace_swap
                && self.graph.nodes[node.0 as usize].tala_id == 6_334_824_724_549_167_320
                && self.graph.nodes[candidate.0 as usize].tala_id == 2_039_569_393
            {
                eprintln!(
                    "SWAP_SCORE_RUST node={} candidate={} swapped1={} swapped2={}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    swapped_node_score,
                    swapped_candidate_score
                );
            }
            self.graph
                .move_active_node_abs_with_children(node, node_position);
            self.graph
                .move_active_node_abs_with_children(candidate, candidate_position);
            if trace_swap {
                eprintln!(
                    "SWAP_TRIAL_RUST_RESTORED node={} candidate={} after203={:?}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[candidate.0 as usize].tala_id,
                    self.graph
                        .nodes
                        .iter()
                        .find(|n| n.tala_id == 2_039_569_393)
                        .and_then(|n| n.position),
                );
            }
            let combined = swapped_node_score + swapped_candidate_score;
            if swapped_node_score + PRECISION < current_score
                && combined + PRECISION < current_score + candidate_score
                && combined + PRECISION < best_score
            {
                best_score = combined;
                best = Some(candidate);
            }
        }
        if let Some(session) = owner_session {
            session
                .update_current_member_positions(self.graph)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        }
        Ok(best)
    }

    fn hub_spokes(&self, node: NodeId) -> Option<Vec<NodeId>> {
        self.graph.hubs.get(&node)
    }

    fn with_placement_points<'owner, 'context, R>(
        &mut self,
        node: NodeId,
        median: Point,
        minimum: f64,
        minimize: bool,
        owner_session: &mut Option<&mut OwnerScoreSession<'owner>>,
        owner_guard: &mut Option<&mut OptimizationWorkGuard<'context>>,
        evaluate: impl FnOnce(
            &mut Self,
            &mut Vec<Point>,
            &mut Option<&mut OwnerScoreSession<'owner>>,
            &mut Option<&mut OptimizationWorkGuard<'context>>,
        ) -> Result<R, OptimizationError>,
    ) -> Result<R, OptimizationError> {
        let owning = owner_session.is_some();
        let mut points = if let Some(session) = owner_session.as_deref_mut() {
            let canonical = session
                .canonical_node(node)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            self.placement_scratch.generate(
                session.owner_mut(),
                Some(canonical),
                Some(&median),
                minimum,
                minimize,
                self.cell_size,
                self.fixed_origin,
                owner_guard.as_deref_mut().expect("owner generation guard"),
            )?;
            self.placement_scratch
                .points
                .take()
                .expect("successful generated points")
        } else {
            self.placements(node, median, minimum, minimize)
        };
        // The same allocation is shuffled and scored. Return it on every exit,
        // including panic; no point clone conceals partial shuffle/error state.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            evaluate(self, &mut points, owner_session, owner_guard)
        }));
        if owning {
            self.placement_scratch.points = Some(points);
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(super) fn initialize_owner_setup(
        &mut self,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn super::limits::Context,
    ) -> Result<(), OptimizationError> {
        session
            .update_current_member_positions(self.graph)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        let root = self
            .nodes
            .first()
            .copied()
            .map(|node| session.canonical_node(node))
            .transpose()
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?
            .map(|node| super::owner_placement::owner_owning_container(session.owner(), node))
            .transpose()
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?
            .flatten();
        let (slots, present) = session.abductions().map_or_else(
            || (Vec::new(), false),
            |records| {
                (
                    records
                        .records
                        .iter()
                        .copied()
                        .map(Some)
                        .collect::<Vec<_>>(),
                    records.present,
                )
            },
        );
        let header = session
            .owner_mut()
            .allocate_owner_abduction_header(&slots, slots.len(), present)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        let receiver = session.current_view();
        let setup = setup::initialize(
            session.owner_mut(),
            Some(receiver),
            root,
            &header,
            self.obstacles.len(),
            context,
        )?;
        self.cell_size = setup.cell;
        self.fixed_origin = setup.origin;
        Ok(())
    }

    fn owner_closest_distance(
        &mut self,
        node: NodeId,
        median: Point,
        minimize: bool,
        checked: &mut Option<super::optimizer_spatial_index::CheckedOffsets>,
        session: &mut OwnerScoreSession<'_>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<f64, OptimizationError> {
        let canonical = session
            .canonical_node(node)
            .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        let receiver = session.current_view();
        // The search only reads this fresh proposal; nothing else can hold or
        // write it, so the plain point is the same value without an allocation.
        self.spatial_index.closest_distance(
            session.owner_mut(),
            receiver,
            Some(canonical),
            Some(&median),
            self.cell_size,
            self.fixed_origin,
            minimize,
            checked,
            guard,
        )
    }

    fn owner_node_eligible(
        node: NodeId,
        session: &mut OwnerScoreSession<'_>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<bool, OptimizationError> {
        let canonical = session
            .canonical_node(node)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let receiver = session.current_view();
        super::owner_near_eligibility::can_optimize_owner_node_guarded(
            session.owner_mut(),
            Some(canonical),
            Some(receiver),
            guard,
        )
        .map_err(OptimizationError::from)
    }

    fn owner_neighbor_median(
        &mut self,
        node: NodeId,
        temperature: f64,
        session: &mut OwnerScoreSession<'_>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Point, OptimizationError> {
        session
            .update_current_member_positions(self.graph)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        self.owner_neighbor_median_after_sync(node, temperature, session, guard)
    }

    fn owner_neighbor_median_after_sync(
        &mut self,
        node: NodeId,
        _temperature: f64,
        session: &mut OwnerScoreSession<'_>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Point, OptimizationError> {
        let canonical = session
            .canonical_node(node)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        // optimizerMedianToNeighbors consumes the current scope's
        // actual records for connected nodes too. Retain record
        // identities; do not revive projected endpoint snapshots.
        let (slots, present) = session.abductions().map_or_else(
            || (Vec::new(), false),
            |records| {
                (
                    records
                        .records
                        .iter()
                        .copied()
                        .map(Some)
                        .collect::<Vec<_>>(),
                    records.present,
                )
            },
        );
        let abductions = session
            .owner_mut()
            .allocate_owner_abduction_header(&slots, slots.len(), present)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let materialized_incidents = session
            .owner_mut()
            .materialize_owner_node_incident_header(canonical)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let adjacents = super::owner_adjacents::optimizer_owner_adjacents(
            session.owner_mut(),
            Some(canonical),
            &abductions,
            guard,
        );
        if materialized_incidents {
            session
                .owner_mut()
                .release_materialized_owner_node_incident_header(canonical);
        }
        let adjacents = adjacents.map_err(OptimizationError::from)?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_SIZED_OWNER_MEDIAN") {
            let owner = session.owner();
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "sized-owner-median-inputs",
                    "node": self.graph.nodes[node.0 as usize].tala_id,
                    "temperature_bits": format!("{:016x}", _temperature.to_bits()),
                    "all_nodes": owner.nodes.iter().enumerate().map(|(index, member)| serde_json::json!({
                        "id": member.tala_id,
                        "position": owner.position(NodeId(index as u32)),
                        "width": member.rect.size.width,
                        "height": member.rect.size.height,
                    })).collect::<Vec<_>>(),
                    "neighbors": adjacents.iter_slots().map(|id| id.map(|id| {
                        let member = &owner.nodes[id.0 as usize];
                        serde_json::json!({
                            "id": member.tala_id,
                            "position": owner.position(id),
                            "width": member.rect.size.width,
                            "height": member.rect.size.height,
                            "graph": owner.topology_node_owner(id).map(|graph| graph.0),
                        })
                    })).collect::<Vec<_>>(),
                })
            );
        }
        super::owner_median::optimizer_owner_median(session.owner_mut(), &adjacents, true, guard)
            .map_err(OptimizationError::from)
    }

    fn retry_without_hub_spokes(
        &mut self,
        node: NodeId,
        spokes: &[NodeId],
        temperature: f64,
        checked: &mut Option<super::optimizer_spatial_index::CheckedOffsets>,
        mut owner_session: Option<&mut OwnerScoreSession>,
        mut owner_guard: Option<&mut OptimizationWorkGuard<'_>>,
    ) -> Result<(), OptimizationError> {
        let old_edges = crate::internal::IncidentEdges::from_header(
            self.graph.nodes[node.0 as usize].edges.header().clone(),
        );
        let protruding_children = self.graph.sized_protruding_children(node);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(session) = owner_session.as_deref_mut() {
                let canonical = session
                    .canonical_node(node)
                    .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
                let owner = session.owner();
                let hubs = owner
                    .owner_graph_view(session.current_view())
                    .expect("owner hub receiver")
                    .hubs
                    .header();
                // AddHubs runs on the numerical graph, where an aggregate
                // is represented by its first member. The scoring session
                // uses the allocated vessel pointer. Translate both the hub
                // key and its spokes before sized_optimizer.go's retry;
                // otherwise the vessel lookup silently skips that whole call
                // (including its draws from the shared placement RNG).
                let source_spokes = hubs.get(&Some(canonical)).or_else(|| {
                    hubs.map_entries(|key, spokes| {
                        key.filter(|&key| owner.original_owner_node(key) == canonical)
                            .map(|_| spokes.clone())
                    })
                    .into_iter()
                    .flatten()
                    .next()
                });
                let Some(spokes) = source_spokes else {
                    return Ok(());
                };
                let spokes = if spokes
                    .iter_slots()
                    .flatten()
                    .any(|spoke| owner.original_owner_node(spoke) != spoke)
                {
                    let translated = spokes
                        .iter_slots()
                        .map(|spoke| spoke.map(|spoke| owner.original_owner_node(spoke)))
                        .collect::<Vec<_>>();
                    crate::internal::container_slices::SharedNodeSlice::from_slots(&translated)
                        .map_err(|error| OptimizationError::Invalid(error.to_string()))?
                } else {
                    spokes
                };
                super::owner_hub_spokes::with_hub_spokes_suppressed(
                    session,
                    Some(canonical),
                    &spokes,
                    owner_guard
                        .as_deref_mut()
                        .expect("owner hub retry retains its work guard"),
                    |session, guard| {
                        let remaining = session
                            .owner()
                            .owner_node_incident_slots(canonical)
                            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
                        let mut local_edges = BTreeMap::new();
                        for edge in old_edges.read().iter().copied() {
                            let canonical_edge = session
                                .canonical_edge(edge)
                                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
                            local_edges.entry(canonical_edge).or_insert(edge);
                        }
                        // The canonical working header supplies occurrence order,
                        // including repeated slots containing the same edge pointer.
                        let working = remaining
                            .into_iter()
                            .filter_map(|edge| {
                                edge.and_then(|edge| local_edges.get(&edge).copied())
                            })
                            .collect::<Vec<_>>();
                        self.graph.nodes[node.0 as usize].edges.replace_header(
                            crate::internal::IncidentEdges::literal(&working).header(),
                        );
                        self.retry_hub_with_current_incidents(
                            node,
                            temperature,
                            checked,
                            &protruding_children,
                            old_edges.len(),
                            Some(session),
                            Some(guard),
                        )
                    },
                )
            } else {
                self.graph.nodes[node.0 as usize].edges.replace_header(
                    crate::internal::IncidentEdges::literal(&edges_without_hub_spokes(
                        self.graph,
                        node,
                        spokes,
                        &old_edges.read(),
                    ))
                    .header(),
                );
                self.retry_hub_with_current_incidents(
                    node,
                    temperature,
                    checked,
                    &protruding_children,
                    old_edges.len(),
                    None,
                    None,
                )
            }
        }));
        self.graph.nodes[node.0 as usize]
            .edges
            .replace_header(old_edges.header());
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn retry_hub_with_current_incidents(
        &mut self,
        node: NodeId,
        temperature: f64,
        checked: &mut Option<super::optimizer_spatial_index::CheckedOffsets>,
        protruding_children: &[super::ProjectedAdjacent],
        _original_edge_count: usize,
        mut owner_session: Option<&mut OwnerScoreSession>,
        mut owner_guard: Option<&mut OptimizationWorkGuard<'_>>,
    ) -> Result<(), OptimizationError> {
        let (median, minimizing_self) = if let Some(session) = owner_session.as_deref_mut() {
            let (point, child_count) = self.owner_candidate_median(
                node,
                temperature,
                session,
                owner_guard
                    .as_deref_mut()
                    .expect("owner hub retry retains its work guard"),
            )?;
            (point, child_count == 0)
        } else {
            (
                self.median_point_from_base(node, temperature, protruding_children, None),
                protruding_children.is_empty(),
            )
        };
        let minimum = if let Some(session) = owner_session.as_deref_mut() {
            self.owner_closest_distance(
                node,
                median,
                minimizing_self,
                checked,
                session,
                owner_guard.as_deref_mut().expect("owner hub search guard"),
            )?
        } else {
            self.closest_unoccupied_distance(node, median, minimizing_self, checked)
        };
        self.with_placement_points(node,median,minimum,minimizing_self,&mut owner_session,&mut owner_guard,|this,points,owner_session,owner_guard|{
        if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_RAW")
            && crate::internal::trace_env_value("TALA_TRACE_SIZED_NODE")
                .as_deref()
                .is_some_and(|target| {
                    target == "all"
                        || target.parse::<i64>().ok()
                            == Some(this.graph.nodes[node.0 as usize].tala_id)
                })
        {
            eprint!(
                "SIZED_RAW_POINTS_RUST node={} median={},{} distance={}",
                this.graph.nodes[node.0 as usize].tala_id, median.x, median.y, minimum
            );
            for point in points.iter() {
                eprint!(" {},{}", point.x, point.y);
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SHUFFLE_CALLS") {
            eprintln!(
                "SHUFFLE_CALL_RUST kind=hub-points node={} len={}",
                this.graph.nodes[node.0 as usize].tala_id,
                points.len()
            );
        }
        point_shuffle::shuffle(&mut this.rng, points, owner_session.as_deref_mut(), owner_guard.as_deref_mut())?;
        if crate::internal::trace_env_enabled("TALA_TRACE_RNG_OPS")
            && this.graph.nodes.len() == 10
        {
            eprintln!(
                "RNG_OP_RUST kind=hub-points node={} len={}",
                this.graph.nodes[node.0 as usize].tala_id,
                points.len()
            );
        }
        if crate::internal::trace_env_value("TALA_TRACE_HUB_RETRY_TARGET").is_some_and(|target| {
            target == "all"
                || target.parse::<i64>().ok() == Some(this.graph.nodes[node.0 as usize].tala_id)
        }) {
            eprint!(
                "HUB_RETRY_POINTS_RUST node={}",
                this.graph.nodes[node.0 as usize].tala_id
            );
            for point in points.iter() {
                eprint!(" {:.0},{:.0}", point.x, point.y);
            }
            eprintln!();
        }
        #[cfg(feature = "diagnostic-traces")]
        let suppressed_edges = _original_edge_count - this.graph.nodes[node.0 as usize].edges.len();
        let retry_result = this.move_to_best_with_owner_nears(
            node, points, temperature == 0.0, owner_session.as_deref_mut(), owner_guard.as_deref_mut(),
        );
        #[cfg(feature = "diagnostic-traces")]
        let retry_moved = retry_result?;
        #[cfg(not(feature = "diagnostic-traces"))]
        retry_result?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("TALA_TRACE_HUB_RETRY_TARGET").is_some_and(|target| {
            target == "all"
                || target.parse::<i64>().ok() == Some(this.graph.nodes[node.0 as usize].tala_id)
        }) {
            eprintln!(
                "HUB_RETRY_RUST node={} spokes_suppressed={} median={:?} distance={} points={} moved={} end={:?}",
                this.graph.nodes[node.0 as usize].tala_id,
                suppressed_edges,
                median,
                minimum,
                points.len(),
                retry_moved,
                this.graph.position(node),
            );
        }
        Ok(())
        })
    }

    pub(super) fn optimize_one(&mut self, temperature: f64) -> bool {
        self.optimize_limit(temperature, 1, None, None)
            .expect("unbound sized optimization has no owner input errors")
    }

    pub(super) fn optimize(&mut self, temperature: f64) -> bool {
        self.optimize_limit(temperature, usize::MAX, None, None)
            .expect("unbound sized optimization has no owner input errors")
    }

    pub(super) fn optimize_with_owner_nears(
        &mut self,
        temperature: f64,
        first_node_only: bool,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn super::limits::Context,
    ) -> Result<bool, OptimizationError> {
        self.optimize_with_owner_limit(
            temperature,
            first_node_only,
            session,
            context,
            super::limits::MAX_OPTIMIZATION_WORK_UNITS,
        )
    }

    fn optimize_with_owner_limit(
        &mut self,
        temperature: f64,
        first_node_only: bool,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn super::limits::Context,
        work_limit: u64,
    ) -> Result<bool, OptimizationError> {
        let transaction_error = |error| match error {
            super::limits::Error::Context { location, cause } => {
                OptimizationError::Context { location, cause }
            }
            error => OptimizationError::Invalid(error.to_string()),
        };
        // EnsureTransactionWorkGuard first checks a retained request guard's
        // original context, or constructs the new aggregate guard.
        let _transaction_guard = match context.retained_transaction_guard_check() {
            Some(result) => {
                result.map_err(transaction_error)?;
                None
            }
            None => Some(
                super::limits::WorkGuard::new(
                    Some(context),
                    "LocalOptimizeTransactions",
                    super::limits::MAX_TRANSACTION_WORK_UNITS,
                )
                .map_err(transaction_error)?,
            ),
        };
        #[cfg(feature = "diagnostic-traces")]
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(context),
            "LocalOptimize",
            work_limit,
            session.owner_mut(),
        )?;
        #[cfg(not(feature = "diagnostic-traces"))]
        let mut guard = OptimizationWorkGuard::new(Some(context), "LocalOptimize", work_limit)?;
        // The owner and dense adapter represent one source graph. Publish the
        // preceding pass before saving its canonical mutation state.
        session
            .update_current_member_positions(self.graph)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let mut scratch = std::mem::take(&mut self.mutation_scratch);
        let operation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.capture_optimizer_mutation_state_into(&mut guard, &mut scratch)?;
            let workspace = rollback::WorkspaceSnapshot::capture(self.graph, session, &scratch)?;
            // sized_optimizer.go255–278 arms only after capture succeeds.
            let trial = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let changed = self.optimize_limit(
                    temperature,
                    if first_node_only { 1 } else { usize::MAX },
                    Some(session),
                    Some(&mut guard),
                )?;
                #[cfg(feature = "diagnostic-traces")]
                guard.finish_with_owner(session.owner_mut())?;
                #[cfg(not(feature = "diagnostic-traces"))]
                guard.finish()?;
                Ok(changed)
            }));
            if !matches!(&trial, Ok(Ok(_))) {
                scratch
                    .restore(session.owner_mut())
                    .expect("captured sized optimizer owner binding");
                workspace.restore(self.graph);
            }
            match trial {
                Ok(result) => result,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }));
        scratch.release();
        self.mutation_scratch = scratch;
        match operation {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(super) fn compact(
        &mut self,
        horizontal: bool,
        factor: f64,
    ) -> Result<(), OptimizationError> {
        // Recovered Graph.compaction always sees the complete materialized
        // subgraph. Fixed nodes are retained as anchors and skipped only by
        // the move loop inside compact_sized_axis.
        self.graph.compact_sized_axis(horizontal, factor)
    }

    pub(super) fn compact_with_owner_nears(
        &mut self,
        horizontal: bool,
        factor: f64,
        session: &mut OwnerScoreSession,
    ) -> Result<(), OptimizationError> {
        self.graph
            .compact_sized_axis_with_owner_nears(horizontal, factor, Some(session))
    }

    pub(super) fn join_distanced_clusters(&mut self) {
        self.graph.join_distanced_clusters();
    }

    pub(super) fn join_distanced_clusters_with_owner(
        &mut self,
        session: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<(), OptimizationError> {
        self.join_distanced_clusters();
        // Source JoinDistancedClusters moves the shared Node.TopLeft points
        // through MoveWithChildren. Publish those movements before the next
        // caller reads actual graph bounds or herd fences.
        if let Some(session) = session {
            session
                .update_current_member_positions(self.graph)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) fn sync_herd_fences(&mut self) {
        self.graph.sync_herd_fences();
    }

    pub(super) fn sync_herd_fences_with_owner(
        &mut self,
        session: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<(), OptimizationError> {
        let Some(session) = session else {
            self.sync_herd_fences();
            return Ok(());
        };
        // Go reads this receiver's current member headers and live entity
        // fields. Dense correspondence and workspace geometry are separate
        // retained projections; neither defines Graph.Nodes or Graph.Edges.
        let (members, low, high) = {
            let owner = session.owner();
            let view = owner
                .owner_graph_view(session.current_view())
                .ok_or_else(|| {
                    OptimizationError::Invalid(
                        OwnerTopologyError::UnknownGraph(session.current_view()).to_string(),
                    )
                })?;
            let members = view.nodes.require_nodes().collect::<Vec<_>>();
            let Some((low, high)) = owner.fixed_node_bounds(&members) else {
                return Ok(());
            };
            let (low, high) =
                owner.include_selected_graph_edge_bounds(low, high, &view.edges.read());
            (members, low, high)
        };
        let low = Point {
            x: low.x.round(),
            y: low.y.round(),
        };
        let high = Point {
            x: high.x.round(),
            y: high.y.round(),
        };
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_HERD_OWNER_BOUNDS") {
            let bits = |v: f64| format!("{:016x}", v.to_bits());
            let node_row = |graph: &ArenaGraph, id: NodeId| {
                let node = &graph.nodes[id.0 as usize];
                serde_json::json!({"id":node.tala_id,"point":graph.position(id).map(|p|[bits(p.x),bits(p.y)]),"size":[bits(node.rect.size.width),bits(node.rect.size.height)],"fixed":node.fixed_top_left.map(|p|[bits(p.x),bits(p.y)])})
            };
            let workspace_members = self.graph.graph_node_order();
            let workspace_bounds = self
                .graph
                .fixed_node_bounds(&workspace_members)
                .map(|(a, b)| self.graph.include_graph_edge_bounds(a, b));
            eprintln!(
                "HERD_BOUNDS850:{}",
                serde_json::json!({"members":members.iter().map(|&id|node_row(session.owner(),id)).collect::<Vec<_>>(),"bounds":[bits(low.x),bits(low.y),bits(high.x),bits(high.y)],"workspace":workspace_members.iter().map(|&id|node_row(self.graph,id)).collect::<Vec<_>>(),"workspace_bounds":workspace_bounds.map(|(a,b)|[bits(a.x.round()),bits(a.y.round()),bits(b.x.round()),bits(b.y.round())])})
            );
        }
        // Source SyncHerdFences reads the current assignment and fixed flag,
        // including replacement records. Only cardinal orientations write Val.
        for canonical in members {
            let owner = session.owner_mut();
            let node = owner.topology_node(canonical).ok_or_else(|| {
                OptimizationError::Invalid(OwnerTopologyError::UnknownNode(canonical).to_string())
            })?;
            if node.fixed_top_left.is_some() {
                continue;
            }
            let Some(herd) = owner
                .owner_node_herd(canonical)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?
            else {
                continue;
            };
            let mut fields = owner
                .owner_herd_fields(&herd)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            fields.value = match fields.orientation {
                4 => low.y,
                5 => high.x,
                6 => high.y,
                7 => low.x,
                _ => continue,
            };
            owner
                .set_owner_herd_fields(&herd, fields)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
        }
        // Refresh only detached scalar caches. Calling the numerical sync
        // here could mutate aliased owner records for removed/fixed members
        // using stale workspace eligibility or orientation.
        for (local, &canonical) in session.current_mapping().nodes().iter().enumerate() {
            let Some(cache) = self.graph.nodes.get_mut(local) else {
                continue;
            };
            let herd = session
                .owner()
                .owner_node_herd(canonical)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
            if let Some(herd) = herd {
                let fields = session
                    .owner()
                    .owner_herd_fields(&herd)
                    .map_err(|e| OptimizationError::Invalid(e.to_string()))?;
                if let Some(assignment) = cache.herd_assignment.as_mut() {
                    assignment.value = fields.value;
                }
            } else {
                cache.herd_assignment = None;
            }
        }
        Ok(())
    }

    fn step_owner_work(
        session: Option<&mut OwnerScoreSession<'_>>,
        guard: Option<&mut OptimizationWorkGuard<'_>>,
    ) -> Result<(), OptimizationError> {
        let Some(guard) = guard else {
            return Ok(());
        };
        #[cfg(feature = "diagnostic-traces")]
        if let Some(session) = session {
            return guard.step_with_owner(session.owner_mut());
        }
        #[cfg(not(feature = "diagnostic-traces"))]
        let _ = session;
        guard.step()
    }

    fn optimize_limit(
        &mut self,
        temperature: f64,
        limit: usize,
        mut owner_session: Option<&mut OwnerScoreSession<'_>>,
        mut owner_guard: Option<&mut OptimizationWorkGuard<'_>>,
    ) -> Result<bool, OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(SizedOptimize);
        let _transpose_buffers = super::optimization::TransposeRollbackBuffers;
        let trace_sized_graph = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_ALL")
            || self.graph.nodes.len() >= 17
            || self.graph.nodes.len() == 4
            || (crate::internal::trace_env_enabled("TALA_TRACE_SIZED_ROOT")
                && self.graph.nodes.len() == 7);
        let trace_call = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL")
            && trace_sized_graph
            && (!crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL_ROOT_ONLY")
                || self.graph.nodes.len() == 7);
        // A node count is not a scope identity: nested placement graphs can
        // have the same cardinality as the root graph.  For focused parity
        // captures, require the caller to provide the complete stable ID set
        // of the graph being inspected.  The probe remains diagnostic-only.
        let trace_root_state = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_STATE")
            && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
            && trace_sized_optimizer_scope(&self.nodes, self.graph);
        if trace_root_state {
            eprint!(
                "SIZED_STATE_RUST begin temp={} limit={} ",
                temperature, limit
            );
            for node in &self.nodes {
                eprint!(
                    "{}@{:?} ",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.position(*node)
                );
            }
            eprintln!();
        }
        if owner_guard.is_some() && self.nodes.len() > 10_000 {
            return Err(OptimizationError::Invalid(
                "TALA LocalOptimize node count exceeds limit 10000".into(),
            ));
        }
        for _ in 0..self.nodes.len() {
            Self::step_owner_work(owner_session.as_deref_mut(), owner_guard.as_deref_mut())?;
        }
        let mut nodes = self.nodes.clone();
        if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_ORDER") {
            eprint!("SIZED_BASE_RUST");
            for node in &nodes {
                eprint!(" {}", self.graph.nodes[node.0 as usize].tala_id);
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SHUFFLE_CALLS") {
            eprintln!("SHUFFLE_CALL_RUST kind=node-order len={}", nodes.len());
        }
        trace_sized_optimizer_order("before-shuffle", temperature, &nodes, self.graph);
        if let Some(guard) = owner_guard.as_deref_mut() {
            if let Some(session) = owner_session.as_deref_mut() {
                self.rng
                    .shuffle_with_owner(&mut nodes, session.owner_mut(), guard)?;
            } else {
                self.rng.shuffle_guarded(&mut nodes, guard)?;
            }
        } else {
            self.rng.shuffle(&mut nodes);
        }
        trace_sized_optimizer_order("after-shuffle", temperature, &nodes, self.graph);
        if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_ORDER_ANY") {
            eprint!("SIZED_ORDER_ANY_RUST len={}", nodes.len());
            for node in &nodes {
                eprint!(" {}", self.graph.nodes[node.0 as usize].tala_id);
            }
            eprintln!();
        }
        if trace_root_state {
            eprint!("SIZED_ORDER_RUST");
            for node in &nodes {
                eprint!(" {}", self.graph.nodes[node.0 as usize].tala_id);
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_RNG_OPS") && self.graph.nodes.len() == 10
        {
            eprintln!("RNG_OP_RUST kind=nodes len={}", nodes.len());
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_ORDER")
            && trace_sized_graph
            && (!crate::internal::trace_env_enabled("TALA_TRACE_ROOT_ONLY")
                || self.graph.nodes.len() == 15)
        {
            eprint!("SIZED_ORDER_RUST");
            for node in &nodes {
                eprint!(" {}", self.graph.nodes[node.0 as usize].tala_id);
            }
            eprintln!();
        }
        let mut changed = false;
        let mut processed = 0;
        for (pass, node) in nodes.into_iter().enumerate() {
            Self::step_owner_work(owner_session.as_deref_mut(), owner_guard.as_deref_mut())?;
            if owner_guard.is_some() && self.graph.position(node).is_none() {
                return Err(OptimizationError::Invalid(
                    "TALA LocalOptimize found an unpositioned graph node".into(),
                ));
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_RANKING_MODEL")
                && self.graph.nodes.len() == 15
            {
                if let Some(candidate) = self
                    .graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == 363508928)
                    && let Some(position) = candidate.position
                {
                    eprintln!(
                        "RANK_BEFORE_RUST node={} pos={},{}",
                        self.graph.nodes[node.0 as usize].tala_id, position.x, position.y
                    );
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_MEMBER_STATE") {
                    eprint!(
                        "MEMBER_STATE_RUST after={} ",
                        self.graph.nodes[node.0 as usize].tala_id
                    );
                    for (vessel, members) in &self.graph.transaction_external_aggregate_children {
                        eprint!(" cluster={} ", vessel);
                        for member in members {
                            eprint!("{}={:?} ", member.tala_id, member.position);
                        }
                    }
                    eprintln!();
                }
            }
            let arena_node = &self.graph.nodes[node.0 as usize];
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PRE")
                && self.graph.nodes.len() == 10
            {
                eprint!(
                    "SIZED_PRE_RUST pass={} node={}",
                    pass, self.graph.nodes[node.0 as usize].tala_id
                );
                for state_node in &self.nodes {
                    let state = self.graph.position(*state_node).unwrap_or_default();
                    eprint!(
                        " {}@{:.0},{:.0}",
                        self.graph.nodes[state_node.0 as usize].tala_id, state.x, state.y
                    );
                }
                eprintln!();
            }
            if arena_node.fixed_top_left.is_some()
                || arena_node.rect.size.width > self.cell_size * 100.0
                || arena_node.rect.size.height > self.cell_size * 100.0
            {
                continue;
            }
            // sized_optimizer.go310–338 checks Near eligibility when needed,
            // then NodeToTree membership for every node, including connected
            // nodes. A present nil value still marks a tree member.
            if let Some(session) = owner_session.as_deref_mut() {
                if !Self::owner_node_eligible(
                    node,
                    session,
                    owner_guard
                        .as_deref_mut()
                        .expect("owner inputs retain a work guard"),
                )? {
                    continue;
                }
            } else if arena_node.edges.is_empty()
                && !arena_node.nears.keys().into_iter().any(|near| {
                    arena_node
                        .container
                        .is_none_or(|container| self.graph.is_descendant_of(near, container))
                })
            {
                continue;
            }
            // TALA leaves synthetic aggregate vessels out of this optimizer.
            // They remain active graph nodes for later publication, but
            // letting them consume the shared RNG changes the candidate order
            // for subsequent ordinary nodes.
            if self.graph.active_node_is_aggregate(node) {
                continue;
            }
            if processed == limit {
                break;
            }
            processed += 1;
            self.graph.reconcile_projected_offsets_after_rollback();
            let trace_start = self.graph.position(node);
            let (median, minimizing_self) = if let Some(session) = owner_session.as_deref_mut() {
                let (point, child_count) = self.owner_candidate_median(
                    node,
                    temperature,
                    session,
                    owner_guard
                        .as_deref_mut()
                        .expect("owner inputs retain a work guard"),
                )?;
                (point, child_count == 0)
            } else {
                let children = self.graph.sized_protruding_children(node);
                (
                    self.median_point_from_base(node, temperature, &children, None),
                    children.is_empty(),
                )
            };
            if let Some(session) = owner_session.as_deref_mut() {
                let receiver = session.current_view();
                self.spatial_index.rebuild(
                    session.owner_mut(),
                    receiver,
                    owner_guard
                        .as_deref_mut()
                        .expect("owner spatial index retains its work guard"),
                )?;
            }
            // Recovered optimize shares this offset cache with the optional
            // hub-spoke retry. It is allocated only for larger optimizer
            // graphs, matching the release's `len(nodeIndices) > 10` guard.
            let mut checked_positions = (self.nodes.len() > 10)
                .then(super::optimizer_spatial_index::CheckedOffsets::default);
            let minimum = if let Some(session) = owner_session.as_deref_mut() {
                self.owner_closest_distance(
                    node,
                    median,
                    minimizing_self,
                    &mut checked_positions,
                    session,
                    owner_guard.as_deref_mut().expect("owner search guard"),
                )?
            } else {
                self.closest_unoccupied_distance(
                    node,
                    median,
                    minimizing_self,
                    &mut checked_positions,
                )
            };
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DISTANCE")
                && self.graph.nodes.len() == 10
                && (self.graph.nodes[node.0 as usize].tala_id == 3809224601
                    || self.graph.nodes[node.0 as usize].tala_id == 3977000791)
            {
                eprintln!(
                    "SIZED_DISTANCE_RUST node={} size={:.17},{:.17} median={:.17},{:.17} distance={:.17}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.nodes[node.0 as usize].rect.size.width,
                    self.graph.nodes[node.0 as usize].rect.size.height,
                    median.x,
                    median.y,
                    minimum
                );
            }
            let trace_search_target =
                crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE");
            let trace_search = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE")
                && trace_search_target.as_deref() == Some("all")
                || trace_search_target.as_deref().is_some_and(|targets| {
                    (crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE")
                        || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_SEARCH_ALWAYS"))
                        && targets.split(',').any(|target| {
                            target.parse::<i64>().ok()
                                == Some(self.graph.nodes[node.0 as usize].tala_id)
                        })
                });
            if trace_search {
                eprint!(
                    "SIZED_SEARCH_RUST node={} median={},{} distance={} minimizingSelf={} protruding=",
                    self.graph.nodes[node.0 as usize].tala_id,
                    median.x,
                    median.y,
                    minimum,
                    minimizing_self
                );
                #[cfg(feature = "diagnostic-traces")]
                for child in self.graph.sized_protruding_children(node) {
                    eprint!("{},", child.tala_id);
                }
                eprint!(" adjacents=");
                for edge in self.graph.nodes[node.0 as usize]
                    .edges
                    .read()
                    .iter()
                    .copied()
                {
                    if let Some(projected) = self.graph.sized_adjacent_overrides.get(&(node, edge))
                    {
                        let owner = self.graph.position(projected.owner).unwrap();
                        eprint!(
                            "{}:{},{}:{},{}:owner={}:offset={},{}:",
                            projected.tala_id,
                            owner.x + projected.offset.x,
                            owner.y + projected.offset.y,
                            projected.size.width,
                            projected.size.height,
                            self.graph.nodes[projected.owner.0 as usize].tala_id,
                            projected.offset.x,
                            projected.offset.y
                        );
                    } else {
                        let adjacent = self.graph.adjacent(node, edge);
                        let position = self.graph.position(adjacent).unwrap();
                        let size = self.graph.nodes[adjacent.0 as usize].rect.size;
                        eprint!(
                            "{}:{},{}:{},{},",
                            self.graph.nodes[adjacent.0 as usize].tala_id,
                            position.x,
                            position.y,
                            size.width,
                            size.height
                        );
                    }
                }
                eprint!(" abductions=");
                for abduction in &self.graph.sized_edge_abductions {
                    if abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        != Some(node)
                        && abduction
                            .current_to
                            .and_then(|endpoint| endpoint.local_node())
                            != Some(node)
                    {
                        continue;
                    }
                    eprint!(
                        "{:?}>{:?}:{}>{},",
                        abduction
                            .current_from
                            .and_then(|endpoint| self.graph.abduction_current_tala_id(endpoint)),
                        abduction
                            .current_to
                            .and_then(|endpoint| self.graph.abduction_current_tala_id(endpoint)),
                        abduction
                            .originally_from
                            .map(|projected| projected.tala_id)
                            .unwrap_or(0),
                        abduction
                            .originally_to
                            .map(|projected| projected.tala_id)
                            .unwrap_or(0),
                    );
                }
                eprintln!();
            }
            let (start_position, moved, point_count) = self.with_placement_points(
                node,
                median,
                minimum,
                minimizing_self,
                &mut owner_session,
                &mut owner_guard,
                |this, points, owner_session, owner_guard| {
                    if crate::internal::trace_env_enabled("TALA_TRACE_POINT_COUNTS")
                        && this.graph.nodes.len() == 4
                    {
                        eprintln!(
                            "POINT_COUNT_RUST pass={} node={} count={}",
                            pass,
                            this.graph.nodes[node.0 as usize].tala_id,
                            points.len()
                        );
                    }
                    crate::internal::pipeline::trace_placement_candidates_graph(
                        "raw",
                        this.graph,
                        node,
                        median,
                        minimum,
                        minimizing_self,
                        points,
                    );
                    if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_RAW")
                        && crate::internal::trace_env_value("TALA_TRACE_SIZED_NODE")
                            .as_deref()
                            .is_some_and(|target| {
                                target == "all"
                                    || target.parse::<i64>().ok()
                                        == Some(this.graph.nodes[node.0 as usize].tala_id)
                            })
                    {
                        eprint!(
                            "SIZED_RAW_POINTS_RUST node={} median={},{} distance={}",
                            this.graph.nodes[node.0 as usize].tala_id, median.x, median.y, minimum
                        );
                        for point in points.iter() {
                            eprint!(" {},{}", point.x, point.y);
                        }
                        eprintln!();
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_SHUFFLE_CALLS") {
                        eprintln!(
                            "SHUFFLE_CALL_RUST kind=points node={} len={}",
                            this.graph.nodes[node.0 as usize].tala_id,
                            points.len()
                        );
                    }
                    point_shuffle::shuffle(
                        &mut this.rng,
                        points,
                        owner_session.as_deref_mut(),
                        owner_guard.as_deref_mut(),
                    )?;
                    crate::internal::pipeline::trace_placement_candidates_graph(
                        "shuffled",
                        this.graph,
                        node,
                        median,
                        minimum,
                        minimizing_self,
                        points,
                    );
                    if crate::internal::trace_env_enabled("TALA_TRACE_RNG_OPS")
                        && this.graph.nodes.len() == 10
                    {
                        eprintln!(
                            "RNG_OP_RUST kind=points node={} len={} pos={:?}",
                            this.graph.nodes[node.0 as usize].tala_id,
                            points.len(),
                            this.graph.position(node)
                        );
                        if crate::internal::trace_env_enabled("TALA_TRACE_RNG_STATE")
                            && (this.graph.nodes[node.0 as usize].tala_id == 3809224601
                                || this.graph.nodes[node.0 as usize].tala_id == 3977000791)
                        {
                            eprint!("RNG_STATE_RUST");
                            for state_node in &this.nodes {
                                let state = this.graph.position(*state_node).unwrap_or_default();
                                eprint!(
                                    " {}@{:.0},{:.0}",
                                    this.graph.nodes[state_node.0 as usize].tala_id,
                                    state.x,
                                    state.y
                                );
                            }
                            eprintln!();
                        }
                    }
                    if trace_call && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_POINTS") {
                        eprint!(
                            "SIZED_POINTS_RUST node={}",
                            this.graph.nodes[node.0 as usize].tala_id
                        );
                        for point in points.iter() {
                            eprint!(" {:.0},{:.0}", point.x, point.y);
                        }
                        eprintln!();
                    }
                    if crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE").is_some_and(
                        |target| {
                            target == "all"
                                || target.split(',').any(|target| {
                                    target.parse::<i64>().ok()
                                        == Some(this.graph.nodes[node.0 as usize].tala_id)
                                })
                        },
                    ) && (!crate::internal::trace_env_enabled("TALA_TRACE_ROOT_ONLY")
                        || this.graph.nodes.len() == 15)
                    {
                        eprintln!(
                            "SIZED_PASS_RUST pass={} node={} arena_index={} matches={:?}",
                            pass,
                            this.graph.nodes[node.0 as usize].tala_id,
                            node.0,
                            this.graph
                                .nodes
                                .iter()
                                .enumerate()
                                .filter(|(_, candidate)| candidate.tala_id
                                    == this.graph.nodes[node.0 as usize].tala_id)
                                .map(|(index, candidate)| (index, candidate.position))
                                .collect::<Vec<_>>()
                        );
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_HUB_RETRY") {
                        eprint!(
                            "HUB_RETRY_RUST node={} len={}",
                            this.graph.nodes[node.0 as usize].tala_id,
                            points.len()
                        );
                        for point in points.iter() {
                            eprint!(" {},{}", point.x, point.y);
                        }
                        eprintln!();
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_SHUFFLED")
                        && crate::internal::trace_env_value("TALA_TRACE_SIZED_NODE")
                            .as_deref()
                            .is_some_and(|target| {
                                target == "all"
                                    || target.parse::<i64>().ok()
                                        == Some(this.graph.nodes[node.0 as usize].tala_id)
                            })
                    {
                        eprint!(
                            "SIZED_SHUFFLED_POINTS_RUST node={}",
                            this.graph.nodes[node.0 as usize].tala_id
                        );
                        for point in points.iter() {
                            eprint!(" {},{}", point.x, point.y);
                        }
                        eprintln!();
                    }
                    let start_position = this.graph.position(node).unwrap_or_default();
                    let moved = this.move_to_best_with_owner_nears(
                        node,
                        points,
                        temperature == 0.0,
                        owner_session.as_deref_mut(),
                        owner_guard.as_deref_mut(),
                    )?;
                    Ok((start_position, moved, points.len()))
                },
            )?;
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_MOVE_RESULTS")
                && trace_sized_graph_scope(self.graph)
            {
                let position = self.graph.position(node).unwrap_or_default();
                let current_node = &self.graph.nodes[node.0 as usize];
                let event = serde_json::json!({
                    "event": "sized-node-result",
                    "node": trace_sized_node_debug_id(&current_node.external_id, current_node.tala_id),
                    "temperature_bits": format!("{:016x}", temperature.to_bits()),
                    "moved": moved,
                    "start_x_bits": format!("{:016x}", start_position.x.to_bits()),
                    "start_y_bits": format!("{:016x}", start_position.y.to_bits()),
                    "x_bits": format!("{:016x}", position.x.to_bits()),
                    "y_bits": format!("{:016x}", position.y.to_bits()),
                    "positions": self.graph.nodes.iter().filter_map(|node| {
                        node.position.map(|position| (
                            trace_sized_node_debug_id(&node.external_id, node.tala_id),
                            serde_json::json!({
                                "x_bits": format!("{:016x}", position.x.to_bits()),
                                "y_bits": format!("{:016x}", position.y.to_bits()),
                            }),
                        ))
                    }).collect::<BTreeMap<_, _>>(),
                    "external_positions": trace_sized_external_positions(self.graph),
                });
                eprintln!("{event}");
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SHUFFLE_CALLS") {
                eprintln!(
                    "MOVED_CALL_RUST node={} moved={}",
                    self.graph.nodes[node.0 as usize].tala_id, moved
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_DETAIL")
                && self.graph.nodes.len() == 4
            {
                eprintln!(
                    "SIZED_RUST_NODE pass={} id={} start={:?} median={:?} distance={} points={} moved={} end={:?}",
                    pass,
                    self.graph.nodes[node.0 as usize].tala_id,
                    trace_start,
                    median,
                    minimum,
                    point_count,
                    moved,
                    self.graph.position(node)
                );
            }
            if (crate::internal::trace_env_enabled("TALA_TRACE_SIZED_SELECTED")
                && self.graph.nodes[node.0 as usize].tala_id == 727256374)
                || (crate::internal::trace_env_enabled("TALA_TRACE_SIZED_SELECTED_ALL")
                    && (!crate::internal::trace_env_enabled("TALA_TRACE_ROOT_ONLY")
                        || self.graph.nodes.len() == 15))
            {
                let selected = self.graph.position(node).unwrap();
                eprintln!(
                    "SIZED_SELECTED_RUST node={} moved={} current={},{}",
                    self.graph.nodes[node.0 as usize].tala_id, moved, selected.x, selected.y
                );
            }
            if moved {
                self.sync_herd_fences_with_owner(owner_session.as_deref_mut())?;
                changed = true;
            } else {
                let candidate = self.best_swap_candidate(node, owner_session.as_deref_mut())?;
                let trace_fallback = trace_sized_graph_scope(self.graph)
                    && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_FALLBACK")
                    && (crate::internal::trace_env_enabled("TALA_TRACE_SIZED_FALLBACK_ALL")
                        || matches!(
                            self.graph.nodes[node.0 as usize].tala_id,
                            923577301 | 856466825
                        ))
                    && (crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS").is_none()
                        || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"));
                if trace_fallback {
                    eprintln!(
                        "SIZED_FALLBACK_RUST node={} current={:?} swap={}",
                        self.graph.nodes[node.0 as usize].tala_id,
                        self.graph.position(node),
                        candidate
                            .map(|swap| self.graph.nodes[swap.0 as usize].tala_id)
                            .unwrap_or_default(),
                    );
                    eprintln!(
                        "SIZED_FALLBACK_RUST edges={} hierarchy={} fixed={} tree={} abductions_some={} abductions={}",
                        self.graph.active_edge_ids(node).len(),
                        self.graph.nodes[node.0 as usize].hierarchy.is_some(),
                        self.graph.nodes[node.0 as usize].fixed_top_left.is_some(),
                        self.graph.is_tree_sentinel(node),
                        self.edge_abduction_nodes.is_some(),
                        self.edge_abduction_nodes.as_ref().map_or(0, BTreeSet::len),
                    );
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_SHUFFLE_CALLS") {
                    eprintln!(
                        "SWAP_RESULT_RUST node={} candidate={}",
                        self.graph.nodes[node.0 as usize].tala_id,
                        candidate
                            .map(|candidate| self.graph.nodes[candidate.0 as usize].tala_id)
                            .unwrap_or(0)
                    );
                }
                let trace_path = crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PATH")
                    && (crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE")
                        .as_deref()
                        .is_some_and(|target| {
                            target == "all"
                                || target.split(',').any(|target| {
                                    target.parse::<i64>().ok()
                                        == Some(self.graph.nodes[node.0 as usize].tala_id)
                                })
                        }));
                if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_SWAPS")
                    && self.graph.nodes.len() == 10
                {
                    eprintln!(
                        "SWAP_RESULT_RUST node={} candidate={}",
                        self.graph.nodes[node.0 as usize].tala_id,
                        candidate
                            .map(|candidate| self.graph.nodes[candidate.0 as usize].tala_id)
                            .unwrap_or_default()
                    );
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_MOVE_ALWAYS")
                    && crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE").is_some_and(
                        |target| {
                            target == "all"
                                || target.split(',').any(|target| {
                                    target.parse::<i64>().ok()
                                        == Some(self.graph.nodes[node.0 as usize].tala_id)
                                })
                        },
                    )
                {
                    eprintln!(
                        "SIZED_FALLBACK_RUST pass={} node={} swap={} current={},{}",
                        pass,
                        self.graph.nodes[node.0 as usize].tala_id,
                        candidate
                            .map(|candidate| self.graph.nodes[candidate.0 as usize].tala_id)
                            .unwrap_or_default(),
                        self.graph.position(node).unwrap().x,
                        self.graph.position(node).unwrap().y,
                    );
                }
                if trace_search {
                    eprintln!(
                        "SIZED_FALLBACK_RUST pass={} node={} swap={} current={},{}",
                        pass,
                        self.graph.nodes[node.0 as usize].tala_id,
                        candidate
                            .map(|candidate| self.graph.nodes[candidate.0 as usize].tala_id)
                            .unwrap_or_default(),
                        self.graph.position(node).unwrap().x,
                        self.graph.position(node).unwrap().y,
                    );
                }
                if let Some(candidate) = candidate {
                    if trace_path {
                        eprintln!(
                            "SIZED_PATH_RUST node={} path=swap candidate={}",
                            self.graph.nodes[node.0 as usize].tala_id,
                            self.graph.nodes[candidate.0 as usize].tala_id
                        );
                    }
                    let node_position = self.graph.position(node).unwrap();
                    let candidate_position = self.graph.position(candidate).unwrap();
                    self.graph
                        .move_active_node_abs_with_children(node, candidate_position);
                    self.graph
                        .move_active_node_abs_with_children(candidate, node_position);
                    // optimizerSwapPositions mutates optim.g before
                    // SyncHerdFences (sized_optimizer.go:386–392); publish the
                    // workspace swap to the owner so the fence sees it.
                    if let Some(session) = owner_session.as_deref_mut() {
                        session
                            .update_current_member_positions(self.graph)
                            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
                    }
                    self.sync_herd_fences_with_owner(owner_session.as_deref_mut())?;
                    changed = true;
                } else {
                    if crate::internal::trace_env_value("TALA_TRACE_SIZED_MOVE_NODE").is_some_and(
                        |target| {
                            target == "all"
                                || target.parse::<i64>().ok()
                                    == Some(self.graph.nodes[node.0 as usize].tala_id)
                        },
                    ) {
                        eprintln!(
                            "HUB_RETRY_RUST node={} has={} temp={}",
                            self.graph.nodes[node.0 as usize].tala_id,
                            self.graph.hubs.contains_key(&node),
                            temperature
                        );
                    }
                    let edge_abduction_mode = self.edge_abduction_nodes.is_some();
                    let participates_in_abduction = self
                        .edge_abduction_nodes
                        .as_ref()
                        .is_some_and(|nodes| nodes.contains(&node));
                    // Recovered Graph.transpose rejects a node when it is
                    // either the original or current endpoint of any supplied
                    // EdgeAbduction before checking edge count or rotation.
                    let transposed = !participates_in_abduction
                        && self.graph.transpose_node_with_owner_routes(
                            node,
                            edge_abduction_mode,
                            owner_session.as_deref_mut(),
                        )?;
                    if trace_fallback {
                        eprintln!(
                            "SIZED_FALLBACK_RUST node={} transpose={} pos={:?}",
                            self.graph.nodes[node.0 as usize].tala_id,
                            transposed,
                            self.graph.position(node),
                        );
                    }
                    if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_TRANSPOSE")
                        && self.graph.nodes.len() == 10
                    {
                        let a = self
                            .graph
                            .nodes
                            .iter()
                            .find(|candidate| candidate.tala_id == 3_826_002_220)
                            .and_then(|candidate| candidate.position);
                        eprintln!(
                            "TRANSPOSE_RESULT_RUST pass={} node={} ok={} a={:?}",
                            pass, self.graph.nodes[node.0 as usize].tala_id, transposed, a
                        );
                    }
                    if trace_search {
                        eprintln!(
                            "SIZED_FALLBACK_RUST node={} transposed={transposed} participatesInAbduction={participates_in_abduction} allContainersNone={}",
                            self.graph.nodes[node.0 as usize].tala_id,
                            self.graph
                                .nodes
                                .iter()
                                .all(|candidate| candidate.container.is_none())
                        );
                    }
                    if transposed {
                        if trace_path {
                            eprintln!(
                                "SIZED_PATH_RUST node={} path=transpose",
                                self.graph.nodes[node.0 as usize].tala_id
                            );
                        }
                        changed = true;
                    } else if temperature != 0.0
                        && let Some(spokes) = self.hub_spokes(node)
                    {
                        if trace_path {
                            eprintln!(
                                "SIZED_PATH_RUST node={} path=hub spokes={}",
                                self.graph.nodes[node.0 as usize].tala_id,
                                spokes.len()
                            );
                        }
                        if trace_search {
                            eprintln!(
                                "SIZED_HUB_RETRY_RUST node={} spokes={:?}",
                                self.graph.nodes[node.0 as usize].tala_id,
                                spokes
                                    .iter()
                                    .map(|spoke| self.graph.nodes[spoke.0 as usize].tala_id)
                                    .collect::<Vec<_>>()
                            );
                        }
                        self.retry_without_hub_spokes(
                            node,
                            &spokes,
                            temperature,
                            &mut checked_positions,
                            owner_session.as_deref_mut(),
                            owner_guard.as_deref_mut(),
                        )?;
                        if trace_fallback {
                            eprintln!(
                                "SIZED_FALLBACK_RUST node={} hub-after={:?}",
                                self.graph.nodes[node.0 as usize].tala_id,
                                self.graph.position(node),
                            );
                        }
                    }
                }
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_MOVE_RESULTS")
                && trace_sized_graph_scope(self.graph)
            {
                let positions = self
                    .graph
                    .nodes
                    .iter()
                    .filter_map(|node| {
                        node.position.map(|position| {
                            (
                                trace_sized_node_debug_id(&node.external_id, node.tala_id),
                                serde_json::json!({
                                    "x_bits": format!("{:016x}", position.x.to_bits()),
                                    "y_bits": format!("{:016x}", position.y.to_bits()),
                                }),
                            )
                        })
                    })
                    .collect::<BTreeMap<_, _>>();
                let event = serde_json::json!({
                    "event": "sized-node-finish",
                    "node": trace_sized_node_debug_id(
                        &self.graph.nodes[node.0 as usize].external_id,
                        self.graph.nodes[node.0 as usize].tala_id,
                    ),
                    "temperature_bits": format!("{:016x}", temperature.to_bits()),
                    "positions": positions,
                    "external_positions": trace_sized_external_positions(self.graph),
                });
                eprintln!("{event}");
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_NODE_STATE_AFTER_SELECTION") {
                let tracked = self
                    .graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == 2_039_569_393)
                    .and_then(|candidate| candidate.position);
                eprintln!(
                    "NODE_STATE_AFTER_RUST selected={} tracked203={tracked:?}",
                    self.graph.nodes[node.0 as usize].tala_id
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_STEP") {
                eprintln!(
                    "SIZED_STEP_RUST node={} n0={:?}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph
                        .nodes
                        .iter()
                        .find(|candidate| candidate.tala_id == 87_416_211)
                        .and_then(|candidate| candidate.position)
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_RANKING_MODEL")
                && self.graph.nodes.len() == 15
                && let Some(candidate) = self
                    .graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.tala_id == 363508928)
                && let Some(position) = candidate.position
            {
                eprintln!(
                    "RANK_AFTER_RUST node={} pos={},{}",
                    self.graph.nodes[node.0 as usize].tala_id, position.x, position.y
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZED_FINAL")
                && (!crate::internal::trace_env_enabled("TALA_TRACE_ROOT_ONLY")
                    || self.graph.nodes.len() == 15)
            {
                let selected = self.graph.position(node).unwrap();
                eprintln!(
                    "SIZED_FINAL_RUST node={} moved={} current={},{}",
                    self.graph.nodes[node.0 as usize].tala_id, moved, selected.x, selected.y
                );
            }
            if trace_call {
                eprintln!(
                    "SIZED_DETAIL_RUST pass={} node={} start={:?} median={:?} distance={} points={} moved={} end={:?}",
                    pass,
                    self.graph.nodes[node.0 as usize].tala_id,
                    trace_start,
                    median,
                    minimum,
                    point_count,
                    moved,
                    self.graph.position(node),
                );
            }
            if crate::internal::trace_env_value("TALA_TRACE_TARGET_STATE").is_some_and(|target| {
                target == "all" || target == self.graph.nodes[node.0 as usize].tala_id.to_string()
            }) {
                eprintln!(
                    "TARGET_STATE_RUST node={} pass={} start={:?} median={:?} distance={} points={} moved={} end={:?}",
                    self.graph.nodes[node.0 as usize].tala_id,
                    pass,
                    trace_start,
                    median,
                    minimum,
                    point_count,
                    moved,
                    self.graph.position(node),
                );
            }
            if trace_root_state
                && crate::internal::trace_env_enabled("TALA_TRACE_SIZED_GRAPH_STATE")
            {
                eprint!(
                    "SIZED_GRAPH_STATE_RUST after-{}-{}",
                    self.graph.nodes[node.0 as usize].tala_id, node.0
                );
                for graph_node in &self.graph.nodes {
                    eprint!(" {}@{:?}", graph_node.tala_id, graph_node.position);
                }
                eprintln!();
            }
        }
        if trace_root_state {
            eprint!(
                "SIZED_STATE_RUST end temp={} changed={} ",
                temperature, changed
            );
            for node in &self.nodes {
                eprint!(
                    "{}@{:?} ",
                    self.graph.nodes[node.0 as usize].tala_id,
                    self.graph.position(*node)
                );
            }
            eprintln!();
        }
        trace_sized_optimizer_result(temperature, changed, self.graph);
        if let Some(guard) = owner_guard {
            #[cfg(feature = "diagnostic-traces")]
            if let Some(session) = owner_session {
                guard.finish_with_owner(session.owner_mut())?;
            } else {
                guard.finish()?;
            }
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.finish()?;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod root_origin_tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    #[test]
    fn root_medians_respect_translated_fixed_origin_under_random_perturbation() {
        let mut input = Graph::default();
        let moving = input.add_node(Node::new(
            "moving",
            Size {
                width: 200.0,
                height: 200.0,
            },
        ));
        let mut anchor = Node::new(
            "anchor",
            Size {
                width: 40.0,
                height: 40.0,
            },
        );
        anchor.locked_position = Some(Point { x: 100.0, y: 100.0 });
        let anchor = input.add_node(anchor);
        input.add_edge(Edge {
            source: moving,
            target: anchor,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(moving, Point { x: 400.0, y: 400.0 });
        graph.set_position(anchor, Point { x: 130.0, y: 140.0 });
        // optimizerFixedOrigin searches the root's siblings too. Their
        // current-minus-fixed displacement supplies the median's floor.
        let mut optimizer = SizedOptimizer::new(&mut graph, GoRng::new(1), None);
        for _ in 0..32 {
            let median = optimizer.median_point(moving, 10.0, &[]);
            assert!(median.x >= 30.0 && median.y >= 40.0, "{median:?}");
        }
        let points = optimizer.placements(moving, Point { x: 30.0, y: 40.0 }, 1.0, true);
        assert!(!points.is_empty());
        assert!(points.iter().all(|p| p.x >= 30.0 && p.y >= 40.0));
        assert!(optimizer.search_point_is_occupied(moving, Point { x: 29.0, y: 1000.0 }));
    }

    #[test]
    fn sized_candidate_score_avoids_fixed_ancestor_obstacles() {
        let mut input = Graph::default();
        let moving = input.add_node(Node::new(
            "moving",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 10.0;
        graph.set_turn_cost(10.0);
        graph.set_position(moving, Point { x: 0.0, y: 0.0 });
        let obstacle = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 100.0,
                height: 100.0,
            },
        };
        let mut optimizer =
            SizedOptimizer::new(&mut graph, GoRng::new(1), None).with_obstacles(vec![obstacle]);

        assert!(optimizer.move_to_best(
            moving,
            &[Point { x: 0.0, y: 0.0 }, Point { x: 500.0, y: 500.0 }],
            false,
        ));
        assert_eq!(
            optimizer.graph().position(moving),
            Some(Point { x: 500.0, y: 500.0 })
        );
    }

    #[test]
    fn retry_without_hub_spokes_preserves_parallel_edges() {
        let mut input = Graph::default();
        let hub = input.add_node(Node::new(
            "hub",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let spoke = input.add_node(Node::new(
            "spoke",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let other = input.add_node(Node::new(
            "other",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        input.add_edge(Edge {
            source: hub,
            target: spoke,
        });
        input.add_edge(Edge {
            source: hub,
            target: spoke,
        });
        input.add_edge(Edge {
            source: hub,
            target: other,
        });

        let mut graph = ArenaGraph::from_input(&input);
        for node in [hub, spoke, other] {
            graph.set_position(node, Point { x: 0.0, y: 0.0 });
        }
        let optimizer = SizedOptimizer::new(&mut graph, GoRng::new(1), None);
        let old_edges = optimizer.graph.nodes[hub.0 as usize].edges.read();
        let remaining = edges_without_hub_spokes(optimizer.graph, hub, &[spoke], &old_edges);
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|edge| {
            let edge = &optimizer.graph.edge_records[edge.0 as usize];
            edge.from == hub && edge.to == spoke
        }));
        assert!(remaining.iter().any(|edge| {
            let edge = &optimizer.graph.edge_records[edge.0 as usize];
            edge.from == hub && edge.to == other
        }));
    }
}
