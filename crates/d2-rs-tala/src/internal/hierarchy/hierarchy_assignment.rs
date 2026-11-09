// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recognition and assignment of components to layered hierarchy layout.
//!
//! Eligibility, source/sink flow, density, degree, and rank-quality predicates
//! conservatively decide whether a component leaves general placement.

use super::hierarchy_network_simplex::{HierarchyDag, HierarchyRank, rank_hierarchy_dag};
use super::*;

impl ArenaGraph {
    /// Recover TALA's `hierarchicalNodes` predicate.  Assignment builds its
    /// temporary graph from *all* direct children, then rejects a component
    /// containing even one node absent from this set; do not silently drop
    /// ineligible siblings here.
    fn is_hierarchy_simple_node(&self, node: NodeId) -> bool {
        // hierarchy/eligibility.go isSimpleCandidate also rejects retained
        // `Graph.Trees` sentinels: their branching trees are placed by
        // trees.Place after ordinary placement, never by hierarchy placement.
        if self.is_tree_sentinel(node) {
            return false;
        }
        let node = &self.nodes[node.0 as usize];
        !node.is_container
            && !self.tree_routing_nodes.contains_key(&node.input_id)
            && node.cluster.is_none()
            && node.sequence.is_none()
            && node.hierarchy.is_none()
    }

    fn has_hierarchy_container_shape(&self, node: NodeId) -> bool {
        matches!(
            self.nodes[node.0 as usize].shape,
            ShapeKind::Rectangle
                | ShapeKind::Step
                | ShapeKind::Queue
                | ShapeKind::Square
                | ShapeKind::Package
                | ShapeKind::StoredData
        )
    }

    /// Recovered recursive `isHierarchicalContainer` gate. A container is
    /// eligible only when its contents are internally disconnected: an edge
    /// from any direct child to any descendant of the container rejects it.
    /// TALA can then descend into that container and consider its children as
    /// a separate prospective hierarchy scope.
    pub(super) fn is_hierarchical_container(&self, node: NodeId) -> bool {
        if !self.nodes[node.0 as usize].is_container || !self.has_hierarchy_container_shape(node) {
            return false;
        }
        // hierarchy/eligibility.go: Candidates skips NodeToTree members and
        // tree sentinels before considering a container, and
        // isEligibleContainer rejects any such direct child. A container whose
        // child retains a branching tree (for example `c: {a -> b; a -> d}`)
        // therefore stays out of the parent hierarchy, so the ordinary
        // placeNodes recursion reaches trees.Place for that child.
        if self.tree_routing_nodes.contains_key(&node) || self.is_tree_sentinel(node) {
            return false;
        }
        let descendants = self.descendants(node).into_iter().collect::<BTreeSet<_>>();
        let children = self.containers.get(&Some(node)).unwrap_or_default();
        for child in children {
            if !(self.is_hierarchy_simple_node(child) || self.is_hierarchical_container(child)) {
                return false;
            }
            if self.nodes[child.0 as usize]
                .edges
                .read()
                .iter()
                .any(|edge_id| {
                    let edge = &self.edge_records[edge_id.0 as usize];
                    // hierarchy/eligibility.go ignores self-loops: they
                    // reserve routing space but do not connect hierarchy ranks.
                    if edge.from == edge.to {
                        return false;
                    }
                    let adjacent = if edge.from == child {
                        edge.to
                    } else {
                        edge.from
                    };
                    // Node.IsDescendantOf includes the container itself.
                    adjacent == node || descendants.contains(&adjacent)
                })
            {
                return false;
            }
        }
        true
    }

    fn is_hierarchy_candidate_node(&self, node: NodeId) -> bool {
        self.is_hierarchy_simple_node(node) || self.is_hierarchical_container(node)
    }

    /// Enumerate only automatic hierarchy candidates that TALA's
    /// `AssignNodeHierarchy` could publish.  This is intentionally read-only
    /// until the full `PlaceHierarchies` transaction owns copyback: publishing
    /// membership without its placement would make downstream ordinary layout
    /// skip the nodes.
    pub(super) fn automatic_hierarchy_candidates(&self) -> Vec<(Vec<NodeId>, HierarchyRank)> {
        if self.nodes.iter().any(|node| node.fixed_top_left.is_some()) {
            return Vec::new();
        }
        let mut result = Vec::new();
        self.collect_automatic_hierarchy_candidates(None, &mut result);
        result
    }

    fn collect_automatic_hierarchy_candidates(
        &self,
        scope: Option<NodeId>,
        result: &mut Vec<(Vec<NodeId>, HierarchyRank)>,
    ) {
        let children = self.container_node_order(scope);
        let candidates = children
            .iter()
            .copied()
            .filter(|child| self.is_hierarchy_candidate_node(*child))
            .collect::<BTreeSet<_>>();
        // This mirrors the recovered recursion boundary: an eligible
        // container becomes an atomic prospective hierarchy member; only an
        // ineligible container is recursively inspected.
        for child in &children {
            if self.nodes[child.0 as usize].is_container && !candidates.contains(child) {
                self.collect_automatic_hierarchy_candidates(Some(*child), result);
            }
        }

        let scope_edges = self.hierarchy_scope_edges(scope);
        if crate::internal::trace_env_enabled("TALA_TRACE_HIERARCHY_ASSIGNMENT") {
            eprintln!(
                "HIER_ASSIGN_RUST scope={:?} children={:?} candidates={:?} edges={:?}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                children
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                candidates
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                scope_edges
                    .iter()
                    .map(|(from, to, directed)| (
                        self.nodes[from.0 as usize].tala_id,
                        self.nodes[to.0 as usize].tala_id,
                        *directed
                    ))
                    .collect::<Vec<_>>()
            );
        }
        for component in self.hierarchy_components(&children, &scope_edges) {
            if component.iter().all(|node| candidates.contains(node))
                && let Some(rank) = self.rank_hierarchy_component(&component, &scope_edges, false)
            {
                result.push((component, rank));
            }
        }
    }

    pub(super) fn hierarchy_scope_edges(
        &self,
        scope: Option<NodeId>,
    ) -> Vec<(NodeId, NodeId, bool)> {
        self.member_edges()
            .filter_map(|edge| {
                let from = self.direct_child_in_hierarchy_scope(edge.from, scope)?;
                let to = self.direct_child_in_hierarchy_scope(edge.to, scope)?;
                (from != to).then_some((from, to, edge.source_arrow != edge.target_arrow))
            })
            .collect()
    }

    pub(super) fn hierarchy_components(
        &self,
        children: &[NodeId],
        scope_edges: &[(NodeId, NodeId, bool)],
    ) -> Vec<Vec<NodeId>> {
        // SplitSubgraphs walks each node's endpoint-local edge slice. The
        // slice is rebuilt by tree extraction and can differ from the global
        // Graph.Edges order, so using the latter changes component BFS order
        // (and consequently hierarchy placement shuffles). Reconstruct the
        // same adjacency order from the live node edge slices, with a small
        // fallback for projected/test edges that have no arena endpoint.
        let allowed = scope_edges
            .iter()
            .map(|(from, to, _)| {
                if from <= to {
                    (*from, *to)
                } else {
                    (*to, *from)
                }
            })
            .collect::<BTreeSet<_>>();
        let child_set = children.iter().copied().collect::<BTreeSet<_>>();
        let direct_child = |mut node: NodeId| {
            loop {
                node = self.numerical_aggregate_node(node);
                if child_set.contains(&node) {
                    return Some(node);
                }
                node = self.active_node_container(node)?;
            }
        };
        let mut adjacent = children
            .iter()
            .copied()
            .map(|node| (node, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        let mut ordered_edge_ids = self
            .edge_order
            .read()
            .iter()
            .copied()
            .filter(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                !self.tree_routing_nodes.contains_key(&edge.from)
                    && !self.tree_routing_nodes.contains_key(&edge.to)
            })
            .collect::<Vec<_>>();
        let mut seen_edges = ordered_edge_ids.iter().copied().collect::<BTreeSet<_>>();
        for node in self.tree_nodes_in_extraction_order() {
            let edge_id = self.tree_routing_nodes[&node].sentinel_edge;
            if seen_edges.insert(edge_id) {
                ordered_edge_ids.push(edge_id);
            }
        }
        for edge_id in self.edge_order.read().iter().copied() {
            if seen_edges.insert(edge_id) {
                ordered_edge_ids.push(edge_id);
            }
        }
        // layoutgraph.abductEdges reconnects a descendant endpoint to its
        // direct-scope ancestor. reconnect appends to that ancestor's Edges;
        // its existing incidences stay ahead of every promoted endpoint even
        // when the promoted edge precedes them in Graph.Edges.
        let mut promoted_incidents = Vec::new();
        for edge_id in ordered_edge_ids {
            let edge = &self.edge_records[edge_id.0 as usize];
            let Some(from) = direct_child(edge.from) else {
                continue;
            };
            let Some(to) = direct_child(edge.to) else {
                continue;
            };
            if !child_set.contains(&from) || !child_set.contains(&to) || from == to {
                continue;
            }
            let key = if from <= to { (from, to) } else { (to, from) };
            if allowed.contains(&key) {
                if self.numerical_aggregate_node(edge.from) == from {
                    adjacent.get_mut(&from).unwrap().push(to);
                } else {
                    promoted_incidents.push((from, to));
                }
                if self.numerical_aggregate_node(edge.to) == to {
                    adjacent.get_mut(&to).unwrap().push(from);
                } else {
                    promoted_incidents.push((to, from));
                }
            }
        }
        for (node, neighbor) in promoted_incidents {
            adjacent.get_mut(&node).unwrap().push(neighbor);
        }
        let mut unseen = children.iter().copied().collect::<BTreeSet<_>>();
        let mut components = Vec::new();
        while let Some(first) = children.iter().copied().find(|node| unseen.contains(node)) {
            let mut queue = VecDeque::from([first]);
            let mut component = Vec::new();
            unseen.remove(&first);
            while let Some(node) = queue.pop_front() {
                component.push(node);
                for neighbor in &adjacent[&node] {
                    if unseen.remove(neighbor) {
                        queue.push_back(*neighbor);
                    }
                }
            }
            components.push(component);
        }
        components
    }

    /// The TALA source/sink predicates over one temporary hierarchy graph.
    fn hierarchy_has_source_and_sink(
        &self,
        component: &[NodeId],
        edges: &[(NodeId, NodeId, bool)],
    ) -> bool {
        // hierarchy.assign abducts edges before descending into a child
        // scope (layoutgraph/layout.go). Cross-scope incidences have already
        // been reconnected to ancestors, so isSource/isSink see only this
        // scope's structural edges, not the arena's retained endpoints.
        let mut has_source = false;
        let mut has_sink = false;
        for &node in component {
            let incident = || {
                edges
                    .iter()
                    .filter(|(from, to, _)| *from == node || *to == node)
            };
            let is_source = incident().all(|(from, _, directed)| *directed && *from == node);
            let is_sink = incident().all(|(_, to, directed)| *directed && *to == node);
            if is_source {
                has_source = true;
            } else if is_sink {
                has_sink = true;
            }
        }
        has_source && has_sink
    }

    /// Build the recovered DAG and run the recovered network simplex. This is
    /// intentionally an internal candidate until `PlaceHierarchies` lands;
    /// publishing membership early would make downstream placement skip these
    /// nodes without giving them TALA's level layout.
    pub(super) fn rank_hierarchy_component(
        &self,
        component: &[NodeId],
        scope_edges: &[(NodeId, NodeId, bool)],
        force: bool,
    ) -> Option<HierarchyRank> {
        if component.len() <= 1
            || (!force && !self.hierarchy_has_source_and_sink(component, scope_edges))
        {
            return None;
        }
        // Go's buildHierarchy deliberately leaves an all-SQL-table component
        // on the ordinary placement path. Tables create routing dummy nodes
        // for their columns; assigning a layered hierarchy here changes the
        // table-column crossing optimization and can move an otherwise
        // identical graph to a different local minimum. Explicit
        // `forceHierarchy` still bypasses this automatic-only guard.
        if !force
            && component
                .iter()
                .all(|node| self.nodes[node.0 as usize].shape == ShapeKind::SqlTable)
        {
            return None;
        }
        let members = component.iter().copied().collect::<BTreeSet<_>>();
        let edges = scope_edges
            .iter()
            .copied()
            .filter(|(from, to, _)| members.contains(from) && members.contains(to))
            .collect::<Vec<_>>();
        let tie_keys = component
            .iter()
            .copied()
            .map(|node| (node, self.nodes[node.0 as usize].tala_id))
            .collect::<BTreeMap<_, _>>();
        let dag = HierarchyDag::from_scope_with_ties(component.to_vec(), &edges, tie_keys);
        let rank = rank_hierarchy_dag(&dag)?;
        let valid = force || self.hierarchy_is_valid(component, &edges, &rank);
        if crate::internal::trace_env_enabled("TALA_TRACE_HIERARCHY_ASSIGNMENT") {
            eprintln!(
                "HIER_RANK_RUST component={:?} level_count={} levels={:?} edges={:?} valid={}",
                component
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                rank.level_count,
                rank.level
                    .iter()
                    .map(|(node, level)| (self.nodes[node.0 as usize].tala_id, *level))
                    .collect::<Vec<_>>(),
                edges
                    .iter()
                    .map(|(from, to, directed)| (
                        self.nodes[from.0 as usize].tala_id,
                        self.nodes[to.0 as usize].tala_id,
                        *directed
                    ))
                    .collect::<Vec<_>>(),
                valid
            );
        }
        valid.then_some(rank)
    }

    /// Recovered `Hierarchy.isValid`, evaluated against the original arena's
    /// descendant ownership and the temporary children's-edge projection.
    fn hierarchy_is_valid(
        &self,
        component: &[NodeId],
        edges: &[(NodeId, NodeId, bool)],
        rank: &HierarchyRank,
    ) -> bool {
        if rank.level_count < 3 {
            return false;
        }
        let mut nodes = component.iter().copied().collect::<BTreeSet<_>>();
        for &node in component {
            nodes.extend(self.descendants(node));
        }
        let aspect = nodes.len() as f64 / (rank.level_count * rank.level_count) as f64;
        let forward = edges
            .iter()
            .filter(|(from, to, directed)| *directed && rank.level[to] > rank.level[from])
            .count();
        let back_or_neutral = edges.len() - forward;
        let all_edges_forward = back_or_neutral == 0;
        let mut incoming = BTreeSet::new();
        let mut outgoing = BTreeSet::new();
        for &(from, to, directed) in edges {
            if directed {
                outgoing.insert(from);
                incoming.insert(to);
            }
        }
        let source_count = component
            .iter()
            .filter(|node| !incoming.contains(node))
            .count();
        let sink_count = component
            .iter()
            .filter(|node| !outgoing.contains(node))
            .count();
        // The workflow exception is about parallel top-level blocks. Nested
        // descendants contribute to the aspect-ratio and size checks above,
        // but cannot turn a one-node-per-rank chain into a branched workflow.
        let branched_workflow = component.len() > rank.level_count
            && source_count == 1
            && sink_count == 1
            && all_edges_forward
            && nodes.len() <= 128
            && edges.len() <= 256;
        if aspect > 2.0 || (aspect < 0.5 && !branched_workflow) {
            return false;
        }
        if !branched_workflow && (forward as f64) < 1.5 * back_or_neutral as f64 {
            return false;
        }
        let max_edges = (2.0 * (nodes.len() as f64).sqrt().ceil()) as usize;
        if nodes
            .iter()
            .any(|node| self.nodes[node.0 as usize].edges.len() > max_edges)
        {
            return false;
        }
        !self.hierarchy_is_1_n_1(component, edges, rank)
    }

    fn hierarchy_is_1_n_1(
        &self,
        component: &[NodeId],
        edges: &[(NodeId, NodeId, bool)],
        rank: &HierarchyRank,
    ) -> bool {
        if rank.level_count != 3 {
            return false;
        }
        let levels = (0..3)
            .map(|level| {
                component
                    .iter()
                    .copied()
                    .filter(|node| rank.level[node] == level)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if levels[0].len() != 1 || levels[2].len() != 1 {
            return false;
        }

        // hierarchy/discovery.go:is1N1 compares the original endpoints of
        // AbductEdges before treating the outer source and sink as single
        // nodes. Distinct descendants of either container are distinct
        // endpoints, even when their projected rank edges are identical.
        // Retain both optional fields: an unchanged endpoint is nil in Go,
        // including when the opposite endpoint was abducted.
        let scope = self.nodes[component[0].0 as usize].container;
        let mut source_abduction = None;
        let mut sink_abduction = None;
        for edge in self.member_edges() {
            let Some(from) = self.direct_child_in_hierarchy_scope(edge.from, scope) else {
                continue;
            };
            let Some(to) = self.direct_child_in_hierarchy_scope(edge.to, scope) else {
                continue;
            };
            if from == to || (from == edge.from && to == edge.to) {
                continue;
            }
            let original = (
                (from != edge.from).then_some(edge.from),
                (to != edge.to).then_some(edge.to),
            );
            for (endpoint, prior) in [
                (levels[0][0], &mut source_abduction),
                (levels[2][0], &mut sink_abduction),
            ] {
                if from == endpoint {
                    if let Some((original_from, _)) = *prior {
                        if original.0 != original_from {
                            return false;
                        }
                    } else {
                        *prior = Some(original);
                    }
                }
                if to == endpoint {
                    if let Some((_, original_to)) = *prior {
                        if original.1 != original_to {
                            return false;
                        }
                    } else {
                        *prior = Some(original);
                    }
                }
            }
        }
        let source = levels[0][0];
        levels[1].iter().all(|middle| {
            let above = edges.iter().any(|(from, to, _)| {
                (*from == *middle && rank.level[to] == 0)
                    || (*to == *middle && rank.level[from] == 0)
            });
            let below = edges.iter().any(|(from, to, _)| {
                (*from == *middle && rank.level[to] == 2)
                    || (*to == *middle && rank.level[from] == 2)
            });
            above && below
        }) && edges
            .iter()
            .filter(|(from, to, _)| *from == source || *to == source)
            .all(|(from, to, _)| {
                let adjacent = if *from == source { *to } else { *from };
                rank.level[&adjacent] == 1
            })
    }

    fn direct_child_in_hierarchy_scope(
        &self,
        mut node: NodeId,
        scope: Option<NodeId>,
    ) -> Option<NodeId> {
        // Containers uses the numerical carrier of a live sequence/cluster,
        // while AddSequences reconnects canonical edges to its allocated
        // vessel. Keep both views in the same identity space for Assign's
        // AbductEdges/SplitSubgraphs projection (hierarchy/discovery.go).
        node = self.numerical_aggregate_node(node);
        match scope {
            None => {
                while let Some(parent) = self.active_node_container(node) {
                    node = self.numerical_aggregate_node(parent);
                }
                Some(node)
            }
            Some(scope) => loop {
                let parent = self.active_node_container(node)?;
                if parent == scope {
                    return Some(node);
                }
                node = self.numerical_aggregate_node(parent);
            },
        }
    }

    /// Materialize the explicit half of recovered `AssignNodeHierarchy`.
    ///
    /// D2's `shape: hierarchy` does not change the rendered shape. The adapter
    /// sets `Graph.IsRootHierarchy` or `Node.ForceHierarchy`; TALA then assigns
    /// one hierarchy object per connected component of the forced scope and
    /// publishes that same pointer to every descendant of a component node.
    ///
    /// Automatic `Hierarchy.isValid` selection is deliberately separate: it
    /// requires the recovered DAG/network-simplex levels and must not be
    /// approximated with container-count or fixture-shape predicates.
    pub(super) fn assign_forced_hierarchies(&mut self) {
        self.clear_hierarchy_memberships();

        let mut scopes = Vec::new();
        if self.root_hierarchy {
            scopes.push(None);
        }
        scopes.extend(
            self.nodes
                .iter()
                .filter(|node| node.force_hierarchy)
                .map(|node| Some(node.input_id)),
        );

        let mut next_hierarchy = 0usize;
        for scope in scopes {
            let children = self.container_node_order(scope);
            if children.len() <= 1 {
                continue;
            }
            let scope_edges = self.hierarchy_scope_edges(scope);
            for component in self.hierarchy_components(&children, &scope_edges) {
                if component.len() <= 1 {
                    continue;
                }

                // `buildHierarchy(..., force=true)` still constructs the
                // simple DAG and runs network simplex; force only bypasses
                // the source/sink and `isValid` rejection gates.
                let Some(rank) = self.rank_hierarchy_component(&component, &scope_edges, true)
                else {
                    continue;
                };

                let hierarchy_id = next_hierarchy;
                next_hierarchy += 1;
                for node in component {
                    if self.nodes[node.0 as usize].hierarchy.is_none() {
                        self.assign_hierarchy_member(
                            node,
                            hierarchy_id,
                            scope,
                            rank.level[&node],
                            rank.level_count,
                        );
                    }
                    for descendant in self.descendants(node) {
                        if self.nodes[descendant.0 as usize].hierarchy.is_none() {
                            self.assign_hierarchy_member(
                                descendant,
                                hierarchy_id,
                                scope,
                                rank.level[&node],
                                rank.level_count,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Materialize the non-forced half of recovered `AssignNodeHierarchy`.
    ///
    /// The recursive scope walk deliberately mirrors candidate discovery:
    /// an atomic eligible container is one member of its parent's hierarchy;
    /// only a non-atomic container opens a nested assignment scope.  This is
    /// important because assigning both levels would let an outer hierarchy
    /// steal descendants which TALA keeps owned by the inner transaction.
    pub(super) fn assign_automatic_hierarchies(&mut self) {
        let _ = self.assign_automatic_hierarchies_traced();
    }

    pub(super) fn assign_automatic_hierarchies_traced(&mut self) -> Vec<HierarchyAssignmentTrace> {
        if self.nodes.iter().any(|node| node.fixed_top_left.is_some()) {
            return Vec::new();
        }
        let mut next_hierarchy = self
            .nodes
            .iter()
            .filter_map(|node| node.hierarchy.clone().map(|membership| membership.id + 1))
            .max()
            .unwrap_or(0);
        let mut traces = Vec::new();
        self.assign_automatic_hierarchies_in_scope(None, &mut next_hierarchy, &mut traces);
        traces
    }

    fn assign_automatic_hierarchies_in_scope(
        &mut self,
        scope: Option<NodeId>,
        next_hierarchy: &mut usize,
        traces: &mut Vec<HierarchyAssignmentTrace>,
    ) {
        let children = self.container_node_order(scope);
        let candidates = children
            .iter()
            .copied()
            .filter(|child| self.is_hierarchy_candidate_node(*child))
            .collect::<BTreeSet<_>>();

        // Descend before materializing this scope, as the recovered source
        // does. An accepted atomic container has no nested assignment pass.
        for child in &children {
            if self.nodes[child.0 as usize].is_container && !candidates.contains(child) {
                self.assign_automatic_hierarchies_in_scope(Some(*child), next_hierarchy, traces);
            }
        }

        let scope_edges = self.hierarchy_scope_edges(scope);
        for component in self.hierarchy_components(&children, &scope_edges) {
            let eligible = component.iter().all(|node| candidates.contains(node));
            let rank_attempted = eligible
                && component.len() > 1
                && self.hierarchy_has_source_and_sink(&component, &scope_edges);
            if !eligible {
                traces.push(HierarchyAssignmentTrace {
                    scope,
                    component,
                    candidate_nodes: candidates.iter().copied().collect(),
                    eligible,
                    rank_attempted,
                    accepted: false,
                    rng_draws_before: 0,
                    rng_draws_after: 0,
                });
                continue;
            }
            let rank = self.rank_hierarchy_component(&component, &scope_edges, false);
            if crate::internal::trace_env_enabled("TALA_TRACE_HIERARCHY_ASSIGNMENT") {
                eprintln!(
                    "HIER_COMPONENT_RUST scope={:?} component={:?} eligible={} rank={:?}",
                    scope.map(|node| self.nodes[node.0 as usize].tala_id),
                    component
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    eligible,
                    rank.as_ref().map(|rank| (
                        rank.level_count,
                        rank.level
                            .iter()
                            .map(|(node, level)| (self.nodes[node.0 as usize].tala_id, *level))
                            .collect::<Vec<_>>()
                    )),
                );
            }
            let Some(rank) = rank else {
                traces.push(HierarchyAssignmentTrace {
                    scope,
                    component,
                    candidate_nodes: candidates.iter().copied().collect(),
                    eligible,
                    rank_attempted,
                    accepted: false,
                    rng_draws_before: 0,
                    rng_draws_after: 0,
                });
                continue;
            };
            traces.push(HierarchyAssignmentTrace {
                scope,
                component: component.clone(),
                candidate_nodes: candidates.iter().copied().collect(),
                eligible,
                rank_attempted,
                accepted: true,
                rng_draws_before: 0,
                rng_draws_after: 0,
            });
            let hierarchy_id = *next_hierarchy;
            *next_hierarchy += 1;
            for node in component {
                if self.nodes[node.0 as usize].hierarchy.is_none() {
                    self.assign_hierarchy_member(
                        node,
                        hierarchy_id,
                        scope,
                        rank.level[&node],
                        rank.level_count,
                    );
                }
                for descendant in self.descendants(node) {
                    if self.nodes[descendant.0 as usize].hierarchy.is_none() {
                        self.assign_hierarchy_member(
                            descendant,
                            hierarchy_id,
                            scope,
                            rank.level[&node],
                            rank.level_count,
                        );
                    }
                }
            }
        }
    }

    pub(super) fn first_node_owns_hierarchy(&self) -> bool {
        self.nodes
            .first()
            .is_some_and(|node| node.hierarchy.is_some())
    }

    /// Mirrors TALA's `RemoveIsolatedMemberships` cleanup after hierarchy
    /// placement. A leaf that inherited its container's hierarchy but owns no
    /// edge is an ordinary routing/packing obstacle; retaining the membership
    /// incorrectly classifies its enclosing BinPack component as hierarchical.
    pub(super) fn remove_isolated_hierarchy_memberships(&mut self) {
        for index in 0..self.nodes.len() {
            let node = &self.nodes[index];
            if node.hierarchy.is_some()
                && self.owner_node_incident_count(NodeId(index as u32)) == 0
                && !node.is_container
            {
                self.remove_hierarchy_member(NodeId(index as u32));
            }
        }
    }

    /// Mirrors the root-level cleanup in TALA's hierarchy discovery. Nears
    /// are placement-only proximity constraints; once a node belongs to an
    /// automatic hierarchy, both that node's links and links from other
    /// hierarchy members are discarded. Retain only relations whose two
    /// endpoints remain ordinary root-placement nodes.
    pub(super) fn remove_hierarchy_nears(&mut self) {
        let original = self
            .nodes
            .iter()
            .map(|node| node.nears.keys())
            .collect::<Vec<_>>();
        for node in &mut self.nodes {
            node.nears.clear();
        }
        for (index, original_nears) in original.iter().enumerate().take(self.nodes.len()) {
            if self.nodes[index].hierarchy.is_some() {
                continue;
            }
            for near in original_nears.iter().copied() {
                if self.nodes[near.0 as usize].hierarchy.is_some() {
                    continue;
                }
                if !self.nodes[index].nears.contains(&near) {
                    self.nodes[index].nears.push(near);
                }
                let index_id = NodeId(index as u32);
                if !self.nodes[near.0 as usize].nears.contains(&index_id) {
                    self.nodes[near.0 as usize].nears.push(index_id);
                }
            }
        }
    }

    /// Build the ranked placement graph for one materialized hierarchy. This
    /// is the direct bridge between `AssignNodeHierarchy`'s level map and
    /// recovered `placeNodesInHierarchy`; copyback remains owned by the outer
    /// hierarchy transaction.
    fn build_hierarchy_placement(
        &self,
        hierarchy_id: usize,
        rng: &mut go_rng::GoRng,
    ) -> Option<(Vec<NodeId>, super::hierarchy_placement::HierarchyPlacement)> {
        let membership = self.nodes.iter().find_map(|node| {
            node.hierarchy
                .clone()
                .filter(|membership| membership.id == hierarchy_id)
        })?;
        // The Go placement stage first builds an induced subgraph with
        // SplitSubgraphs. Its node slice follows the graph's live traversal
        // order after tree preprocessing, rather than the declaration-order
        // container slice. Preserve that order before the BFS component walk.
        let scope_children = self
            .preprocessed_tree_children
            .get(&membership.scope)
            .cloned()
            .unwrap_or_else(|| self.container_node_order(membership.scope));
        if crate::internal::trace_env_enabled("TALA_TRACE_HIER_ORDER") {
            eprintln!(
                "HIER_SCOPE_CHILDREN_RUST {:?}",
                scope_children
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        // `PlaceHierarchies` does not rebuild a hierarchy subgraph in
        // container declaration order. It calls `SplitSubgraphs`, whose BFS
        // preserves graph-node start order and each node's edge order. Reuse
        // the same recovered component traversal used during assignment.
        let direct_members = self
            .hierarchy_components(
                &scope_children,
                &self.hierarchy_scope_edges(membership.scope),
            )
            .into_iter()
            .find(|component| {
                component.iter().any(|node| {
                    self.nodes[node.0 as usize]
                        .hierarchy
                        .clone()
                        .is_some_and(|candidate| candidate.id == hierarchy_id)
                })
            })?
            .into_iter()
            .filter(|node| {
                self.nodes[node.0 as usize]
                    .hierarchy
                    .clone()
                    .is_some_and(|candidate| candidate.id == hierarchy_id)
            })
            .collect::<Vec<_>>();
        if crate::internal::trace_env_enabled("TALA_TRACE_HIER_ORDER") {
            eprintln!(
                "HIER_DIRECT_MEMBERS_RUST {:?}",
                direct_members
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        if direct_members.len() <= 1 {
            return None;
        }
        let rank = HierarchyRank {
            level: direct_members
                .iter()
                .map(|node| {
                    (
                        *node,
                        usize::try_from(
                            self.nodes[node.0 as usize]
                                .hierarchy
                                .as_ref()
                                .unwrap()
                                .level(),
                        )
                        .expect("nonnegative placed level"),
                    )
                })
                .collect(),
            level_count: usize::try_from(membership.level_count())
                .expect("nonnegative placed level count"),
        };
        let placement = super::hierarchy_placement::HierarchyPlacement::build(
            self,
            &direct_members,
            &rank,
            rng,
        );
        Some((direct_members, placement))
    }

    pub(super) fn hierarchy_placement_preview(
        &self,
        hierarchy_id: usize,
        rng: &mut go_rng::GoRng,
    ) -> Option<BTreeMap<NodeId, Point>> {
        let (_, mut placement) = self.build_hierarchy_placement(hierarchy_id, rng)?;
        let mut graph = self.clone();
        Some(placement.place_ranked_nodes(&mut graph))
    }

    /// Apply the recovered ranked placement and its immediate
    /// `syncContainers` copyback for one hierarchy object. The caller owns
    /// the surrounding `PlaceHierarchies` traversal and direction transforms.
    pub(super) fn apply_hierarchy_placement(
        &mut self,
        hierarchy_id: usize,
        rng: &mut go_rng::GoRng,
    ) -> bool {
        let Some((roots, mut placement)) = self.build_hierarchy_placement(hierarchy_id, rng) else {
            return false;
        };
        let horizontal = self.begin_hierarchy_direction(&roots);
        let points = if horizontal {
            placement.place_ranked_nodes_horizontal(self)
        } else {
            placement.place_ranked_nodes(self)
        };
        for (node, point) in points {
            self.set_position(node, point);
        }
        self.sync_hierarchy_containers(&roots);
        self.finish_hierarchy_direction(&roots, horizontal);
        true
    }

    pub(super) fn apply_hierarchy_placement_traced(
        &mut self,
        hierarchy_id: usize,
        rng: &mut go_rng::GoRng,
    ) -> Option<HierarchyPlacementTrace> {
        let rng_draws_before_build = rng.draw_count();
        let (roots, mut placement) = self.build_hierarchy_placement(hierarchy_id, rng)?;
        let rng_draws_after_build = rng.draw_count();
        let horizontal = self.begin_hierarchy_direction(&roots);
        let (points, orders, alignments) = placement.place_ranked_nodes_traced(self, horizontal);
        for (node, point) in points {
            self.set_position(node, point);
        }
        self.sync_hierarchy_containers(&roots);
        self.finish_hierarchy_direction(&roots, horizontal);
        Some(HierarchyPlacementTrace {
            hierarchy_id,
            roots,
            rng_draws_before_build,
            rng_draws_after_build,
            rng_draws_after_placement: rng.draw_count(),
            orders,
            alignments,
        })
    }

    /// `iterContainersBFS` (hierarchy/placement.go:518–529): the hierarchy's
    /// direct nodes, then every container's children, breadth first.
    fn hierarchy_nodes_bfs(&self, roots: &[NodeId]) -> Vec<NodeId> {
        let mut queue = roots.to_vec();
        let mut index = 0;
        while index < queue.len() {
            let node = queue[index];
            index += 1;
            if self.nodes[node.0 as usize].is_container {
                queue.extend(self.containers.get(&Some(node)).unwrap_or_default());
            }
        }
        queue
    }

    /// `placeNodesInHierarchy` (hierarchy/placement.go:225–249) lays levels
    /// out top to bottom. A horizontal hierarchy (`IsHorizontal`,
    /// orientation.go:8–21) is transposed before placement and container
    /// wrapping; container padding is recomputed from labels, untransposed.
    fn begin_hierarchy_direction(&mut self, roots: &[NodeId]) -> bool {
        let horizontal = roots.first().is_some_and(|first| {
            matches!(
                self.directions.get(&self.nodes[first.0 as usize].container),
                Some(crate::Direction::Left | crate::Direction::Right)
            )
        }) || roots
            .iter()
            .any(|node| self.nodes[node.0 as usize].shape == ShapeKind::SqlTable);
        if horizontal {
            self.transpose_hierarchy_nodes(roots);
        }
        horizontal
    }

    fn finish_hierarchy_direction(&mut self, roots: &[NodeId], horizontal: bool) {
        if horizontal {
            self.transpose_hierarchy_nodes(roots);
        }
        let direction = roots.first().and_then(|first| {
            self.directions
                .get(&self.nodes[first.0 as usize].container)
                .copied()
        });
        match direction {
            Some(crate::Direction::Left) => self.mirror_hierarchy_nodes(roots, true, false),
            Some(crate::Direction::Up) => self.mirror_hierarchy_nodes(roots, false, true),
            _ => {}
        }
    }

    /// Node.transpose (layoutgraph/node.go:2409–2416).
    fn transpose_hierarchy_nodes(&mut self, roots: &[NodeId]) {
        for node in self.hierarchy_nodes_bfs(roots) {
            let item = &mut self.nodes[node.0 as usize];
            std::mem::swap(&mut item.rect.size.width, &mut item.rect.size.height);
            std::mem::swap(&mut item.node_padding.left, &mut item.node_padding.top);
            std::mem::swap(&mut item.node_padding.right, &mut item.node_padding.bottom);
            std::mem::swap(&mut item.layout_margins.left, &mut item.layout_margins.top);
            std::mem::swap(
                &mut item.layout_margins.right,
                &mut item.layout_margins.bottom,
            );
            if let Some(mut point) = self.position(node) {
                std::mem::swap(&mut point.x, &mut point.y);
                self.set_position(node, point);
            }
        }
    }

    /// Node.mirror (layoutgraph/node.go:2086–2130), including loop offsets.
    fn mirror_hierarchy_nodes(&mut self, roots: &[NodeId], x: bool, y: bool) {
        for node in self.hierarchy_nodes_bfs(roots) {
            let Some(mut point) = self.position(node) else {
                continue;
            };
            let item = &self.nodes[node.0 as usize];
            let offset = |side: i64| {
                item.loop_offsets
                    .as_ref()
                    .and_then(|offsets| offsets.get(&side))
                    .copied()
                    .unwrap_or(0.0)
            };
            if x {
                point.x = -point.x - item.rect.size.width + offset(7) - offset(5);
            }
            if y {
                point.y = -point.y - item.rect.size.height + offset(4) - offset(6);
            }
            self.set_position(node, point);
        }
    }

    /// Recovered `syncContainers`: breadth-first discover containers from a
    /// hierarchy's direct placement nodes, then wrap them deepest-first. A
    /// container is translated to its fitted inside origin; its children keep
    /// their already-established absolute coordinates.
    pub(super) fn sync_hierarchy_containers(&mut self, roots: &[NodeId]) {
        let mut queue = roots.to_vec();
        let mut containers = Vec::new();
        while let Some(node) = queue.first().copied() {
            queue.remove(0);
            if !self.nodes[node.0 as usize].is_container {
                continue;
            }
            containers.push(node);
            queue.extend(self.containers.get(&Some(node)).unwrap_or_default());
        }
        for container in containers.into_iter().rev() {
            let children = self.containers.get(&Some(container)).unwrap_or_default();
            let Some((mut top_left, mut bottom_right)) = self.fixed_node_bounds(&children) else {
                continue;
            };
            // hierarchy/brandes_kopf.go syncContainers calls Node.WrapChildren,
            // whose fixed-bounds pass is followed by expandForLabels.  This is
            // observable after a horizontal hierarchy transpose: a boundary
            // child's centered label can overhang its now-narrow body by a
            // fraction, extending the fitted container by one pixel.
            Self::expand_for_child_labels(
                &mut top_left,
                &mut bottom_right,
                children.iter().map(|child| &self.nodes[child.0 as usize]),
            );
            let content = Size {
                width: bottom_right.x - top_left.x,
                height: bottom_right.y - top_left.y,
            };
            let padding = self.shape_fit_padding(container);
            if crate::internal::trace_env_enabled("TALA_TRACE_FIT_ALL") {
                let item = &self.nodes[container.0 as usize];
                eprintln!(
                    "SYNC_FIT_RUST id={} before={:?} bounds={:?}->{:?} padding={:?} content={:?}",
                    item.tala_id, item.rect.size, top_left, bottom_right, padding, content
                );
            }
            self.nodes[container.0 as usize].rect.size =
                self.bin_pack_shape_dimensions_to_fit(container, content, padding);
            if crate::internal::trace_env_enabled("TALA_TRACE_FIT_ALL") {
                let item = &self.nodes[container.0 as usize];
                eprintln!(
                    "SYNC_FIT_RUST_AFTER id={} width={:.17} height={:.17}",
                    item.tala_id, item.rect.size.width, item.rect.size.height
                );
            }
            let inside = self.bin_pack_shape_inside_placement(container, content, padding);
            self.set_position(
                container,
                Point {
                    x: top_left.x - inside.x,
                    y: top_left.y - inside.y,
                },
            );
        }
    }
}
