// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Extraction, specialized placement, and restoration of nonbranching trees.
//!
//! Repeated leaf peeling separates fringe trees from the general-layout core;
//! sentinels preserve constrained or cyclic attachment points.

use super::*;

#[derive(Clone, Debug)]
struct PlacementTree {
    node: NodeId,
    sentinel_edge: Option<EdgeId>,
    children: Vec<PlacementTree>,
    orientation: Orientation,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TreeEdgeDirection {
    Outwards,
    Inwards,
    Bidirectional,
    Undirected,
}

impl PlacementTree {
    fn size(&self) -> usize {
        1 + self.children.iter().map(Self::size).sum::<usize>()
    }

    fn descendants(&self, output: &mut Vec<NodeId>) {
        // Go's treeDescendants walks Children with a growing queue. Keep the
        // breadth-first order here because callers use this list for placement
        // initialization, overlap checks, and trace payloads.
        let mut queue = self.children.iter().collect::<Vec<_>>();
        let mut cursor = 0;
        while cursor < queue.len() {
            let child = queue[cursor];
            cursor += 1;
            output.push(child.node);
            queue.extend(child.children.iter());
        }
    }

    fn contains(&self, node: NodeId) -> bool {
        self.node == node || self.children.iter().any(|child| child.contains(node))
    }

    fn set_orientation(&mut self, orientation: Orientation) {
        self.orientation = orientation;
        for child in &mut self.children {
            child.set_orientation(orientation);
        }
    }
}

impl ArenaGraph {
    // trees/preprocess_helpers.go558–576 builds make(0), then assigns only
    // after finding the removed node. Captured old headers remain untouched.
    fn remove_tree_container_node(&self, scope: Option<NodeId>, removed: NodeId) -> bool {
        let current = self.containers.get_header(&scope).unwrap_or_default();
        let mut filtered = SharedNodeSlice::allocated_empty();
        let mut found = false;
        for child in current.iter_slots() {
            if child == Some(removed) {
                found = true;
            } else {
                filtered = filtered
                    .append(&[child])
                    .expect("tree container filter append");
            }
        }
        if found {
            self.containers.replace_header(scope, filtered);
        }
        found
    }

    // trees/layout.go329–342 replaces the old sentinel using a fresh make(0)
    // filter and appends the promoted sentinel to that new header.
    fn promote_tree_container_sentinel(&self, scope: Option<NodeId>, old: NodeId, new: NodeId) {
        let current = self.containers.get_header(&scope).unwrap_or_default();
        let mut filtered = SharedNodeSlice::allocated_empty();
        for child in current.iter_slots() {
            if child != Some(old) {
                filtered = filtered
                    .append(&[child])
                    .expect("tree sentinel filter append");
            }
        }
        filtered = filtered.append_node(new).expect("tree sentinel append");
        self.containers.replace_header(scope, filtered);
    }

    fn trace_tree_placement_event(
        &self,
        event: &'static str,
        tree: &PlacementTree,
        orientation: Orientation,
        distance: f64,
        ratio: f64,
        descendants: &[NodeId],
    ) {
        let enabled = match event {
            "tree-trial" => crate::internal::trace_env_enabled("TALA_TRACE_TREE_TRIALS"),
            _ => crate::internal::trace_env_enabled("TALA_TRACE_TREE_PLACEMENT"),
        };
        if !enabled {
            return;
        }
        let nodes = descendants
            .iter()
            .map(|node_id| {
                let node = &self.nodes[node_id.0 as usize];
                let position = self.position(*node_id);
                serde_json::json!({
                    "id": node.external_id,
                    "x_bits": position.map(|point| format!("0x{:016x}", point.x.to_bits())),
                    "y_bits": position.map(|point| format!("0x{:016x}", point.y.to_bits())),
                })
            })
            .collect::<Vec<_>>();
        let root = &self.nodes[tree.node.0 as usize];
        let root_position = self.position(tree.node);
        eprintln!(
            "{}",
            serde_json::json!({
                "event": event,
                "root": root.external_id,
                "root_x_bits": root_position.map(|point| format!("0x{:016x}", point.x.to_bits())).unwrap_or_default(),
                "root_y_bits": root_position.map(|point| format!("0x{:016x}", point.y.to_bits())).unwrap_or_default(),
                "orientation": format!("{:?}", orientation),
                "distance_bits": format!("0x{:016x}", distance.to_bits()),
                "ratio_bits": format!("0x{:016x}", ratio.to_bits()),
                "nodes": nodes,
            })
        );
    }

    /// Return tree children in the order in which Go peeled them from the
    /// source container slice. Stable IDs are deliberately not the ordering
    /// key: several later placement scans observe this slice order.
    pub(super) fn tree_children_in_extraction_order(&self, parent: NodeId) -> Vec<NodeId> {
        let mut children = self
            .tree_routing_nodes
            .iter()
            .filter_map(|(&node, state)| (state.parent == parent).then_some(node))
            .collect::<Vec<_>>();
        children.sort_by_key(|node| {
            self.tree_routing_nodes
                .get(node)
                .map_or(usize::MAX, |state| state.extraction_order)
        });
        children
    }

    /// Return all retained tree nodes in extraction order for order-sensitive
    /// graph-wide scans which append them after active ordinary nodes.
    pub(super) fn tree_nodes_in_extraction_order(&self) -> Vec<NodeId> {
        let mut nodes = self.tree_routing_nodes.keys().copied().collect::<Vec<_>>();
        nodes.sort_by_key(|node| {
            self.tree_routing_nodes
                .get(node)
                .map_or(usize::MAX, |state| state.extraction_order)
        });
        nodes
    }

    fn extracted_tree_edge_direction(&self, node: NodeId, edge_id: EdgeId) -> TreeEdgeDirection {
        let edge = &self.edge_records[edge_id.0 as usize];
        if edge.source_arrow && edge.target_arrow {
            TreeEdgeDirection::Bidirectional
        } else if !edge.source_arrow && !edge.target_arrow {
            TreeEdgeDirection::Undirected
        } else if edge.to == node {
            TreeEdgeDirection::Inwards
        } else {
            TreeEdgeDirection::Outwards
        }
    }

    fn dominant_extracted_tree_direction(
        &self,
        root: NodeId,
        extracted: &BTreeMap<NodeId, TreeRoutingNode>,
        ranks: &BTreeMap<NodeId, usize>,
    ) -> TreeEdgeDirection {
        let state = extracted[&root];
        let own = self.extracted_tree_edge_direction(root, state.sentinel_edge);
        if own != TreeEdgeDirection::Undirected {
            return own;
        }
        let mut children = extracted
            .iter()
            .filter_map(|(&candidate, tree)| (tree.parent == root).then_some(candidate))
            .collect::<Vec<_>>();
        children.sort_by_key(|child| ranks.get(child).copied().unwrap_or(usize::MAX));
        children
            .into_iter()
            .map(|child| self.dominant_extracted_tree_direction(child, extracted, ranks))
            .find(|direction| *direction != TreeEdgeDirection::Undirected)
            .unwrap_or(TreeEdgeDirection::Undirected)
    }

    fn dominant_extracted_children_direction(
        &self,
        root: NodeId,
        extracted: &BTreeMap<NodeId, TreeRoutingNode>,
        children: &BTreeMap<NodeId, Vec<NodeId>>,
        ranks: &BTreeMap<NodeId, usize>,
    ) -> TreeEdgeDirection {
        let mut ordered_children = children.get(&root).cloned().unwrap_or_default();
        ordered_children.sort_by_key(|child| ranks.get(child).copied().unwrap_or(usize::MAX));
        ordered_children
            .into_iter()
            .map(|child| self.dominant_extracted_tree_direction(child, extracted, ranks))
            .find(|direction| *direction != TreeEdgeDirection::Undirected)
            .unwrap_or_else(|| {
                self.extracted_tree_edge_direction(root, extracted[&root].sentinel_edge)
            })
    }

    /// `rootsByTreeDirection` calls the upstream dominant-direction helper
    /// with `includeRoot=false`: the root's own sentinel edge is ignored
    /// while descendants are searched in preorder, and is used only as the
    /// all-undirected fallback. This differs from the root-inclusive helper
    /// used while merging fringe nodes.
    fn dominant_extracted_direction_excluding_root(
        &self,
        root: NodeId,
        extracted: &BTreeMap<NodeId, TreeRoutingNode>,
        ranks: &BTreeMap<NodeId, usize>,
    ) -> TreeEdgeDirection {
        let mut children = extracted
            .iter()
            .filter_map(|(&candidate, tree)| (tree.parent == root).then_some(candidate))
            .collect::<Vec<_>>();
        children.sort_by_key(|child| ranks.get(child).copied().unwrap_or(usize::MAX));
        for child in children {
            let direction =
                self.extracted_tree_edge_direction(child, extracted[&child].sentinel_edge);
            if direction != TreeEdgeDirection::Undirected {
                return direction;
            }
            let direction = self.dominant_extracted_tree_direction(child, extracted, ranks);
            if direction != TreeEdgeDirection::Undirected {
                return direction;
            }
        }
        self.extracted_tree_edge_direction(root, extracted[&root].sentinel_edge)
    }

    pub(super) fn initialize_tree_routing_state(&mut self) {
        self.rebuild_tree_routing_state(false);
    }

    pub(super) fn preprocess_trees(&mut self) {
        // ExtractTrees disconnects shared Node.Edges, not just Graph.Edges.
        // Keep the stable topology for numerical tree reconstruction while
        // publishing the actual incident field used by owner optimizers.
        for index in 0..self.nodes.len() {
            self.materialize_owner_node_incident_header(NodeId(index as u32))
                .expect("preprocessing owner incident field");
        }
        self.rebuild_tree_routing_state(true);
    }

    fn disconnect_owner_tree_edge(&mut self, edge: EdgeId) {
        let from = self.edge_records[edge.0 as usize].from;
        let to = self.edge_records[edge.0 as usize].to;
        self.remove_owner_incident_slot(from, Some(edge))
            .expect("tree From incident field");
        if to != from {
            self.remove_owner_incident_slot(to, Some(edge))
                .expect("tree To incident field");
        }
    }

    fn reconnect_owner_tree_edge(&mut self, edge: EdgeId) {
        let from = self.edge_records[edge.0 as usize].from;
        let to = self.edge_records[edge.0 as usize].to;
        for node in [Some(from), (to != from).then_some(to)]
            .into_iter()
            .flatten()
        {
            if let Ok(header) = self.owner_node_incident_header(node) {
                if !header.iter_slots().any(|slot| slot == Some(edge)) {
                    self.append_owner_incident_slot(node, Some(edge))
                        .expect("tree reconnect incident field");
                }
            }
        }
    }

    /// Materialize the topology retained by TALA's `Graph.NodeToTree`.
    ///
    /// `ExtractTrees` repeatedly peels non-sentinel fringe nodes within each
    /// container. Each peeled node owns the edge to the still-active parent as
    /// its sentinel edge. Containers, fixed/near nodes, and SQL-table scopes
    /// remain ordinary graph nodes.
    fn rebuild_tree_routing_state(&mut self, publish_node_order: bool) {
        if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
            eprint!("TREE_INITIAL_ORDER_RUST ");
            for node in &self.node_order {
                eprint!("{},", self.nodes[node.0 as usize].tala_id);
            }
            eprintln!();
            eprint!("TREE_CONTAINER_RUST nodes=");
            for node in self.containers.get(&None).into_iter().flatten() {
                eprint!(
                    "{}(edges={},ext={}),",
                    self.nodes[node.0 as usize].tala_id,
                    self.nodes[node.0 as usize].edges.len(),
                    self.external_edge_counts[node.0 as usize]
                );
            }
            eprintln!();
        }
        self.tree_routing_nodes.clear();
        self.tree_sentinels.clear();
        self.preprocessed_tree_children.clear();
        self.restored_tree_edges.clear();
        self.published_tree_node_edge_order.clear();
        let mut extracted = BTreeMap::<NodeId, TreeRoutingNode>::new();
        let mut extraction_order = BTreeMap::<Option<NodeId>, Vec<NodeId>>::new();
        let mut surviving_children = BTreeMap::<Option<NodeId>, Vec<NodeId>>::new();
        let mut sequence_owner = BTreeMap::<NodeId, NodeId>::new();
        let mut sequence_vessels = BTreeMap::new();
        for (index, sequence) in self.sequences.iter().enumerate() {
            let Some(owner) = sequence.members.first().copied() else {
                continue;
            };
            for member in &sequence.members {
                sequence_owner.insert(*member, owner);
            }
            if let Some(vessel) = self.topology_vessel_node(TopologyVessel::Sequence(index)) {
                sequence_owner.insert(vessel, owner);
                sequence_vessels.insert(owner, vessel);
            }
        }
        // trees/layout.go marks actual sequence vessels as terminal nodes.
        // This numerical view keeps their carriers, but reads incidence from
        // the allocated vessel and projects both endpoints consistently.
        let tree_incidents = |graph: &ArenaGraph, node: NodeId| {
            let actual = sequence_vessels.get(&node).copied().unwrap_or(node);
            graph.nodes[actual.0 as usize].edges.read()
        };

        let container_entries = self.containers.iter().collect::<Vec<_>>();
        for (scope, raw_scope_nodes) in &container_entries {
            // PreprocessTrees sees the concrete graph after AddSequences:
            // members are gone and each sequence vessel was appended.
            let mut scope_nodes = raw_scope_nodes
                .iter()
                .copied()
                .filter(|node| !sequence_owner.contains_key(node))
                .collect::<Vec<_>>();
            scope_nodes.extend(
                self.sequences
                    .iter()
                    .filter(|sequence| sequence.container == *scope)
                    .filter_map(|sequence| sequence.members.first().copied()),
            );
            if scope_nodes
                .iter()
                .any(|node| self.nodes[node.0 as usize].shape == ShapeKind::SqlTable)
            {
                surviving_children.insert(*scope, scope_nodes);
                continue;
            }

            let mut active = scope_nodes.iter().copied().collect::<BTreeSet<_>>();
            let mut sentinels = scope_nodes
                .iter()
                .copied()
                .filter(|node| {
                    let node = &self.nodes[node.0 as usize];
                    node.is_container || !node.nears.is_empty() || node.fixed_top_left.is_some()
                })
                .collect::<BTreeSet<_>>();
            sentinels.extend(
                self.sequences
                    .iter()
                    .filter(|sequence| sequence.container == *scope)
                    .filter_map(|sequence| sequence.members.first().copied()),
            );
            let mut sentinel_roots = BTreeMap::<NodeId, Vec<NodeId>>::new();
            let mut sentinel_arrowheads =
                BTreeMap::<NodeId, BTreeSet<(bool, Option<String>)>>::new();

            loop {
                // Go's treeFringeNodes ranges the container's node slice.
                // Keep that observable order here instead of iterating the
                // stable-ID set, since fringe batches become tree-child order.
                let fringe = scope_nodes
                    .iter()
                    .copied()
                    .filter(|node| active.contains(node))
                    .filter(|node| !sentinels.contains(node))
                    .filter_map(|node| {
                        // Go's getFringeNodes tests the complete aliased
                        // Node.Edges slice, not only edges present in the
                        // temporary childrenGraph. Projected scopes carry
                        // omitted external edges in this side channel.
                        let mut live_degree = self.external_edge_counts[node.0 as usize];
                        let mut adjacent = Vec::new();
                        for edge_id in tree_incidents(self, node) {
                            let edge = &self.edge_records[edge_id.0 as usize];
                            let from = sequence_owner.get(&edge.from).copied().unwrap_or(edge.from);
                            let to = sequence_owner.get(&edge.to).copied().unwrap_or(edge.to);
                            if edge.from == edge.to {
                                // Node.Edges retains one entry for a self-loop.
                                // Go's treeFringeNodes counts that entry in the
                                // degree test, even though its adjacent node is
                                // the same node. It prevents a loop-bearing
                                // endpoint from being peeled as an ordinary
                                // leaf, and a loop-only node is promoted to a
                                // sentinel by the two-node guard below.
                                if from == node {
                                    live_degree += 1;
                                    adjacent.push((node, edge_id));
                                }
                                continue;
                            }
                            if from == to {
                                continue;
                            }
                            let other = if from == node {
                                to
                            } else if to == node {
                                from
                            } else {
                                continue;
                            };
                            // Recovered `Nodes(containerNodes).getFringeNodes`
                            // first requires one complete live edge, then
                            // separately requires its adjacent endpoint to
                            // remain in the current container-node set.
                            // A sole cross-container edge therefore keeps a
                            // node out of the fringe.
                            if self.nodes[other.0 as usize].container != *scope
                                || active.contains(&other)
                            {
                                live_degree += 1;
                                adjacent.push((other, edge_id));
                            }
                        }
                        (live_degree == 1 && adjacent.len() == 1 && active.contains(&adjacent[0].0))
                            .then(|| {
                                let (parent, sentinel_edge) = adjacent[0];
                                let parent_degree = self.external_edge_counts[parent.0 as usize]
                                    + self
                                        .member_edges()
                                        .filter(|edge| {
                                            let from = sequence_owner
                                                .get(&edge.from)
                                                .copied()
                                                .unwrap_or(edge.from);
                                            let to = sequence_owner
                                                .get(&edge.to)
                                                .copied()
                                                .unwrap_or(edge.to);
                                            if from == to {
                                                // Match the complete Node.Edges
                                                // length: a real self-loop is
                                                // still one incident edge on
                                                // its owner. Edges collapsed
                                                // through a sequence alias are
                                                // not self-loops in the source
                                                // graph and remain excluded.
                                                return edge.from == edge.to
                                                    && from == parent
                                                    && active.contains(&parent);
                                            }
                                            let other = if from == parent {
                                                to
                                            } else if to == parent {
                                                from
                                            } else {
                                                return false;
                                            };
                                            self.nodes[other.0 as usize].container != *scope
                                                || active.contains(&other)
                                        })
                                        .count();
                                (node, parent, sentinel_edge, parent_degree)
                            })
                    })
                    .collect::<Vec<_>>();
                if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
                    eprint!("TREE_FRINGE_RUST ");
                    for (node, parent, _, degree) in &fringe {
                        eprint!(
                            "{}>{}({}),",
                            self.nodes[node.0 as usize].tala_id,
                            self.nodes[parent.0 as usize].tala_id,
                            degree
                        );
                    }
                    eprintln!();
                }
                if fringe.is_empty() {
                    break;
                }

                let mut removed_any = false;
                for (node, parent, sentinel_edge, parent_degree) in fringe.iter().copied() {
                    if !active.contains(&node) || sentinels.contains(&node) {
                        continue;
                    }
                    // An isolated two-node component retains one endpoint as
                    // the sentinel; the other endpoint is its sole tree node.
                    if parent_degree == 1 && !sentinels.contains(&parent) {
                        sentinels.insert(node);
                        continue;
                    }

                    extracted.insert(
                        node,
                        TreeRoutingNode {
                            parent,
                            sentinel_edge,
                            orientation: Orientation::None,
                            extraction_order: usize::MAX,
                        },
                    );
                    sentinel_roots.entry(parent).or_default().push(node);
                    let edge = &self.edge_records[sentinel_edge.0 as usize];
                    let arrowhead = if edge.to == parent {
                        (edge.target_arrow, edge.target_arrowhead.clone())
                    } else {
                        (edge.source_arrow, edge.source_arrowhead.clone())
                    };
                    sentinel_arrowheads
                        .entry(parent)
                        .or_default()
                        .insert(arrowhead);

                    let child_roots = sentinel_roots
                        .get(&node)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|root| !sentinels.contains(root))
                        .collect::<Vec<_>>();
                    if !child_roots.is_empty() {
                        let ranks = extraction_order
                            .get(scope)
                            .into_iter()
                            .flatten()
                            .copied()
                            .chain(fringe.iter().map(|(node, ..)| *node))
                            .enumerate()
                            .map(|(rank, node)| (node, rank))
                            .collect::<BTreeMap<_, _>>();
                        let directions = child_roots
                            .iter()
                            .copied()
                            .map(|root| {
                                self.dominant_extracted_tree_direction(root, &extracted, &ranks)
                            })
                            .collect::<BTreeSet<_>>();
                        let arrowhead_count =
                            sentinel_arrowheads.get(&node).map_or(0, BTreeSet::len);
                        if directions.len() > 1 || arrowhead_count > 1 {
                            sentinels.insert(node);
                            continue;
                        }

                        let sentinel_direction =
                            self.extracted_tree_edge_direction(node, sentinel_edge);
                        let sentinel_is_terminal = !directions
                            .contains(&TreeEdgeDirection::Undirected)
                            && sentinel_direction != TreeEdgeDirection::Undirected
                            && !directions.contains(&sentinel_direction);
                        if sentinel_is_terminal {
                            sentinels.insert(parent);
                        }
                    }
                }

                // TALA decides the complete fringe batch before it mutates the
                // graph. A node promoted to a sentinel by a sibling's edge
                // direction remains live even if its candidate tree record was
                // initialized earlier in this batch.
                for (node, ..) in &fringe {
                    if sentinels.contains(node) {
                        extracted.remove(node);
                    }
                }
                for (node, ..) in fringe {
                    if sentinels.contains(&node) {
                        continue;
                    }
                    active.remove(&node);
                    extraction_order.entry(*scope).or_default().push(node);
                    if publish_node_order {
                        self.disconnect_owner_tree_edge(extracted[&node].sentinel_edge);
                        // This is a native projection precondition; the Go
                        // helper's context polls/error propagation remain a
                        // separate preprocessing contract.
                        assert!(
                            self.remove_tree_container_node(*scope, node),
                            "peeled tree node requires container membership"
                        );
                    }
                    removed_any = true;
                }
                if !removed_any {
                    break;
                }
            }
            surviving_children.insert(
                *scope,
                scope_nodes
                    .iter()
                    .copied()
                    .filter(|node| active.contains(node))
                    .collect(),
            );
        }

        // `handleTreeSubgraph` reroots an isolated multi-root tree when one
        // root can be reversed into a new sentinel. For an all-undirected
        // tree every root is a candidate, in extraction order. The selected
        // root must be a non-branching chain: its terminal node becomes the
        // graph sentinel, the chain is reversed, and the old sentinel becomes
        // a tree node owning the selected root's former sentinel edge.
        //
        // This is structural state, not a routing special case. In particular,
        // later `Graph.dejitter` uses NodeToTree membership to avoid moving
        // those nodes after their sentinel routes have been established.
        let extraction_ranks = extraction_order
            .values()
            .flatten()
            .enumerate()
            .map(|(rank, node)| (*node, rank))
            .collect::<BTreeMap<_, _>>();
        for (node, state) in &mut extracted {
            state.extraction_order = extraction_ranks.get(node).copied().unwrap_or(usize::MAX);
        }

        let initial_children = {
            let mut children = BTreeMap::<NodeId, Vec<NodeId>>::new();
            for (&node, tree) in &extracted {
                if extracted.contains_key(&tree.parent) {
                    children.entry(tree.parent).or_default().push(node);
                }
            }
            for nodes in children.values_mut() {
                nodes.sort_by_key(|node| extraction_ranks.get(node).copied().unwrap_or(usize::MAX));
            }
            children
        };
        let mut initial_roots = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for (&node, tree) in &extracted {
            if !extracted.contains_key(&tree.parent) {
                initial_roots.entry(tree.parent).or_default().push(node);
            }
        }
        for roots in initial_roots.values_mut() {
            roots.sort_by_key(|node| extraction_ranks.get(node).copied().unwrap_or(usize::MAX));
        }
        for (sentinel, roots) in initial_roots {
            // ExtractTrees preserves a sequence sentinel instead of passing
            // it to handleTreeSubgraph (trees/layout.go201–205).
            if sequence_owner.contains_key(&sentinel) {
                continue;
            }
            // Go's handleTreeSubgraph runs after every extracted sentinel edge
            // has been disconnected. Stable Rust keeps the arena edge slice
            // immutable during preprocessing, so derive the same guard from
            // the set of peeled nodes: an edge to any node not peeled is the
            // equivalent of a still-present `rootSentinel.Edges` entry.
            let peeled = extraction_order
                .values()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>();
            let has_live_edge = tree_incidents(self, sentinel).iter().any(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                let from = sequence_owner.get(&edge.from).copied().unwrap_or(edge.from);
                let to = sequence_owner.get(&edge.to).copied().unwrap_or(edge.to);
                let other = if from == sentinel {
                    to
                } else if to == sentinel {
                    from
                } else {
                    return false;
                };
                !peeled.contains(&other)
            });
            if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
                eprintln!(
                    "TREE_HANDLE_RUST sentinel={} roots={} live={} peeled={:?} edges={:?}",
                    self.nodes[sentinel.0 as usize].tala_id,
                    roots.len(),
                    has_live_edge,
                    peeled
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.nodes[sentinel.0 as usize]
                        .edges
                        .read()
                        .iter()
                        .map(|edge| {
                            let edge = &self.edge_records[edge.0 as usize];
                            (
                                self.nodes[edge.from.0 as usize].tala_id,
                                self.nodes[edge.to.0 as usize].tala_id,
                            )
                        })
                        .collect::<Vec<_>>()
                );
            }
            if roots.len() <= 1 || has_live_edge || self.nodes[sentinel.0 as usize].is_container {
                continue;
            }
            let isolated = tree_incidents(self, sentinel).iter().all(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                let from = sequence_owner.get(&edge.from).copied().unwrap_or(edge.from);
                let to = sequence_owner.get(&edge.to).copied().unwrap_or(edge.to);
                let adjacent = if from == sentinel {
                    to
                } else if to == sentinel {
                    from
                } else {
                    return false;
                };
                roots.contains(&adjacent)
            });
            if !isolated {
                continue;
            }

            let directions = roots
                .iter()
                .copied()
                .map(|root| {
                    (
                        root,
                        self.dominant_extracted_children_direction(
                            root,
                            &extracted,
                            &initial_children,
                            &extraction_ranks,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let count = |direction| {
                directions
                    .iter()
                    .filter(|(_, candidate)| *candidate == direction)
                    .count()
            };
            let outwards = count(TreeEdgeDirection::Outwards);
            let inwards = count(TreeEdgeDirection::Inwards);
            let bidirectional = count(TreeEdgeDirection::Bidirectional);
            let undirected = count(TreeEdgeDirection::Undirected);
            let mut candidate_roots = Vec::new();
            let mut append_direction =
                |direction| {
                    candidate_roots.extend(directions.iter().filter_map(|(root, candidate)| {
                        (*candidate == direction).then_some(*root)
                    }));
                };
            if outwards == 1 && inwards + undirected + 1 == roots.len() {
                append_direction(TreeEdgeDirection::Outwards);
            }
            if inwards == 1 && outwards + undirected + 1 == roots.len() {
                append_direction(TreeEdgeDirection::Inwards);
            }
            if undirected != 0 && outwards > 1 && outwards + undirected == roots.len() {
                append_direction(TreeEdgeDirection::Undirected);
            }
            if undirected != 0 && inwards > 1 && inwards + undirected == roots.len() {
                append_direction(TreeEdgeDirection::Undirected);
            }
            if bidirectional + undirected == roots.len() {
                append_direction(TreeEdgeDirection::Bidirectional);
                append_direction(TreeEdgeDirection::Undirected);
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
                eprintln!(
                    "TREE_CANDIDATES_RUST sentinel={} dirs={:?} candidates={:?}",
                    self.nodes[sentinel.0 as usize].tala_id,
                    directions
                        .iter()
                        .map(|(root, direction)| (self.nodes[root.0 as usize].tala_id, *direction))
                        .collect::<Vec<_>>(),
                    candidate_roots
                        .iter()
                        .map(|root| self.nodes[root.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }

            let candidate_path = candidate_roots.into_iter().find_map(|root| {
                let mut path = vec![root];
                let mut current = root;
                loop {
                    let children = initial_children
                        .get(&current)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    match children {
                        [] => return Some(path),
                        [child] => {
                            current = *child;
                            path.push(current);
                        }
                        _ => return None,
                    }
                }
            });
            let Some(path) = candidate_path else {
                continue;
            };

            let selected_root = path[0];
            let selected_edge = extracted[&selected_root].sentinel_edge;
            let reversed_edges = path
                .iter()
                .skip(1)
                .map(|node| extracted[node].sentinel_edge)
                .collect::<Vec<_>>();
            let new_sentinel = *path.last().unwrap();
            extracted.remove(&new_sentinel);
            for (pair, sentinel_edge) in path.windows(2).zip(reversed_edges) {
                let parent = pair[1];
                let node = pair[0];
                let tree = extracted.get_mut(&node).unwrap();
                tree.parent = parent;
                tree.sentinel_edge = sentinel_edge;
            }
            extracted.insert(
                sentinel,
                TreeRoutingNode {
                    parent: selected_root,
                    sentinel_edge: selected_edge,
                    orientation: Orientation::None,
                    extraction_order: usize::MAX,
                },
            );
            // handleTreeSubgraph removes the former sentinel from its
            // container and appends the promoted endpoint. The subsequent
            // putBackNonBranchingTrees range observes that updated slice, so
            // the promoted sentinel initiates reconnection of the reversed
            // chain.
            let scope = self.nodes[sentinel.0 as usize].container;
            if let Some(survivors) = surviving_children.get_mut(&scope) {
                survivors.retain(|node| *node != sentinel);
                survivors.push(new_sentinel);
            }
            if publish_node_order {
                self.promote_tree_container_sentinel(scope, sentinel, new_sentinel);
            }
        }

        // `putBackNonBranchingTrees` reconnects every simple chain before
        // `buildNodeToTree` publishes the routing carrier. Only genuinely
        // branching roots remain, except that an isolated sentinel with
        // multiple roots retains the complete fanout.
        let mut children = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for (&node, tree) in &extracted {
            if extracted.contains_key(&tree.parent) {
                children.entry(tree.parent).or_default().push(node);
            }
        }
        for nodes in children.values_mut() {
            nodes.sort_by_key(|node| {
                extracted
                    .get(node)
                    .map_or(usize::MAX, |state| state.extraction_order)
            });
        }
        let branches = |root: NodeId| {
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                let node_children = children.get(&node).map(Vec::as_slice).unwrap_or(&[]);
                if node_children.len() > 1 {
                    return true;
                }
                stack.extend(node_children.iter().copied());
            }
            false
        };
        let mut roots_by_sentinel = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for (&node, tree) in &extracted {
            if !extracted.contains_key(&tree.parent) {
                roots_by_sentinel.entry(tree.parent).or_default().push(node);
            }
        }
        let mut retained_roots = BTreeSet::new();
        for (sentinel, roots) in roots_by_sentinel {
            let sentinel_node = &self.nodes[sentinel.0 as usize];
            let isolated = !sentinel_node.is_container
                && tree_incidents(self, sentinel).iter().all(|edge_id| {
                    let edge = &self.edge_records[edge_id.0 as usize];
                    let from = sequence_owner.get(&edge.from).copied().unwrap_or(edge.from);
                    let to = sequence_owner.get(&edge.to).copied().unwrap_or(edge.to);
                    let adjacent = if from == sentinel {
                        to
                    } else if to == sentinel {
                        from
                    } else {
                        return false;
                    };
                    roots.contains(&adjacent)
                });
            if isolated
                && !sequence_owner.contains_key(&sentinel)
                && (roots.len() > 1 || roots.iter().copied().any(&branches))
            {
                retained_roots.extend(roots);
            } else {
                retained_roots.extend(roots.into_iter().filter(|root| branches(*root)));
            }
        }
        let mut stack = retained_roots.into_iter().collect::<Vec<_>>();
        while let Some(node) = stack.pop() {
            let Some(tree) = extracted.get(&node).copied() else {
                continue;
            };
            if self.tree_routing_nodes.insert(node, tree).is_some() {
                continue;
            }
            stack.extend(children.get(&node).into_iter().flatten().copied());
        }
        self.tree_sentinels = self
            .tree_routing_nodes
            .values()
            .map(|tree| tree.parent)
            .collect();
        // Keep the surviving Graph.Containers slice order.  Go's
        // handleTreeSubgraph/AddNode lifecycle leaves those nodes in
        // Graph.Nodes before the newly promoted sentinel is appended; using
        // extraction order here reverses that observable order for a grid.
        let survivor_order_by_scope = surviving_children.clone();

        // `putBackNonBranchingTrees` appends each restored root and its
        // descendants to Graph.Nodes/Containers in recursive tree order. That
        // temporal child order is observed by AddClusters, even though the
        // stable Rust arena never physically removes those nodes.
        fn append_reconnected(
            node: NodeId,
            children: &BTreeMap<NodeId, Vec<NodeId>>,
            retained: &BTreeMap<NodeId, TreeRoutingNode>,
            ranks: &BTreeMap<NodeId, usize>,
            output: &mut Vec<NodeId>,
        ) {
            if retained.contains_key(&node) {
                return;
            }
            output.push(node);
            let mut descendants = children.get(&node).cloned().unwrap_or_default();
            descendants.sort_by_key(|child| ranks.get(child).copied().unwrap_or(usize::MAX));
            for child in descendants {
                append_reconnected(child, children, retained, ranks, output);
            }
        }

        for (scope, survivors) in surviving_children {
            if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
                eprintln!(
                    "TREE_SURVIVORS_RUST scope={:?} survivors={:?}",
                    scope.map(|node| self.nodes[node.0 as usize].tala_id),
                    survivors
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
            let ordered_extracted = extraction_order.get(&scope).cloned().unwrap_or_default();
            let ranks = ordered_extracted
                .iter()
                .enumerate()
                .map(|(rank, node)| (*node, rank))
                .collect::<BTreeMap<_, _>>();
            let mut order = survivors.clone();
            // Go ranges over the pre-reconnection child-slice length, so only
            // surviving sentinels initiate restoration in this pass.
            for sentinel in survivors {
                let mut roots = extracted
                    .iter()
                    .filter_map(|(&node, tree)| (tree.parent == sentinel).then_some(node))
                    .collect::<Vec<_>>();
                roots.sort_by_key(|root| ranks.get(root).copied().unwrap_or(usize::MAX));
                for root in roots {
                    append_reconnected(
                        root,
                        &children,
                        &self.tree_routing_nodes,
                        &ranks,
                        &mut order,
                    );
                }
            }
            self.preprocessed_tree_children.insert(scope, order);
        }

        // Pipeline.PreprocessTrees physically removes every peeled node from
        // Graph.Nodes. putBackNonBranchingTrees then walks containers
        // deepest/rightmost first and appends only the restored (non-routing)
        // tree nodes. Arena construction also initializes this structural
        // state for temporary SplitSubgraphs, but those graphs do not execute
        // the physical pipeline stage again.
        if !publish_node_order {
            return;
        }
        let extracted_nodes = extracted.keys().copied().collect::<BTreeSet<_>>();
        let ever_extracted = extraction_order
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        self.node_order
            .retain(|node| !extracted_nodes.contains(node) && !ever_extracted.contains(node));
        let mut scope_order = self
            .hierarchy_node_order()
            .into_iter()
            .rev()
            .map(Some)
            .collect::<Vec<_>>();
        scope_order.push(None);
        let restoration_scope_order = scope_order.clone();
        let mut promoted_by_scope = BTreeMap::<Option<NodeId>, Vec<NodeId>>::new();
        for scope in scope_order.iter().copied() {
            // handleTreeSubgraph can promote one formerly extracted root into
            // the new sentinel. Graph.AddNode appends that promoted sentinel
            // before putBackNonBranchingTrees restores the remaining chain.
            for node in survivor_order_by_scope
                .get(&scope)
                .into_iter()
                .flatten()
                .copied()
            {
                if !extracted_nodes.contains(&node) && !self.node_order.contains(&node) {
                    self.node_order.push(node);
                    promoted_by_scope.entry(scope).or_default().push(node);
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
            eprintln!(
                "TREE_PROMOTED_RUST {:?}",
                promoted_by_scope
                    .iter()
                    .map(|(scope, nodes)| (
                        scope.map(|n| self.nodes[n.0 as usize].tala_id),
                        nodes
                            .iter()
                            .map(|n| self.nodes[n.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let mut restored_by_scope = BTreeMap::<Option<NodeId>, Vec<NodeId>>::new();
        for scope in scope_order {
            let restored = self
                .preprocessed_tree_children
                .get(&scope)
                .into_iter()
                .flatten()
                .copied()
                .filter(|node| {
                    extracted_nodes.contains(node)
                        && !self.tree_routing_nodes.contains_key(node)
                        && !self.node_order.contains(node)
                })
                .collect::<Vec<_>>();
            for node in restored {
                if extracted_nodes.contains(&node)
                    && !self.tree_routing_nodes.contains_key(&node)
                    && !self.node_order.contains(&node)
                {
                    self.node_order.push(node);
                    restored_by_scope.entry(scope).or_default().push(node);
                }
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
            eprintln!(
                "TREE_RESTORED_RUST {:?}",
                restored_by_scope
                    .iter()
                    .map(|(scope, nodes)| (
                        scope.map(|n| self.nodes[n.0 as usize].tala_id),
                        nodes
                            .iter()
                            .map(|n| self.nodes[n.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }

        // ExtractTrees disconnects every peeled sentinel edge. The surviving
        // core keeps its original order; putBackNonBranchingTrees then appends
        // restored edges in deepest-container and recursive-tree order.
        // Retained branching-tree edges remain absent until PlaceTrees.
        let extracted_edges = extracted
            .values()
            .map(|tree| tree.sentinel_edge)
            .collect::<BTreeSet<_>>();
        self.edge_order
            .retain(|edge| !extracted_edges.contains(edge));
        for scope in restoration_scope_order {
            for node in restored_by_scope.get(&scope).into_iter().flatten() {
                // reconnectTreeGuarded publishes each node's parent header
                // before reconnecting its edge, in restoration scope order.
                self.containers.append_node(scope, *node);
                let edge = extracted[node].sentinel_edge;
                if !self.edge_order.contains(&edge) {
                    self.edge_order.push(edge);
                }
                self.restored_tree_edges.insert(edge);
                self.reconnect_owner_tree_edge(edge);
            }
        }
        if publish_node_order {
            self.publish_preprocessed_owner_trees();
        }
        // Placement diagnostics follow Go's shared `Containers` slices. The
        // stable arena keeps cloned trace carriers so an induced graph can
        // still expose descendants after its local container map is reduced;
        // refresh those carriers after the tree remove/append lifecycle so
        // their order matches the now-current owner slices.
        self.trace_container_children = self
            .containers
            .iter()
            .filter_map(|(scope, children)| {
                let parent = scope.map(|node| self.nodes[node.0 as usize].tala_id)?;
                Some((
                    parent,
                    children
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].clone())
                        .collect::<Vec<_>>(),
                ))
            })
            .collect();
        if crate::internal::trace_env_enabled("TALA_TRACE_TREE_STATE") {
            eprintln!(
                "TREE_STATE_RUST nodes={:?} edges={:?} restored={:?} trees={:?}",
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.edge_order
                    .read()
                    .iter()
                    .map(|edge| edge.0)
                    .collect::<Vec<_>>(),
                self.restored_tree_edges,
                self.tree_routing_nodes
                    .iter()
                    .map(|(node, tree)| (*node, tree.parent))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "TREE_CONTAINER_STATE_RUST {:?}",
                self.containers
                    .get(&None)
                    .into_iter()
                    .flatten()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
    }

    pub(super) fn is_tree_edge(&self, edge: EdgeId) -> bool {
        let Some((_, state)) = self
            .tree_routing_nodes
            .iter()
            .find(|(_, tree)| tree.sentinel_edge == edge)
        else {
            return false;
        };
        if self.tree_routing_nodes.contains_key(&state.parent) {
            // Tree.recordInternalTreeEdges includes descendant edges but not
            // the root's edge to its sentinel.
            return true;
        }

        // addIsolatedTreeEdgesToMap adds root sentinel edges only when every
        // edge incident to that non-container sentinel leads to one of its
        // tree roots.
        let sentinel = state.parent;
        if self.nodes[sentinel.0 as usize].is_container {
            return false;
        }
        let root_edges = self
            .tree_routing_nodes
            .values()
            .filter_map(|root| {
                (!self.tree_routing_nodes.contains_key(&root.parent) && root.parent == sentinel)
                    .then_some(root.sentinel_edge)
            })
            .collect::<BTreeSet<_>>();
        self.nodes[sentinel.0 as usize]
            .edges
            .read()
            .iter()
            .all(|candidate| root_edges.contains(candidate))
    }

    /// Mirrors `layoutgraph.Edge.IsTreeEdge`, which is deliberately broader
    /// than `Graph.TreeEdgeMap`: the Go edge method checks whether either
    /// endpoint is present in the owning graph's `NodeToTree` index. Port
    /// refinement uses this predicate even for a root sentinel edge that is
    /// still routed by ordinary graph routing.
    pub(super) fn edge_touches_tree_node(&self, edge: EdgeId) -> bool {
        let Some(edge) = self.edge_records.get(edge.0 as usize) else {
            return false;
        };
        self.tree_routing_nodes.contains_key(&edge.from)
            || self.tree_routing_nodes.contains_key(&edge.to)
    }

    fn placement_tree_from_node(
        &self,
        node: NodeId,
        children: &BTreeMap<NodeId, Vec<NodeId>>,
    ) -> PlacementTree {
        let state = self.tree_routing_nodes[&node];
        PlacementTree {
            node,
            sentinel_edge: Some(state.sentinel_edge),
            children: children
                .get(&node)
                .into_iter()
                .flatten()
                .copied()
                .map(|child| self.placement_tree_from_node(child, children))
                .collect(),
            orientation: Orientation::None,
        }
    }

    fn tree_edge_direction(&self, tree: &PlacementTree) -> TreeEdgeDirection {
        let edge =
            &self.edge_records[tree.sentinel_edge.expect("non-sentinel tree node").0 as usize];
        if edge.source_arrow && edge.target_arrow {
            return TreeEdgeDirection::Bidirectional;
        }
        if !edge.source_arrow && !edge.target_arrow {
            return tree
                .children
                .iter()
                .map(|child| self.tree_edge_direction(child))
                .find(|direction| *direction != TreeEdgeDirection::Undirected)
                .unwrap_or(TreeEdgeDirection::Undirected);
        }
        let arrow_points_to_node = (edge.to == tree.node && edge.target_arrow)
            || (edge.from == tree.node && edge.source_arrow);
        if arrow_points_to_node {
            TreeEdgeDirection::Inwards
        } else {
            TreeEdgeDirection::Outwards
        }
    }

    fn edge_arrowhead_towards(&self, edge_id: EdgeId, node: NodeId) -> (bool, Option<String>) {
        let edge = &self.edge_records[edge_id.0 as usize];
        if edge.to == node {
            (edge.target_arrow, edge.target_arrowhead.clone())
        } else {
            (edge.source_arrow, edge.source_arrowhead.clone())
        }
    }

    fn tree_is_isolated(&self, sentinel: NodeId, roots: &[PlacementTree]) -> bool {
        if self.nodes[sentinel.0 as usize].is_container {
            return false;
        }
        // The stable arena retains every physical edge, while TALA removes
        // all tree sentinel edges from Node.Edges until reconnectTree. Ignore
        // those inactive edges when reproducing isIsolatedTree.
        // isIsolatedTree checks against every root of the sentinel
        // (g.Trees[node]), not just the placement tree being placed: a
        // sibling placement tree's reconnected root edge still counts.
        let root_edges = roots
            .iter()
            .filter_map(|root| root.sentinel_edge)
            .chain(
                self.tree_routing_nodes
                    .values()
                    .filter(|state| state.parent == sentinel)
                    .map(|state| state.sentinel_edge),
            )
            .collect::<BTreeSet<_>>();
        self.nodes[sentinel.0 as usize]
            .edges
            .read()
            .iter()
            .all(|edge| root_edges.contains(edge))
    }

    /// Recovered `Graph.getPlacementTrees`, represented with stable arena IDs.
    fn placement_trees_for_component(&self, component: &BTreeSet<NodeId>) -> Vec<PlacementTree> {
        let mut children = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut roots = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for (&node, state) in &self.tree_routing_nodes {
            if self.tree_routing_nodes.contains_key(&state.parent) {
                children.entry(state.parent).or_default().push(node);
            } else if component.contains(&state.parent) {
                roots.entry(state.parent).or_default().push(node);
            }
        }
        for nodes in children.values_mut() {
            nodes.sort_by_key(|node| {
                self.tree_routing_nodes
                    .get(node)
                    .map_or(usize::MAX, |state| state.extraction_order)
            });
        }
        for nodes in roots.values_mut() {
            nodes.sort_by_key(|node| {
                self.tree_routing_nodes
                    .get(node)
                    .map_or(usize::MAX, |state| state.extraction_order)
            });
        }

        let mut result = Vec::new();
        for sentinel in self.node_order.iter().copied() {
            let Some(root_nodes) = roots.get(&sentinel) else {
                continue;
            };
            let raw_roots = root_nodes
                .iter()
                .copied()
                .map(|root| self.placement_tree_from_node(root, &children))
                .collect::<Vec<_>>();
            if !self.tree_is_isolated(sentinel, &raw_roots) {
                result.extend(raw_roots.into_iter().map(|root| PlacementTree {
                    node: sentinel,
                    sentinel_edge: None,
                    children: vec![root],
                    orientation: Orientation::None,
                }));
                continue;
            }

            let ranks = self
                .tree_routing_nodes
                .iter()
                .map(|(&node, state)| (node, state.extraction_order))
                .collect::<BTreeMap<_, _>>();
            let mut by_direction = BTreeMap::<TreeEdgeDirection, Vec<PlacementTree>>::new();
            for root in raw_roots {
                let direction = self.dominant_extracted_direction_excluding_root(
                    root.node,
                    &self.tree_routing_nodes,
                    &ranks,
                );
                by_direction.entry(direction).or_default().push(root);
            }
            let undirected = by_direction
                .remove(&TreeEdgeDirection::Undirected)
                .unwrap_or_default();
            for matching_roots in by_direction.into_values() {
                let mut by_arrowhead =
                    BTreeMap::<(bool, Option<String>), Vec<PlacementTree>>::new();
                for root in matching_roots {
                    let arrowhead = self.edge_arrowhead_towards(
                        root.sentinel_edge.expect("tree root edge"),
                        sentinel,
                    );
                    by_arrowhead.entry(arrowhead).or_default().push(root);
                }
                result.extend(by_arrowhead.into_values().map(|children| PlacementTree {
                    node: sentinel,
                    sentinel_edge: None,
                    children,
                    orientation: Orientation::None,
                }));
            }
            if !undirected.is_empty() {
                let candidate = result
                    .iter()
                    .enumerate()
                    .filter(|(_, tree)| tree.node == sentinel)
                    .filter(|(_, tree)| {
                        let root = &tree.children[0];
                        !self
                            .edge_arrowhead_towards(
                                root.sentinel_edge.expect("tree root edge"),
                                sentinel,
                            )
                            .0
                    })
                    .max_by_key(|(_, tree)| tree.children.len())
                    .map(|(index, _)| index);
                if let Some(index) = candidate {
                    result[index].children.extend(undirected);
                } else {
                    result.push(PlacementTree {
                        node: sentinel,
                        sentinel_edge: None,
                        children: undirected,
                        orientation: Orientation::None,
                    });
                }
            }
        }
        result
    }

    fn tree_levels(tree: &PlacementTree) -> Vec<Vec<NodeId>> {
        fn visit(tree: &PlacementTree, level: usize, levels: &mut Vec<Vec<NodeId>>) {
            if levels.len() == level {
                levels.push(Vec::new());
            }
            levels[level].push(tree.node);
            for child in &tree.children {
                visit(child, level + 1, levels);
            }
        }
        let mut levels = Vec::new();
        visit(tree, 0, &mut levels);
        levels
    }

    fn translate_tree_subtree(&mut self, tree: &PlacementTree, delta: Point) {
        self.translate_node_with_children(tree.node, delta);
        for child in &tree.children {
            self.translate_tree_subtree(child, delta);
        }
    }

    fn move_tree_node_abs(&mut self, node: NodeId, target: Point) {
        self.move_node_abs_with_children(node, target);
    }

    fn shift_tree_horizontally(&mut self, tree: &PlacementTree, delta: f64) {
        self.translate_tree_subtree(tree, Point { x: delta, y: 0.0 });
    }

    fn tree_level_lefts(&self, tree: &PlacementTree) -> Vec<f64> {
        Self::tree_levels(tree)
            .into_iter()
            .map(|level| {
                level
                    .into_iter()
                    .map(|node| self.position(node).expect("positioned tree node").x)
                    .fold(f64::INFINITY, f64::min)
            })
            .collect()
    }

    fn tree_level_rights(&self, tree: &PlacementTree) -> Vec<f64> {
        Self::tree_levels(tree)
            .into_iter()
            .map(|level| {
                level
                    .into_iter()
                    .map(|node| {
                        self.position(node).expect("positioned tree node").x
                            + self.nodes[node.0 as usize].rect.size.width
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            })
            .collect()
    }

    fn spacing_before(&self, tree: &PlacementTree, child_index: usize) -> f64 {
        let rights = self.tree_level_rights(&tree.children[child_index - 1]);
        let lefts = self.tree_level_lefts(&tree.children[child_index]);
        rights
            .into_iter()
            .zip(lefts)
            .map(|(right, left)| left - right)
            .fold(f64::INFINITY, f64::min)
    }

    fn total_children_spacing(&self, tree: &PlacementTree) -> f64 {
        tree.children
            .windows(2)
            .map(|pair| {
                let left = pair[0].node;
                let right = pair[1].node;
                self.position(right).expect("positioned child").x
                    - (self.position(left).expect("positioned child").x
                        + self.nodes[left.0 as usize].rect.size.width)
            })
            .sum()
    }

    fn space_children_evenly(&mut self, tree: &PlacementTree) {
        if tree.children.len() < 2 {
            return;
        }
        let mut max_spacing: f64 = 50.0;
        let mut child_spacing = vec![0.0];
        for pair in tree.children.windows(2) {
            let first = pair[0].node;
            let second = pair[1].node;
            let spacing = self.position(second).expect("positioned child").x
                - (self.position(first).expect("positioned child").x
                    + self.nodes[first.0 as usize].rect.size.width);
            max_spacing = max_spacing.max(spacing);
            child_spacing.push(spacing);
        }
        for index in 1..tree.children.len() {
            let shift = max_spacing - child_spacing[index];
            if shift == 0.0 {
                continue;
            }
            if self.spacing_before(tree, index) + shift > 500.0 {
                return;
            }
            if index + 1 < tree.children.len() {
                child_spacing[index + 1] -= shift;
            }
        }
        for (index, spacing) in child_spacing.iter().copied().enumerate().skip(1) {
            let shift = max_spacing - spacing;
            if shift > 0.0 {
                self.shift_tree_horizontally(&tree.children[index], shift);
            }
        }
    }

    fn children_center_x(&self, tree: &PlacementTree) -> f64 {
        tree.children
            .iter()
            .map(|child| {
                self.position(child.node).expect("positioned child").x
                    + self.nodes[child.node.0 as usize].rect.size.width / 2.0
            })
            .sum::<f64>()
            / tree.children.len() as f64
    }

    fn spacing_to_children(&self, tree: &PlacementTree) -> f64 {
        let max_label_size = tree
            .children
            .iter()
            .filter_map(|child| child.sentinel_edge)
            .map(|edge| {
                let edge = &self.edge_records[edge.0 as usize];
                if matches!(tree.orientation, Orientation::Left | Orientation::Right) {
                    edge.min_width
                } else {
                    edge.min_height
                }
            })
            .fold(0.0_f64, f64::max);
        if max_label_size == 0.0 {
            return 100.0;
        }
        let with_arrowheads = max_label_size + 40.0;
        if tree.children.len() == 1 {
            100.0_f64.max(with_arrowheads)
        } else {
            50.0_f64.max(with_arrowheads) + 50.0
        }
    }

    fn layout_tree(&mut self, tree: &PlacementTree) {
        let levels = Self::tree_levels(tree);
        let level_heights = levels
            .iter()
            .map(|level| {
                level
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].rect.size.height)
                    .fold(0.0_f64, f64::max)
            })
            .collect::<Vec<_>>();
        let mut level_spacings = vec![0.0_f64; levels.len()];
        for (level, nodes) in levels
            .iter()
            .enumerate()
            .take(levels.len().saturating_sub(1))
        {
            for node in nodes {
                let subtree = Self::find_tree(tree, *node).expect("tree level node");
                level_spacings[level + 1] =
                    level_spacings[level + 1].max(self.spacing_to_children(subtree));
            }
        }
        let mut level_bottom = 0.0;
        for level in 1..levels.len() {
            let level_top = level_bottom + level_spacings[level];
            for node in &levels[level] {
                let center_offset =
                    ((level_heights[level] - self.nodes[node.0 as usize].rect.size.height) / 2.0)
                        .round();
                let x = self.position(*node).unwrap_or_default().x;
                self.move_tree_node_abs(
                    *node,
                    Point {
                        x,
                        y: level_top + center_offset,
                    },
                );
            }
            level_bottom = level_top + level_heights[level];
        }

        for level in (1..levels.len()).rev() {
            let mut next_position = 0.0;
            for node in &levels[level] {
                let subtree = Self::find_tree(tree, *node).expect("tree level node");
                if subtree.children.is_empty() {
                    let y = self.position(*node).expect("positioned tree node").y;
                    self.move_tree_node_abs(
                        *node,
                        Point {
                            x: next_position,
                            y,
                        },
                    );
                } else {
                    let last_child_position = levels[level + 1]
                        .iter()
                        .rposition(|candidate| {
                            Self::find_tree(tree, *candidate).is_some_and(|candidate| {
                                candidate.sentinel_edge.and_then(|_| {
                                    self.tree_routing_nodes
                                        .get(&candidate.node)
                                        .map(|state| state.parent)
                                }) == Some(*node)
                            })
                        })
                        .unwrap_or(levels[level + 1].len());
                    let spacing_before = self.total_children_spacing(subtree);
                    self.space_children_evenly(subtree);
                    let total_shift = self.total_children_spacing(subtree) - spacing_before;
                    if total_shift > 0.0 {
                        for later in levels[level + 1].iter().skip(last_child_position + 1) {
                            let later_tree = Self::find_tree(tree, *later).expect("later tree");
                            self.shift_tree_horizontally(later_tree, total_shift);
                        }
                    }
                    let sibling_center = self.children_center_x(subtree)
                        - self.nodes[node.0 as usize].rect.size.width / 2.0;
                    let y = self.position(*node).expect("positioned tree node").y;
                    self.move_tree_node_abs(
                        *node,
                        Point {
                            x: sibling_center.round(),
                            y,
                        },
                    );
                    let diff = next_position - self.position(*node).expect("positioned").x;
                    if diff > 0.0 {
                        self.shift_tree_horizontally(subtree, diff);
                        for later in levels[level + 1].iter().skip(last_child_position + 1) {
                            let later_tree = Self::find_tree(tree, *later).expect("later tree");
                            self.shift_tree_horizontally(later_tree, diff);
                        }
                    }
                }
                next_position = self.position(*node).expect("positioned").x
                    + self.nodes[node.0 as usize].rect.size.width
                    + 50.0;
            }
        }
    }

    fn find_tree(tree: &PlacementTree, node: NodeId) -> Option<&PlacementTree> {
        if tree.node == node {
            return Some(tree);
        }
        tree.children
            .iter()
            .find_map(|child| Self::find_tree(child, node))
    }

    fn reconnect_tree_edge_order(&mut self, tree: &PlacementTree) {
        if let Some(edge) = tree.sentinel_edge
            && !self.edge_order.contains(&edge)
        {
            self.reconnect_edge_adjacency_order(edge);
            self.edge_order.push(edge);
        }
        for child in &tree.children {
            self.reconnect_tree_edge_order(child);
        }
    }

    /// Publish retained tree edges back to the owning graph after a temporary
    /// placement subgraph has called `PlaceTrees`.
    ///
    /// Recovered `Graph.placeNodes` does not copy the temporary graph's edge
    /// slice wholesale. It visits the temporary graph's nodes, then each
    /// sentinel's roots in their extraction order. For each root it appends
    /// `trees.Descendants(root)` (breadth-first) followed by the root itself.
    /// This owner-copyback order differs from the sorted placement-tree
    /// preorder used inside the temporary graph.
    pub(super) fn publish_placed_tree_edges(
        &mut self,
        component: &[NodeId],
        reconnects_in_owner: bool,
    ) {
        #[cfg(feature = "diagnostic-traces")]
        self.trace_tree_copyback(
            "before",
            component
                .first()
                .and_then(|n| self.nodes[n.0 as usize].container),
        );
        #[cfg(feature = "diagnostic-traces")]
        let trace_publish = crate::internal::trace_env_enabled("TALA_TRACE_TREE_PUBLISH");
        let (reconnect_nodes, reconnect_edges) =
            self.placement_tree_reconnect_nodes_and_edges(component);
        let (published_nodes, published_edges) = if reconnects_in_owner {
            (reconnect_nodes.clone(), reconnect_edges.clone())
        } else {
            self.placement_tree_owner_copyback_nodes_and_edges(component)
        };
        for edge in reconnect_edges {
            self.reconnect_owner_tree_edge(edge);
            if !self.published_tree_node_edge_order.contains(&edge) {
                self.published_tree_node_edge_order.push(edge);
            }
        }
        for edge in published_edges.iter().copied() {
            if !self.edge_order.contains(&edge) {
                self.edge_order.push(edge);
            }
        }
        if !published_nodes.is_empty() {
            let published = published_nodes.iter().copied().collect::<BTreeSet<_>>();
            self.node_order.retain(|node| !published.contains(node));
            self.node_order.extend(published_nodes.iter().copied());
            // reconnectTree appends Containers in placement preorder. The
            // owner Graph.Nodes publication above has a different order and
            // must not replace this live header or its backing allocation.
            if let Some(scope) = component
                .first()
                .map(|node| self.nodes[node.0 as usize].container)
            {
                for node in reconnect_nodes {
                    if self.nodes[node.0 as usize].container == scope
                        && !self
                            .containers
                            .get(&scope)
                            .unwrap_or_default()
                            .contains(&node)
                    {
                        self.containers.append_node(scope, node);
                    }
                }
            }
            if self.node_order_membership_valid {
                self.refresh_node_order_membership();
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if trace_publish {
            eprintln!(
                "TREE_PUBLISH_AFTER nodes={:?} edges={:?}",
                published_nodes
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                published_edges
                    .iter()
                    .map(|edge| self.edge_records[edge.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
            );
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(crate) fn trace_tree_copyback(&self, phase: &str, scope: Option<NodeId>) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_TREE_COPYBACK") {
            return;
        }
        let id = |n| self.active_node_tala_id(n).to_string();
        let children = self.containers.get_header(&scope).unwrap_or_default();
        eprintln!(
            "{}",
            serde_json::json!({"event":"tree-copyback", "phase":phase,
            "scope":scope.map_or_else(|| "0".to_owned(),id),
            "nodes":self.graph_node_order().into_iter().map(id).collect::<Vec<_>>(),
            "children":children.iter_slots().map(|n|n.map(id)).collect::<Vec<_>>(),
            "capacity":children.capacity(), "nil":children.is_nil()})
        );
    }

    fn flip_tree(&mut self, tree: &PlacementTree) {
        let position = self.position(tree.node).unwrap_or_default();
        let size = self.nodes[tree.node.0 as usize].rect.size;
        self.move_tree_node_abs(
            tree.node,
            Point {
                x: -(position.x + size.width),
                y: -(position.y + size.height),
            },
        );
        for child in &tree.children {
            self.flip_tree(child);
        }
    }

    fn swap_tree_dimensions(&mut self, tree: &PlacementTree) {
        let position = self.position(tree.node).unwrap_or_default();
        self.move_tree_node_abs(
            tree.node,
            Point {
                x: position.y,
                y: position.x,
            },
        );
        let node = &mut self.nodes[tree.node.0 as usize];
        std::mem::swap(&mut node.rect.size.width, &mut node.rect.size.height);
        for child in &tree.children {
            self.swap_tree_dimensions(child);
        }
    }

    fn invert_tree_to_bottom(&mut self, tree: &PlacementTree, orientation: Orientation) {
        match orientation {
            Orientation::Top => self.flip_tree(tree),
            Orientation::Right => self.swap_tree_dimensions(tree),
            Orientation::Left => {
                self.swap_tree_dimensions(tree);
                self.flip_tree(tree);
            }
            _ => {}
        }
    }

    /// Direct translation of recovered `Tree.positionEdgeLabels`.
    ///
    /// The caller has temporarily transformed the complete placement tree
    /// back to Bottom geometry while retaining each tree node's final
    /// orientation. That combination is intentional: geometry determines the
    /// percentage along the orthogonal sentinel edge, while the stored
    /// orientation determines which dimension and final mirrored side apply.
    fn position_tree_edge_labels(
        &mut self,
        tree: &PlacementTree,
        parent: NodeId,
        sibling_count: usize,
    ) {
        for child in &tree.children {
            self.position_tree_edge_labels(child, tree.node, tree.children.len());
        }

        let Some(edge_id) = tree.sentinel_edge else {
            return;
        };
        let edge = &self.edge_records[edge_id.0 as usize];
        if edge.label.is_none() {
            return;
        }
        let label_size = if matches!(tree.orientation, Orientation::Left | Orientation::Right) {
            edge.min_width
        } else {
            edge.min_height
        };
        if label_size == 0.0 {
            return;
        }

        let (position, percentage) = if sibling_count == 1 {
            (LabelPosition::UnlockedMiddle, 0.5)
        } else {
            let child_position = self.position(tree.node).expect("positioned tree child");
            let child_size = self.nodes[tree.node.0 as usize].rect.size;
            let parent_position = self.position(parent).expect("positioned tree parent");
            let parent_size = self.nodes[parent.0 as usize].rect.size;
            let child_center_x = child_position.x + child_size.width * 0.5;
            let parent_center_x = parent_position.x + parent_size.width * 0.5;
            let dx = child_center_x - parent_center_x;
            let dy = child_position.y - (parent_position.y + parent_size.height);
            let total_length = dx.abs() + dy;
            let child_segment_length = (dy - 50.0) * 0.5;
            let edge_from_child = edge.from == tree.node;
            let distance_along_edge = if edge_from_child {
                child_segment_length
            } else {
                total_length - child_segment_length
            };
            let mut position = if dx < 0.0 {
                LabelPosition::UnlockedBottom
            } else {
                LabelPosition::UnlockedTop
            };
            if edge_from_child {
                position = position.mirrored();
            }
            if matches!(tree.orientation, Orientation::Left | Orientation::Right) {
                position = position.mirrored();
            }
            (position, distance_along_edge / total_length)
        };

        let label = self.edge_records[edge_id.0 as usize]
            .label
            .as_mut()
            .expect("checked tree edge label");
        label.position = position;
        label.percentage = percentage;
    }

    fn construct_tree_to_orientation(
        &mut self,
        tree: &mut PlacementTree,
        orientation: Orientation,
    ) {
        tree.set_orientation(orientation);
        self.invert_tree_to_bottom(tree, orientation);
        self.layout_tree(tree);
        if !tree.children.is_empty() {
            let center = self.children_center_x(tree);
            let position = self.position(tree.node).unwrap_or_default();
            let width = self.nodes[tree.node.0 as usize].rect.size.width;
            let offset = (position.x + width / 2.0 - center).round();
            for child in &tree.children {
                self.translate_tree_subtree(child, Point { x: offset, y: 0.0 });
            }
        }
        self.invert_tree_to_bottom(tree, orientation);
    }

    fn tree_border_position(&self, node: NodeId, orientation: Orientation) -> f64 {
        let position = self.position(node).expect("positioned placement node");
        let size = self.nodes[node.0 as usize].rect.size;
        match orientation {
            Orientation::Top => position.y,
            Orientation::Right => position.x + size.width,
            Orientation::Left => position.x,
            _ => position.y + size.height,
        }
    }

    fn move_placement_tree(
        &mut self,
        tree: &PlacementTree,
        orientation: Orientation,
        from: f64,
        to: f64,
    ) {
        let amount = (to - from).round();
        let delta = if matches!(orientation, Orientation::Left | Orientation::Right) {
            Point { x: amount, y: 0.0 }
        } else {
            Point { x: 0.0, y: amount }
        };
        for child in &tree.children {
            self.translate_tree_subtree(child, delta);
        }
    }

    fn normalized_aspect_ratio_for(&self, nodes: &BTreeSet<NodeId>) -> f64 {
        let Some((top_left, bottom_right)) =
            self.bin_pack_bounding_box(&nodes.iter().copied().collect::<Vec<_>>())
        else {
            return 1.0;
        };
        let width = bottom_right.x - top_left.x;
        let height = bottom_right.y - top_left.y;
        if width >= height {
            width / height.max(f64::EPSILON)
        } else {
            height / width.max(f64::EPSILON)
        }
    }

    fn place_at_orientation(
        &mut self,
        tree: &PlacementTree,
        orientation: Orientation,
        active_nodes: &BTreeSet<NodeId>,
    ) -> f64 {
        let root_position = self.tree_border_position(tree.node, orientation);
        let mut border_position = root_position;
        for node in active_nodes
            .iter()
            .copied()
            .filter(|node| !tree.contains(*node))
        {
            let position = self.tree_border_position(node, orientation);
            let beyond = if matches!(orientation, Orientation::Left | Orientation::Top) {
                position < border_position
            } else {
                position > border_position
            };
            if beyond {
                border_position = position;
            }
        }
        let mut current_best_position = border_position;
        let mut current_best_distance = (border_position - root_position).abs();
        self.move_placement_tree(tree, orientation, 0.0, border_position);

        if let Some(origin) =
            self.container_fixed_origin(self.nodes[tree.node.0 as usize].container)
        {
            let mut descendants = vec![tree.node];
            tree.descendants(&mut descendants);
            if descendants.into_iter().any(|node| {
                self.position(node)
                    .is_some_and(|position| position.x < origin.x || position.y < origin.y)
            }) {
                current_best_distance = f64::INFINITY;
            }
        }

        let mut obstacles = active_nodes
            .iter()
            .copied()
            .filter(|node| !tree.contains(*node))
            .map(|node| self.tree_border_position(node, orientation))
            .filter(|position| {
                if matches!(orientation, Orientation::Left | Orientation::Top) {
                    border_position < *position && *position < root_position
                } else {
                    border_position > *position && *position > root_position
                }
            })
            .collect::<Vec<_>>();
        obstacles.push(root_position);

        for obstacle in obstacles {
            let existing_overlaps = self.existing_overlap_pairs();
            let existing_exact = self.exact_overlap_pairs();
            let mut trial = self.clone();
            trial.move_placement_tree(tree, orientation, current_best_position, obstacle);
            if crate::internal::trace_env_enabled("TALA_TRACE_TREE_CANDIDATES")
                && self.nodes[tree.node.0 as usize].tala_id == 3463334650
            {
                eprintln!(
                    "TREE_CAND_RUST root={} orientation={:?} obstacle={} current={} root_pos={:?} child_pos={:?} overlap={}",
                    self.nodes[tree.node.0 as usize].tala_id,
                    orientation,
                    obstacle,
                    current_best_position,
                    self.position(tree.node),
                    tree.children
                        .first()
                        .and_then(|child| self.position(child.node)),
                    trial.transaction_has_new_overlap(&existing_overlaps),
                );
            }
            // `Graph.placeTree` reconnects the peeled tree nodes into
            // `Graph.Nodes` before validating each transaction. The compact
            // arena keeps those nodes out of the ordinary active order until
            // publication, so the generic transaction predicate cannot see a
            // moved tree descendant colliding with a live core node. Recheck
            // those pairs explicitly with the same spacing predicate before
            // accepting the candidate.
            let mut moved_tree_nodes = Vec::new();
            tree.descendants(&mut moved_tree_nodes);
            let mut tree_has_new_overlap = false;
            'tree_pairs: for moved in moved_tree_nodes {
                let Some(a) = trial.position(moved) else {
                    continue;
                };
                let left_size = trial.active_node_size(moved);
                for other in active_nodes.iter().copied() {
                    if tree.contains(other) {
                        continue;
                    }
                    let Some(b) = trial.position(other) else {
                        continue;
                    };
                    let pair = if moved < other {
                        (moved, other)
                    } else {
                        (other, moved)
                    };
                    if existing_overlaps.contains(&pair) {
                        continue;
                    }
                    let right_size = trial.active_node_size(other);
                    let delta = trial.spacing_delta_with_loops(moved, other, a);
                    if a.x < b.x + right_size.width + delta
                        && b.x < a.x + left_size.width + delta
                        && a.y < b.y + right_size.height + delta
                        && b.y < a.y + left_size.height + delta
                    {
                        tree_has_new_overlap = true;
                        break 'tree_pairs;
                    }
                }
            }
            if tree_has_new_overlap
                || trial.transaction_has_new_overlap(&existing_overlaps)
                || trial.existing_spacing_overlap_became_exact(&existing_overlaps, &existing_exact)
                || !trial.transaction_containment_is_valid()
                || !trial.transaction_external_containers_are_valid()
            {
                continue;
            }
            let distance = (obstacle - root_position).abs();
            if distance < current_best_distance {
                current_best_distance = distance;
                current_best_position = obstacle;
                *self = trial;
            }
        }
        current_best_distance
    }

    pub(super) fn tree_placement_direction(&self, node: NodeId) -> Orientation {
        self.scoring_directions
            .get(&self.nodes[node.0 as usize].container)
            .copied()
            .map(layout_orientation)
            .unwrap_or(Orientation::None)
    }

    fn place_one_tree(
        &mut self,
        tree: &mut PlacementTree,
        existing: &[Orientation],
        active_nodes: &mut BTreeSet<NodeId>,
    ) -> Orientation {
        let mut descendants = Vec::new();
        tree.descendants(&mut descendants);
        for node in &descendants {
            self.set_position(*node, Point::default());
            active_nodes.insert(*node);
        }

        let default_order = [
            Orientation::Bottom,
            Orientation::Top,
            Orientation::Left,
            Orientation::Right,
        ];
        let isolated = self.tree_is_isolated(tree.node, &tree.children);
        let (best, best_distance, best_ratio) = if existing.len() == 1 {
            (
                existing
                    .iter()
                    .next()
                    .copied()
                    .unwrap_or(Orientation::Bottom)
                    .opposite(),
                f64::INFINITY,
                f64::INFINITY,
            )
        } else {
            let mut order = default_order.to_vec();
            if isolated {
                order.retain(|orientation| !existing.contains(orientation));
                if order.is_empty() {
                    order = default_order.to_vec();
                }
            }
            // `Graph.GetDirection(container)` returns `geo.NONE` when the
            // placement scope has no explicit direction. The adapter's
            // ordinary `directions` map carries a Down fallback for layout,
            // which must not become placeTree's 200-point preference.
            let requested = self.tree_placement_direction(tree.node);
            let mut best_orientation = Orientation::None;
            let mut best_distance = f64::INFINITY;
            let mut best_ratio = f64::INFINITY;
            for orientation in order {
                self.construct_tree_to_orientation(tree, orientation);
                let mut distance = self.place_at_orientation(tree, orientation, active_nodes);
                let ratio = self.normalized_aspect_ratio_for(active_nodes);
                self.trace_tree_placement_event(
                    "tree-trial",
                    tree,
                    orientation,
                    distance,
                    ratio,
                    &descendants,
                );
                if crate::internal::trace_env_enabled("TALA_TRACE_TREE_TRIALS") {
                    eprintln!(
                        "TREE_TRY_RUST root={} orientation={:?} distance={} ratio={} requested={:?}",
                        self.nodes[tree.node.0 as usize].tala_id,
                        orientation,
                        distance,
                        ratio,
                        requested,
                    );
                }
                if requested != orientation {
                    distance += 200.0;
                }
                if distance < best_distance || (distance == best_distance && ratio < best_ratio) {
                    best_distance = distance;
                    best_ratio = ratio;
                    best_orientation = orientation;
                }
            }
            (best_orientation, best_distance, best_ratio)
        };
        self.construct_tree_to_orientation(tree, best);
        self.place_at_orientation(tree, best, active_nodes);
        self.trace_tree_placement_event(
            "tree-selected",
            tree,
            best,
            best_distance,
            best_ratio,
            &descendants,
        );
        if crate::internal::trace_env_enabled("TALA_TRACE_TREE_PLACEMENT") {
            eprintln!(
                "TREE_PLACE_RUST node={} existing={:?} requested={:?} best={:?}",
                self.nodes[tree.node.0 as usize].tala_id,
                existing,
                self.tree_placement_direction(tree.node),
                best,
            );
        }
        tree.set_orientation(best);
        // Recovered `placeTree` temporarily restores Bottom geometry, then
        // positions labels either on each isolated root or below the first
        // non-isolated root. The second inversion restores final geometry.
        self.invert_tree_to_bottom(tree, best);
        for root in &tree.children {
            if isolated {
                self.position_tree_edge_labels(root, tree.node, tree.children.len());
            } else {
                for child in &root.children {
                    self.position_tree_edge_labels(child, root.node, root.children.len());
                }
            }
        }
        self.invert_tree_to_bottom(tree, best);
        for node in descendants {
            if let Some(state) = self.tree_routing_nodes.get_mut(&node) {
                // `placeTree` publishes the chosen orientation to the whole
                // placement tree. `Graph.direct`/`mirrorAxes` may then flip
                // that carrier when the component is reflected; preserve the
                // pre-mirror value here so routing observes the same state.
                state.orientation = best;
                self.publish_tree_orientation(node, best);
            }
        }
        best
    }

    /// Recovered `Graph.PlaceTrees`, called after ordinary placement and
    /// before component packing.
    ///
    /// Return the node order produced by `reconnectTree` for the placement
    /// trees belonging to component. Go appends these nodes to the temporary
    /// graph's `Nodes` slice in sorted placement-tree preorder. The stable
    /// arena keeps all tree nodes allocated, so callers that materialize a
    /// temporary graph need this order explicitly instead of iterating the
    /// dense node IDs.
    pub(super) fn placement_tree_reconnect_order(&self, component: &[NodeId]) -> Vec<NodeId> {
        self.placement_tree_reconnect_nodes_and_edges(component).0
    }

    fn placement_tree_reconnect_nodes_and_edges(
        &self,
        component: &[NodeId],
    ) -> (Vec<NodeId>, Vec<EdgeId>) {
        let component_set = component.iter().copied().collect::<BTreeSet<_>>();
        let mut trees = self.placement_trees_for_component(&component_set);
        trees.sort_by(|left, right| {
            right.size().cmp(&left.size()).then_with(|| {
                self.nodes[left.children[0].node.0 as usize]
                    .tala_id
                    .cmp(&self.nodes[right.children[0].node.0 as usize].tala_id)
            })
        });
        fn append_preorder(tree: &PlacementTree, nodes: &mut Vec<NodeId>, edges: &mut Vec<EdgeId>) {
            for child in &tree.children {
                nodes.push(child.node);
                if let Some(edge) = child.sentinel_edge {
                    edges.push(edge);
                }
                append_preorder(child, nodes, edges);
            }
        }

        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for tree in trees {
            append_preorder(&tree, &mut nodes, &mut edges);
        }
        (nodes, edges)
    }

    fn placement_tree_owner_copyback_nodes_and_edges(
        &self,
        component: &[NodeId],
    ) -> (Vec<NodeId>, Vec<EdgeId>) {
        let mut children = BTreeMap::<NodeId, Vec<NodeId>>::new();
        let mut roots = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for (&node, state) in &self.tree_routing_nodes {
            if self.tree_routing_nodes.contains_key(&state.parent) {
                children.entry(state.parent).or_default().push(node);
            } else if component.contains(&state.parent) {
                roots.entry(state.parent).or_default().push(node);
            }
        }
        for nodes in children.values_mut().chain(roots.values_mut()) {
            nodes.sort_by_key(|node| {
                self.tree_routing_nodes
                    .get(node)
                    .map_or(usize::MAX, |state| state.extraction_order)
            });
        }

        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for sentinel in component {
            let Some(root_nodes) = roots.get(sentinel) else {
                continue;
            };
            for root in root_nodes {
                // Go's trees.Descendants is breadth-first and excludes its
                // argument. Graph.placeNodes appends that sequence before the
                // root, preserving every node's sentinel edge alongside it.
                let mut queue = vec![*root];
                let mut cursor = 0;
                let mut descendants = Vec::new();
                while cursor < queue.len() {
                    let current = queue[cursor];
                    cursor += 1;
                    if let Some(current_children) = children.get(&current) {
                        descendants.extend(current_children.iter().copied());
                        queue.extend(current_children.iter().copied());
                    }
                }
                for node in descendants.into_iter().chain(std::iter::once(*root)) {
                    nodes.push(node);
                    edges.push(self.tree_routing_nodes[&node].sentinel_edge);
                }
            }
        }
        (nodes, edges)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(crate) fn placement_tree_owner_copyback_order(&self, component: &[NodeId]) -> Vec<NodeId> {
        self.placement_tree_owner_copyback_nodes_and_edges(component)
            .0
    }

    pub(super) fn place_trees(&mut self, component: &[NodeId]) {
        let component_set = component.iter().copied().collect::<BTreeSet<_>>();
        let mut trees = self.placement_trees_for_component(&component_set);
        if crate::internal::trace_env_enabled("TALA_TRACE_TREE_GROUPS") {
            for tree in &trees {
                eprint!(
                    "TREE_GROUP_RUST root={} children=",
                    self.nodes[tree.node.0 as usize].tala_id
                );
                for child in &tree.children {
                    eprint!("{},", self.nodes[child.node.0 as usize].tala_id);
                }
                eprintln!();
            }
        }
        trees.sort_by(|left, right| {
            right.size().cmp(&left.size()).then_with(|| {
                // GraphJSON carries the same durable node ID used by the Go
                // layoutgraph Node.ID comparator. It is preserved as tala_id
                // in the arena; dense NodeId is only an allocation index.
                self.nodes[left.children[0].node.0 as usize]
                    .tala_id
                    .cmp(&self.nodes[right.children[0].node.0 as usize].tala_id)
            })
        });
        let mut active_nodes = component_set.clone();
        let mut placed = BTreeMap::<NodeId, Vec<Orientation>>::new();
        for tree in &mut trees {
            // Recovered `reconnectTree` appends the root edge and then walks
            // child trees recursively before `placeTree` evaluates any
            // orientation.
            self.reconnect_tree_edge_order(tree);
            let existing = placed.entry(tree.node).or_default().clone();
            let orientation = self.place_one_tree(tree, &existing, &mut active_nodes);
            placed.entry(tree.node).or_default().push(orientation);
        }
        // `active_nodes` is a set for membership checks, not a recovered
        // Graph.Nodes traversal. Go appends each retained tree node while
        // reconnectTree walks the already sorted placement trees in preorder.
        // Preserve that order here; dense arena order can differ from source
        // tree order and changes later packing, routing, and trace events.
        fn append_preorder(tree: &PlacementTree, output: &mut Vec<NodeId>) {
            for child in &tree.children {
                output.push(child.node);
                append_preorder(child, output);
            }
        }
        let mut reconnected_nodes = Vec::new();
        for tree in &trees {
            append_preorder(tree, &mut reconnected_nodes);
        }
        let mut published = BTreeSet::new();
        for node in reconnected_nodes {
            if !active_nodes.contains(&node)
                || component_set.contains(&node)
                || !published.insert(node)
            {
                continue;
            }
            if !self.node_order.contains(&node) {
                self.node_order.push(node);
            }
            if !self
                .containers
                .get(&None)
                .unwrap_or_default()
                .contains(&node)
            {
                self.containers.append_node(None, node);
            }
        }
        // reconnectTree changes component membership before CombineSubgraphs;
        // refresh the stable-ID carrier after publishing those nodes/edges.
        self.refresh_placement_components();
    }

    pub(super) fn is_tree_sentinel(&self, node: NodeId) -> bool {
        self.tree_sentinels.contains(&node)
    }
}

#[cfg(test)]
mod geometry_crosswalk_tests {
    use super::*;
    use crate::{ContentAlignment, Insets, LabelPosition, Node, ShapeKind, Size};

    fn node(name: &str, width: f64, height: f64) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size { width, height },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: None,
            constrained_x: None,
            constrained_y: None,
            near: None,
            fixed_width: false,
            fixed_height: false,
            direction: None,
            force_hierarchy: false,
            grid_rows: None,
            grid_columns: None,
            canvas_position: None,
            content_insets: Insets::uniform(60.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }

    fn leaf(node: NodeId) -> PlacementTree {
        PlacementTree {
            node,
            sentinel_edge: None,
            children: Vec::new(),
            orientation: Orientation::Bottom,
        }
    }

    #[test]
    fn tree_container_removal_uses_fresh_header_and_preserves_old_capacity_slots() {
        let arena = ArenaGraph::from_input(&Graph::default());
        let removed = NodeId(1);
        let survivor = NodeId(2);
        let captured = SharedNodeSlice::with_len_cap(3, 5).unwrap();
        for (index, child) in [
            Some(removed),
            None,
            Some(survivor),
            Some(removed),
            Some(survivor),
        ]
        .into_iter()
        .enumerate()
        {
            captured.write_capacity_slot(index, child).unwrap();
        }
        arena.containers.replace_header(None, captured.clone());
        assert!(arena.remove_tree_container_node(None, removed));
        let filtered = arena.containers.get_header(&None).unwrap();
        assert_eq!(filtered.visible_slots(), vec![None, Some(survivor)]);
        assert!(!filtered.same_backing_allocation(&captured));
        assert_eq!(
            captured.capacity_slots(),
            vec![
                Some(removed),
                None,
                Some(survivor),
                Some(removed),
                Some(survivor)
            ]
        );
        assert!(!arena.remove_tree_container_node(None, NodeId(9)));
        assert!(filtered.same_backing_allocation(&arena.containers.get_header(&None).unwrap()));

        arena
            .containers
            .replace_header(None, SharedNodeSlice::from_nodes(&[removed]).unwrap());
        assert!(arena.remove_tree_container_node(None, removed));
        let empty = arena.containers.get_header(&None).unwrap();
        assert!(!empty.is_nil());
        assert_eq!((empty.len(), empty.capacity()), (0, 0));
    }

    #[test]
    fn tree_sentinel_promotion_is_fresh_and_reconnection_appends_into_its_spare_slots() {
        let arena = ArenaGraph::from_input(&Graph::default());
        let old = NodeId(1);
        let survivor = NodeId(2);
        let promoted = NodeId(3);
        let restored = NodeId(4);
        let original = SharedNodeSlice::from_nodes(&[old, survivor]).unwrap();
        arena.containers.replace_header(None, original.clone());
        arena.promote_tree_container_sentinel(None, old, promoted);
        let promoted_header = arena.containers.get_header(&None).unwrap();
        assert_eq!(
            promoted_header.visible_slots(),
            vec![Some(survivor), Some(promoted)]
        );
        assert_eq!(original.visible_slots(), vec![Some(old), Some(survivor)]);
        assert!(!original.same_backing_allocation(&promoted_header));
        // Source append growth is tested by ContainerMap; use actual spare
        // capacity here to verify the restoration writer keeps header copies.
        let spare = SharedNodeSlice::with_len_cap(2, 4).unwrap();
        spare.write_capacity_slot(0, Some(survivor)).unwrap();
        spare.write_capacity_slot(1, Some(promoted)).unwrap();
        arena.containers.replace_header(None, spare.clone());
        arena.containers.append_node(None, restored);
        assert_eq!(spare.len(), 2);
        assert_eq!(spare.read_capacity_slot(2), Some(Some(restored)));
        assert!(spare.same_backing_allocation(&arena.containers.get_header(&None).unwrap()));
    }

    #[test]
    fn descendant_order_matches_upstream_breadth_first_walk() {
        let left_leaf = leaf(NodeId(4));
        let right_leaf = leaf(NodeId(5));
        let tree = PlacementTree {
            node: NodeId(0),
            sentinel_edge: None,
            children: vec![
                PlacementTree {
                    node: NodeId(1),
                    sentinel_edge: None,
                    children: vec![left_leaf],
                    orientation: Orientation::Bottom,
                },
                PlacementTree {
                    node: NodeId(2),
                    sentinel_edge: None,
                    children: vec![right_leaf],
                    orientation: Orientation::Bottom,
                },
            ],
            orientation: Orientation::Bottom,
        };

        let mut descendants = Vec::new();
        tree.descendants(&mut descendants);

        assert_eq!(
            descendants,
            vec![NodeId(1), NodeId(2), NodeId(4), NodeId(5)]
        );
    }

    #[test]
    fn tree_traversal_level_extrema_and_adjacent_spacing_match_recovered_helpers() {
        let mut input = Graph::default();
        let root = input.add_node(node("root", 50.0, 40.0));
        let left = input.add_node(node("left", 30.0, 20.0));
        let right = input.add_node(node("right", 40.0, 20.0));
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(root, Point { x: 100.0, y: 0.0 });
        arena.set_position(left, Point { x: 20.0, y: 100.0 });
        arena.set_position(right, Point { x: 100.0, y: 100.0 });

        let tree = PlacementTree {
            node: root,
            sentinel_edge: None,
            children: vec![leaf(left), leaf(right)],
            orientation: Orientation::Bottom,
        };

        assert_eq!(tree.size(), 3);
        let mut descendants = Vec::new();
        tree.descendants(&mut descendants);
        assert_eq!(descendants, vec![left, right]);
        assert_eq!(
            ArenaGraph::tree_levels(&tree),
            vec![vec![root], vec![left, right]]
        );
        assert_eq!(arena.tree_level_lefts(&tree), vec![100.0, 20.0]);
        assert_eq!(arena.tree_level_rights(&tree), vec![150.0, 140.0]);
        assert_eq!(arena.spacing_before(&tree, 1), 50.0);
    }
}
