// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pointer identities visible to Graph.Containers while clusters are active.
use super::*;

/// A vessel and its first member share an arena index, but are distinct Go
/// nodes. Keep that distinction when traversing retained container ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum ConnectedNode {
    Active(NodeId),
    ClusterMember(NodeId),
}

impl ConnectedNode {
    pub(super) fn arena_node(self) -> NodeId {
        match self {
            Self::Active(n) | Self::ClusterMember(n) => n,
        }
    }
}

// A walk has no callbacks or writes. Read each live ownership pointer once,
// retaining the distinction between a vessel and its first retained member.
#[derive(Clone, Default)]
struct ConnectedWalkNode {
    owner: std::cell::OnceCell<NodeId>,
    retained: std::cell::OnceCell<ConnectedNode>,
    parents: [std::cell::OnceCell<Option<ConnectedNode>>; 2],
}

struct ConnectedWalkView<'a> {
    graph: &'a ArenaGraph,
    nodes: Vec<ConnectedWalkNode>,
}

impl ConnectedWalkView<'_> {
    fn owner(&self, n: NodeId) -> NodeId {
        *self.nodes[n.0 as usize]
            .owner
            .get_or_init(|| self.graph.active_aggregate_owner(n))
    }

    fn retained(&self, n: NodeId) -> ConnectedNode {
        *self.nodes[n.0 as usize]
            .retained
            .get_or_init(|| self.graph.retained_container_ref(n))
    }

    fn canonical(&self, node: ConnectedNode) -> ConnectedNode {
        match node {
            ConnectedNode::Active(n) => ConnectedNode::Active(self.owner(n)),
            retained => retained,
        }
    }

    fn index(node: ConnectedNode) -> usize {
        match node {
            ConnectedNode::Active(n) => 2 * n.0 as usize,
            ConnectedNode::ClusterMember(n) => 2 * n.0 as usize + 1,
        }
    }

    fn parent(&self, node: ConnectedNode) -> Option<ConnectedNode> {
        let index = Self::index(node);
        *self.nodes[index / 2].parents[index % 2].get_or_init(|| match node {
            ConnectedNode::ClusterMember(n) => Some(ConnectedNode::Active(self.owner(n))),
            ConnectedNode::Active(n) => self
                .graph
                .active_node_container(n)
                .map(|p| self.retained(p)),
        })
    }

    fn is_descendant(&self, mut node: ConnectedNode, ancestor: ConnectedNode) -> bool {
        loop {
            if node == ancestor {
                return true;
            }
            let Some(parent) = self.parent(node) else {
                return false;
            };
            node = parent;
        }
    }
}

impl ArenaGraph {
    pub(super) fn connected_node_id(&self, node: ConnectedNode) -> i64 {
        match node {
            ConnectedNode::Active(n) => self.active_node_tala_id(n),
            ConnectedNode::ClusterMember(n) => self.nodes[n.0 as usize].tala_id,
        }
    }

    pub(super) fn connected_node_box(&self, node: ConnectedNode) -> Option<(Point, Size)> {
        match node {
            ConnectedNode::Active(n) => {
                Some((self.active_node_position(n)?, self.active_node_size(n)))
            }
            ConnectedNode::ClusterMember(n) => {
                Some((self.position(n)?, self.nodes[n.0 as usize].rect.size))
            }
        }
    }

    pub(super) fn connected_owning_container(&self, node: ConnectedNode) -> Option<NodeId> {
        self.active_node_container(node.arena_node())
    }

    pub(super) fn connected_spacing_delta(&self, node: ConnectedNode, other: ConnectedNode) -> f64 {
        let (proposed, node_size) = self
            .connected_node_box(node)
            .expect("positioned connected node");
        if let (ConnectedNode::Active(n), ConnectedNode::Active(o)) = (node, other) {
            return self.spacing_delta_with_loops(n, o, proposed);
        }
        let (other_position, other_size) = self
            .connected_node_box(other)
            .expect("positioned connected node");
        let decorations = |view: ConnectedNode| {
            let n = view.arena_node();
            if matches!(view, ConnectedNode::Active(_)) && self.active_node_is_aggregate(n) {
                (false, Insets::uniform(0.0))
            } else {
                (
                    self.nodes[n.0 as usize].shape == ShapeKind::SqlTable,
                    self.nodes[n.0 as usize].layout_margins,
                )
            }
        };
        let (node_table, node_margins) = decorations(node);
        let (other_table, other_margins) = decorations(other);
        // Every retained member edge was reconnected to the vessel.
        let delta = self.spacing_delta_for_geometry(
            proposed,
            node_size,
            other_position,
            other_size,
            node_table || other_table,
            node_margins,
            other_margins,
            (false, f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        let loops = |view| match view {
            ConnectedNode::Active(n) if self.active_node_is_aggregate(n) => None,
            ConnectedNode::Active(n) | ConnectedNode::ClusterMember(n) => {
                self.nodes[n.0 as usize].loop_offsets.as_ref()
            }
        };
        Self::spacing_delta_with_loop_geometry(
            delta,
            proposed,
            node_size,
            other_position,
            other_size,
            loops(node),
            loops(other),
        )
    }

    pub(super) fn translate_connected_node_boxes(&mut self, nodes: &[ConnectedNode], delta: Point) {
        let retained = nodes
            .iter()
            .filter_map(|&view| match view {
                ConnectedNode::ClusterMember(n) => self.position(n).map(|p| (n, p)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let active = nodes
            .iter()
            .filter_map(|&view| match view {
                ConnectedNode::Active(n) => Some(n),
                _ => None,
            })
            .collect::<Vec<_>>();
        self.translate_active_node_boxes(&active, delta);
        // Translate each retained box from its own baseline. Vessel fallback
        // representation may already have materialized a member translation.
        for (n, p) in retained {
            let target = Point {
                x: p.x + delta.x,
                y: p.y + delta.y,
            };
            if let Ok(point) = self.owner_node_point(n) {
                self.write_owner_point(&point, target)
                    .expect("translated member Point");
                self.nodes[n.0 as usize].rect.origin = target;
                self.nodes[n.0 as usize].unpositioned_scope_translation = None;
            } else {
                self.set_position(n, target);
            }
        }
    }

    pub(super) fn retained_container_ref(&self, node: NodeId) -> ConnectedNode {
        if self.active_cluster_index(node).is_some() {
            ConnectedNode::ClusterMember(node)
        } else {
            ConnectedNode::Active(node)
        }
    }

    /// Node.isDescendantOf follows Container first, then the Cluster vessel.
    /// AddCluster cleared the retained members' Container pointers.
    pub(super) fn connected_parent(&self, node: ConnectedNode) -> Option<ConnectedNode> {
        match node {
            ConnectedNode::ClusterMember(n) => {
                Some(ConnectedNode::Active(self.active_aggregate_owner(n)))
            }
            ConnectedNode::Active(n) => self
                .active_node_container(n)
                .map(|p| self.retained_container_ref(p)),
        }
    }

    pub(super) fn connected_is_descendant(
        &self,
        mut node: ConnectedNode,
        ancestor: ConnectedNode,
    ) -> bool {
        loop {
            if node == ancestor {
                return true;
            }
            let Some(parent) = self.connected_parent(node) else {
                return false;
            };
            node = parent;
        }
    }

    /// Ordered Node.connectedNodes traversal, retaining cluster member keys
    /// in Graph.Containers. Upstream does not define order when multiple
    /// ownership-map entries enqueue different parents at the same BFS step.
    pub(super) fn connected_node_views(
        &self,
        start: ConnectedNode,
        excluded: &[ConnectedNode],
    ) -> Vec<ConnectedNode> {
        let view = ConnectedWalkView {
            graph: self,
            nodes: vec![ConnectedWalkNode::default(); self.nodes.len()],
        };
        let start = view.canonical(start);
        let excluded = excluded
            .iter()
            .copied()
            .map(|n| view.canonical(n))
            .collect::<Vec<_>>();
        // Built on first use: many walks never consult a source child order.
        let node_by_tala_id_cell = std::cell::OnceCell::new();
        let node_by_tala_id = || {
            node_by_tala_id_cell.get_or_init(|| {
                let mut index = crate::internal::IdHashMap::default();
                index.reserve(self.nodes.len());
                for node in &self.nodes {
                    index.entry(node.tala_id).or_insert(node.input_id);
                }
                index
            })
        };
        #[cfg(feature = "diagnostic-traces")]
        let trace_connected = crate::internal::trace_env_value("D2RS_TRACE_CONNECTED_ORDER")
            .is_some_and(|target| {
                target == "all" || target == self.connected_node_id(start).to_string()
            });
        let child_views = |n| {
            let active = self
                .container_node_order(Some(n))
                .into_iter()
                .map(|child| ConnectedNode::Active(view.owner(child)))
                .collect::<Vec<_>>();
            let container = view.retained(n);
            let active_children = active.iter().copied().collect::<BTreeSet<_>>();
            let Some(connected_order) = self
                .connected_container_child_order
                .get(&self.connected_node_id(container))
                // Go's retained Graph.Containers slice can preserve a
                // different child order from the reprojected active vector.
                // Use that order when it accounts for every active child,
                // including a retained extracted child. A partial stale
                // source list must not reorder or omit the live children.
                .filter(|source| {
                    let source_children = source
                        .iter()
                        .filter_map(|child_id| node_by_tala_id().get(child_id))
                        .map(|&child| ConnectedNode::Active(view.owner(child)))
                        .collect::<BTreeSet<_>>();
                    active_children.is_subset(&source_children)
                })
            else {
                return active;
            };
            let mut ordered = Vec::with_capacity(active.len());
            let mut seen = BTreeSet::new();
            for &child_tala_id in connected_order {
                let Some(&child) = node_by_tala_id().get(&child_tala_id) else {
                    continue;
                };
                let child = ConnectedNode::Active(view.owner(child));
                if seen.insert(child) {
                    ordered.push(child);
                }
            }
            for child in active {
                if seen.insert(child) {
                    ordered.push(child);
                }
            }
            ordered
        };
        // Ownership is immutable during this BFS. Resolve each child slice
        // once, not once per visited node or inverse-parent check.
        let containers = self
            .nodes
            .iter()
            .filter(|n| n.is_container)
            .map(|n| (view.retained(n.input_id), child_views(n.input_id)))
            .collect::<Vec<_>>();
        let clusters = self
            .clusters
            .iter()
            .filter(|c| self.cluster_is_active(c))
            .map(|c| {
                (
                    ConnectedNode::Active(c.members[0]),
                    c.members
                        .iter()
                        .copied()
                        .filter(|&n| self.nodes[n.0 as usize].is_container)
                        .map(child_views)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let mut containers = containers;
        // Go's ConnectedNodeSet delegates to connectedNodes below 32 nodes,
        // which ranges over Graph.Containers and Graph.Clusters maps. Go 1.27
        // changes that visitation order across processes; the repeatability
        // report records the resulting trace-only divergence. In this small-
        // graph path, retain a stable Tala-ID order as Rust's deterministic
        // choice rather than claim one randomized Go run is canonical.
        if self.nodes.len() < 32 {
            containers.sort_by_key(|(container, _)| self.connected_node_id(*container));
        }
        #[cfg(feature = "diagnostic-traces")]
        let (trace_cluster_order, trace_container_order) = if trace_connected {
            (
                clusters
                    .iter()
                    .map(|(vessel, _)| self.connected_node_id(*vessel))
                    .collect::<Vec<_>>(),
                containers
                    .iter()
                    .map(|(container, _)| self.connected_node_id(*container))
                    .collect::<Vec<_>>(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        #[cfg(feature = "diagnostic-traces")]
        if trace_connected {
            let source = containers
                .iter()
                .map(|(container, children)| {
                    (
                        self.connected_node_id(*container).to_string(),
                        children
                            .iter()
                            .map(|child| self.connected_node_id(*child).to_string())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let active = self
                .nodes
                .iter()
                .filter(|n| n.is_container)
                .map(|n| {
                    (
                        self.connected_node_id(view.retained(n.input_id))
                            .to_string(),
                        self.container_node_order(Some(n.input_id))
                            .into_iter()
                            .map(|child| {
                                self.connected_node_id(ConnectedNode::Active(view.owner(child)))
                                    .to_string()
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let source_orders = self
                .nodes
                .iter()
                .filter(|n| n.is_container)
                .map(|n| {
                    let container = view.retained(n.input_id);
                    (
                        self.connected_node_id(container).to_string(),
                        self.connected_container_child_order
                            .get(&self.connected_node_id(container))
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            let raw_containers = self
                .nodes
                .iter()
                .filter(|n| n.is_container)
                .map(|n| {
                    (
                        n.tala_id.to_string(),
                        self.containers
                            .get(&Some(n.input_id))
                            .into_iter()
                            .flatten()
                            .map(|child| self.nodes[child.0 as usize].tala_id.to_string())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "CONNECTED_MAP_RUST start={} containers={:?} active_containers={:?} source_orders={:?} raw_containers={:?}",
                self.connected_node_id(start),
                source,
                active,
                source_orders,
                raw_containers,
            );
        }
        // Flags are membership only: queue order still comes from the same
        // edge, cluster and container slices as the pointer walk.
        let mut flags = vec![0u8; self.nodes.len() * 2];
        for &node in &excluded {
            flags[ConnectedWalkView::index(node)] = 1;
        }
        let is_excluded = |node| flags[ConnectedWalkView::index(node)] != 0;
        // Exclusion and ownership cannot change during this walk. Build the
        // inverse links in container order once, rather than scan all entries
        // and their children again for every dequeued node. Forward and inverse
        // links share a list so nested-container enqueue order stays exact.
        let mut container_links = if containers.is_empty() {
            Vec::new()
        } else {
            vec![Vec::new(); self.nodes.len() * 2]
        };
        for (container, children) in &containers {
            if is_excluded(*container) || children.iter().copied().any(is_excluded) {
                continue;
            }
            container_links[ConnectedWalkView::index(*container)].extend_from_slice(children);
            for &child in children {
                container_links[ConnectedWalkView::index(child)].push(*container);
            }
        }
        let mut cluster_parents = if clusters.is_empty() {
            Vec::new()
        } else {
            vec![Vec::new(); self.nodes.len() * 2]
        };
        for (vessel, members) in &clusters {
            for children in members {
                if children.iter().copied().any(is_excluded) {
                    continue;
                }
                for &child in children {
                    cluster_parents[ConnectedWalkView::index(child)].push(*vessel);
                }
            }
        }
        let mut queue = VecDeque::from([start]);
        let mut seen = vec![false; self.nodes.len() * 2];
        seen[ConnectedWalkView::index(start)] = true;
        let mut result = Vec::new();
        while let Some(current) = queue.pop_front() {
            if excluded.iter().any(|&e| {
                view.is_descendant(current, e)
                    || (view.is_descendant(e, current) && !view.is_descendant(start, current))
            }) {
                continue;
            }
            result.push(current);
            #[cfg(feature = "diagnostic-traces")]
            let mut trace_queue_adds = Vec::new();
            let mut add = |next| {
                if !std::mem::replace(&mut seen[ConnectedWalkView::index(next)], true) {
                    queue.push_back(next);
                    #[cfg(feature = "diagnostic-traces")]
                    if trace_connected {
                        trace_queue_adds.push(self.connected_node_id(next).to_string());
                    }
                }
            };
            // Reconnect moved every cluster member incident edge to its
            // vessel. Stable member edge slices are historical here.
            if let ConnectedNode::Active(n) = current {
                // Go's ConnectedNodeSet walks the pointer-visible Node.Edges
                // slice. ExtractTrees may remove an edge from Graph.Edges
                // while leaving that slice intact, so filtering through the
                // active edge order here would omit a tree connection and
                // change the BFS order (and the gap trace) for ordinary
                // descendants. Aggregate vessels still reconstruct their
                // abducted incidence below via active_edge_ids.
                let edge_ids = if self.active_aggregate_members(n).is_some() {
                    self.active_edge_ids(n).into_iter().collect::<Vec<_>>()
                } else {
                    self.nodes[n.0 as usize].edges.read()
                };
                for edge in edge_ids {
                    let next = ConnectedNode::Active(self.active_adjacent(n, edge));
                    if !is_excluded(next) {
                        add(next);
                    }
                }
                if let Some((_, members)) = clusters.iter().find(|(vessel, _)| *vessel == current) {
                    for children in members {
                        for &child in children {
                            add(child);
                        }
                    }
                }
            }
            if let Some(parents) = cluster_parents.get(ConnectedWalkView::index(current)) {
                for &parent in parents {
                    add(parent);
                }
            }
            if let Some(neighbors) = container_links.get(ConnectedWalkView::index(current)) {
                for &next in neighbors {
                    add(next);
                }
            }
            #[cfg(feature = "diagnostic-traces")]
            if trace_connected {
                eprintln!(
                    "CONNECTED_STEP_RUST start={} current={} excluded={:?} enqueued={:?} clusters={:?} containers={:?}",
                    self.connected_node_id(start),
                    self.connected_node_id(current),
                    excluded
                        .iter()
                        .map(|node| self.connected_node_id(*node).to_string())
                        .collect::<Vec<_>>(),
                    trace_queue_adds,
                    trace_cluster_order,
                    trace_container_order
                );
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if trace_connected {
            eprintln!(
                "CONNECTED_WALK_RUST start={} result={:?}",
                self.connected_node_id(start),
                result
                    .iter()
                    .map(|node| self.connected_node_id(*node).to_string())
                    .collect::<Vec<_>>()
            );
        }
        result
    }
}

#[cfg(test)]
mod walk_view_tests {
    use super::*;

    fn nested() -> ArenaGraph {
        let input = crate::internal::graphjson::graphjson_to_graph(
            br#"{"nodes":[{"id":1},{"id":2},{"id":3},{"id":4},{"id":5}],"containers":{"0":[1],"1":[2,5],"2":[3,4]}}"#,
        )
        .unwrap();
        ArenaGraph::from_input(&input)
    }

    fn active(ids: &[u32]) -> Vec<ConnectedNode> {
        ids.iter()
            .map(|&id| ConnectedNode::Active(NodeId(id - 1)))
            .collect()
    }

    #[test]
    fn nested_links_interleave_forward_and_inverse_entries_in_source_order() {
        let graph = nested();
        assert_eq!(
            graph.connected_node_views(active(&[2])[0], &[]),
            active(&[2, 1, 3, 4, 5])
        );
        assert_eq!(
            graph.connected_node_views(active(&[3])[0], &active(&[4])),
            active(&[3])
        );
        // An excluded descendant blocks that container's links, but does not
        // block the ancestors of the starting container or their other children.
        assert_eq!(
            graph.connected_node_views(active(&[2])[0], &active(&[4])),
            active(&[2, 1, 5])
        );
    }

    #[test]
    fn overlapping_duplicate_links_and_live_replacement_do_not_leave_stale_views() {
        let mut graph = nested();
        graph.nodes[4].is_container = true;
        graph
            .containers
            .insert(Some(NodeId(4)), vec![NodeId(2), NodeId(2)]);
        graph.connected_container_child_order.clear();
        // The ordinary child projection follows Node.Container. An overlapping
        // retained ownership view comes from the complete source-order slice.
        graph.connected_container_child_order.insert(5, vec![3, 3]);
        graph
            .containers
            .insert(Some(NodeId(1)), vec![NodeId(2), NodeId(3), NodeId(2)]);
        assert_eq!(
            graph.connected_node_views(active(&[3])[0], &[]),
            active(&[3, 2, 5, 1, 4])
        );
        graph.containers.insert(Some(NodeId(4)), vec![]);
        graph.connected_container_child_order.insert(5, vec![]);
        assert_eq!(
            graph.connected_node_views(active(&[3])[0], &[]),
            active(&[3, 2, 1, 4, 5])
        );
        assert_eq!(
            graph.connected_node_views(active(&[3])[0], &active(&[5])),
            active(&[3, 2, 4])
        );
    }
}
