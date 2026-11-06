// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Checked packing reachability and component classification.
use super::*;

/// A retained member and its temporary vessel are different Go pointers even
/// when the numerical placement arena uses the first member as a carrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum PackingNodeRef {
    Node(NodeId),
    Cluster(usize),
    Sequence(usize),
}

/// Reachability's visited set: dense flags for canonical nodes, a hash set
/// for vessels and out-of-range witnesses. Membership is all it answers.
pub(super) struct PackingVisited {
    nodes: Vec<bool>,
    other: crate::internal::IdHashSet<PackingNodeRef>,
}
impl PackingVisited {
    fn new(node_count: usize) -> Self {
        Self {
            nodes: vec![false; node_count],
            other: Default::default(),
        }
    }
    fn contains(&self, node: &PackingNodeRef) -> bool {
        if let PackingNodeRef::Node(id) = *node
            && let Some(&seen) = self.nodes.get(id.0 as usize)
        {
            return seen;
        }
        self.other.contains(node)
    }
    fn insert(&mut self, node: PackingNodeRef) {
        if let PackingNodeRef::Node(id) = node
            && let Some(seen) = self.nodes.get_mut(id.0 as usize)
        {
            *seen = true;
            return;
        }
        self.other.insert(node);
    }
}

/// Ancestry chains for one stable-topology reachability walk: the current
/// visit's chain and, per Graph.Nodes slot, a range of `store`. A slot holds
/// `Some(None)` when its chain is cyclic.
struct PackingAncestryCache {
    store: Vec<PackingNodeRef>,
    slots: Vec<Option<Option<(usize, usize)>>>,
    current: Vec<PackingNodeRef>,
    current_acyclic: Option<bool>,
}
impl PackingAncestryCache {
    fn new(slot_count: usize) -> Self {
        Self {
            store: Vec::new(),
            slots: vec![None; slot_count],
            current: Vec::new(),
            current_acyclic: None,
        }
    }
    /// packing_descendant_checked(chain[0], target, false): one step per
    /// visited ancestor and one for the terminal nil.
    fn search(
        chain: &[PackingNodeRef],
        target: PackingNodeRef,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let (found, visits) = match chain.iter().position(|&node| node == target) {
            Some(index) => (true, index + 1),
            None => (false, chain.len() + 1),
        };
        guard.steps(visits)?;
        Ok(found)
    }
}

impl ArenaGraph {
    pub(super) fn packing_vessel(&self, cluster: bool, index: usize) -> PackingNodeRef {
        let (binding, id) = if cluster {
            (
                TopologyVessel::Cluster(index),
                self.clusters[index].vessel_tala_id,
            )
        } else {
            (
                TopologyVessel::Sequence(index),
                self.sequences[index].vessel_tala_id,
            )
        };
        self.topology_vessel_node(binding)
            .or_else(|| {
                self.nodes
                    .iter()
                    .find(|n| n.tala_id == id)
                    .map(|n| n.input_id)
            })
            .map(PackingNodeRef::Node)
            .unwrap_or(if cluster {
                PackingNodeRef::Cluster(index)
            } else {
                PackingNodeRef::Sequence(index)
            })
    }

    fn packing_cluster_backlink(&self, node: PackingNodeRef) -> Option<usize> {
        let node = match node {
            PackingNodeRef::Node(node) => node,
            PackingNodeRef::Sequence(index) => *self.sequences[index].members.first()?,
            PackingNodeRef::Cluster(_) => return None,
        };
        self.nodes[node.0 as usize].cluster.or_else(|| {
            (!self.cluster_backlinks_complete)
                .then(|| self.clusters.iter().position(|c| c.members.contains(&node)))
                .flatten()
        })
    }

    fn packing_sequence_backlink(&self, node: PackingNodeRef) -> Option<usize> {
        let PackingNodeRef::Node(node) = node else {
            return None;
        };
        self.nodes[node.0 as usize].sequence.or_else(|| {
            (!self.sequence_backlinks_complete)
                .then(|| {
                    self.sequences
                        .iter()
                        .position(|s| s.members.contains(&node))
                })
                .flatten()
        })
    }

    pub(super) fn packing_group_active(&self, cluster: bool, index: usize) -> bool {
        match self.packing_vessel(cluster, index) {
            PackingNodeRef::Node(node) => {
                if self.shared_topology_initialized() {
                    self.topology_node_owner(node).is_some()
                } else {
                    self.node_order.contains(&node)
                }
            }
            PackingNodeRef::Cluster(i) => self.cluster_is_active(&self.clusters[i]),
            PackingNodeRef::Sequence(i) => self.sequence_is_active(&self.sequences[i]),
        }
    }

    pub(super) fn packing_effective_node(&self, node: NodeId) -> PackingNodeRef {
        let owner = self.active_aggregate_owner(node);
        if self.bin_pack_node_is_vessel_carrier(owner) {
            if let Some(index) = self.active_cluster_index(owner) {
                return self.packing_vessel(true, index);
            }
            if let Some(index) = self.active_sequence_index(owner) {
                return self.packing_vessel(false, index);
            }
        }
        PackingNodeRef::Node(node)
    }

    pub(super) fn packing_raw_container(&self, node: PackingNodeRef) -> Option<PackingNodeRef> {
        let parent = match node {
            PackingNodeRef::Node(node) => self.nodes[node.0 as usize].container,
            PackingNodeRef::Sequence(index) => {
                if !self.packing_group_active(false, index) {
                    return None;
                }
                self.sequences[index].container
            }
            PackingNodeRef::Cluster(index) => {
                if !self.packing_group_active(true, index) {
                    return None;
                }
                let cluster = &self.clusters[index];
                if let Some(id) = self
                    .serialized_cluster_containers_by_tala
                    .get(&cluster.vessel_tala_id)
                {
                    return id.and_then(|id| {
                        self.nodes
                            .iter()
                            .find(|n| n.tala_id == id)
                            .map(|n| PackingNodeRef::Node(n.input_id))
                    });
                }
                self.nodes[cluster.members[0].0 as usize].container
            }
        };
        parent.map(PackingNodeRef::Node)
    }

    fn packing_ancestry_parent(&self, node: PackingNodeRef) -> Option<PackingNodeRef> {
        self.packing_raw_container(node)
            .or_else(|| {
                self.packing_cluster_backlink(node)
                    .map(|i| self.packing_vessel(true, i))
            })
            .or_else(|| {
                self.packing_sequence_backlink(node)
                    .map(|i| self.packing_vessel(false, i))
            })
    }

    pub(super) fn packing_descendant_checked(
        &self,
        mut node: Option<PackingNodeRef>,
        ancestor: Option<PackingNodeRef>,
        check_cycles: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        // Ancestry chains are short: test the first visits in place and
        // spill to an ordered set only for deep chains.
        let mut near = [PackingNodeRef::Cluster(0); 16];
        let mut near_len = 0;
        let mut seen = BTreeSet::new();
        loop {
            guard.step()?;
            if node == ancestor {
                return Ok(true);
            }
            let Some(current) = node else {
                return Ok(false);
            };
            if check_cycles {
                let repeated = if near[..near_len].contains(&current) {
                    true
                } else if near_len < near.len() {
                    near[near_len] = current;
                    near_len += 1;
                    false
                } else {
                    !seen.insert(current)
                };
                if repeated {
                    return Err(limits::Error::Invalid(
                        "layout invariant violated: BinPack found a cycle in node ancestry".into(),
                    ));
                }
            }
            node = self.packing_ancestry_parent(current);
            // packing/guard.go stops at a terminal node; node.go's
            // reachability ancestry performs a further charged nil visit.
            if check_cycles && node.is_none() {
                return Ok(ancestor.is_none());
            }
        }
    }

    fn packing_cluster_members(&self, node: PackingNodeRef) -> Option<Vec<PackingNodeRef>> {
        self.clusters.iter().enumerate().find_map(|(i, c)| {
            (self.packing_vessel(true, i) == node).then(|| {
                c.members
                    .iter()
                    .map(|&member| {
                        // A compact cluster can contain a previously created sequence
                        // vessel; its carrier is not the sequence's retained first step.
                        if self.bin_pack_node_is_vessel_carrier(member) {
                            if let Some(s) = self.active_sequence_index(member) {
                                if self.sequences[s].members.first() == Some(&member) {
                                    return self.packing_vessel(false, s);
                                }
                            }
                        }
                        PackingNodeRef::Node(member)
                    })
                    .collect()
            })
        })
    }

    pub(super) fn packing_incidents(&self, node: PackingNodeRef) -> Vec<EdgeId> {
        self.packing_incident_slots(node)
            .into_iter()
            .map(|edge| edge.expect("nil packing incident edge"))
            .collect()
    }

    pub(super) fn packing_incident_slots(&self, node: PackingNodeRef) -> Vec<Option<EdgeId>> {
        // Map-only decoded vessels are detached, freshly deserialized nodes.
        // Their retained members keep their own edges; borrowing a member's
        // incidence here invents edges on the vessel and changes early exits.
        if matches!(node, PackingNodeRef::Cluster(i) if !self.packing_group_active(true,i))
            || matches!(node, PackingNodeRef::Sequence(i) if !self.packing_group_active(false,i))
        {
            return Vec::new();
        }
        let (carrier, projected) = match node {
            PackingNodeRef::Node(node) => (node, false),
            PackingNodeRef::Cluster(i) => (self.clusters[i].members[0], true),
            PackingNodeRef::Sequence(i) => (self.sequences[i].members[0], true),
        };
        if !projected && let Ok(slots) = self.owner_node_incident_slots(carrier) {
            return slots;
        }
        let edges = if projected {
            self.active_edge_ids(carrier).into_iter().collect()
        } else {
            self.nodes[carrier.0 as usize].edges.read()
        };
        edges
            .into_iter()
            .filter(|edge| self.edge_order.contains(edge))
            .filter(|&edge| {
                projected
                    || self.packing_endpoint(edge, false) == node
                    || self.packing_endpoint(edge, true) == node
            })
            .map(Some)
            .collect()
    }

    pub(super) fn packing_endpoint(&self, edge: EdgeId, is_to: bool) -> PackingNodeRef {
        let node = self
            .owner_edge_endpoint(edge, is_to)
            .expect("packing edge allocation")
            .expect("nil packing endpoint");
        self.packing_effective_node(node)
    }

    fn packing_adjacent(&self, node: PackingNodeRef, edge: EdgeId) -> PackingNodeRef {
        let from = self.packing_endpoint(edge, false);
        if node == from {
            self.packing_endpoint(edge, true)
        } else {
            from
        }
    }

    fn packing_ordered_nears(&self, node: PackingNodeRef) -> Vec<PackingNodeRef> {
        let PackingNodeRef::Node(node) = node else {
            return Vec::new();
        };
        let mut nears = self.nodes[node.0 as usize].nears.keys();
        nears.sort_by_key(|n| self.nodes[n.0 as usize].tala_id);
        nears.into_iter().map(PackingNodeRef::Node).collect()
    }

    fn packing_near_vessel(&self, node: PackingNodeRef) -> PackingNodeRef {
        if let Some(i) = self.packing_cluster_backlink(node) {
            if self.packing_group_active(true, i) {
                return self.packing_vessel(true, i);
            }
        }
        if let Some(i) = self.packing_sequence_backlink(node) {
            if self.packing_group_active(false, i) {
                return self.packing_vessel(false, i);
            }
        }
        node
    }

    fn packing_enqueue(
        &self,
        node: PackingNodeRef,
        ignore: Option<PackingNodeRef>,
        root: Option<PackingNodeRef>,
        visited: &mut PackingVisited,
        queue: &mut VecDeque<PackingNodeRef>,
        visit_error: &mut Option<limits::Error>,
        guard: &mut limits::WorkGuard<'_>,
    ) {
        if Some(node) == ignore || visited.contains(&node) {
            return;
        }
        if let Some(root) = root {
            if visit_error.is_some() {
                return;
            }
            match self.packing_descendant_checked(Some(node), Some(root), true, guard) {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    *visit_error = Some(error);
                    return;
                }
            }
        }
        visited.insert(node);
        queue.push_back(node);
    }

    /// Append `node` and its packing ancestors to `out`, nearest first.
    /// Distinct refs are bounded by the arena, so a longer chain repeats;
    /// a cyclic chain is removed again and reported as false.
    fn packing_ancestry_chain_into(
        &self,
        node: PackingNodeRef,
        out: &mut Vec<PackingNodeRef>,
    ) -> bool {
        let start = out.len();
        let limit = self.nodes.len() + self.clusters.len() + self.sequences.len();
        let mut current = Some(node);
        while let Some(n) = current {
            if out.len() - start > limit {
                out.truncate(start);
                return false;
            }
            out.push(n);
            current = self.packing_ancestry_parent(n);
        }
        true
    }

    /// `descendant(current, other) || descendant(other, current)` without
    /// cycle checks, from cached chains. Each search charges exactly the
    /// walk's steps; cyclic chains keep the live, guard-bounded walk.
    fn packing_related_cached(
        &self,
        cache: &mut PackingAncestryCache,
        slot: usize,
        current: PackingNodeRef,
        other: PackingNodeRef,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let current_acyclic = match cache.current_acyclic {
            Some(acyclic) => acyclic,
            None => {
                cache.current.clear();
                let acyclic = self.packing_ancestry_chain_into(current, &mut cache.current);
                cache.current_acyclic = Some(acyclic);
                acyclic
            }
        };
        let found = if current_acyclic {
            PackingAncestryCache::search(&cache.current, other, guard)?
        } else {
            self.packing_descendant_checked(Some(current), Some(other), false, guard)?
        };
        if found {
            return Ok(true);
        }
        let range = match cache.slots[slot] {
            Some(range) => range,
            None => {
                let start = cache.store.len();
                let range = self
                    .packing_ancestry_chain_into(other, &mut cache.store)
                    .then_some((start, cache.store.len()));
                cache.slots[slot] = Some(range);
                range
            }
        };
        match range {
            Some((start, end)) => {
                PackingAncestryCache::search(&cache.store[start..end], current, guard)
            }
            None => self.packing_descendant_checked(Some(other), Some(current), false, guard),
        }
    }

    /// node.go::reachableNodesGuarded for packing's includeContainers=true,
    /// traverseTrees=false calls. The predicate's error is deferred exactly
    /// like packGuarded's visitErr; a later traversal guard error wins.
    pub(super) fn packing_reachable_checked(
        &self,
        start: PackingNodeRef,
        ignore: Option<PackingNodeRef>,
        root: Option<PackingNodeRef>,
        include_nears: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Vec<PackingNodeRef>, limits::Error> {
        // Go node.go::reachableNodesGuarded reads Graph.Nodes directly. The
        // arena instead resolves each slot to its concrete or virtual vessel.
        // With cancellation-only polls, retain that resolution for this walk;
        // fill lazily after the charged visit to preserve failure ordering.
        let mut effective_nodes = guard
            .context()
            .stable_packing_topology()
            .then(|| vec![None; self.node_order.len()]);
        // The same stability fixes every ancestry chain for this walk, so the
        // relation scan below replays cached chains with identical charges.
        let mut ancestry = effective_nodes
            .as_ref()
            .map(|_| PackingAncestryCache::new(self.node_order.len()));
        let mut queue = VecDeque::from([start]);
        let mut visited = PackingVisited::new(self.nodes.len());
        visited.insert(start);
        let mut result = Vec::new();
        let mut visit_error = None;
        while let Some(current) = queue.pop_front() {
            guard.step()?;
            result.push(current);
            if let Some(cache) = &mut ancestry {
                cache.current_acyclic = None;
            }
            for edge in self.packing_incidents(current) {
                guard.step()?;
                self.packing_enqueue(
                    self.packing_adjacent(current, edge),
                    ignore,
                    root,
                    &mut visited,
                    &mut queue,
                    &mut visit_error,
                    guard,
                );
            }
            if include_nears {
                let mut nears = self.packing_ordered_nears(current);
                if let Some(members) = self.packing_cluster_members(current) {
                    for member in members {
                        nears.extend(self.packing_ordered_nears(member));
                    }
                }
                // Source sorts each member's map separately, then appends;
                // duplicates across members still consume work before queueing.
                for near in nears {
                    guard.step()?;
                    let near = self.packing_near_vessel(near);
                    if self.packing_raw_container(near) != self.packing_raw_container(start) {
                        continue;
                    }
                    self.packing_enqueue(
                        near,
                        ignore,
                        root,
                        &mut visited,
                        &mut queue,
                        &mut visit_error,
                        guard,
                    );
                }
            }
            if let Some(i) = self.packing_sequence_backlink(current) {
                for &member in &self.sequences[i].members {
                    guard.step()?;
                    self.packing_enqueue(
                        PackingNodeRef::Node(member),
                        ignore,
                        root,
                        &mut visited,
                        &mut queue,
                        &mut visit_error,
                        guard,
                    );
                }
            }
            if let Some(members) = self.packing_cluster_members(current) {
                for member in members {
                    guard.step()?;
                    self.packing_enqueue(
                        member,
                        ignore,
                        root,
                        &mut visited,
                        &mut queue,
                        &mut visit_error,
                        guard,
                    );
                }
            }
            for (slot, &other) in self.node_order.iter().enumerate() {
                guard.step()?;
                let other = match &mut effective_nodes {
                    Some(nodes) => {
                        *nodes[slot].get_or_insert_with(|| self.packing_effective_node(other))
                    }
                    None => self.packing_effective_node(other),
                };
                if visited.contains(&other) {
                    continue;
                }
                let related = match &mut ancestry {
                    Some(cache) => {
                        self.packing_related_cached(cache, slot, current, other, guard)?
                    }
                    None => {
                        self.packing_descendant_checked(Some(current), Some(other), false, guard)?
                            || self.packing_descendant_checked(
                                Some(other),
                                Some(current),
                                false,
                                guard,
                            )?
                    }
                };
                if related {
                    self.packing_enqueue(
                        other,
                        ignore,
                        root,
                        &mut visited,
                        &mut queue,
                        &mut visit_error,
                        guard,
                    );
                }
            }
        }
        if let Some(error) = visit_error {
            return Err(error);
        }
        Ok(result)
    }

    fn packing_descendant_children(&self, node: PackingNodeRef) -> Vec<PackingNodeRef> {
        let mut children = Vec::new();
        // Reverse pushes implement the source container/cluster/sequence preorder.
        for (i, s) in self.sequences.iter().enumerate() {
            if self.packing_vessel(false, i) == node {
                children.extend(s.members.iter().rev().map(|&n| PackingNodeRef::Node(n)));
            }
        }
        if let Some(members) = self.packing_cluster_members(node) {
            children.extend(members.into_iter().rev());
        }
        if let PackingNodeRef::Node(node) = node {
            if self.nodes[node.0 as usize].is_container {
                children.extend(
                    self.containers
                        .get(&Some(node))
                        .unwrap_or_default()
                        .into_iter()
                        .rev()
                        .map(|n| self.packing_effective_node(n)),
                );
            }
        }
        children
    }

    pub(super) fn packing_descendants_checked(
        &self,
        root: PackingNodeRef,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Vec<PackingNodeRef>, limits::Error> {
        let mut seen = BTreeSet::from([root]);
        let mut stack = Vec::new();
        let mut result = Vec::new();
        for child in self.packing_descendant_children(root) {
            guard.step()?;
            stack.push(child);
        }
        while let Some(node) = stack.pop() {
            guard.step()?;
            if !seen.insert(node) {
                continue;
            }
            result.push(node);
            for child in self.packing_descendant_children(node) {
                guard.step()?;
                stack.push(child);
            }
        }
        guard.finish()?;
        Ok(result)
    }

    pub(super) fn packing_external_checked(
        &self,
        node: PackingNodeRef,
        container: Option<PackingNodeRef>,
        near: bool,
        container_is_external: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let Some(container) = container else {
            return Ok(false);
        };
        let descendants = self.packing_descendants_checked(node, guard)?;
        let mut to_visit = vec![node];
        for descendant in descendants {
            guard.step()?;
            to_visit.push(descendant);
        }
        for current in to_visit {
            guard.step()?;
            if near {
                let raw = match current {
                    PackingNodeRef::Node(n) => self.nodes[n.0 as usize].nears.raw_keys(),
                    _ => Vec::new(),
                };
                if raw.len() > 10_000 {
                    return Err(limits::Error::Invalid(
                        "layout invariant violated: BinPack node near count exceeds node limit"
                            .into(),
                    ));
                }
                let mut nears = Vec::with_capacity(raw.len());
                for near in raw {
                    guard.step()?;
                    let Some(near) = near else {
                        return Err(limits::Error::Invalid(
                            "layout invariant violated: BinPack found a nil near".into(),
                        ));
                    };
                    nears.push(PackingNodeRef::Node(near));
                }
                let mut width = nears.len();
                while width > 1 {
                    for _ in &nears {
                        guard.step()?;
                    }
                    width = width.div_ceil(2);
                }
                nears.sort_by_key(|node| self.packing_ref_id(*node));
                guard.finish()?;
                for adjacent in nears {
                    if !self.packing_descendant_checked(
                        Some(adjacent),
                        Some(container),
                        true,
                        guard,
                    )? {
                        return Ok(true);
                    }
                }
            } else {
                for edge in self.packing_incidents(current) {
                    guard.step()?;
                    let adjacent = self.packing_adjacent(current, edge);
                    if container_is_external && adjacent == container {
                        return Ok(true);
                    }
                    if !self.packing_descendant_checked(
                        Some(adjacent),
                        Some(container),
                        true,
                        guard,
                    )? {
                        return Ok(true);
                    }
                }
            }
        }
        guard.finish()?;
        Ok(false)
    }

    pub(super) fn packing_ref_id(&self, node: PackingNodeRef) -> i64 {
        match node {
            PackingNodeRef::Node(n) => self.nodes[n.0 as usize].tala_id,
            PackingNodeRef::Cluster(i) => self.clusters[i].vessel_tala_id,
            PackingNodeRef::Sequence(i) => self.sequences[i].vessel_tala_id,
        }
    }

    pub(super) fn packing_ref_for_id(&self, id: i64) -> Option<PackingNodeRef> {
        self.nodes
            .iter()
            .find(|n| n.tala_id == id)
            .map(|n| PackingNodeRef::Node(n.input_id))
            .or_else(|| {
                self.clusters
                    .iter()
                    .position(|g| g.vessel_tala_id == id)
                    .map(|i| self.packing_vessel(true, i))
            })
            .or_else(|| {
                self.sequences
                    .iter()
                    .position(|g| g.vessel_tala_id == id)
                    .map(|i| self.packing_vessel(false, i))
            })
    }

    pub(super) fn packing_owning_container(&self, node: PackingNodeRef) -> Option<PackingNodeRef> {
        if let Some(i) = self.packing_cluster_backlink(node) {
            if self.packing_group_active(true, i) {
                return self.packing_raw_container(self.packing_vessel(true, i));
            }
        }
        if let Some(i) = self.packing_sequence_backlink(node) {
            if self.packing_group_active(false, i) {
                return self.packing_raw_container(self.packing_vessel(false, i));
            }
        }
        self.packing_raw_container(node)
    }

    pub(super) fn packing_direct_refs(&self, scope: Option<NodeId>) -> Vec<PackingNodeRef> {
        let raw = self
            .containers
            .get(&scope)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        let compact = raw
            .iter()
            .any(|&n| self.bin_pack_node_is_vessel_carrier(self.active_aggregate_owner(n)));
        if compact {
            self.bin_pack_direct_children(scope)
        } else {
            raw
        }
        .into_iter()
        .map(|n| self.packing_effective_node(n))
        .collect::<Vec<_>>()
    }

    pub(super) fn packing_subgraphs_checked(
        &self,
        scope: Option<NodeId>,
        edges_placed: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(Vec<Vec<PackingNodeRef>>, Vec<Vec<PackingNodeRef>>, usize), limits::Error> {
        let root = scope.map(PackingNodeRef::Node);
        let direct = self.packing_direct_refs(scope);
        let mut added = BTreeSet::new();
        let mut fixed = Vec::new();
        for &node in &direct {
            guard.step()?;
            let PackingNodeRef::Node(n) = node else {
                continue;
            };
            if self.nodes[n.0 as usize].fixed_top_left.is_none() {
                continue;
            }
            for reached in self.packing_reachable_checked(node, root, None, true, guard)? {
                guard.step()?;
                if added.insert(reached) {
                    fixed.push(reached);
                }
            }
        }
        let mut contained_set = BTreeSet::new();
        let mut contained = Vec::new();
        for &node in &direct {
            guard.step()?;
            if added.contains(&node) {
                continue;
            }
            let reachable = self.packing_reachable_checked(node, root, root, true, guard)?;
            let mut cross = false;
            for &reached in &reachable {
                guard.step()?;
                if matches!(reached,PackingNodeRef::Node(n) if self.nodes[n.0 as usize].hierarchy.is_some())
                    || self.packing_external_checked(reached, root, false, edges_placed, guard)?
                    || self.packing_external_checked(reached, root, true, false, guard)?
                {
                    cross = true;
                    break;
                }
            }
            if !cross {
                for &reached in &reachable {
                    guard.step()?;
                    added.insert(reached);
                    contained_set.insert(reached);
                }
                contained.push(reachable);
            }
        }
        let mut packed = Vec::new();
        for node in direct {
            guard.step()?;
            if added.contains(&node) {
                continue;
            }
            let mut filtered = Vec::new();
            for reached in self.packing_reachable_checked(node, root, None, false, guard)? {
                guard.step()?;
                if !contained_set.contains(&reached) {
                    filtered.push(reached);
                }
            }
            for &reached in &filtered {
                guard.step()?;
                added.insert(reached);
            }
            packed.push(filtered);
        }
        let cross = packed.len();
        if !fixed.is_empty() {
            packed.push(fixed);
        }
        Ok((packed, contained, cross))
    }

    /// Only the downstream numerical placement layer still needs carriers.
    /// The checked walk, its visited set and diagnostic output retain both
    /// vessel and member identities and charge their actual separate visits.
    pub(super) fn packing_numerical_groups(
        &self,
        groups: Vec<Vec<PackingNodeRef>>,
    ) -> Vec<Vec<NodeId>> {
        groups
            .into_iter()
            .map(|group| {
                let mut seen = BTreeSet::new();
                group
                    .into_iter()
                    .filter_map(|n| {
                        let n = match n {
                            PackingNodeRef::Node(n) => n,
                            PackingNodeRef::Cluster(i) => self.clusters[i].members[0],
                            PackingNodeRef::Sequence(i) => self.sequences[i].members[0],
                        };
                        seen.insert(n).then_some(n)
                    })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::binpack_guard::PackingContext;
    use super::*;

    fn graph(bytes: &[u8]) -> ArenaGraph {
        let input = crate::internal::graphjson::graphjson_to_graph(bytes).unwrap();
        let mut arena = ArenaGraph::from_input(&input);
        arena.initialize_shared_topology();
        arena
    }
    #[test]
    fn isolated_reachability_charges_both_ancestry_walks_including_nil() {
        let graph = graph(br#"{"nodes":[{"id":1},{"id":2}],"containers":{"0":[1,2]}}"#);
        let start = PackingNodeRef::Node(NodeId(0));
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .packing_reachable_checked(start, None, None, true, &mut guard)
                .unwrap(),
            vec![start]
        );
        assert_eq!(guard.used(), 7);
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 6).unwrap();
        assert!(matches!(
            graph.packing_reachable_checked(start, None, None, true, &mut guard),
            Err(limits::Error::Limit { .. })
        ));
        assert_eq!(guard.used(), 7);
    }
    #[test]
    fn concrete_vessel_and_retained_members_are_separate_queue_entries() {
        let graph=graph(br#"{"nodes":[{"id":9},{"id":7},{"id":2}],"edges":[{"id":-1,"from":7,"to":2}],"containers":{"0":[9,7,2]},"clusters":{"9":{"Nodes":[7,2],"Arrangement":"Row","DesiredArrangement":"Row"}}}"#);
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        let nodes = graph
            .packing_reachable_checked(
                PackingNodeRef::Node(NodeId(0)),
                None,
                None,
                true,
                &mut guard,
            )
            .unwrap();
        assert_eq!(
            nodes
                .iter()
                .map(|&n| graph.packing_ref_id(n))
                .collect::<Vec<_>>(),
            vec![9, 7, 2]
        );
        assert_eq!(guard.used(), 10);
    }
    #[test]
    fn detached_vessel_has_no_incidents_or_parent_but_retains_its_descendants() {
        let graph=graph(br#"{"nodes":[{"id":5},{"id":7},{"id":8},{"id":9}],"edges":[{"id":-1,"from":7,"to":9}],"containers":{"0":[5,9],"5":[7,8]},"clusters":{"10":{"Nodes":[7,8],"Container":5,"Arrangement":"Row","DesiredArrangement":"Row"}},"clusterVessels":{"10":{"id":10}}}"#);
        let vessel = graph.packing_ref_for_id(10).unwrap();
        assert!(graph.packing_incidents(vessel).is_empty());
        assert_eq!(graph.packing_raw_container(vessel), None);
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        assert!(
            graph
                .packing_external_checked(
                    vessel,
                    Some(PackingNodeRef::Node(NodeId(0))),
                    false,
                    false,
                    &mut guard
                )
                .unwrap()
        );
        assert_eq!(guard.used(), 10);
    }
    #[test]
    fn traversal_guard_failure_precedes_a_deferred_predicate_cycle_error() {
        let mut graph=graph(br#"{"nodes":[{"id":1},{"id":2},{"id":3}],"edges":[{"id":-1,"from":1,"to":2}],"containers":{"0":[1,2,3]}}"#);
        // A direct field mutation after decoding isolates shouldVisit's
        // delayed error contract from GraphJSON's separate topology preflight.
        graph.nodes[1].container = Some(NodeId(1));
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 20).unwrap();
        let root = Some(PackingNodeRef::Node(NodeId(2)));
        assert!(matches!(
            graph.packing_reachable_checked(
                PackingNodeRef::Node(NodeId(0)),
                root,
                root,
                false,
                &mut guard
            ),
            Err(limits::Error::Limit { .. })
        ));
        assert_eq!(guard.used(), 21);
    }
    #[test]
    fn external_near_budget_check_precedes_the_nil_key_diagnostic() {
        let mut graph = graph(br#"{"nodes":[{"id":1},{"id":2}],"containers":{"0":[1,2]}}"#);
        graph.nodes[1].nears = NodeNearMap::from_raw_keys([None]);
        for limit in [1, 2] {
            let mut guard =
                limits::WorkGuard::new(Some(&PackingContext), "BinPack", limit).unwrap();
            let error = graph
                .packing_external_checked(
                    PackingNodeRef::Node(NodeId(1)),
                    Some(PackingNodeRef::Node(NodeId(0))),
                    true,
                    false,
                    &mut guard,
                )
                .unwrap_err();
            if limit == 1 {
                assert!(matches!(error, limits::Error::Limit { .. }));
            } else {
                assert_eq!(
                    error.to_string(),
                    "layout invariant violated: BinPack found a nil near"
                );
            }
            assert_eq!(guard.used(), 2);
        }
    }
}
