// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Deterministic post-placement swap, direction, and transpose optimization.
//!
//! Each proposal captures every mutable projected view, then commits or rolls
//! back the whole trial under recovered score and order rules.

use super::limits::OptimizationError;
use super::owner_score_session::OwnerScoreSessionError;
use super::*;

#[derive(Clone)]
struct SwapTrialSnapshot {
    geometry: super::trial_geometry::TrialGeometrySnapshot,
}
impl SwapTrialSnapshot {
    fn capture(graph: &ArenaGraph) -> Self {
        Self {
            geometry: graph.trial_geometry_snapshot(),
        }
    }
    fn restore(self, graph: &mut ArenaGraph) {
        graph.restore_trial_geometry_snapshot(self.geometry);
    }
}

/// All candidates for one source node start from the same rolled-back graph.
/// Go retains its Transaction state until a swap commits (swap.go:86,272).
/// Keep the compact geometry and moved-node baseline across those trials too.
struct SwapTrialBaseline {
    state: SwapTrialSnapshot,
    geometry: Vec<(NodeId, Option<Point>, Size, Option<Point>, Size)>,
}

impl SwapTrialBaseline {
    fn capture(graph: &ArenaGraph) -> Self {
        Self {
            state: SwapTrialSnapshot::capture(graph),
            geometry: graph
                .graph_node_order()
                .into_iter()
                .map(|node| {
                    (
                        node,
                        graph.active_node_position(node),
                        graph.active_node_size(node),
                        graph.nodes[node.0 as usize].position,
                        graph.nodes[node.0 as usize].rect.size,
                    )
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ActiveMirrorNode {
    Stable(NodeId),
    SequenceVessel(usize),
    ClusterVessel(usize),
}

#[derive(Clone, Copy, Debug)]
struct SequenceMirrorProjection {
    first_member_position: Point,
    vessel_position: Point,
}

/// swap.go chooses the swap form by local cost before comparing its global
/// cost. A nonzero regular global score also makes the both-zero early return
/// impossible. In that case the smart global/crossing scores are unused when
/// its local cost loses that choice or cannot improve the current node.
fn smart_swap_requires_global_score(
    regular: (f64, f64, usize),
    smart_local: f64,
    current_local: f64,
) -> bool {
    // Explicit score diagnostics still report the complete original terms.
    // These gates are constant false without diagnostic-traces.
    if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_SCORE")
        || crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS")
    {
        return true;
    }
    if regular.1 == 0.0 {
        return true;
    }
    if smart_local == 0.0 {
        return regular.0 == 0.0;
    }
    let better = |candidate: f64, current: f64| candidate < current - 0.0001;
    better(smart_local, current_local) && (regular.0 == 0.0 || better(smart_local, regular.0))
}

/// A nonzero regular local score that cannot improve is rejected regardless
/// of its global score. Zero local scores still select by global cost.
fn regular_swap_requires_global_score(local: f64, current_local: f64) -> bool {
    crate::internal::trace_env_enabled("TALA_TRACE_SWAP_SCORE")
        || crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS")
        || local == 0.0
        || local < current_local - 0.0001
}

/// An omitted regular global score only matters again if an improving smart
/// form has an actual zero global score: swap.go rejects two zero globals.
fn regular_swap_requires_zero_check(
    regular_local: f64,
    smart: (f64, f64, usize),
    current_local: f64,
) -> bool {
    smart.1 == 0.0
        && smart.0 != 0.0
        && smart.0 < current_local - 0.0001
        && smart.0 < regular_local - 0.0001
}

impl ArenaGraph {
    pub(super) fn edge_is_directed(&self, edge: EdgeId) -> bool {
        let edge = &self.edge_records[edge.0 as usize];
        edge.source_arrow != edge.target_arrow
    }

    /// Exact `Graph.GetDirection(node.getContainer())` absence predicate.
    ///
    /// The layout adapter keeps a fallback direction for placement, but TALA
    /// returns `geo.NONE` for scoring unless the direct container scope has a
    /// `Directions` entry.
    pub(super) fn container_direction_is_unset(&self, node: NodeId) -> bool {
        let container = self.nodes[node.0 as usize].container;
        let container_tala_id = self.nodes[node.0 as usize].scoring_container_parent;
        if let Some(parent) = container_tala_id {
            return !self.scoring_directions_by_tala.contains_key(&Some(parent));
        }
        if self.scoring_directions.contains_key(&container) {
            return false;
        }
        !self
            .scoring_directions_by_tala
            .contains_key(&container_tala_id)
    }

    pub(super) fn root_direction_is_unset(&self) -> bool {
        !self.scoring_directions.contains_key(&None)
            && !self.scoring_directions_by_tala.contains_key(&None)
    }

    // Recovered Node.edgeLength direction selection. In the NONE-direction
    // sized path, three or more labels between the same pair deliberately
    // induce a strong horizontal preference so parallel labels can fan out.
    pub(super) fn edge_length_direction(
        &self,
        node: NodeId,
        include_sizes: bool,
        use_recovered_unset_scoring: bool,
    ) -> (Orientation, f64) {
        if !use_recovered_unset_scoring {
            let container = self.nodes[node.0 as usize].container;
            let container_tala_id = self.nodes[node.0 as usize].scoring_container_parent;
            let direction = if let Some(parent) = container_tala_id {
                // Induced scopes retain the owner's explicit scoring map,
                // while their ordinary direction map uses local identities.
                self.scoring_directions_by_tala
                    .get(&Some(parent))
                    .copied()
                    .or_else(|| self.directions_by_tala.get(&Some(parent)).copied())
            } else {
                self.directions
                    .get(&container)
                    .copied()
                    .or_else(|| self.directions_by_tala.get(&container_tala_id).copied())
            }
            .unwrap_or(Direction::Right);
            return (
                layout_orientation(direction),
                if include_sizes { 6.0 } else { 1.5 },
            );
        }
        if self.nodes[node.0 as usize].shape == ShapeKind::SqlTable {
            return (Orientation::Right, 0.5);
        }
        if let Some(arrangement) = self.nodes[node.0 as usize]
            .scoring_cluster_arrangement
            .or_else(|| {
                self.active_cluster_index(node)
                    .filter(|cluster| self.clusters[*cluster].members.first() == Some(&node))
                    .map(|cluster| self.clusters[cluster].arrangement)
            })
        {
            return (
                match arrangement {
                    ClusterArrangement::Row => Orientation::Bottom,
                    ClusterArrangement::Column => Orientation::Right,
                },
                0.2,
            );
        }
        if include_sizes {
            let active_edges = self.active_edge_ids(node);
            let labeled_nonloops = active_edges
                .iter()
                .copied()
                .filter(|edge_id| {
                    let edge = &self.edge_records[edge_id.0 as usize];
                    edge.label.is_some() && self.active_adjacent(node, *edge_id) != node
                })
                .count();
            if labeled_nonloops < 3 {
                return (Orientation::BottomRight, 0.3);
            }
            let mut label_counts = BTreeMap::<NodeId, usize>::new();
            let mut outgoing_label_counts = BTreeMap::<NodeId, usize>::new();
            let mut multi_label_node = None;
            for edge_id in active_edges {
                let edge = &self.edge_records[edge_id.0 as usize];
                if edge.label.is_none() {
                    continue;
                }
                let adjacent = self.active_adjacent(node, edge_id);
                if adjacent == node {
                    continue;
                }
                let count = label_counts.entry(adjacent).or_default();
                *count += 1;
                // DirectedEndpoints reverses a source-only arrow. Both
                // undirected forms retain their declaration endpoint.
                let semantic_from = if edge.source_arrow && !edge.target_arrow {
                    edge.to
                } else {
                    edge.from
                };
                if self.active_aggregate_owner(semantic_from) == node {
                    *outgoing_label_counts.entry(adjacent).or_default() += 1;
                }
                if *count > 2 {
                    multi_label_node = Some(adjacent);
                }
            }
            if let Some(adjacent) = multi_label_node {
                let labels = label_counts[&adjacent];
                let outgoing = outgoing_label_counts.get(&adjacent).copied().unwrap_or(0);
                return (
                    if outgoing > labels / 2 {
                        Orientation::Left
                    } else {
                        Orientation::Right
                    },
                    10.0,
                );
            }
        }
        (Orientation::BottomRight, 0.3)
    }

    pub(super) fn is_majority_target(&self, node: NodeId) -> bool {
        let mut counter = 0_i32;
        for edge_id in self.active_edge_ids(node) {
            if !self.edge_is_directed(edge_id) {
                continue;
            }
            let edge = &self.edge_records[edge_id.0 as usize];
            let targeted = (self.active_aggregate_owner(edge.to) == node && edge.target_arrow)
                || (self.active_aggregate_owner(edge.from) == node && edge.source_arrow);
            counter += if targeted { 1 } else { -1 };
        }
        counter > 0
    }

    // Direct translations of moveNodeWithChildren and moveNodeAbsWithChildren.
    pub(super) fn direction_transforms(&self, container: Option<NodeId>) -> (bool, bool) {
        self.direction_transforms_with_mode(container, false)
    }

    // TALA's initial direct(false) call walks the temporary graph, where a
    // cluster is already represented by its full synthetic vessel. The later
    // direct(true) call in SwapStuff walks the current active carrier view;
    // retaining member boxes there counts the same cluster edge more than
    // once and changes the precision-sensitive mirror tie.
    fn direction_transforms_with_mode(
        &self,
        container: Option<NodeId>,
        active_vessel_view: bool,
    ) -> (bool, bool) {
        let mut counts = [
            (Orientation::Right, 0_usize),
            (Orientation::Bottom, 0),
            (Orientation::Left, 0),
            (Orientation::Top, 0),
        ];
        let mut seen = BTreeSet::new();
        let nodes = if active_vessel_view {
            self.container_node_order(container)
        } else {
            self.containers.get(&container).unwrap_or_default()
        };
        if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT")
            && container.is_none()
            && !active_vessel_view
        {
            eprintln!(
                "DIRECT_OWNERS_RUST node_order={:?} active={:?} valid={}",
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.active_graph_node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.node_order_membership_valid,
            );
        }
        for node in nodes {
            // Go's Graph.Containers slice aliases the parent graph, while
            // Graph.direct counts only nodes whose Graph pointer is the
            // current graph. The Rust arena keeps those aliases in one
            // container map; the current node_order is the equivalent live
            // Graph.Nodes ownership set for this scope.
            let graph_owns_node = if self.active_graph_node_order.is_empty() {
                self.node_order.contains(&node)
            } else {
                self.active_graph_node_order.contains(&node)
            };
            if !active_vessel_view && !graph_owns_node {
                continue;
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT")
                && container.is_none()
                && !active_vessel_view
            {
                let n = &self.nodes[node.0 as usize];
                eprintln!(
                    "DIRECT_NODE_RUST node={} container={:?} seq={:?} cluster={:?} is_container={} scoring_container={} tree_children={} edges={}",
                    n.external_id,
                    n.container
                        .map(|id| self.nodes[id.0 as usize].external_id.clone()),
                    n.sequence,
                    n.cluster,
                    n.is_container,
                    n.scoring_is_container,
                    self.preprocessed_tree_children.contains_key(&Some(node)),
                    n.edges.len(),
                );
            }
            let edge_ids = if active_vessel_view {
                self.active_edge_ids(node).into_iter().collect()
            } else {
                self.nodes[node.0 as usize].edges.read()
            };
            for edge_id in edge_ids {
                if !seen.insert(edge_id) {
                    continue;
                }
                let edge = &self.edge_records[edge_id.0 as usize];
                // Graph.DirectedEndpoints preserves declaration order for a
                // target-arrow edge and reverses it for a source-arrow edge.
                // Both forms are directed when exactly one endpoint carries
                // an arrow; only the no-arrow and bidirectional forms are
                // excluded.  Keeping this distinction is material for
                // direct()/mirrorAxes, which derives the dominant flow from
                // the semantic source and target boxes.
                if edge.source_arrow == edge.target_arrow {
                    continue;
                }
                // restoreEdgeAbductions removes an edge from both carrier
                // nodes when both original endpoints live below those
                // carriers. Graph.direct cannot observe that edge through
                // either carrier's Edge slice.
                // An endpoint reconnected to an active cluster vessel at
                // this level is an AddClusters reconnection, not a placement
                // abduction: RestoreEdgeAbductions leaves the edge on the
                // vessel's Edges, so containerEdgeDirections still counts it
                // (placement/direct.go containerEdgeDirections).
                let restored_endpoint = |carrier: NodeId, receiver: NodeId| {
                    self.sized_adjacent_overrides
                        .get(&(receiver, edge_id))
                        .is_some_and(|original| {
                            !self
                                .transaction_external_aggregate_children
                                .get(&self.nodes[carrier.0 as usize].tala_id)
                                .is_some_and(|members| {
                                    members
                                        .iter()
                                        .any(|member| member.tala_id == original.tala_id)
                                })
                        })
                };
                let source_was_abducted = restored_endpoint(edge.from, edge.to);
                let target_was_abducted = restored_endpoint(edge.to, edge.from);
                if source_was_abducted && target_was_abducted {
                    continue;
                }
                // For an edge that remains visible on one carrier, reuse
                // the recovered endpoint projection as sized edge
                // scoring; fully abducted edges were excluded above.
                let (semantic_from, semantic_to) = if edge.source_arrow {
                    (edge.to, edge.from)
                } else {
                    (edge.from, edge.to)
                };
                let direct_source_tala = if !active_vessel_view {
                    // RestoreEdgeAbductions puts cross-container edges back
                    // on their original node pointers before Graph.direct.
                    // The arena edge can still point at its carrier, so use
                    // the retained source endpoint identities for this
                    // diagnostic/current-graph view.
                    self.source_edge_endpoints_by_id
                        .get(&edge.tala_id)
                        .map(|(source, target)| {
                            if edge.source_arrow {
                                (*target, *source)
                            } else {
                                (*source, *target)
                            }
                        })
                } else {
                    None
                };
                let (from_node, to_node) = (
                    self.active_aggregate_owner(semantic_from),
                    self.active_aggregate_owner(semantic_to),
                );
                let (mut from_box, mut to_box) = if active_vessel_view {
                    let Some((from_box, to_box)) =
                        self.sized_edge_boxes(semantic_from, semantic_to, edge_id)
                    else {
                        continue;
                    };
                    (from_box, to_box)
                } else {
                    // Graph.direct runs after edge abductions have been
                    // restored. Its edge endpoints are the current vessel
                    // pointers, so aggregate members must be measured using
                    // the live vessel box rather than the sized scoring
                    // override for the original leaf endpoint.
                    let direct_box_for_tala = |tala_id: i64| {
                        // AddClusters reconnects an edge to its synthetic
                        // vessel before Graph.direct runs. The stable arena
                        // keeps the original member ID in the source table,
                        // so recover that pointer-visible vessel box from
                        // the projected aggregate members first. Returning a
                        // member's raw box here makes a large column appear
                        // as a stack of unrelated nodes and changes the
                        // diagonal direction counts used by direct(false).
                        if let Some((vessel_tala_id, members)) = self
                            .transaction_external_aggregate_children
                            .iter()
                            .find(|(_, members)| members.iter().any(|node| node.tala_id == tala_id))
                        {
                            let positioned = members
                                .iter()
                                .filter_map(|node| {
                                    node.position.map(|position| (position, node.rect.size))
                                })
                                .collect::<Vec<_>>();
                            if !positioned.is_empty() {
                                let origin = Point {
                                    x: positioned
                                        .iter()
                                        .map(|(position, _)| position.x)
                                        .fold(f64::INFINITY, f64::min),
                                    y: positioned
                                        .iter()
                                        .map(|(position, _)| position.y)
                                        .fold(f64::INFINITY, f64::min),
                                };
                                let layout = self
                                    .transaction_external_cluster_layouts
                                    .get(vessel_tala_id);
                                let arrangement = layout.map(|layout| layout.arrangement);
                                let padding = layout.map(|layout| layout.padding).unwrap_or(20.0);
                                let size = match arrangement {
                                    Some(ClusterArrangement::Row) => Size {
                                        width: positioned
                                            .iter()
                                            .map(|(_, size)| size.width)
                                            .sum::<f64>()
                                            + padding * positioned.len().saturating_sub(1) as f64,
                                        height: positioned
                                            .iter()
                                            .map(|(_, size)| size.height)
                                            .fold(0.0, f64::max),
                                    },
                                    _ => Size {
                                        width: positioned
                                            .iter()
                                            .map(|(_, size)| size.width)
                                            .fold(0.0, f64::max),
                                        height: positioned
                                            .iter()
                                            .map(|(_, size)| size.height)
                                            .sum::<f64>()
                                            + padding * positioned.len().saturating_sub(1) as f64,
                                    },
                                };
                                return Some((origin, size));
                            }
                        }
                        if let Some((index, _)) = self
                            .nodes
                            .iter()
                            .enumerate()
                            .find(|(_, node)| node.tala_id == tala_id)
                        {
                            let node = NodeId(index as u32);
                            if let Some(position) = self.active_node_position(node) {
                                return Some((position, self.active_node_size(node)));
                            }
                        }
                        if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT") {
                            eprintln!(
                                "DIRECT_BOX_FALLBACK_RUST tala={} aggregates={:?} containers={:?}",
                                tala_id,
                                self.transaction_external_aggregate_children
                                    .iter()
                                    .map(|(owner, children)| (
                                        *owner,
                                        children
                                            .iter()
                                            .map(|node| (
                                                node.tala_id,
                                                node.position,
                                                node.rect.size
                                            ))
                                            .collect::<Vec<_>>(),
                                    ))
                                    .collect::<Vec<_>>(),
                                self.transaction_external_containers
                                    .iter()
                                    .map(|node| (node.tala_id, node.position, node.rect.size))
                                    .collect::<Vec<_>>()
                            );
                        }
                        self.projected_node_by_tala(tala_id).and_then(|node| {
                            node.position.map(|position| (position, node.rect.size))
                        })
                    };
                    if let Some((from_tala, to_tala)) = direct_source_tala {
                        let Some(from_box) = direct_box_for_tala(from_tala) else {
                            continue;
                        };
                        let Some(to_box) = direct_box_for_tala(to_tala) else {
                            continue;
                        };
                        (from_box, to_box)
                    } else {
                        let Some(from_position) = self
                            .active_node_position(from_node)
                            .or_else(|| self.position(semantic_from))
                        else {
                            continue;
                        };
                        let Some(to_position) = self
                            .active_node_position(to_node)
                            .or_else(|| self.position(semantic_to))
                        else {
                            continue;
                        };
                        (
                            (from_position, self.active_node_size(from_node)),
                            (to_position, self.active_node_size(to_node)),
                        )
                    }
                };
                if active_vessel_view {
                    if self.active_node_is_aggregate(from_node)
                        && let Some(position) = self.active_node_position(from_node)
                    {
                        from_box = (position, self.active_node_size(from_node));
                    }
                    if self.active_node_is_aggregate(to_node)
                        && let Some(position) = self.active_node_position(to_node)
                    {
                        to_box = (position, self.active_node_size(to_node));
                    }
                }
                let orientation = self.sized_box_orientation(from_box, to_box).opposite();
                if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT") {
                    eprintln!(
                        "DIRECT_EDGE_RUST edge={} raw={}->{} from={} to={} source={} target={} orientation={:?} from_box={:?} to_box={:?}",
                        edge.tala_id,
                        self.nodes[edge.from.0 as usize].tala_id,
                        self.nodes[edge.to.0 as usize].tala_id,
                        self.nodes[from_node.0 as usize].tala_id,
                        self.nodes[to_node.0 as usize].tala_id,
                        edge.source_arrow,
                        edge.target_arrow,
                        orientation,
                        from_box,
                        to_box,
                    );
                }
                for (direction, count) in &mut counts {
                    let contributes = match *direction {
                        Orientation::Right => matches!(
                            orientation,
                            Orientation::Right | Orientation::TopRight | Orientation::BottomRight
                        ),
                        Orientation::Bottom => matches!(
                            orientation,
                            Orientation::Bottom
                                | Orientation::BottomLeft
                                | Orientation::BottomRight
                        ),
                        Orientation::Left => matches!(
                            orientation,
                            Orientation::Left | Orientation::TopLeft | Orientation::BottomLeft
                        ),
                        Orientation::Top => matches!(
                            orientation,
                            Orientation::Top | Orientation::TopLeft | Orientation::TopRight
                        ),
                        _ => false,
                    };
                    *count += usize::from(contributes);
                }
            }
        }

        let requested = self
            .scoring_directions
            .get(&container)
            .copied()
            .map(layout_orientation)
            .unwrap_or(Orientation::None);
        counts.sort_by(
            |(left_direction, left_count), (right_direction, right_count)| {
                right_count.cmp(left_count).then_with(|| {
                    if *left_direction == requested {
                        std::cmp::Ordering::Less
                    } else if *right_direction == requested {
                        std::cmp::Ordering::Greater
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
            },
        );

        let primary = counts[0];
        let mut secondary = counts[1];
        if secondary.0 == primary.0.opposite() {
            secondary = counts[2];
        }
        let direction = if requested == Orientation::None {
            if primary.0.is_horizontal() {
                Orientation::Right
            } else {
                Orientation::Bottom
            }
        } else {
            requested
        };
        let (x_direction, y_direction) = if direction.is_horizontal() {
            (direction, Orientation::Bottom)
        } else {
            (Orientation::Right, direction)
        };
        let (mut mirror_x, mut mirror_y) = if primary.0.is_horizontal() {
            (primary.0 != x_direction, false)
        } else {
            (false, primary.0 != y_direction)
        };
        if secondary.1 > counts[3].1 {
            if secondary.0.is_horizontal() {
                mirror_x = secondary.0 != x_direction;
            } else {
                mirror_y = secondary.0 != y_direction;
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT") {
            eprintln!(
                "DIRECT_RUST requested={:?} primary={:?} secondary={:?} tail={:?} direction={:?} mirrors={},{}",
                requested, primary, secondary, counts[3], direction, mirror_x, mirror_y,
            );
        }
        (mirror_x, mirror_y)
    }

    // Recovered Graph.mirrorAxes over the current Graph.Nodes ownership. The
    // stable arena retains nodes that TALA temporarily replaces with aggregate
    // vessels, so walk only the current roots and descend through the same
    // container/aggregate ownership before publishing every mirrored pointer
    // into the flattened alias carriers.
    pub(super) fn mirror_axes(&mut self, mirror_x: bool, mirror_y: bool) {
        self.mirror_axes_with_projected_refits(mirror_x, mirror_y);
    }

    pub(super) fn mirror_axes_with_projected_refits(
        &mut self,
        mirror_x: bool,
        mirror_y: bool,
    ) -> BTreeSet<i64> {
        self.mirror_axes_with_reachable(mirror_x, mirror_y, None)
    }

    /// Mirror using the reachability set recovered from the owning placement
    /// graph.  An induced graph carries only component roots in `nodes`, while
    /// its projected container map still contains the original pointer tree;
    /// recomputing reachability from the induced arena therefore loses the
    /// container identities that decide which descendants TALA reflects.
    pub(super) fn mirror_axes_with_reachable(
        &mut self,
        mirror_x: bool,
        mirror_y: bool,
        supplied_reachable: Option<&BTreeSet<Option<NodeId>>>,
    ) -> BTreeSet<i64> {
        if !mirror_x && !mirror_y {
            return BTreeSet::new();
        }

        self.trace_mirror_state("before");
        let roots = self.graph_node_order();
        let graph_node_membership = roots.iter().copied().collect::<BTreeSet<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_MIRROR") {
            eprintln!(
                "MIRROR_STAGE_RUST begin mirrors={},{} roots={:?}",
                mirror_x,
                mirror_y,
                roots
                    .iter()
                    .map(|root| self.active_mirror_tala_id(self.active_mirror_node(*root)))
                    .collect::<Vec<_>>()
            );
            for root in roots.iter().copied() {
                let node = self.active_mirror_node(root);
                eprintln!(
                    "MIRROR_STAGE_RUST before id={} pos={:?}",
                    self.active_mirror_tala_id(node),
                    self.active_node_position(root)
                );
                if self.active_mirror_tala_id(node) == 457364737 {
                    for (parent, children) in &self.transaction_external_container_children {
                        if *parent == 457364737 {
                            eprintln!(
                                "MIRROR_STAGE_RUST before_children id=457364737 children={:?}",
                                children
                                    .iter()
                                    .map(|child| (child.tala_id, child.position))
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }
        }
        let reachable_containers = supplied_reachable.cloned().unwrap_or_else(|| {
            let mut containers = BTreeSet::new();
            let mut reachable_nodes = BTreeSet::new();
            for root in roots.iter().copied() {
                self.collect_active_mirror_reachable_containers(
                    self.active_mirror_node(root),
                    &graph_node_membership,
                    &mut reachable_nodes,
                    &mut containers,
                );
            }
            containers
        });
        self.trace_target_shared_geometry("mirror-before");

        // Go shares this map across every Graph.Nodes root. A child reached by
        // an earlier root is not reflected again when its own slice entry is
        // encountered later.
        let mut mirrored = BTreeSet::new();
        // Current graph carriers are represented by stable IDs. Descendant
        // pointers retained in the shared Containers projection remain
        // external aliases; Go's rdfsWalk visits those aliases independently.
        let mut represented_active_tala_ids = BTreeSet::new();
        for root in &roots {
            let mut reachable_active_nodes = BTreeSet::new();
            self.collect_active_mirror_reach(
                self.active_mirror_node(*root),
                &mut reachable_active_nodes,
                &mut BTreeSet::new(),
            );
            represented_active_tala_ids.extend(
                reachable_active_nodes
                    .into_iter()
                    .map(|node| self.active_mirror_tala_id(node)),
            );
        }
        let mut projected_refits = BTreeSet::new();
        let mut sequence_mirror_projections = BTreeMap::new();
        let mut projected_visited = BTreeSet::new();
        for root in roots {
            let root = self.active_mirror_node(root);
            if mirrored.contains(&root) {
                continue;
            }
            self.mirror_active_subtree_postorder(
                root,
                mirror_x,
                mirror_y,
                &reachable_containers,
                &represented_active_tala_ids,
                &mut mirrored,
                &mut projected_refits,
                &mut sequence_mirror_projections,
                &mut projected_visited,
            );
        }

        self.publish_mirrored_active_positions(&mirrored, &sequence_mirror_projections);
        self.trace_mirror_state("after-publish");
        self.trace_target_shared_geometry("mirror-after-publish");
        if crate::internal::trace_env_enabled("TALA_TRACE_MIRROR") {
            for root in self.graph_node_order() {
                let node = self.active_mirror_node(root);
                let tala_id = self.active_mirror_tala_id(node);
                eprintln!(
                    "MIRROR_STAGE_RUST after id={} pos={:?}",
                    tala_id,
                    self.active_node_position(root)
                );
            }
            if let Some(children) = self.transaction_external_container_children.get(&457364737) {
                eprintln!(
                    "MIRROR_STAGE_RUST after_children id=457364737 children={:?}",
                    children
                        .iter()
                        .map(|child| (child.tala_id, child.position))
                        .collect::<Vec<_>>()
                );
            }
        }
        // Node pointers in the recovered graph are shared by all endpoint and
        // obstruction views. Rebuild the flattened owner-relative views from
        // their now-mirrored absolute boxes, without arranging clusters at a
        // lifecycle boundary Graph.mirrorAxes does not contain.
        self.reconcile_projected_offsets_after_rollback();
        self.trace_mirror_state("after-reconcile");
        self.trace_target_shared_geometry("mirror-after-reconcile");
        projected_refits
    }

    fn trace_mirror_state(&self, phase: &str) {
        if !crate::internal::trace_env_enabled("TALA_TRACE_MIRROR_STATE") {
            return;
        }
        let stable = self
            .nodes
            .iter()
            .filter_map(|node| {
                node.position.map(|position| {
                    (
                        node.tala_id,
                        position,
                        node.rect.size,
                        node.container
                            .map(|container| self.nodes[container.0 as usize].tala_id),
                    )
                })
            })
            .collect::<Vec<_>>();
        let external = self
            .transaction_external_container_children
            .iter()
            .map(|(parent, children)| {
                (
                    *parent,
                    children
                        .iter()
                        .filter_map(|child| {
                            child
                                .position
                                .map(|position| (child.tala_id, position, child.rect.size))
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        eprintln!(
            "MIRROR_STATE_RUST phase={} stable={:?} external={:?}",
            phase, stable, external
        );
    }

    // Recovered Graph.direct root path. Fixed layouts are intentionally not
    // reoriented. With check_edge_length, the mirror is transactional and is
    // retained only when it does not worsen TALA's directional global score.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub(super) fn direct_score_is_worse(candidate: f64, current: f64) -> bool {
        // geo.PrecisionCompare uses an open equality boundary. If either
        // operand is NaN, Go falls through both comparisons and returns 1.
        !((candidate - current).abs() < 0.0001) && !(candidate < current)
    }

    pub(super) fn direct(&mut self, check_edge_length: bool) -> (bool, bool) {
        self.direct_with_projected_mirror_refits(check_edge_length)
            .0
    }

    pub(super) fn direct_with_projected_mirror_refits(
        &mut self,
        check_edge_length: bool,
    ) -> ((bool, bool), BTreeSet<i64>) {
        self.direct_with_reachable(check_edge_length, None)
    }

    /// Upstream `hasFixedDescendant` follows original container and aggregate
    /// member pointers, including children absent from the temporary Nodes
    /// slice. Traverse the retained pointer relationships by durable identity.
    fn direct_has_fixed_descendant(&self, root: NodeId, fixed_nodes: &BTreeSet<i64>) -> bool {
        let mut pending = vec![
            self.nodes[root.0 as usize].tala_id,
            self.active_mirror_tala_id(self.active_mirror_node(root)),
        ];
        let mut visited = BTreeSet::new();
        while let Some(tala_id) = pending.pop() {
            if !visited.insert(tala_id) {
                continue;
            }
            if fixed_nodes.contains(&tala_id) {
                return true;
            }
            for node in self.nodes.iter().filter(|node| node.tala_id == tala_id) {
                if node.is_container || node.scoring_is_container {
                    pending.extend(
                        self.containers
                            .get(&Some(node.input_id))
                            .into_iter()
                            .flatten()
                            .map(|child| self.nodes[child.0 as usize].tala_id),
                    );
                }
            }
            // These maps are the retained Graph.Containers and aggregate
            // member slices, rather than all external nodes in the arena.
            for children in [
                self.transaction_external_container_children.get(&tala_id),
                self.transaction_external_aggregate_children.get(&tala_id),
            ]
            .into_iter()
            .flatten()
            {
                pending.extend(children.iter().map(|child| child.tala_id));
            }
            for cluster in self
                .clusters
                .iter()
                .filter(|cluster| cluster.vessel_tala_id == tala_id)
            {
                pending.extend(
                    cluster
                        .members
                        .iter()
                        .map(|member| self.nodes[member.0 as usize].tala_id),
                );
            }
            for sequence in self
                .sequences
                .iter()
                .filter(|sequence| sequence.vessel_tala_id == tala_id)
            {
                pending.extend(
                    sequence
                        .members
                        .iter()
                        .map(|member| self.nodes[member.0 as usize].tala_id),
                );
            }
            for node in self
                .projected_transaction_nodes
                .iter()
                .filter(|node| node.node.tala_id == tala_id)
            {
                if node.node.is_container
                    || node.node.scoring_is_container
                    || node.node.scoring_is_aggregate_vessel
                {
                    pending.extend(node.child_tala_ids.iter().copied());
                }
            }
        }
        false
    }

    pub(super) fn direct_with_reachable(
        &mut self,
        check_edge_length: bool,
        supplied_reachable: Option<&BTreeSet<Option<NodeId>>>,
    ) -> ((bool, bool), BTreeSet<i64>) {
        #[cfg(feature = "diagnostic-traces")]
        self.trace_direct_tree_state(
            if check_edge_length {
                "enter-checked"
            } else {
                "enter-unchecked"
            },
            false,
            false,
            0.0,
        );
        // Recovered Graph.direct tests the first component node's concrete
        // Hierarchy pointer. Placement subgraphs preserve component order and
        // clone that membership, so no structural inference is needed.
        let current_nodes = self.graph_node_order();
        if current_nodes
            .first()
            .is_some_and(|node| self.nodes[node.0 as usize].hierarchy.is_some())
        {
            return ((false, false), BTreeSet::new());
        }
        // Most layouts have no fixed nodes. Index them once rather than
        // rescanning every retained node view for every candidate root.
        let fixed_nodes = self
            .retained_scoring_nodes()
            .filter(|node| node.fixed_top_left.is_some())
            .map(|node| node.tala_id)
            .collect::<BTreeSet<_>>();
        if !fixed_nodes.is_empty()
            && current_nodes
                .iter()
                .any(|node| self.direct_has_fixed_descendant(*node, &fixed_nodes))
        {
            return ((false, false), BTreeSet::new());
        }
        // Recovered Graph.direct returns before computing transforms when an
        // active sequence vessel/member owns any abducted external edge.
        // Stable-ID Rust retains members instead of swapping in a pointer-only
        // vessel, so the sequence state carries the same non-empty predicate.
        if self.node_order.iter().any(|node| {
            self.nodes[node.0 as usize]
                .sequence
                .is_some_and(|sequence| {
                    self.sequences
                        .get(sequence)
                        .is_some_and(|sequence| sequence.has_edge_abductions)
                })
        }) || self
            .sized_edge_abductions
            .iter()
            .any(|abduction| abduction.sequence_abduction)
        {
            return ((false, false), BTreeSet::new());
        }
        // direct(false) is the temporary placement graph; direct(true) is the
        // post-placement SwapStuff graph whose aggregate vessel is exposed by
        // the active carrier view.
        let (mirror_x, mirror_y) = self.direction_transforms_with_mode(None, check_edge_length);
        if !mirror_x && !mirror_y {
            return ((false, false), BTreeSet::new());
        }
        self.trace_direct_geometry("before-mirror");
        let current =
            check_edge_length.then(|| self.global_sized_edge_length_after_tree_restoration(true));
        // v0.9.0 GraphState includes Tree.Orientation as well as geometry
        // and cluster state (transaction_snapshot.go319-329). A rejected
        // mirror must restore the whole pre-transaction arena.
        #[cfg(feature = "diagnostic-traces")]
        if let Some(cost) = current {
            self.trace_direct_tree_state("before", mirror_x, mirror_y, cost);
        }
        let original = check_edge_length.then(|| self.clone());
        let prior_overlaps = check_edge_length.then(|| self.refinement_overlap_pairs());
        #[cfg(feature = "diagnostic-traces")]
        if !check_edge_length {
            self.trace_direct_tree_state("before-unchecked", mirror_x, mirror_y, 0.0);
        }
        let projected_refits =
            self.mirror_axes_with_reachable(mirror_x, mirror_y, supplied_reachable);
        #[cfg(feature = "diagnostic-traces")]
        if !check_edge_length {
            self.trace_direct_tree_state("after-unchecked", mirror_x, mirror_y, 0.0);
        }
        self.trace_direct_geometry("after-mirror");
        self.trace_target_shared_geometry("direct-after-mirror");
        if check_edge_length {
            // Graph.direct wraps mirrorAxes in an AffectContainers transaction:
            // refit ordinary containers first, then publish aggregate carriers
            // before measuring the candidate.  The flattened arena has already
            // mirrored the shared boxes above; preserve the same commit phase.
            self.reposition_ordinary_containers_without_sync();
            self.trace_direct_geometry("after-reposition");
            self.trace_target_shared_geometry("direct-after-reposition");
            let (overlaps, _) = prior_overlaps.as_ref().unwrap();
            // direct.go commits an AffectContainers transaction before scoring
            // the mirror. Fixed outside icons do not mirror with their nodes,
            // so refitting can introduce a new overlap even when edge cost falls.
            if !self.transaction_containment_is_valid()
                || !self.transaction_external_containers_are_valid()
                || self.transaction_containers_have_bad_state_overlap(overlaps)
            {
                *self = original.unwrap();
                return ((false, false), BTreeSet::new());
            }
            self.sync_clusters_with_descendants();
            self.sync_sequences();
            self.trace_direct_geometry("after-sync");
            self.trace_target_shared_geometry("direct-after-sync");
            let (overlaps, exact_overlaps) = prior_overlaps.as_ref().unwrap();
            if !self.transaction_containment_is_valid()
                || !self.transaction_fixed_origin_is_valid()
                || self.transaction_has_bad_state_overlap(overlaps)
                || self.existing_spacing_overlap_became_exact(overlaps, exact_overlaps)
            {
                *self = original.unwrap();
                return ((false, false), BTreeSet::new());
            }
        }
        let worse = current.is_some_and(|current| {
            let candidate = self.global_sized_edge_length_after_tree_restoration(true);
            if crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_ROOT") {
                eprintln!(
                    "DIRECT_RUST_SCORE current={} candidate={} worse={}",
                    current,
                    candidate,
                    Self::direct_score_is_worse(candidate, current)
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            self.trace_direct_tree_state("candidate", mirror_x, mirror_y, candidate);
            Self::direct_score_is_worse(candidate, current)
        });
        if worse {
            *self = original.expect("edge-length check records rollback state");
            #[cfg(feature = "diagnostic-traces")]
            self.trace_direct_tree_state("restored", mirror_x, mirror_y, current.unwrap());
            return ((false, false), BTreeSet::new());
        }
        ((mirror_x, mirror_y), projected_refits)
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_direct_tree_state(&self, phase: &str, mirror_x: bool, mirror_y: bool, cost: f64) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_DIRECT_TREE_STATE") {
            return;
        }
        let mut trees = BTreeMap::new();
        for (&node, tree) in &self.tree_routing_nodes {
            let orientation = match tree.orientation {
                Orientation::TopLeft => 0,
                Orientation::TopRight => 1,
                Orientation::BottomLeft => 2,
                Orientation::BottomRight => 3,
                Orientation::Top => 4,
                Orientation::Right => 5,
                Orientation::Bottom => 6,
                Orientation::Left => 7,
                Orientation::None => 8,
            };
            assert!(
                trees
                    .insert(self.nodes[node.0 as usize].tala_id.to_string(), orientation)
                    .is_none(),
                "duplicate diagnostic tree identity"
            );
        }
        eprintln!(
            "{}",
            serde_json::json!({"event":"direct-trees", "phase":phase,
            "mirror":[mirror_x,mirror_y], "cost":cost, "trees":trees})
        );
    }

    fn trace_direct_geometry(&self, phase: &str) {
        if !crate::internal::trace_env_enabled("TALA_TRACE_DIRECT_GEOMETRY") {
            return;
        }
        const IDS: [i64; 6] = [
            2600858170, 1308591071, 4248473833, 3802298614, 365607126, 2531950148,
        ];
        for node in &self.nodes {
            if IDS.contains(&node.tala_id) {
                eprintln!(
                    "DIRECT_GEOM_RUST phase={} id={} pos={:?} size={:?}",
                    phase, node.tala_id, node.position, node.rect.size
                );
            }
        }
    }

    /// Apply the hierarchy reach of TALA's pointer-sharing `Graph.mirrorAxes`.
    ///
    /// The recovered function builds `reachableContainers`, then recursively
    /// walks each component root's descendants in postorder.  The root is
    /// mirrored unconditionally; descendants are mirrored when their direct
    /// container is in the reachable set, and every visited container gets
    /// `positionContainerChildren(true)` afterward.
    pub(super) fn mirror_subtree_axes(
        &mut self,
        root: NodeId,
        mirror_x: bool,
        mirror_y: bool,
        reachable_containers: &BTreeSet<Option<NodeId>>,
    ) {
        if !mirror_x && !mirror_y {
            return;
        }
        let root = self.active_mirror_node(root);
        let mut reachable_nodes = BTreeSet::new();
        self.collect_active_mirror_reach(root, &mut reachable_nodes, &mut BTreeSet::new());
        let represented_active_tala_ids = reachable_nodes
            .iter()
            .map(|node| self.active_mirror_tala_id(*node))
            .collect::<BTreeSet<_>>();
        self.mirror_active_subtree_postorder(
            root,
            mirror_x,
            mirror_y,
            reachable_containers,
            &represented_active_tala_ids,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
        );
    }

    pub(super) fn mirror_tree_routing_orientation(
        &mut self,
        node: NodeId,
        mirror_x: bool,
        mirror_y: bool,
    ) {
        let Some(tree) = self.tree_routing_nodes.get_mut(&node) else {
            return;
        };
        if (mirror_x && tree.orientation.is_horizontal())
            || (mirror_y && tree.orientation.is_vertical())
        {
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("TALA_TRACE_TREE_ORIENT")
                && matches!(
                    self.nodes[node.0 as usize].tala_id,
                    1171112347 | 1154334728 | 1271778061 | 1187889966
                )
            {
                eprintln!(
                    "TREE_ORIENT_MIRROR_RUST tala={} from={:?} to={:?} mirror=({}, {})",
                    self.nodes[node.0 as usize].tala_id,
                    tree.orientation,
                    tree.orientation.opposite(),
                    mirror_x,
                    mirror_y
                );
            }
            tree.orientation = tree.orientation.opposite();
            let orientation = tree.orientation;
            self.publish_tree_orientation(node, orientation);
        }
    }

    fn active_mirror_node(&self, node: NodeId) -> ActiveMirrorNode {
        if let Some(cluster_index) = self.active_cluster_index(node)
            && self.clusters[cluster_index].members.first()
                == Some(&self.active_sequence_owner(node))
            && self.cluster_is_active(&self.clusters[cluster_index])
        {
            return ActiveMirrorNode::ClusterVessel(cluster_index);
        }
        if let Some(sequence_index) = self.active_sequence_index(node)
            && self.sequences[sequence_index].members.first() == Some(&node)
            && self.sequence_is_active(&self.sequences[sequence_index])
        {
            return ActiveMirrorNode::SequenceVessel(sequence_index);
        }
        ActiveMirrorNode::Stable(node)
    }

    fn active_mirror_children(&self, node: ActiveMirrorNode) -> Vec<ActiveMirrorNode> {
        match node {
            ActiveMirrorNode::Stable(stable) if self.nodes[stable.0 as usize].is_container => {
                let mut children = self.container_node_order(Some(stable));
                // An induced placement graph can retain the owning container
                // in Graph.Containers while its direct children live only in
                // the transaction projection.  Go walks those shared
                // pointers during mirrorAxes; include the corresponding
                // stable carriers here so each pointer is reflected once.
                let parent_tala_id = self.nodes[stable.0 as usize].tala_id;
                if let Some(projected_children) = self
                    .transaction_external_container_children
                    .get(&parent_tala_id)
                {
                    for projected_child in projected_children {
                        let child = self
                            .nodes
                            .iter()
                            .enumerate()
                            .find(|(_, node)| node.tala_id == projected_child.tala_id)
                            .map(|(index, _)| NodeId(index as u32))
                            .or_else(|| {
                                self.clusters
                                    .iter()
                                    .find(|cluster| {
                                        cluster.vessel_tala_id == projected_child.tala_id
                                    })
                                    .and_then(|cluster| cluster.members.first().copied())
                            });
                        if let Some(child) = child
                            && !children.contains(&child)
                        {
                            children.push(child);
                        }
                    }
                }
                children
                    .into_iter()
                    .map(|child| self.active_mirror_node(child))
                    .collect()
            }
            ActiveMirrorNode::ClusterVessel(cluster_index) => self.clusters[cluster_index]
                .members
                .iter()
                .copied()
                .map(|member| {
                    self.active_sequence_index(member)
                        .filter(|&sequence_index| {
                            self.sequence_is_active(&self.sequences[sequence_index])
                                && self.sequences[sequence_index].members.first() == Some(&member)
                        })
                        .map_or(ActiveMirrorNode::Stable(member), |sequence_index| {
                            ActiveMirrorNode::SequenceVessel(sequence_index)
                        })
                })
                .collect(),
            ActiveMirrorNode::SequenceVessel(sequence_index) => self.sequences[sequence_index]
                .members
                .iter()
                .copied()
                .map(ActiveMirrorNode::Stable)
                .collect(),
            ActiveMirrorNode::Stable(_) => Vec::new(),
        }
    }

    fn active_mirror_container(&self, node: ActiveMirrorNode) -> Option<NodeId> {
        match node {
            ActiveMirrorNode::Stable(stable) => self.active_node_container(stable),
            ActiveMirrorNode::ClusterVessel(cluster_index) => self.clusters[cluster_index]
                .members
                .first()
                .and_then(|owner| self.active_node_container(*owner)),
            ActiveMirrorNode::SequenceVessel(sequence_index) => {
                self.sequences[sequence_index].container
            }
        }
    }

    fn active_mirror_tala_id(&self, node: ActiveMirrorNode) -> i64 {
        match node {
            ActiveMirrorNode::Stable(stable) => self.nodes[stable.0 as usize].tala_id,
            ActiveMirrorNode::SequenceVessel(sequence_index) => {
                self.sequences[sequence_index].vessel_tala_id
            }
            ActiveMirrorNode::ClusterVessel(cluster_index) => {
                self.clusters[cluster_index].vessel_tala_id
            }
        }
    }

    fn collect_active_mirror_reach(
        &self,
        node: ActiveMirrorNode,
        visited: &mut BTreeSet<ActiveMirrorNode>,
        containers: &mut BTreeSet<Option<NodeId>>,
    ) {
        if !visited.insert(node) {
            return;
        }
        containers.insert(self.active_mirror_container(node));
        for child in self.active_mirror_children(node) {
            self.collect_active_mirror_reach(child, visited, containers);
        }
    }

    /// Build Go's `reachableContainers` set, which is narrower than the
    /// `WalkRDFS` tree used by the later mirror callbacks. `AllReachableNodes`
    /// follows active cluster and sequence members, and ordinary descendants
    /// that are still in the current graph's node slice. It does not discover
    /// a nested container's retained `Graph.Containers` aliases merely by
    /// walking that container. Those aliases still receive
    /// `PositionContainerChildren` during the mirror walk, but are not
    /// individually reflected unless their parent is in this set.
    fn collect_active_mirror_reachable_containers(
        &self,
        node: ActiveMirrorNode,
        graph_node_membership: &BTreeSet<NodeId>,
        visited: &mut BTreeSet<ActiveMirrorNode>,
        containers: &mut BTreeSet<Option<NodeId>>,
    ) {
        if !visited.insert(node) {
            return;
        }
        containers.insert(self.active_mirror_container(node));

        let children = match node {
            ActiveMirrorNode::Stable(stable) if self.nodes[stable.0 as usize].is_container => self
                .container_node_order(Some(stable))
                .into_iter()
                .filter(|child| graph_node_membership.contains(child))
                .map(|child| self.active_mirror_node(child))
                .collect(),
            ActiveMirrorNode::ClusterVessel(_) | ActiveMirrorNode::SequenceVessel(_) => {
                self.active_mirror_children(node)
            }
            ActiveMirrorNode::Stable(_) => Vec::new(),
        };
        for child in children {
            self.collect_active_mirror_reachable_containers(
                child,
                graph_node_membership,
                visited,
                containers,
            );
        }
    }

    fn publish_mirrored_active_positions(
        &mut self,
        mirrored: &BTreeSet<ActiveMirrorNode>,
        sequence_mirror_projections: &BTreeMap<NodeId, SequenceMirrorProjection>,
    ) {
        // A live sequence vessel and its first step are distinct Go pointers,
        // but the stable arena uses the first step's slot for the vessel. Keep
        // the vessel's final position so publishing the first-step pointer by
        // TALA ID cannot leave that active slot carrying the member box.
        let final_sequence_vessel_states = sequence_mirror_projections
            .keys()
            .filter_map(|owner| {
                Some((
                    *owner,
                    (
                        self.position(*owner)?,
                        self.nodes[owner.0 as usize].transaction_anchor_offset,
                    ),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let positions = mirrored
            .iter()
            .filter_map(|node| match *node {
                ActiveMirrorNode::Stable(stable) => {
                    let mut position = self.position(stable)?;
                    if let Some(projection) = sequence_mirror_projections.get(&stable) {
                        let final_vessel_position = final_sequence_vessel_states.get(&stable)?.0;
                        position.x = projection.first_member_position.x + final_vessel_position.x
                            - projection.vessel_position.x;
                        position.y = projection.first_member_position.y + final_vessel_position.y
                            - projection.vessel_position.y;
                    }
                    Some((
                        self.nodes[stable.0 as usize].tala_id,
                        position,
                        final_sequence_vessel_states.get(&stable).copied().map(
                            |(vessel_position, anchor_offset)| {
                                (stable, vessel_position, anchor_offset)
                            },
                        ),
                    ))
                }
                ActiveMirrorNode::SequenceVessel(sequence_index) => {
                    let owner = self.sequences[sequence_index].members.first().copied()?;
                    Some((
                        self.sequences[sequence_index].vessel_tala_id,
                        self.position(owner)?,
                        None,
                    ))
                }
                ActiveMirrorNode::ClusterVessel(cluster_index) => Some((
                    self.clusters[cluster_index].vessel_tala_id,
                    *self.pending_cluster_vessel_positions.get(&cluster_index)?,
                    None,
                )),
            })
            .collect::<Vec<_>>();
        for (tala_id, position, restore_active_sequence_vessel) in positions {
            self.set_projected_node_position(tala_id, position);
            if let Some((owner, vessel_position, anchor_offset)) = restore_active_sequence_vessel {
                self.set_position(owner, vessel_position);
                self.nodes[owner.0 as usize].transaction_anchor_offset = anchor_offset;
            }
        }
    }

    fn mirror_active_subtree_postorder(
        &mut self,
        node: ActiveMirrorNode,
        mirror_x: bool,
        mirror_y: bool,
        reachable_containers: &BTreeSet<Option<NodeId>>,
        represented_active_tala_ids: &BTreeSet<i64>,
        mirrored: &mut BTreeSet<ActiveMirrorNode>,
        projected_refits: &mut BTreeSet<i64>,
        sequence_mirror_projections: &mut BTreeMap<NodeId, SequenceMirrorProjection>,
        projected_mirrored: &mut BTreeSet<i64>,
    ) {
        // Synthetic vessels are separate Go pointers. Snapshot their box
        // before walking retained members because the stable arena aliases a
        // vessel onto its first member.
        let synthetic_box = match node {
            ActiveMirrorNode::ClusterVessel(cluster_index) => {
                let owner = self.clusters[cluster_index].members.first().copied();
                owner.and_then(|owner| {
                    let position = self
                        .pending_cluster_vessel_positions
                        .get(&cluster_index)
                        .copied()
                        .or_else(|| self.position(owner))?;
                    self.pending_cluster_vessel_positions
                        .entry(cluster_index)
                        .or_insert(position);
                    Some((position, self.cluster_vessel_size(cluster_index)))
                })
            }
            ActiveMirrorNode::SequenceVessel(sequence_index) => self.sequences[sequence_index]
                .members
                .first()
                .and_then(|owner| self.position(*owner))
                .map(|position| (position, self.sequence_vessel_size(sequence_index))),
            ActiveMirrorNode::Stable(_) => None,
        };

        for child in self.active_mirror_children(node) {
            self.mirror_active_subtree_postorder(
                child,
                mirror_x,
                mirror_y,
                reachable_containers,
                represented_active_tala_ids,
                mirrored,
                projected_refits,
                sequence_mirror_projections,
                projected_mirrored,
            );
        }

        // SplitSubgraphs retains ordinary descendant pointers in Graph.Containers
        // even when their carriers are absent from the induced Graph.Nodes slice.
        // They must participate in rdfsWalk's postorder
        // positionContainerChildren(true) callback, but are not themselves roots
        // of the induced graph and therefore must not be reflected here.
        self.position_projected_mirror_descendants_postorder(
            self.active_mirror_tala_id(node),
            represented_active_tala_ids,
            projected_refits,
            mirror_x,
            mirror_y,
            reachable_containers,
            projected_mirrored,
        );

        // `Graph.mirrorAxes` keeps a map keyed by the actual Node pointer and
        // tests only reachableContainers[n.getContainer()]. Nil is an ordinary
        // map key; there is no unconditional root exception.
        if mirrored.contains(&node) {
            return;
        }
        let logical_container = self.active_mirror_container(node);
        if reachable_containers.contains(&logical_container) {
            match node {
                ActiveMirrorNode::Stable(stable) => {
                    self.mirror_stable_node_box(stable, mirror_x, mirror_y);
                    // Source mirrorNode mutates the shared NodeToTree entry
                    // once, in the same reachable-container branch as its box.
                    self.mirror_tree_routing_orientation(stable, mirror_x, mirror_y);
                }
                ActiveMirrorNode::ClusterVessel(cluster_index) => {
                    if let Some((mut position, size)) = synthetic_box {
                        if mirror_x {
                            position.x = -position.x - size.width;
                        }
                        if mirror_y {
                            position.y = -position.y - size.height;
                        }
                        self.pending_cluster_vessel_positions
                            .insert(cluster_index, position);
                    }
                }
                ActiveMirrorNode::SequenceVessel(sequence_index) => {
                    if let Some((mut position, size)) = synthetic_box {
                        let owner = self.sequences[sequence_index].members.first().copied();
                        let first_member_position = owner.and_then(|owner| self.position(owner));
                        if mirror_x {
                            position.x = -position.x - size.width;
                        }
                        if mirror_y {
                            position.y = -position.y - size.height;
                        }
                        if let Some(owner) = owner {
                            if let Some(first_member_position) = first_member_position {
                                sequence_mirror_projections.insert(
                                    owner,
                                    SequenceMirrorProjection {
                                        first_member_position,
                                        vessel_position: position,
                                    },
                                );
                            }
                            self.set_position(owner, position);
                        }
                    }
                }
            }
            mirrored.insert(node);
        }
        if let ActiveMirrorNode::Stable(stable) = node {
            // `Graph.mirrorAxes` invokes `PositionContainerChildren(true)`
            // for every first-visited real node after its optional mirror,
            // including containers reached through an enclosing graph.  The
            // projected aggregate path below handles synthetic vessels; real
            // containers must use their owning child slice here so nested
            // layouts receive the same second-frame padding/refit pass.
            if self.nodes[stable.0 as usize].is_container && !self.active_node_is_aggregate(stable)
            {
                self.position_container_children(stable, true);
            }
            // A temporary placement graph keeps ordinary descendant pointers
            // in its external Containers projection. Those pointers are not
            // present in the induced Nodes slice, so the active fit above
            // cannot see them. Go's shared pointers make the same
            // PositionContainerChildren call observe both views; refit the
            // projected aliases in the same postorder callback.
            let stable_tala_id = self.nodes[stable.0 as usize].tala_id;
            if self
                .transaction_external_container_children
                .contains_key(&stable_tala_id)
                && self.position_projected_container_children(stable_tala_id, true)
            {
                projected_refits.insert(stable_tala_id);
            }
            self.mirror_projected_aggregate_children(stable_tala_id);
        }
    }

    fn position_projected_mirror_descendants_postorder(
        &mut self,
        parent_tala_id: i64,
        represented_active_tala_ids: &BTreeSet<i64>,
        refitted: &mut BTreeSet<i64>,
        mirror_x: bool,
        mirror_y: bool,
        reachable_containers: &BTreeSet<Option<NodeId>>,
        projected_mirrored: &mut BTreeSet<i64>,
    ) {
        let ordinary_children = self
            .transaction_external_container_children
            .get(&parent_tala_id)
            .cloned()
            .unwrap_or_default();
        for child in &ordinary_children {
            self.position_projected_mirror_node_postorder(
                child.tala_id,
                represented_active_tala_ids,
                refitted,
                mirror_x,
                mirror_y,
                reachable_containers,
                projected_mirrored,
            );
        }
        let aggregate_children = self
            .transaction_external_aggregate_children
            .get(&parent_tala_id)
            .cloned()
            .unwrap_or_default();
        for child in &aggregate_children {
            self.position_projected_mirror_node_postorder(
                child.tala_id,
                represented_active_tala_ids,
                refitted,
                mirror_x,
                mirror_y,
                reachable_containers,
                projected_mirrored,
            );
        }
    }

    /// Reflect one child that survives only as a transaction projection. Go's
    /// `Graph.Containers` keeps the same `*Node` in every owner slice, so the
    /// reverse DFS used by `mirrorAxes` still calls `Node.Mirror` on these
    /// descendants. Rust stores those aliases as value snapshots; update all
    /// snapshots by durable TALA identity before the projected fit pass.
    fn mirror_projected_node_box(&mut self, tala_id: i64, mirror_x: bool, mirror_y: bool) {
        if !mirror_x && !mirror_y {
            return;
        }
        let Some(source) = self.projected_node_by_tala(tala_id) else {
            return;
        };
        let Some(mut position) = source.position else {
            return;
        };
        let [top, right, bottom, left] = loop_cardinal_extents(source.loop_offsets.as_ref());
        if mirror_x {
            position.x = -position.x - source.rect.size.width + left - right;
        }
        if mirror_y {
            position.y = -position.y - source.rect.size.height + top - bottom;
        }
        let update = |node: &mut ArenaNode| {
            if node.tala_id == tala_id {
                node.position = Some(position);
                node.rect.origin = position;
            }
        };
        for node in &mut self.nodes {
            update(node);
        }
        for node in &mut self.transaction_external_containers {
            update(node);
        }
        for children in self.transaction_external_container_children.values_mut() {
            for node in children {
                update(node);
            }
        }
        for children in self.transaction_external_aggregate_children.values_mut() {
            for node in children {
                update(node);
            }
        }
    }

    fn position_projected_mirror_node_postorder(
        &mut self,
        tala_id: i64,
        represented_active_tala_ids: &BTreeSet<i64>,
        refitted: &mut BTreeSet<i64>,
        mirror_x: bool,
        mirror_y: bool,
        reachable_containers: &BTreeSet<Option<NodeId>>,
        projected_mirrored: &mut BTreeSet<i64>,
    ) {
        if represented_active_tala_ids.contains(&tala_id) {
            return;
        }
        // The Go rdfsWalk has no global visited set: a shared pointer can be
        // encountered through multiple roots, and each encounter still runs
        // PositionContainerChildren.  Keep only a path-local guard for the
        // acyclic hierarchy so a repeated alias is replayed faithfully.
        let mut path_guard = BTreeSet::new();
        self.position_projected_mirror_node_postorder_inner(
            tala_id,
            represented_active_tala_ids,
            refitted,
            mirror_x,
            mirror_y,
            reachable_containers,
            projected_mirrored,
            &mut path_guard,
        );
    }

    fn position_projected_mirror_node_postorder_inner(
        &mut self,
        tala_id: i64,
        represented_active_tala_ids: &BTreeSet<i64>,
        refitted: &mut BTreeSet<i64>,
        mirror_x: bool,
        mirror_y: bool,
        reachable_containers: &BTreeSet<Option<NodeId>>,
        projected_mirrored: &mut BTreeSet<i64>,
        path_guard: &mut BTreeSet<i64>,
    ) {
        if represented_active_tala_ids.contains(&tala_id) || !path_guard.insert(tala_id) {
            return;
        }
        let Some(projected) = self.projected_node_by_tala(tala_id) else {
            path_guard.remove(&tala_id);
            return;
        };
        let is_container = projected.is_container;
        let node_reachable =
            self.projected_mirror_container_reachable(&projected, reachable_containers);

        // `rdfsWalk` follows the same shared Containers pointers recursively,
        // even when a descendant is absent from the induced Graph.Nodes slice.
        // Walk the projected child maps first so their postorder mirrors and
        // container fits occur before this node's callback.
        let ordinary_children = self
            .transaction_external_container_children
            .get(&tala_id)
            .cloned()
            .unwrap_or_default();
        for child in &ordinary_children {
            self.position_projected_mirror_node_postorder_inner(
                child.tala_id,
                represented_active_tala_ids,
                refitted,
                mirror_x,
                mirror_y,
                reachable_containers,
                projected_mirrored,
                path_guard,
            );
        }
        let aggregate_children = self
            .transaction_external_aggregate_children
            .get(&tala_id)
            .cloned()
            .unwrap_or_default();
        for child in &aggregate_children {
            self.position_projected_mirror_node_postorder_inner(
                child.tala_id,
                represented_active_tala_ids,
                refitted,
                mirror_x,
                mirror_y,
                reachable_containers,
                projected_mirrored,
                path_guard,
            );
        }

        // The child's effective container is the parent whose callback is
        // walking this slice.  Carrying that reachability bit through the
        // projected recursion is equivalent to Go's pointer identity here:
        // descendants of a reachable container mirror, while descendants of
        // an unreachable container only receive PositionContainerChildren.
        if node_reachable && projected_mirrored.insert(tala_id) {
            self.mirror_projected_node_box(tala_id, mirror_x, mirror_y);
        }
        if is_container && self.position_projected_container_children(tala_id, true) {
            refitted.insert(tala_id);
        }
        path_guard.remove(&tala_id);
    }

    /// Reachability is keyed by the dense node IDs of the owning placement
    /// graph, while projected aliases retain their original owner IDs. Match
    /// both representations through the stable TALA identity so a projected
    /// child still follows the same mirror decision after a scope is compacted.
    fn projected_mirror_container_reachable(
        &self,
        projected: &ArenaNode,
        reachable_containers: &BTreeSet<Option<NodeId>>,
    ) -> bool {
        // `reachable_containers` is produced from the owning temporary graph,
        // while projected aliases retain the original arena `NodeId` in their
        // `container` field. A compact induced graph can reuse that numeric
        // index for an unrelated node, so translating the ID through
        // `self.nodes` by position creates false reachability. The direct
        // scope supplies the durable parent IDs explicitly; use the original
        // pointer ID only for graphs that have no such mapping (unit fixtures
        // and the ordinary non-projected path).
        let Some(parent_tala_id) = projected.scoring_container_parent else {
            return reachable_containers.contains(&None);
        };
        if let Some(reachable_tala_ids) = &self.reachable_container_tala_ids {
            return reachable_tala_ids.contains(&parent_tala_id);
        }
        // An external transaction alias can lose its direct `container`
        // pointer when it is projected into a compact placement graph. In
        // that case `None` is a storage detail, not Go's EffectiveContainer
        // result: resolve its durable parent ID against the reachable keys.
        reachable_containers.iter().flatten().any(|container| {
            self.nodes
                .get(container.0 as usize)
                .is_some_and(|node| node.tala_id == parent_tala_id)
        })
    }

    fn mirror_stable_node_box(&mut self, node: NodeId, mirror_x: bool, mirror_y: bool) {
        let index = node.0 as usize;
        if let Some(mut position) = self.nodes[index].position {
            let size = self.nodes[index].rect.size;
            let [top, right, bottom, left] = self.loop_spacing_extents(node).unwrap_or([0.0; 4]);
            if mirror_x {
                position.x = -position.x - size.width + left - right;
            }
            if mirror_y {
                position.y = -position.y - size.height + top - bottom;
            }
            self.nodes[index].position = Some(position);
            self.nodes[index].rect.origin = position;
        }
    }

    /// Replay the projected container-fit callback for aggregate vessels.
    /// Their shared carrier and member snapshots were already reflected by
    /// the preceding projected DFS; applying a second transform here would
    /// undo that mirror for value-based aliases.
    fn mirror_projected_aggregate_children(&mut self, parent_tala_id: i64) {
        // With the reflected vessel carrier in place, Go's
        // positionContainerChildren(true) contributes the container-shape
        // padding to each aggregate member. The projected map is the Rust
        // equivalent of those shared pointers and avoids applying the local
        // frame twice through the stable member arena.
        self.position_projected_container_children(parent_tala_id, true);
    }

    /// Recovered `Node.positionContainerChildren`.
    ///
    /// `Graph.syncNested` invokes this after `InitializeNodes`, once the
    /// container has its new position but its already-positioned descendants
    /// still occupy the prior coordinate frame.
    pub(super) fn position_container_children(&mut self, node: NodeId, with_padding: bool) {
        if !self.nodes[node.0 as usize].is_container {
            return;
        }
        let children = self.container_node_order(Some(node));
        if children.is_empty() {
            return;
        }
        let Some((mut top_left, mut bottom_right)) = self.transaction_container_bounds(&children)
        else {
            return;
        };
        // node.go positionContainerChildren expands the fixed bounds for
        // centered child labels before computing InsidePlacement. Synthetic
        // vessels have no label; the retained carrier's label is its member's.
        Self::expand_for_child_labels(
            &mut top_left,
            &mut bottom_right,
            children
                .iter()
                .filter(|child| !self.active_node_is_aggregate(**child))
                .map(|child| &self.nodes[child.0 as usize]),
        );
        let content = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        let padding = if with_padding {
            self.shape_fit_padding(node)
        } else {
            Insets::uniform(0.0)
        };
        // GetInsidePlacement is absolute in TALA: the embedded shape's inner
        // box carries the container's current top-left.  positionContainer-
        // Children subtracts the absolute child bounds from that point before
        // translating the retained descendants.
        let inside = if self.position(node).is_some() {
            self.shape_inside_placement_absolute(node, content, padding)
                .expect("position checked above")
        } else {
            // Graph.syncNested also runs for an unpositioned container while
            // recursive placement is being copied back. In that local frame
            // GetInsidePlacement is measured from (0,0), so use the relative
            // shape helper instead of waiting for the parent carrier.
            self.shape_inside_placement(node, content, padding)
        };
        let delta = Point {
            x: inside.x - top_left.x,
            y: inside.y - top_left.y,
        };
        for child in children {
            self.translate_active_node_with_children(child, delta);
        }
    }

    /// Translate only a container's already-laid-out descendants. During
    /// recursive placement TALA can keep a carrier nil while its children
    /// remain in that carrier's local frame; publishing the carrier later
    /// applies this pending frame delta without moving the carrier again.
    pub(super) fn translate_descendants_only(&mut self, node: NodeId, delta: Point) {
        if delta.x == 0.0 && delta.y == 0.0 {
            return;
        }
        for descendant in self.descendants(node) {
            if let Some(mut position) = self.position(descendant) {
                position.x += delta.x;
                position.y += delta.y;
                self.nodes[descendant.0 as usize].position = Some(position);
                self.nodes[descendant.0 as usize].rect.origin = position;
            }
        }
    }

    /// Publish a mirror chosen while placing a container's temporary scope.
    /// The temporary Go graph shares descendant pointers with its parent, so
    /// CombineSubgraphs carries the mirrored child coordinates back inside
    /// the published carrier box. Rust scopes clone those descendants; apply
    /// the same frame reflection after the carrier has been positioned.
    pub(super) fn mirror_descendants_in_frame(
        &mut self,
        node: NodeId,
        mirror_x: bool,
        mirror_y: bool,
        origin: Point,
    ) {
        let size = self.nodes[node.0 as usize].rect.size;
        for descendant in self.descendants(node) {
            let index = descendant.0 as usize;
            let Some(mut position) = self.nodes[index].position else {
                continue;
            };
            let [top, right, bottom, left] =
                self.loop_spacing_extents(descendant).unwrap_or([0.0; 4]);
            if mirror_x {
                position.x = origin.x + size.width
                    - (position.x - origin.x)
                    - self.nodes[index].rect.size.width
                    + left
                    - right;
            }
            if mirror_y {
                position.y = origin.y + size.height
                    - (position.y - origin.y)
                    - self.nodes[index].rect.size.height
                    + top
                    - bottom;
            }
            self.nodes[index].position = Some(position);
            self.nodes[index].rect.origin = position;
            // This is a container-relative box transform. Shared tree
            // orientation changes belong to mirrorAxes' visited-node callback.
        }
    }

    /// `Nodes.SwapOptimize` is a main-graph operation.  Its recovered Go
    /// calls `Node.edgeLength(..., nil, ...)` and `Node.getSymmetry(..., nil,
    /// ...)`; the temporary sized-placement abduction projections belong to
    /// the preceding optimizer and must not leak into this stage.  Rust keeps
    /// those projections in side maps, so temporarily expose the unprojected
    /// graph while evaluating a swap.
    fn with_unprojected_swap_state<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let abductions = std::mem::take(&mut self.sized_edge_abductions);
        let abductions_present = std::mem::replace(&mut self.sized_edge_abductions_present, false);
        let adjacent_overrides = std::mem::take(&mut self.sized_adjacent_overrides);
        let projected_obstructions = std::mem::take(&mut self.sized_projected_obstructions);
        let projected_obstruction_sets = std::mem::take(&mut self.sized_projected_obstruction_sets);
        let symmetry_neighbors = std::mem::take(&mut self.sized_collapsed_symmetry_neighbors);
        let cluster_distance_boxes = std::mem::take(&mut self.sized_cluster_distance_boxes);
        let suppress_aggregate_projection = self.suppress_aggregate_projection;
        // SwapOptimize passes nil edge-abductions, but Node.edgeLength still
        // gathers the active cluster's own edge abductions and scores each
        // retained member box. Only the sized-stage projection caches are
        // cleared here; suppressing aggregate projection would score the
        // temporary vessel rectangle instead of the recovered member.
        self.suppress_aggregate_projection = false;
        let result = f(self);
        self.sized_edge_abductions = abductions;
        self.sized_edge_abductions_present = abductions_present;
        self.sized_adjacent_overrides = adjacent_overrides;
        self.sized_projected_obstructions = projected_obstructions;
        self.sized_projected_obstruction_sets = projected_obstruction_sets;
        self.sized_collapsed_symmetry_neighbors = symmetry_neighbors;
        self.sized_cluster_distance_boxes = cluster_distance_boxes;
        self.suppress_aggregate_projection = suppress_aggregate_projection;
        result
    }

    pub(super) fn swap_local_edge_length(&mut self, node: NodeId) -> f64 {
        self.with_unprojected_swap_state(|graph| {
            graph.sized_edge_length(node, true) + graph.column_to_column_crossing_cost(node, false)
                - graph.sized_symmetry(node, true)
                    * graph.cell_size
                    * graph.active_edge_count(node) as f64
        })
    }

    fn global_swap_edge_length(&mut self) -> f64 {
        let previous = self.swap_scoring_vessel_boxes;
        // The Go transpose pass scores a temporary vessel through the same
        // member-backed NodeEdgeLength path as the local measurement. The
        // stable Rust arena keeps the vessel carrier and member projection in
        // one node, so forcing vessel-box scoring here would select a
        // different route/obstruction view even when the rectangles match.
        self.swap_scoring_vessel_boxes = false;
        let trace = crate::internal::trace_env_enabled("TALA_TRACE_EDGE_LENGTH_TERMS");
        if trace {
            eprintln!("SWAP_GLOBAL_RUST begin");
        }
        // Source Swap runs after Place has restored tree children into the
        // owning Graph.Containers slices. Score that live child surface, as
        // later AlignAxes does: both calls share the graph-owned cache and
        // the same EdgeLengthOptions, so a peeled-child score must not be
        // published under the live graph's key.
        let score = self.global_sized_edge_length_after_tree_restoration(true);
        if trace {
            eprintln!("SWAP_GLOBAL_RUST end value={score}");
        }
        self.swap_scoring_vessel_boxes = previous;
        score
    }

    /// Evaluates one `Nodes.SwapOptimize` transaction. TALA creates the
    /// transaction with `AffectContainers`, so swapping two descendants also
    /// refits their ordinary containers before overlap validation and scoring.
    /// Evaluating only the two raw boxes can accept a swap that the Go
    /// transaction rejects, or score it against a stale container envelope.
    pub(super) fn evaluate_swap_trial(
        &mut self,
        node: NodeId,
        candidate: NodeId,
        smart: bool,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        existing_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> Option<(f64, f64, usize)> {
        self.evaluate_swap_trial_with_score_filter(
            node,
            candidate,
            smart,
            existing_overlaps,
            existing_exact_overlaps,
            None,
            |_| true,
        )
    }

    fn evaluate_swap_trial_with_score_filter(
        &mut self,
        node: NodeId,
        candidate: NodeId,
        smart: bool,
        existing_overlaps: &BTreeSet<(NodeId, NodeId)>,
        existing_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        baseline: Option<&SwapTrialBaseline>,
        needs_global_score: impl FnOnce(f64) -> bool,
    ) -> Option<(f64, f64, usize)> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(SwapTrial);
        // A Go Transaction rollback restores the complete graph state, not
        // just the two swapped boxes. Container wrapping and aggregate sync
        // mutate retained aliases and derived projection maps during a
        // trial, so snapshot those mutable geometry carriers before applying
        // either swap form. Topology, edges, aggregate membership, routing
        // metadata, and scoring tables are read-only for this transaction.
        let captured = baseline.is_none().then(|| SwapTrialBaseline::capture(self));
        let baseline =
            baseline.unwrap_or_else(|| captured.as_ref().expect("individual trial state"));
        if smart {
            self.smart_swap_positions(node, candidate);
        } else {
            self.swap_positions(node, candidate);
        }
        self.reposition_swap_transaction_containers();
        if crate::internal::trace_env_value("TALA_TRACE_SWAP_GEOMETRY").is_some_and(|target| {
            target == "all"
                || target == self.nodes[node.0 as usize].tala_id.to_string()
                || target == self.nodes[candidate.0 as usize].tala_id.to_string()
        }) {
            eprintln!(
                "SWAP_GEOMETRY_RUST node={} cand={} smart={} positions={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[candidate.0 as usize].tala_id,
                smart,
                self.graph_node_order()
                    .into_iter()
                    .filter_map(|id| {
                        let position = self.active_node_position(id)?;
                        Some((
                            self.nodes[id.0 as usize].tala_id,
                            position,
                            self.active_node_size(id),
                        ))
                    })
                    .collect::<Vec<_>>()
            );
        }
        let mut moved = BTreeSet::new();
        let mut fixed_moved = false;
        for (moved_node, original_position, original_size, raw_position, raw_size) in
            baseline.geometry.iter().copied()
        {
            let current_position = self.active_node_position(moved_node);
            let current_size = self.active_node_size(moved_node);
            let raw_geometry_changed = self.nodes[moved_node.0 as usize].position != raw_position
                || self.nodes[moved_node.0 as usize].rect.size != raw_size;
            if current_position != original_position
                || current_size != original_size
                || raw_geometry_changed
            {
                moved.insert(moved_node);
            }
            fixed_moved |= self.nodes[moved_node.0 as usize].fixed_top_left.is_some()
                && self.nodes[moved_node.0 as usize].position != raw_position;
        }
        let bad_state_overlap =
            self.transaction_has_bad_state_overlap_for_nodes(existing_overlaps, &moved);
        let spacing_became_exact =
            self.existing_spacing_overlap_became_exact(existing_overlaps, existing_exact_overlaps);
        let containment_valid = self.transaction_containment_is_valid();
        // The swap preserves child-box minima, but label-aware refitting can
        // move an ancestor behind its container's fixed origin. Go validates
        // that post-refit frame even when the fixed node itself did not move.
        let fixed_origin_valid = self.transaction_fixed_origin_is_valid();
        // Source Transaction.Commit has no graph-size admission guard.
        // Completed SwapStuff checks the real graph bounds after all trials,
        // so an otherwise valid oversized swap must still be scored here.
        let valid = !bad_state_overlap
            && !spacing_became_exact
            && !fixed_moved
            && containment_valid
            && fixed_origin_valid;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("TALA_TRACE_SWAP_DETAIL").is_some_and(|target| {
            target == "all"
                || target == self.nodes[node.0 as usize].tala_id.to_string()
                || target == self.nodes[candidate.0 as usize].tala_id.to_string()
        }) {
            let within_max_size = self.is_within_max_size();
            eprintln!(
                "SWAP_CAND_RUST node={} node_id={} cand={} candidate_id={} smart={} valid={} max={} bad={} exact={} fixed={} contain={} origin={} moved={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[node.0 as usize].external_id,
                self.nodes[candidate.0 as usize].tala_id,
                self.nodes[candidate.0 as usize].external_id,
                smart,
                valid,
                within_max_size,
                bad_state_overlap,
                spacing_became_exact,
                fixed_moved,
                containment_valid,
                fixed_origin_valid,
                moved
                    .iter()
                    .map(|id| self.nodes[id.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
            );
        }
        let result = valid.then(|| {
            let local = self.swap_local_edge_length(node);
            if needs_global_score(local) {
                (
                    local,
                    self.global_swap_edge_length(),
                    self.global_edge_crossings(),
                )
            } else {
                // Only the local score participates in the form selection;
                // these unused terms cannot reach a candidate acceptance.
                (local, 0.0, 0)
            }
        });
        if crate::internal::trace_env_value("TALA_TRACE_SWAP_SCORE").is_some_and(|target| {
            target == "all"
                || target == self.nodes[node.0 as usize].tala_id.to_string()
                || target == self.nodes[candidate.0 as usize].tala_id.to_string()
        }) {
            let node_debug_id = &self.nodes[node.0 as usize].external_id;
            let candidate_debug_id = &self.nodes[candidate.0 as usize].external_id;
            match result {
                Some((local, global, crossings)) => eprintln!(
                    "SWAP_SCORE_RUST node={} node_id={} cand={} candidate_id={} smart={} valid=true local_bits=0x{:016x} global_bits=0x{:016x} crossings={}",
                    self.nodes[node.0 as usize].tala_id,
                    node_debug_id,
                    self.nodes[candidate.0 as usize].tala_id,
                    candidate_debug_id,
                    smart,
                    local.to_bits(),
                    global.to_bits(),
                    crossings,
                ),
                None => eprintln!(
                    "SWAP_SCORE_RUST node={} node_id={} cand={} candidate_id={} smart={} valid=false",
                    self.nodes[node.0 as usize].tala_id,
                    node_debug_id,
                    self.nodes[candidate.0 as usize].tala_id,
                    candidate_debug_id,
                    smart,
                ),
            }
        }
        match captured {
            Some(captured) => captured.state,
            None => baseline.state.clone(),
        }
        .restore(self);
        result
    }

    pub(super) fn hidden_swap_aggregate_member(&self, node: NodeId) -> bool {
        self.active_cluster_index(node).is_some_and(|index| {
            self.clusters[index].members.first() != Some(&node)
                && self.topology_vessel_node(crate::internal::TopologyVessel::Cluster(index))
                    != Some(node)
        }) || self.active_sequence_index(node).is_some_and(|index| {
            self.sequences[index].members.first() != Some(&node)
                && self.topology_vessel_node(crate::internal::TopologyVessel::Sequence(index))
                    != Some(node)
        })
    }

    pub(super) fn swap_optimize(&mut self) -> bool {
        if crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_SWAP_OPTIMIZE") {
            return false;
        }
        // swap.go delegates every strict comparison to geo.PrecisionCompare;
        // the recovered release uses geo.PRECISION (0.0001), not the tighter
        // optimizer-local epsilon.  Keep the same boundary so near-tied
        // trials take the same branch as Go.
        const PRECISION: f64 = 0.0001;
        let better = |candidate: f64, current: f64| candidate < current - PRECISION;
        // SwapOptimize is invoked as `Nodes(g.Nodes).SwapOptimize` in TALA;
        // preserve the concrete Graph.Nodes order rather than the reconstructed
        // active container order used by hierarchy traversals.
        let nodes = self.graph_node_order();
        let node_to_tree = routing::routing_tree_nodes(self);
        // Nodes.SwapOptimize creates one Transaction before entering the node
        // loop. Transaction.UpdateState refreshes its geometry snapshot after
        // an accepted swap, but deliberately retains the overlap maps captured
        // by NewTransactionWithOptions for every later trial in this pass.
        let existing_overlaps = self.existing_overlap_pairs();
        let existing_exact_overlaps = self.exact_overlap_pairs();
        if crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_EXISTING") {
            eprintln!(
                "TRANSPOSE_EXISTING_RUST count={} pairs={:?}",
                existing_overlaps.len(),
                existing_overlaps
                    .iter()
                    .filter_map(|(left, right)| {
                        let left_id = self.nodes[left.0 as usize].tala_id;
                        let right_id = self.nodes[right.0 as usize].tala_id;
                        ((left_id == 560439961 && right_id == 510107104)
                            || (left_id == 510107104 && right_id == 560439961))
                            .then_some((left_id, right_id))
                    })
                    .collect::<Vec<_>>()
            );
        }
        let mut swap_made = false;
        for node in nodes.iter().copied() {
            if crate::internal::trace_env_value("TALA_TRACE_SWAP_PRE").is_some_and(|target| {
                target == "all" || target == self.nodes[node.0 as usize].tala_id.to_string()
            }) {
                eprintln!(
                    "SWAP_PRE_RUST node={} all={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.graph_node_order()
                        .into_iter()
                        .filter_map(|id| Some((
                            self.nodes[id.0 as usize].tala_id,
                            self.active_node_position(id)?,
                            self.active_node_size(id)
                        )))
                        .collect::<Vec<_>>()
                );
            }
            if node_to_tree.contains(&node)
                || self.nodes[node.0 as usize].hierarchy.is_some()
                || self.nodes[node.0 as usize].fixed_top_left.is_some()
                // Go's SwapOptimize iterates visible nodes and temporary
                // aggregate vessels. Members retained in the stable arena
                // remain hidden, while the first member is the carrier that
                // represents that vessel for scoring and swaps.
                || self.hidden_swap_aggregate_member(node)
                || (self.active_edge_count(node) == 0 && !self.has_leaky_edge(node))
            {
                continue;
            }
            let current_global = self.global_swap_edge_length();
            let current_local = self.swap_local_edge_length(node);
            let current_crossings = self.global_edge_crossings();
            if crate::internal::trace_env_value("TALA_TRACE_SWAP_SCORE").is_some_and(|target| {
                target == "all" || target == self.nodes[node.0 as usize].tala_id.to_string()
            }) {
                eprintln!(
                    "SWAP_CURRENT_RUST node={} node_id={} local_bits=0x{:016x} global_bits=0x{:016x} crossings={}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[node.0 as usize].external_id,
                    current_local.to_bits(),
                    current_global.to_bits(),
                    current_crossings,
                );
            }
            let mut best: Option<(NodeId, bool)> = None;
            let mut best_global = f64::INFINITY;

            // Nodes.SwapOptimize gets candidates from the owning container's
            // live child slice.  The sized optimizer has a separate
            // getBestSwapCandidate path that shuffles Graph.Nodes; this
            // SwapStuff pass is the direct SwapOptimize implementation.
            let candidates = self.container_node_order(self.nodes[node.0 as usize].container);
            let mut baseline = None;
            for candidate in candidates {
                if candidate == node
                    || node_to_tree.contains(&candidate)
                    || self.nodes[candidate.0 as usize].hierarchy.is_some()
                    || self.nodes[candidate.0 as usize].fixed_top_left.is_some()
                    || self.hidden_swap_aggregate_member(candidate)
                    || self.nodes[candidate.0 as usize].container
                        != self.nodes[node.0 as usize].container
                {
                    continue;
                }
                let mut regular_global_pruned = false;
                let mut regular = self.evaluate_swap_trial_with_score_filter(
                    node,
                    candidate,
                    false,
                    &existing_overlaps,
                    &existing_exact_overlaps,
                    baseline.as_ref(),
                    |local| {
                        regular_global_pruned =
                            !regular_swap_requires_global_score(local, current_local);
                        !regular_global_pruned
                    },
                );
                if regular_global_pruned {
                    // This form cannot be accepted. A nonzero placeholder
                    // preserves smart's local-first pruning; if smart can
                    // win with a zero global score, resolve the real regular
                    // zero sentinel below before the source comparison tree.
                    regular.as_mut().expect("scored valid trial").1 = f64::INFINITY;
                }
                let regular_score = regular.unwrap_or((0.0, 0.0, 0));
                // The first fresh rollback also normalizes inline point fields
                // to their canonical owner bindings. Capture that state before
                // sharing it with smart and subsequent regular trials.
                let baseline = baseline.get_or_insert_with(|| SwapTrialBaseline::capture(self));
                let smart = self.evaluate_swap_trial_with_score_filter(
                    node,
                    candidate,
                    true,
                    &existing_overlaps,
                    &existing_exact_overlaps,
                    Some(baseline),
                    |local| smart_swap_requires_global_score(regular_score, local, current_local),
                );

                let smart = smart.unwrap_or((0.0, 0.0, 0));
                if regular_global_pruned
                    && regular_swap_requires_zero_check(regular_score.0, smart, current_local)
                {
                    regular = self.evaluate_swap_trial(
                        node,
                        candidate,
                        false,
                        &existing_overlaps,
                        &existing_exact_overlaps,
                    );
                }
                let regular = regular.unwrap_or((0.0, 0.0, 0));
                if regular.1 == 0.0 && smart.1 == 0.0 {
                    continue;
                }

                // Preserve the recovered comparison tree: when both swap
                // forms have a local score, local edge length chooses the
                // form first. Global score only ranks that chosen form among
                // candidates. This deliberate ordering differs from a flat
                // "best tuple" minimization.
                let chosen = if regular.0 != 0.0 && smart.0 != 0.0 {
                    if better(smart.0, regular.0) {
                        Some((true, smart, true))
                    } else {
                        Some((false, regular, true))
                    }
                } else if regular.0 != 0.0 {
                    Some((false, regular, true))
                } else if smart.0 != 0.0 {
                    Some((true, smart, true))
                } else if regular.1 != 0.0 && smart.1 != 0.0 {
                    if better(regular.1, smart.1) {
                        Some((false, regular, false))
                    } else {
                        Some((true, smart, false))
                    }
                } else if regular.1 != 0.0 {
                    Some((false, regular, false))
                } else {
                    Some((true, smart, false))
                };
                let Some((is_smart, (local, global, crossings), compare_local)) = chosen else {
                    continue;
                };
                if crossings <= current_crossings
                    && (!compare_local || better(local, current_local))
                    && better(global, current_global)
                    && better(global, best_global)
                {
                    best = Some((candidate, is_smart));
                    best_global = global;
                }
            }

            if let Some((candidate, smart)) = best {
                if crate::internal::trace_env_enabled("TALA_TRACE_SWAP") {
                    eprintln!(
                        "SWAP_ACCEPT_RUST node={} node_id={} candidate={} candidate_id={} smart={} global_bits=0x{:016x} local_bits=0x{:016x}",
                        self.nodes[node.0 as usize].tala_id,
                        self.nodes[node.0 as usize].external_id,
                        self.nodes[candidate.0 as usize].tala_id,
                        self.nodes[candidate.0 as usize].external_id,
                        smart,
                        best_global.to_bits(),
                        current_local.to_bits(),
                    );
                }
                if smart {
                    self.smart_swap_positions(node, candidate);
                } else {
                    self.swap_positions(node, candidate);
                }
                self.reposition_swap_transaction_containers();
                // Accepted SwapOptimize transactions mutate shared Go node
                // pointers; refresh flattened projected offsets before the
                // next node's global score observes that committed state.
                self.reconcile_projected_offsets_after_rollback();
                swap_made = true;
            }
        }
        swap_made
    }

    /// Apply the `AffectContainers` transaction boundary used by Go's
    /// `Nodes.SwapOptimize`. Cluster synchronization moves each live member
    /// through its container descendants, so the root swap pass must publish
    /// those descendants before scoring or committing the trial.
    fn reposition_swap_transaction_containers(&mut self) {
        self.reposition_ordinary_containers_without_sync();
        self.sync_clusters_with_descendants();
        self.sync_sequences();
    }

    pub(super) fn reachable_without(
        &self,
        start: NodeId,
        excluded: &BTreeSet<NodeId>,
        include_containers: bool,
    ) -> Vec<NodeId> {
        let start = self.active_aggregate_owner(start);
        let excluded = excluded
            .iter()
            .copied()
            .map(|node| self.active_aggregate_owner(node))
            .collect::<BTreeSet<_>>();
        if excluded.contains(&start) {
            return Vec::new();
        }
        let mut reachable = Vec::new();
        let mut seen = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(node) = queue.pop_front() {
            if !self.tree_routing_nodes.contains_key(&node) {
                reachable.push(node);
            }
            let mut enqueue = |candidate: NodeId| {
                let candidate = self.active_aggregate_owner(candidate);
                if !excluded.contains(&candidate) && seen.insert(candidate) {
                    queue.push_back(candidate);
                }
            };
            for edge in self.active_edge_ids(node) {
                enqueue(self.active_adjacent(node, edge));
            }
            // Node.Equidistance calls getAllReachableNodes with includeNears
            // and traverseTrees both true. Extracted tree nodes remain
            // traversable but are omitted from the returned slice.
            if let Some(tree) = self.tree_routing_nodes.get(&node) {
                enqueue(tree.parent);
                for child in self.tree_children_in_extraction_order(node) {
                    enqueue(child);
                }
            } else {
                for root in self.tree_children_in_extraction_order(node) {
                    enqueue(root);
                }
            }

            let mut nears = self.nodes[node.0 as usize].nears.keys();
            if let Some(cluster_index) = self.active_cluster_index(node)
                && self.clusters[cluster_index].members.first() == Some(&node)
            {
                for member in self.clusters[cluster_index].members.iter().copied() {
                    let mut member_nears = self.nodes[member.0 as usize].nears.keys();
                    member_nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
                    nears.extend(member_nears);
                }
            } else {
                nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
            }
            let start_container = self.active_node_container(start);
            for near in nears {
                let near = self.active_aggregate_owner(near);
                if self.active_node_container(near) == start_container {
                    enqueue(near);
                }
            }

            // The recovered includeContainers branch walks Graph.Nodes and
            // applies the pointer-based descendant predicate. Aggregate
            // members are not appended merely because a vessel owns them;
            // they enter only through the vessel's actual edge/near/tree
            // slices above.
            let _ = include_containers;
        }
        reachable
    }

    /// Translation of the recovered `Node.getAllReachableNodes` modes used by
    /// Graph.transpose: ordinary edges are always traversed, tree ownership is
    /// traversed without returning extracted tree nodes, and container
    /// ancestry is optional.
    fn transpose_reachable(
        &self,
        start: NodeId,
        include_containers: bool,
        ignore: &BTreeSet<NodeId>,
    ) -> Vec<NodeId> {
        let start = self.active_aggregate_owner(start);
        let ignore = ignore
            .iter()
            .copied()
            .map(|node| self.active_aggregate_owner(node))
            .collect::<BTreeSet<_>>();
        let mut reachable = Vec::new();
        let mut visited = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        let enqueue =
            |candidate: NodeId, visited: &mut BTreeSet<NodeId>, queue: &mut VecDeque<NodeId>| {
                let candidate = self.active_aggregate_owner(candidate);
                if !ignore.contains(&candidate) && visited.insert(candidate) {
                    queue.push_back(candidate);
                }
            };

        while let Some(current) = queue.pop_front() {
            // Recovered getReachableNodes traverses tree carriers but omits
            // them from the returned slice when traverseTrees is enabled.
            // The carrier remains in the queue so its parent/children are
            // explored, but Graph.transpose moves only the returned nodes.
            if !self.tree_routing_nodes.contains_key(&current) {
                reachable.push(current);
            }
            for edge in self.active_edge_ids(current) {
                enqueue(
                    self.active_adjacent(current, edge),
                    &mut visited,
                    &mut queue,
                );
            }

            if let Some(tree) = self.tree_routing_nodes.get(&current) {
                enqueue(tree.parent, &mut visited, &mut queue);
                for child in self.tree_children_in_extraction_order(current) {
                    enqueue(child, &mut visited, &mut queue);
                }
            } else {
                for root in self.tree_children_in_extraction_order(current) {
                    enqueue(root, &mut visited, &mut queue);
                }
            }

            if include_containers {
                for other in self.nodes.iter().map(|node| node.input_id) {
                    if self.active_is_descendant_of(current, other)
                        || self.active_is_descendant_of(other, current)
                    {
                        enqueue(other, &mut visited, &mut queue);
                    }
                }
            }
        }
        reachable
    }

    pub(super) fn rotate_around(
        &mut self,
        node: NodeId,
        center: NodeId,
        quarter_turns: usize,
        round_to_cell: bool,
    ) {
        for _ in 0..quarter_turns {
            let position = self.active_node_position(node).unwrap();
            let size = self.active_node_size(node);
            let center_position = self.active_node_position(center).unwrap();
            let center_size = self.active_node_size(center);
            let translated_x =
                position.x + size.width * 0.5 - center_position.x - center_size.width * 0.5;
            let translated_y =
                position.y + size.height * 0.5 - center_position.y - center_size.height * 0.5;
            let mut target = Point {
                x: (center_position.x - translated_y + center_size.width * 0.5 - size.width * 0.5)
                    .round(),
                y: (center_position.y + translated_x + center_size.height * 0.5
                    - size.height * 0.5)
                    .round(),
            };
            if round_to_cell {
                target.x = (target.x / self.cell_size).round() * self.cell_size;
                target.y = (target.y / self.cell_size).round() * self.cell_size;
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_ROTATE") {
                let node_tala = self.nodes[node.0 as usize].tala_id;
                let center_tala = self.nodes[center.0 as usize].tala_id;
                eprintln!(
                    "ROTATE_RUST node={} center={} pos={:?} size={:?} center_pos={:?} center_size={:?} target={:?} round={}",
                    node_tala,
                    center_tala,
                    position,
                    size,
                    center_position,
                    center_size,
                    target,
                    round_to_cell
                );
            }
            self.move_active_node_abs_with_children(node, target);
        }
    }

    /// Translation of recovered `Graph.transpose`.
    pub(super) fn transpose_node(&mut self, node: NodeId, local_edge_score: bool) -> bool {
        self.transpose_node_with_mode(node, local_edge_score, false, None)
            .expect("unbound transpose scoring cannot fail")
    }

    pub(super) fn transpose_node_with_owner_routes(
        &mut self,
        node: NodeId,
        local_edge_score: bool,
        owner_session: Option<&mut OwnerScoreSession>,
    ) -> Result<bool, OptimizationError> {
        self.transpose_node_with_mode(node, local_edge_score, false, owner_session)
    }

    fn transpose_edge_length(
        &self,
        node: NodeId,
        owner_session: &mut Option<&mut OwnerScoreSession>,
    ) -> Result<f64, OptimizationError> {
        if let Some(session) = owner_session.as_deref_mut() {
            // Transaction.repositionContainers refits the parent around its
            // children without translating them. Publish the actual boxes,
            // including retained descendants, before reading owner medians
            // or routes; a root delta alone would move those children again.
            session
                .publish_packed_scope_geometry(self)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            let canonical = session
                .canonical_node(node)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            session
                .with_frame(|frame| {
                    self.sized_edge_length_with_owner_routes(
                        node,
                        true,
                        None,
                        frame.owner(),
                        canonical,
                        Some(frame),
                    )
                })
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?
                .map_err(|error| OptimizationError::Invalid(error.to_string()))
        } else {
            Ok(self.sized_edge_length(node, true))
        }
    }

    fn transpose_cluster_only_flip(
        &mut self,
        moved: NodeId,
        owner_session: &mut Option<&mut OwnerScoreSession>,
    ) -> Result<(), OptimizationError> {
        if crate::internal::trace_env_enabled("TALA_TRACE_DISABLE_CLUSTER_FLIP") {
            return Ok(());
        }
        if let Some(index) = self
            .active_cluster_index(moved)
            .filter(|index| self.clusters[*index].members.first() == Some(&moved))
        {
            self.optimize_cluster_only_flip(index);
        } else if self.projected_node_is_cluster_vessel(self.nodes[moved.0 as usize].tala_id)
            && let Some(session) = owner_session.as_deref_mut()
        {
            let flip = |graph: &mut ArenaGraph,
                        session: &mut OwnerScoreSession|
             -> Result<(), OwnerScoreSessionError> {
                session.publish_packed_scope_geometry(graph)?;
                if let Some(projection) = session.transpose_cluster_projection(moved)? {
                    let desired = graph.projected_cluster_desired_arrangement(
                        moved,
                        projection.arrangement,
                        projection.desired_arrangement,
                    );
                    // optimizeCluster updates DesiredArrangement even when
                    // both geometry transactions reject the requested flip.
                    session.set_transpose_cluster_desired(moved, desired)?;
                    graph.optimize_projected_cluster_flip(moved, &projection, Some(session))?;
                }
                Ok(())
            };
            flip(self, session).map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        }
        Ok(())
    }

    fn transpose_node_with_mode(
        &mut self,
        node: NodeId,
        local_edge_score: bool,
        tree_children_restored: bool,
        mut owner_session: Option<&mut OwnerScoreSession>,
    ) -> Result<bool, OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(TransposeTrial);
        let node = self.active_aggregate_owner(node);
        let trace_transpose_candidate =
            crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_CANDIDATES")
                && (!crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS")
                    || crate::internal::trace_env_enabled("TALA_TRACE_SIZED_PASS_ACTIVE"))
                && crate::internal::trace_env_value("TALA_TRACE_TRANSPOSE_NODE").is_some_and(
                    |target| {
                        target == "all"
                            || target.parse::<i64>().ok()
                                == Some(self.nodes[node.0 as usize].tala_id)
                    },
                );
        let node_ref = &self.nodes[node.0 as usize];
        if crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_START") {
            eprintln!(
                "TRANSPOSE_START_RUST node={} pos={:?} edges={}",
                node_ref.tala_id,
                self.active_node_position(node),
                self.active_edge_ids(node).len()
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_CONTEXT")
            && node_ref.tala_id == 22
            && self.active_node_position(node) == Some(Point { x: 312.0, y: 0.0 })
        {
            eprintln!(
                "TRANSPOSE_CONTEXT_RUST nodes={:?} containers={:?} external={:?} external_children={:?} validation_external={:?} projected={:?} source_positions={:?}",
                self.nodes
                    .iter()
                    .map(|candidate| (
                        candidate.tala_id,
                        candidate.position,
                        candidate.rect.size,
                        candidate.is_container,
                        candidate.scoring_container_parent,
                        &candidate.scoring_container_ancestors,
                    ))
                    .collect::<Vec<_>>(),
                self.containers
                    .iter()
                    .map(|(parent, children)| (
                        parent.map(|parent| self.nodes[parent.0 as usize].tala_id),
                        children
                            .iter()
                            .map(|child| self.nodes[child.0 as usize].tala_id)
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>(),
                self.transaction_external_containers
                    .iter()
                    .map(|external| (external.tala_id, external.position, external.rect.size))
                    .collect::<Vec<_>>(),
                self.transaction_external_container_children
                    .iter()
                    .map(|(parent, children)| (
                        *parent,
                        children
                            .iter()
                            .map(|child| (child.tala_id, child.position, child.rect.size))
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>(),
                self.transaction_validation_external_containers
                    .iter()
                    .map(|external| (external.tala_id, external.position, external.rect.size))
                    .collect::<Vec<_>>(),
                self.projected_transaction_nodes
                    .iter()
                    .filter(|projected| [1, 21].contains(&projected.node.tala_id))
                    .map(|projected| (
                        projected.node.tala_id,
                        projected.node.position,
                        projected.node.rect.size,
                        projected.node.is_container,
                        projected.container_tala_id,
                        &projected.ancestor_tala_ids,
                        &projected.child_tala_ids,
                    ))
                    .collect::<Vec<_>>(),
                [1_i64, 21_i64].map(|id| (id, self.source_positions_by_tala.get(&id).copied())),
            );
        }
        let node_to_tree = routing::routing_tree_nodes(self);
        let node_edges = self.active_edge_ids(node);
        // Recovered Graph.transpose rejects a node that participates in any
        // active edge abduction before it considers the one-/two-edge cases.
        // This check is intentionally separate from the later projected-edge
        // scoring: Go tests both the original and current endpoints and
        // returns false without opening a transaction.
        if local_edge_score
            && self.sized_edge_abductions.iter().any(|abduction| {
                abduction
                    .originally_from
                    .as_ref()
                    .is_some_and(|projected| projected.owner == Some(node))
                    || abduction
                        .originally_to
                        .as_ref()
                        .is_some_and(|projected| projected.owner == Some(node))
                    || abduction
                        .current_from
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
                    || abduction
                        .current_to
                        .and_then(|endpoint| endpoint.local_node())
                        == Some(node)
            })
        {
            return Ok(false);
        }
        if node_ref.hierarchy.is_some()
            || node_to_tree.contains(&node)
            || self.is_tree_sentinel(node)
            || node_ref.fixed_top_left.is_some()
            || !matches!(node_edges.len(), 1 | 2)
        {
            return Ok(false);
        }
        let adjacent: Vec<_> = node_edges
            .iter()
            .map(|edge| self.active_adjacent(node, *edge))
            .collect();
        let (transpose_nodes, center) = if adjacent.len() == 1 {
            if self.active_is_descendant_of(adjacent[0], node)
                || self.active_is_descendant_of(node, adjacent[0])
                || self.sized_orientation(node, adjacent[0]).is_diagonal()
            {
                return Ok(false);
            }
            let ancestor = self.nearest_shared_container(node, adjacent[0]);
            let transpose_nodes = if local_edge_score {
                vec![node]
            } else {
                let mut current = node;
                while self.active_node_container(current) != ancestor {
                    let Some(container) = self.active_node_container(current) else {
                        return Ok(false);
                    };
                    current = container;
                }
                let reachable =
                    self.transpose_reachable(current, false, &BTreeSet::from([adjacent[0]]));
                if reachable
                    .iter()
                    .any(|candidate| self.active_is_descendant_of(adjacent[0], *candidate))
                {
                    return Ok(false);
                }
                reachable
            };
            (transpose_nodes, adjacent[0])
        } else {
            if self.active_is_descendant_of(adjacent[0], node)
                || self.active_is_descendant_of(node, adjacent[0])
                || self.active_is_descendant_of(adjacent[1], node)
                || self.active_is_descendant_of(node, adjacent[1])
            {
                return Ok(false);
            }
            let ancestor_a = self.nearest_shared_container(node, adjacent[0]);
            let ancestor_b = self.nearest_shared_container(node, adjacent[1]);
            let mut membership_ignore = BTreeSet::from([node]);
            membership_ignore.extend(ancestor_a);
            if self
                .transpose_reachable(adjacent[0], true, &membership_ignore)
                .contains(&adjacent[1])
            {
                return Ok(false);
            }
            // Both branches must be orthogonal before a transpose is
            // admissible. TALA checks each adjacent orientation before it
            // computes reachability or opens the candidate transaction.
            if self.sized_orientation(node, adjacent[0]).is_diagonal()
                || self.sized_orientation(node, adjacent[1]).is_diagonal()
            {
                return Ok(false);
            }
            let mut branch_a_ignore = BTreeSet::from([node]);
            branch_a_ignore.extend(ancestor_a);
            let branch_a = self.transpose_reachable(adjacent[0], false, &branch_a_ignore);
            let mut branch_b_ignore = BTreeSet::from([node]);
            branch_b_ignore.extend(ancestor_b);
            let branch_b = self.transpose_reachable(adjacent[1], false, &branch_b_ignore);
            if branch_a.len() >= branch_b.len() {
                let moved = if local_edge_score {
                    let mut moved = branch_b;
                    moved.push(node);
                    moved
                } else {
                    let mut current = node;
                    while self.active_node_container(current) != ancestor_b {
                        let Some(container) = self.active_node_container(current) else {
                            return Ok(false);
                        };
                        current = container;
                    }
                    let reachable =
                        self.transpose_reachable(current, false, &BTreeSet::from([adjacent[0]]));
                    if reachable
                        .iter()
                        .any(|candidate| self.active_is_descendant_of(adjacent[0], *candidate))
                    {
                        return Ok(false);
                    }
                    reachable
                };
                (moved, adjacent[0])
            } else {
                let moved = if local_edge_score {
                    let mut moved = branch_a;
                    moved.push(node);
                    moved
                } else {
                    let mut current = node;
                    while self.active_node_container(current) != ancestor_a {
                        let Some(container) = self.active_node_container(current) else {
                            return Ok(false);
                        };
                        current = container;
                    }
                    let reachable =
                        self.transpose_reachable(current, false, &BTreeSet::from([adjacent[1]]));
                    if reachable
                        .iter()
                        .any(|candidate| self.active_is_descendant_of(adjacent[1], *candidate))
                    {
                        return Ok(false);
                    }
                    reachable
                };
                (moved, adjacent[1])
            }
        };
        if transpose_nodes.is_empty()
            || transpose_nodes
                .iter()
                .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
        {
            return Ok(false);
        }
        // Original transpose.calcLength scores the live Graph and each adjacent
        // node with the same EdgeAbductions as primary placement. Carry the
        // owner route frame through every baseline and rotation score.
        let transpose_length = |graph: &ArenaGraph,
                                owner_session: &mut Option<&mut OwnerScoreSession>|
         -> Result<f64, OptimizationError> {
            if local_edge_score {
                let mut sum = graph.transpose_edge_length(node, owner_session)?;
                for adjacent in &adjacent {
                    sum += graph.transpose_edge_length(*adjacent, owner_session)?;
                }
                Ok(sum)
            } else if tree_children_restored {
                Ok(graph.global_sized_edge_length_after_tree_restoration(true))
            } else {
                Ok(graph.global_sized_edge_length())
            }
        };
        let original: Vec<_> = self
            .nodes
            .iter()
            .map(|node| (node.position, node.rect.size))
            .collect();
        let original_owner_points = (0..self.nodes.len())
            .filter_map(|index| {
                let node = NodeId(index as u32);
                self.owner_node_point(node)
                    .ok()
                    .map(|point| (node, point.snapshot()))
            })
            .collect::<Vec<_>>();
        let original_cluster_vessels = self.pending_cluster_vessel_positions.clone();
        // transpose.go runs optimizeCluster(onlyFlip) inside each trial.
        // transaction.go:299-309/1401-1405 restores the shared Cluster policy
        // as well as its vessel's geometry before the next rotation.
        let original_cluster_policies = self
            .clusters
            .iter()
            .enumerate()
            .map(|(index, cluster)| {
                let owner = self
                    .owner_group_binding(TopologyVessel::Cluster(index))
                    .map(|group| {
                        (
                            group,
                            self.owner_cluster_policy(group)
                                .expect("transpose cluster policy"),
                        )
                    });
                (
                    cluster.arrangement,
                    cluster.desired_arrangement,
                    cluster.padding,
                    owner,
                )
            })
            .collect::<Vec<_>>();
        let original_cluster_sizes = self.cluster_vessel_sizes.clone();
        let original_external_cluster_layouts = self.transaction_external_cluster_layouts.clone();
        let original_projected_arrangements = self
            .nodes
            .iter()
            .map(|node| node.scoring_cluster_arrangement)
            .collect::<Vec<_>>();
        let original_owner_cluster_policies = owner_session
            .as_deref()
            .map(|session| session.transpose_cluster_policies(&transpose_nodes))
            .transpose()
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?
            .unwrap_or_default();
        // Transaction.Rollback restores the geometry of the shared external
        // node pointers, not whole nodes, so journal only those fields.
        let original_external_containers =
            super::trial_geometry::trial_external_geometry(&self.transaction_external_containers);
        let original_external_container_children =
            super::trial_geometry::trial_external_map_geometry(
                &self.transaction_external_container_children,
            );
        let original_external_aggregate_children =
            super::trial_geometry::trial_external_map_geometry(
                &self.transaction_external_aggregate_children,
            );
        // The container refit republishes refitted local and external
        // containers, whole node included, into the child entries that alias
        // them, so only those aliases need a full copy for rollback.
        let aliased_tala_ids = self
            .nodes
            .iter()
            .chain(&self.transaction_external_containers)
            .map(|node| node.tala_id)
            .collect::<crate::internal::IdHashSet<_>>();
        let aliased_children = |children: &BTreeMap<i64, Vec<ArenaNode>>| {
            children
                .iter()
                .flat_map(|(&key, children)| {
                    children
                        .iter()
                        .enumerate()
                        .filter(|(_, child)| aliased_tala_ids.contains(&child.tala_id))
                        .map(move |(index, child)| (key, index, child.clone()))
                })
                .collect::<Vec<_>>()
        };
        let mut original_aliased_external_children =
            aliased_children(&self.transaction_external_container_children);
        let mut original_aliased_aggregate_children =
            aliased_children(&self.transaction_external_aggregate_children);
        let mut original_projected_adjacent_overrides = self.sized_adjacent_overrides.clone();
        let mut original_projected_edge_abductions = self.sized_edge_abductions.clone();
        // Copy the two largest projection maps into this thread's retained
        // rollback buffers: consecutive transposes see the same keys, so the
        // copy reuses the previous call's tree and set allocations.
        // A buffer whose generation equals the live map's already holds the
        // same contents (generations name contents), so it needs no copy.
        let (mut original_projected_obstructions, mut original_projected_obstruction_sets) =
            TRANSPOSE_ROLLBACK_MAPS.take();
        if original_projected_obstructions.generation()
            != self.sized_projected_obstructions.generation()
        {
            restore_map_values(
                &mut original_projected_obstructions,
                &self.sized_projected_obstructions,
            );
            original_projected_obstructions
                .restore_unchanged_generation(self.sized_projected_obstructions.generation());
        }
        if original_projected_obstruction_sets.generation()
            != self.sized_projected_obstruction_sets.generation()
        {
            restore_map_values(
                &mut original_projected_obstruction_sets,
                &self.sized_projected_obstruction_sets,
            );
            original_projected_obstruction_sets
                .restore_unchanged_generation(self.sized_projected_obstruction_sets.generation());
        }
        let mut original_projected_symmetry_neighbors =
            self.sized_collapsed_symmetry_neighbors.clone();
        let mut original_projected_cluster_boxes = self.sized_cluster_distance_boxes.clone();
        let original_projection_generations = self.projection_generations();
        let overlap_geometry = self.refinement_validation_geometry();
        let (existing_overlaps, existing_exact_overlaps) = self.refinement_overlap_pairs();
        // transaction.go Commit uses the directional IsBadState pass. With
        // distinct, non-cluster nodes it includes every unordered pair tested
        // by transaction_has_new_overlap, with identical operands. Retain the
        // extra adapter check where IsBadState skips cluster members or where
        // repeated node identities could make the unordered scan test itself.
        let symmetric_overlap_required = trace_transpose_candidate
            || crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP_PAIR")
            || {
                // A membership-only set, sized for the whole order up front.
                let order = self.graph_node_order();
                let mut overlap_nodes = crate::internal::IdHashSet::with_capacity_and_hasher(
                    order.len(),
                    Default::default(),
                );
                order.into_iter().any(|id| {
                    self.nodes[id.0 as usize].cluster.is_some() || !overlap_nodes.insert(id)
                })
            };
        let mut best_length = transpose_length(self, &mut owner_session)?;
        let mut best_rotations = None;
        if trace_transpose_candidate {
            eprintln!(
                "TRANSPOSE_RUST begin node={} center={} moved_count={} length={:.17}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[center.0 as usize].tala_id,
                transpose_nodes.len(),
                best_length
            );
        }
        for rotations in 1..=3 {
            let mut trial_error = None;
            for moved in transpose_nodes.iter().copied() {
                self.rotate_around(moved, center, rotations, local_edge_score);
                if let Err(error) = self.transpose_cluster_only_flip(moved, &mut owner_session) {
                    trial_error = Some(error);
                    break;
                }
            }
            if trace_transpose_candidate {
                let positions = self
                    .node_order
                    .iter()
                    .filter_map(|current| {
                        let position = self.position(*current)?;
                        Some(format!(
                            "{}@{:.17},{:.17}",
                            self.nodes[current.0 as usize].tala_id, position.x, position.y
                        ))
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "TRANSPOSE_CANDIDATE_STATE_RUST node={} center={} rotations={} positions={:?}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[center.0 as usize].tala_id,
                    rotations,
                    positions
                );
            }
            // Transaction.Commit validates positioned Containers immediately
            // after repositionContainers, before publishing aggregate state.
            // The old call to reposition_ordinary_containers() synchronized
            // clusters/sequences too early and could make a rejected
            // transpose appear to have a different vessel/member geometry.
            let refit_valid = self.reposition_ordinary_containers_without_sync_sequential();
            let external_containers_valid =
                refit_valid && self.transaction_external_containers_are_valid();
            if external_containers_valid {
                // Transaction.Commit calls Cluster.SyncGeometry after the
                // refit. A container member can resize its live vessel even
                // when the vessel's position did not change.
                self.sync_clusters_after_container_refit();
                self.sync_sequences();
            }
            // Transaction.Commit also runs the per-node IsBadState overlap
            // predicate.  Transpose candidates must use that same guard;
            // checking only the symmetric "new pair" scan can admit a
            // candidate that the Go transaction rejects.
            let (has_new_overlap, bad_state_overlap) = self.refinement_validation_overlaps(
                &existing_overlaps,
                symmetric_overlap_required,
                overlap_geometry.as_deref(),
            );
            let spacing_became_exact = self.existing_spacing_overlap_became_exact(
                &existing_overlaps,
                &existing_exact_overlaps,
            );
            let containment_valid = self.transaction_containment_is_valid();
            // Graph.IsBadState compares fixed nodes against the transaction
            // snapshot, which already includes container padding/translation.
            // FixedTopLeft is relative authored input, not that saved point.
            // Cluster members return before this predicate in Go.
            let fixed_positions_valid = self.graph_node_order().into_iter().all(|id| {
                let candidate = &self.nodes[id.0 as usize];
                candidate.cluster.is_some()
                    || candidate.fixed_top_left.is_none()
                    || candidate.position.is_some_and(|position| {
                        position == original[id.0 as usize].0.unwrap_or_default()
                    })
            });
            let fixed_origin_valid = self.transaction_fixed_origin_is_valid();
            let valid = !has_new_overlap
                && !bad_state_overlap
                && !spacing_became_exact
                && containment_valid
                && fixed_positions_valid
                && fixed_origin_valid
                && external_containers_valid
                && trial_error.is_none();
            if valid {
                match transpose_length(self, &mut owner_session) {
                    Ok(length) => {
                        if trace_transpose_candidate {
                            eprintln!(
                                "TRANSPOSE_CANDIDATE_RUST node={} center={} rotations={} valid={} length={:.17} best={:.17}",
                                self.nodes[node.0 as usize].tala_id,
                                self.nodes[center.0 as usize].tala_id,
                                rotations,
                                valid,
                                length,
                                best_length
                            );
                        }
                        // Exact geo.PrecisionCompare(length, bestLength,
                        // geo.PRECISION) < 0 semantics: a difference exactly equal
                        // to PRECISION is not considered equal.
                        if (length - best_length).abs() >= 0.0001 && length < best_length {
                            best_length = length;
                            best_rotations = Some(rotations);
                        }
                    }
                    Err(error) => trial_error = Some(error),
                }
            }
            if !valid && trace_transpose_candidate {
                eprintln!(
                    "TRANSPOSE_CANDIDATE_RUST node={} center={} rotations={} valid=false overlap={} exact={} containment={} fixed={} origin={} external={}",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[center.0 as usize].tala_id,
                    rotations,
                    has_new_overlap,
                    spacing_became_exact,
                    containment_valid,
                    fixed_positions_valid,
                    fixed_origin_valid,
                    external_containers_valid
                );
            }
            if local_edge_score {
                // `Transaction.Rollback` restores active graph nodes in
                // graph order with `moveNodeAbsWithChildren` when
                // MoveChildren is set. Restoring a shifted container moves
                // its hidden cluster vessel; the later Cluster.Nodes pass
                // restores concrete members independently.
                for current in self.node_order.clone() {
                    if self.active_node_is_aggregate(current) {
                        continue;
                    }
                    let (target, size) = original[current.0 as usize];
                    if let (Some(target), Some(position)) = (target, self.position(current))
                        && target != position
                    {
                        self.move_node_abs_with_children(current, target);
                    }
                    self.nodes[current.0 as usize].rect.size = size;
                }
                for (arena_node, (position, size)) in
                    self.nodes.iter_mut().zip(original.iter().copied())
                {
                    arena_node.position = position;
                    arena_node.rect.origin = position.unwrap_or_default();
                    arena_node.rect.size = size;
                }
                // TALA's Transaction.Rollback walks the complete graph node
                // slice, which includes temporary cluster vessels.  The
                // stable-ID arena has those vessels in a side map, so restore
                // that snapshot explicitly as well.  Keeping the trial-mutated
                // map here leaks a rejected rotation into the next candidate
                // and changes the later Cluster.sync vessel anchor.
                self.pending_cluster_vessel_positions = original_cluster_vessels.clone();
            } else {
                for (arena_node, (position, size)) in
                    self.nodes.iter_mut().zip(original.iter().copied())
                {
                    arena_node.position = position;
                    arena_node.rect.origin = position.unwrap_or_default();
                    arena_node.rect.size = size;
                }
                self.pending_cluster_vessel_positions
                    .clone_from(&original_cluster_vessels);
            }
            // Transaction.Rollback restores bound TopLeft pointers as well
            // as inline node snapshots before the next rotation is measured.
            for (node, point) in &original_owner_points {
                self.restore_owner_node_point(*node, point)
                    .expect("transpose point rollback");
            }
            for (index, (arrangement, desired, padding, owner)) in
                original_cluster_policies.iter().enumerate()
            {
                let cluster = &mut self.clusters[index];
                cluster.arrangement = *arrangement;
                cluster.desired_arrangement = *desired;
                cluster.padding = *padding;
                if let Some((group, policy)) = owner {
                    self.set_owner_cluster_orientation_policy(
                        *group,
                        policy.arrangement.clone(),
                        policy.desired_arrangement.clone(),
                        policy.padding,
                    )
                    .expect("transpose cluster policy rollback");
                }
            }
            self.cluster_vessel_sizes
                .clone_from(&original_cluster_sizes);
            self.transaction_external_cluster_layouts
                .clone_from(&original_external_cluster_layouts);
            for (node, arrangement) in self.nodes.iter_mut().zip(&original_projected_arrangements) {
                node.scoring_cluster_arrangement = *arrangement;
            }
            super::trial_geometry::restore_trial_external_map_geometry(
                &mut self.transaction_external_aggregate_children,
                &original_external_aggregate_children,
            );
            super::trial_geometry::restore_trial_external_geometry(
                &mut self.transaction_external_containers,
                &original_external_containers,
            );
            super::trial_geometry::restore_trial_external_map_geometry(
                &mut self.transaction_external_container_children,
                &original_external_container_children,
            );
            for (children, aliased) in [
                (
                    &mut self.transaction_external_container_children,
                    &mut original_aliased_external_children,
                ),
                (
                    &mut self.transaction_external_aggregate_children,
                    &mut original_aliased_aggregate_children,
                ),
            ] {
                // The last rotation's rollback is the final use of these
                // private copies, so move them into place.
                if rotations == 3 {
                    for (key, index, child) in std::mem::take(aliased) {
                        if let Some(live) = children
                            .get_mut(&key)
                            .and_then(|children| children.get_mut(index))
                        {
                            *live = child;
                        }
                    }
                } else {
                    for (key, index, child) in aliased.iter() {
                        if let Some(live) = children
                            .get_mut(key)
                            .and_then(|children| children.get_mut(*index))
                        {
                            live.clone_from(child);
                        }
                    }
                }
            }
            // The last rotation's rollback is the final use of the originals.
            if rotations == 3 {
                self.sized_adjacent_overrides =
                    std::mem::take(&mut original_projected_adjacent_overrides);
                self.sized_edge_abductions =
                    std::mem::take(&mut original_projected_edge_abductions);
                // Swap rather than move: the rejected trial maps become the
                // next call's rollback buffers. A map the trials left at its
                // original generation already holds the original contents.
                if self.sized_projected_obstructions.generation()
                    != original_projected_obstructions.generation()
                {
                    std::mem::swap(
                        &mut self.sized_projected_obstructions,
                        &mut original_projected_obstructions,
                    );
                }
                if self.sized_projected_obstruction_sets.generation()
                    != original_projected_obstruction_sets.generation()
                {
                    std::mem::swap(
                        &mut self.sized_projected_obstruction_sets,
                        &mut original_projected_obstruction_sets,
                    );
                }
                TRANSPOSE_ROLLBACK_MAPS.set((
                    std::mem::take(&mut original_projected_obstructions),
                    std::mem::take(&mut original_projected_obstruction_sets),
                ));
                self.sized_collapsed_symmetry_neighbors =
                    std::mem::take(&mut original_projected_symmetry_neighbors);
                self.sized_cluster_distance_boxes =
                    std::mem::take(&mut original_projected_cluster_boxes);
            } else {
                // Generations name contents: a cache still at its original
                // generation needs no copy back.
                if self.sized_adjacent_overrides.generation()
                    != original_projected_adjacent_overrides.generation()
                {
                    restore_map_values(
                        &mut self.sized_adjacent_overrides,
                        &original_projected_adjacent_overrides,
                    );
                }
                if self.sized_edge_abductions.generation()
                    != original_projected_edge_abductions.generation()
                {
                    self.sized_edge_abductions
                        .clone_from(&original_projected_edge_abductions);
                }
                if self.sized_projected_obstructions.generation()
                    != original_projected_obstructions.generation()
                {
                    restore_map_values(
                        &mut self.sized_projected_obstructions,
                        &original_projected_obstructions,
                    );
                }
                if self.sized_projected_obstruction_sets.generation()
                    != original_projected_obstruction_sets.generation()
                {
                    restore_map_values(
                        &mut self.sized_projected_obstruction_sets,
                        &original_projected_obstruction_sets,
                    );
                }
                if self.sized_collapsed_symmetry_neighbors.generation()
                    != original_projected_symmetry_neighbors.generation()
                {
                    restore_map_values(
                        &mut self.sized_collapsed_symmetry_neighbors,
                        &original_projected_symmetry_neighbors,
                    );
                }
                if self.sized_cluster_distance_boxes.generation()
                    != original_projected_cluster_boxes.generation()
                {
                    restore_map_values(
                        &mut self.sized_cluster_distance_boxes,
                        &original_projected_cluster_boxes,
                    );
                }
            }
            // All six projection caches now hold their pre-trial contents
            // again, so they also regain the generations those contents had.
            self.restore_unchanged_projection_generations(original_projection_generations);
            self.reconcile_projected_offsets_after_rollback();
            if let Some(session) = owner_session.as_deref_mut() {
                session
                    .restore_transpose_cluster_policies(&original_owner_cluster_policies)
                    .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
                session
                    .publish_packed_scope_geometry(self)
                    .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
            }
            if let Some(error) = trial_error {
                return Err(error);
            }
        }
        let Some(rotations) = best_rotations else {
            if trace_transpose_candidate {
                eprintln!(
                    "TRANSPOSE_RUST node={} selected=0",
                    self.nodes[node.0 as usize].tala_id
                );
            }
            return Ok(false);
        };
        for moved in transpose_nodes {
            self.rotate_around(moved, center, rotations, local_edge_score);
            self.transpose_cluster_only_flip(moved, &mut owner_session)?;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_RESULT") {
            eprintln!(
                "TRANSPOSE_RUST before-refit node={} center={} rotations={} moved={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[center.0 as usize].tala_id,
                rotations,
                self.nodes
                    .iter()
                    .filter(|candidate| candidate.position.is_some())
                    .map(|candidate| (candidate.tala_id, candidate.position, candidate.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        self.reposition_ordinary_containers_without_sync();
        self.sync_clusters_after_container_refit();
        self.sync_sequences();
        if let Some(session) = owner_session {
            session
                .publish_packed_scope_geometry(self)
                .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        }
        if trace_transpose_candidate {
            eprintln!(
                "TRANSPOSE_RUST node={} selected={}",
                self.nodes[node.0 as usize].tala_id, rotations
            );
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_TRANSPOSE_RESULT") {
            eprintln!(
                "TRANSPOSE_RUST after-refit node={} center={} rotations={} members={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[center.0 as usize].tala_id,
                rotations,
                self.nodes
                    .iter()
                    .filter(|candidate| candidate.position.is_some())
                    .map(|candidate| (candidate.tala_id, candidate.position, candidate.rect.size))
                    .collect::<Vec<_>>()
            );
        }
        Ok(true)
    }

    pub(super) fn transpose_all(&mut self) {
        // placement.TransposeAll recomputes after Swap's direct pass.
        self.compute_cell_size();
        let _transpose_buffers = TransposeRollbackBuffers;
        let nodes = self.graph_node_order();
        for node in nodes {
            // Pipeline.TransposeStage runs on the master graph after
            // PlaceTrees has restored its retained children into the shared
            // Graph.Containers map. Graph.transpose's global score reads that
            // live map. Scope-local and sized-optimizer calls above still use
            // their pre-restoration projection.
            self.transpose_node_with_mode(node, false, true, None)
                .expect("unbound transpose scoring cannot fail");
        }
    }
}

type TransposeRollbackMaps = (
    Tracked<BTreeMap<(NodeId, EdgeId), SharedProjections>>,
    Tracked<BTreeMap<(NodeId, EdgeId), OrderedObstructionInventory>>,
);

thread_local! {
    /// Rollback copies of the projected obstruction maps, retained between
    /// transposes so each snapshot reuses the previous one's buffers. Before
    /// use they are refreshed from the live maps unless their generation
    /// shows they already hold the same contents.
    static TRANSPOSE_ROLLBACK_MAPS: std::cell::Cell<TransposeRollbackMaps> =
        std::cell::Cell::new(Default::default());
}

/// Bounds the retained transpose rollback buffers to one optimizer run, so
/// they do not outlive it into later, larger phases. Dropping it frees them.
pub(super) struct TransposeRollbackBuffers;

impl Drop for TransposeRollbackBuffers {
    fn drop(&mut self) {
        drop(TRANSPOSE_ROLLBACK_MAPS.take());
    }
}

/// `live.clone_from(saved)` for plain-data projection maps. Trials adjust
/// their values in place, so when the key sequence is unchanged the values
/// are assigned without rebuilding the tree; the result is the same map.
fn restore_map_values<K: Ord + Clone, V: Clone>(live: &mut BTreeMap<K, V>, saved: &BTreeMap<K, V>) {
    if live.len() == saved.len() && live.keys().eq(saved.keys()) {
        for (live, saved) in live.values_mut().zip(saved.values()) {
            live.clone_from(saved);
        }
    } else {
        live.clone_from(saved);
    }
}

#[cfg(test)]
mod trial_geometry_controls {
    use super::*;

    #[test]
    fn transpose_sparse_trial_keeps_unrelated_boxes_and_rejects_a_new_collision() {
        let mut input = Graph::default();
        let moving = input.add_node(crate::Node::new(
            "moving",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let center = input.add_node(crate::Node::new(
            "center",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: moving,
            target: center,
        });
        let obstacles = (0..24)
            .map(|index| {
                input.add_node(crate::Node::new(
                    format!("obstacle{index}"),
                    Size {
                        width: 80.0,
                        height: 80.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 120.0;
        graph.set_position(center, Point::default());
        graph.set_position(moving, Point { x: 0.0, y: 240.0 });
        // Occupy each alternative quarter-turn. Every candidate is a sparse
        // one-box change, and a clean receiver must still see that collision.
        for (index, &obstacle) in obstacles.iter().enumerate() {
            let position = match index {
                0 => Point { x: -240.0, y: 0.0 },
                1 => Point { x: 0.0, y: -240.0 },
                2 => Point { x: 240.0, y: 0.0 },
                _ => Point {
                    x: 2000.0 + index as f64 * 240.0,
                    y: 2000.0,
                },
            };
            graph.set_position(obstacle, position);
        }
        let before = graph
            .nodes
            .iter()
            .map(|node| (node.position, node.rect))
            .collect::<Vec<_>>();
        assert!(!graph.transpose_node(moving, false));
        assert_eq!(
            before,
            graph
                .nodes
                .iter()
                .map(|node| (node.position, node.rect))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn smart_swap_pruning_preserves_zero_sentinels_and_local_precision() {
        // A zero regular global score must keep the other global score:
        // swap.go distinguishes two rejected trials from a real zero score.
        for global in [0.0, -0.0] {
            assert!(smart_swap_requires_global_score(
                (10.0, global, 0),
                12.0,
                20.0
            ));
            assert!(smart_swap_requires_global_score(
                (0.0, global, 0),
                0.0,
                20.0
            ));
        }
        // Ordinary and smart forms tie inside the source precision boundary,
        // so the ordinary form wins even if smart's global score is smaller.
        assert!(!smart_swap_requires_global_score(
            (10.0, 50.0, 0),
            10.0,
            20.0
        ));
        assert!(!smart_swap_requires_global_score(
            (10.0, 50.0, 0),
            9.99995,
            20.0
        ));
        assert!(smart_swap_requires_global_score(
            (10.0, 50.0, 0),
            9.9998,
            20.0
        ));
        // Improving the current local score is independently required.
        assert!(!smart_swap_requires_global_score(
            (30.0, 50.0, 0),
            20.0,
            20.0
        ));
        assert!(!smart_swap_requires_global_score(
            (30.0, 50.0, 0),
            19.99995,
            20.0
        ));
        // A zero local score on a container chooses by global cost only when
        // both forms have zero local scores. Negative local costs are real.
        assert!(!smart_swap_requires_global_score(
            (10.0, 50.0, 0),
            0.0,
            20.0
        ));
        assert!(smart_swap_requires_global_score((0.0, 50.0, 0), 0.0, 20.0));
        assert!(smart_swap_requires_global_score((0.0, 50.0, 0), -1.0, 20.0));
    }

    #[test]
    fn regular_swap_pruning_retains_container_scores_and_smart_zero_sentinels() {
        for zero in [0.0, -0.0] {
            assert!(regular_swap_requires_global_score(zero, -1.0));
        }
        assert!(!regular_swap_requires_global_score(10.0, 10.0));
        assert!(!regular_swap_requires_global_score(9.99995, 10.0));
        assert!(regular_swap_requires_global_score(9.9998, 10.0));
        assert!(!regular_swap_requires_global_score(-1.0, -2.0));
        assert!(regular_swap_requires_global_score(-2.0, -1.0));
        // A winning smart zero must resolve the regular global zero sentinel.
        for zero in [0.0, -0.0] {
            assert!(regular_swap_requires_zero_check(10.0, (5.0, zero, 0), 10.0));
        }
        assert!(!regular_swap_requires_zero_check(10.0, (5.0, 1.0, 0), 10.0));
        assert!(!regular_swap_requires_zero_check(10.0, (0.0, 0.0, 0), 10.0));
        assert!(!regular_swap_requires_zero_check(
            10.0,
            (9.99995, 0.0, 0),
            10.0
        ));
        assert!(!regular_swap_requires_zero_check(10.0, (5.0, 0.0, 0), 4.0));
    }

    #[test]
    fn swap_baseline_restores_repeated_trials_and_refreshes_after_commit() {
        let mut input = Graph::default();
        let ids = (0..4)
            .map(|index| {
                input.add_node(crate::Node::new(
                    format!("n{index}"),
                    Size {
                        width: 60.0 + index as f64 * 10.0,
                        height: 40.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        input.add_edge(crate::Edge {
            source: ids[0],
            target: ids[3],
        });
        input.add_edge(crate::Edge {
            source: ids[1],
            target: ids[2],
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 120.0;
        for (index, &node) in ids.iter().enumerate() {
            graph.set_position(
                node,
                Point {
                    x: index as f64 * 240.0,
                    y: 0.0,
                },
            );
            graph.materialize_owner_node_point(node).unwrap();
        }
        let held = graph.owner_node_point(ids[0]).unwrap();
        let original = held.get();
        let overlaps = graph.existing_overlap_pairs();
        let exact = graph.exact_overlap_pairs();
        let bits = |result: Option<(f64, f64, usize)>| {
            result.map(|(local, global, crossings)| (local.to_bits(), global.to_bits(), crossings))
        };
        for committed in [false, true] {
            if committed {
                graph.swap_positions(ids[0], ids[1]);
                graph.reposition_swap_transaction_containers();
                graph.reconcile_projected_offsets_after_rollback();
                assert_ne!(original, held.get());
            }
            // SwapOptimize measures the current state before any trial. Those
            // calls also materialize the graph's lazy routing costs.
            graph.global_swap_edge_length();
            graph.swap_local_edge_length(ids[0]);
            graph.global_edge_crossings();
            graph.evaluate_swap_trial(ids[0], ids[1], false, &overlaps, &exact);
            let baseline = SwapTrialBaseline::capture(&graph);
            for candidate in [ids[1], ids[2], ids[3], ids[1]] {
                for smart in [false, true] {
                    let reference =
                        graph.evaluate_swap_trial(ids[0], candidate, smart, &overlaps, &exact);
                    assert!(reference.is_some(), "trial must exercise scoring");
                    let reference_state = format!("{graph:?}");
                    let reused = graph.evaluate_swap_trial_with_score_filter(
                        ids[0],
                        candidate,
                        smart,
                        &overlaps,
                        &exact,
                        Some(&baseline),
                        |_| true,
                    );
                    assert_eq!(bits(reference), bits(reused));
                    assert_eq!(reference_state, format!("{graph:?}"));
                    assert!(held.same_allocation(&graph.owner_node_point(ids[0]).unwrap()));
                }
            }
        }
    }
}
