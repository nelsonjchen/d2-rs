// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! The owning-graph inventory in layoutgraph/topology_preflight.go.
//!
//! This reads actual nullable headers and record handles. Its entry point
//! requires an owning graph view; constructing that view is a caller operation,
//! not a validation-time reconstruction of source identities or capacities.

use super::*;
use crate::internal::limits::{self, Error, WorkGuard};

#[cfg(feature = "diagnostic-traces")]
#[path = "topology_preflight_probe.rs"]
mod probe;

#[cfg(test)]
#[path = "topology_preflight_tests.rs"]
mod tests;

const MAX_REFERENCES: usize = 1_000_000;
const MAX_NODES: usize = 10_000;
const MAX_EDGES: usize = 50_000;
const MAX_POINTS: usize = 1_000_000;
const MAX_DEPTH: usize = 256;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn nil(kind: &str) -> Error {
    invalid(format!("TALA engine topology contains nil {kind}"))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
enum Entity {
    Node(NodeId),
    Edge(EdgeId),
    Group(OwnerGroupId),
    Tree(OwnerTreeId),
    Abduction(OwnerAbductionId),
    Herd(NodeId),
    Hierarchy(NodeId),
}
impl Entity {
    fn queue(self) -> usize {
        match self {
            Self::Node(_) => 0,
            Self::Edge(_) => 1,
            Self::Group(g) => match g.kind() {
                OwnerGroupKind::Cluster => 2,
                OwnerGroupKind::Sequence => 3,
            },
            Self::Tree(_) => 4,
            Self::Abduction(_) => 5,
            Self::Herd(_) => 6,
            Self::Hierarchy(_) => 7,
        }
    }
}

struct Preflight<'a, 'b> {
    graph: &'a ArenaGraph,
    receiver: OwnerGraphId,
    guard: &'a mut WorkGuard<'b>,
    references: usize,
    points: usize,
    // Membership only; the walks that iterate a set sort it first.
    seen: [IdHashSet<Entity>; 8],
    queues: [VecDeque<Entity>; 8],
    herd_allocations: IdHashSet<usize>,
    hierarchy_allocations: IdHashSet<usize>,
}
impl<'a, 'b> Preflight<'a, 'b> {
    fn queue(&mut self, entity: Option<Entity>, required: bool, kind: &str) -> Result<(), Error> {
        let Some(entity) = entity else {
            return if required { Err(nil(kind)) } else { Ok(()) };
        };
        if self.references >= MAX_REFERENCES {
            return Err(invalid(format!(
                "TALA engine topology references exceed limit {MAX_REFERENCES} while visiting {kind}"
            )));
        }
        self.references += 1;
        self.guard.step()?;
        let index = entity.queue();
        if self.seen[index].contains(&entity) {
            return Ok(());
        }
        if index == 0 && self.seen[0].len() >= MAX_NODES {
            return Err(invalid(format!(
                "TALA engine unique node count exceeds limit {MAX_NODES}"
            )));
        }
        if index == 1 && self.seen[1].len() >= MAX_EDGES {
            return Err(invalid(format!(
                "TALA engine unique edge count exceeds limit {MAX_EDGES}"
            )));
        }
        // A HerdAssignment may be shared by several Node.HerdAssignment fields.
        // The node handle locates the record; the actual allocation de-duplicates it.
        if let Entity::Herd(node) = entity {
            if let Some(Some(herd)) = self.graph.shared_topology.node_herds.get(&node) {
                if !self.herd_allocations.insert(herd.allocation_key()) {
                    return Ok(());
                }
            }
        }
        if let Entity::Hierarchy(node) = entity {
            let hierarchy = self
                .graph
                .owner_node_hierarchy(node)
                .map_err(|error| invalid(error.to_string()))?
                .expect("queued hierarchy");
            if !self
                .hierarchy_allocations
                .insert(hierarchy.allocation_key())
            {
                return Ok(());
            }
        }
        self.seen[index].insert(entity);
        self.queues[index].push_back(entity);
        Ok(())
    }
    /// One seen set in its former BTreeSet order.
    fn sorted_seen(&self, index: usize) -> Vec<Entity> {
        let mut entities = self.seen[index].iter().copied().collect::<Vec<_>>();
        entities.sort_unstable();
        entities
    }
    fn node(&mut self, node: Option<NodeId>, required: bool, kind: &str) -> Result<(), Error> {
        self.queue(node.map(Entity::Node), required, kind)
    }
    fn edge(&mut self, edge: Option<EdgeId>, required: bool, kind: &str) -> Result<(), Error> {
        self.queue(edge.map(Entity::Edge), required, kind)
    }
    fn spare(&mut self, kind: &str, len: usize, cap: usize) -> Result<(), Error> {
        let Some(spare) = cap.checked_sub(len) else {
            return Err(invalid(format!(
                "TALA engine {kind} has invalid slice capacity"
            )));
        };
        if spare > MAX_REFERENCES - self.references {
            return Err(invalid(format!(
                "TALA engine topology references exceed limit {MAX_REFERENCES} while reserving {kind} capacity"
            )));
        }
        self.references += spare;
        self.guard.check()
    }
    fn nodes(
        &mut self,
        header: &SharedNodeSlice,
        capacity_kind: &str,
        kind: &str,
    ) -> Result<(), Error> {
        self.spare(capacity_kind, header.len(), header.capacity())?;
        for node in header.borrowed_slots() {
            self.node(node, true, kind)?;
        }
        Ok(())
    }
    fn endpoints(&self, edge: EdgeId) -> (Option<NodeId>, Option<NodeId>) {
        (
            self.graph
                .owner_edge_endpoint(edge, false)
                .expect("owning edge"),
            self.graph
                .owner_edge_endpoint(edge, true)
                .expect("owning edge"),
        )
    }
    fn view(&self) -> &OwnerGraphView {
        self.graph
            .owner_graph_view(self.receiver)
            .expect("validated owning graph")
    }
    fn inventory_roots(&mut self) -> Result<(), Error> {
        let nodes = self.view().nodes.header();
        self.spare("graph nodes", nodes.len(), nodes.capacity())?;
        for (index, node) in nodes.borrowed_slots().enumerate() {
            if node.is_none() {
                return Err(invalid(format!("graph node at index {index} is nil")));
            }
            self.node(node, true, "graph node")?;
        }
        let edges = self
            .graph
            .owner_graph_edge_header(self.receiver)
            .map_err(|e| invalid(e.to_string()))?;
        self.spare("graph edges", edges.len(), edges.capacity())?;
        for (index, edge) in edges.slice.borrowed_slots().enumerate() {
            let Some(edge) = edge else {
                return Err(invalid(format!("graph edge at index {index} is nil")));
            };
            let (from, to) = self.endpoints(edge);
            if from.is_none() || to.is_none() {
                return Err(invalid(format!(
                    "graph edge {} has unplaced or missing endpoints",
                    self.graph.edge_records[edge.0 as usize].tala_id
                )));
            }
            self.edge(Some(edge), true, "graph edge")?;
        }
        for (container, children) in self.graph.containers.share_map().entries() {
            self.node(container, false, "container key")?;
            self.nodes(&children, "container children", "container child")?;
        }
        for (kind, label) in [
            (OwnerGroupKind::Cluster, "cluster"),
            (OwnerGroupKind::Sequence, "sequence"),
        ] {
            let entries = match kind {
                OwnerGroupKind::Cluster => self.view().clusters.entries(),
                OwnerGroupKind::Sequence => self.view().sequences.entries(),
            };
            for (vessel, group) in entries {
                self.node(vessel, true, &format!("{label} vessel key"))?;
                self.queue(group.map(Entity::Group), true, &format!("{label} record"))?;
            }
        }
        for (root, roots) in self.view().tree_roots.entries() {
            self.node(root, false, "tree root key")?;
            self.spare("tree roots", roots.len(), roots.capacity())?;
            for index in 0..roots.len() {
                self.queue(
                    roots.get(index).flatten().map(Entity::Tree),
                    true,
                    "tree root",
                )?;
            }
        }
        let mut aliases = Vec::new();
        self.view().node_to_tree.validate_entries(|node, tree| {
            aliases.push((node, tree));
            Ok::<_, Error>(())
        })?;
        for (node, tree) in aliases {
            self.node(node, true, "node-to-tree key")?;
            self.queue(tree.map(Entity::Tree), true, "node-to-tree value")?;
        }
        // Hubs and common-sibling raw storage are installed on the owning view.
        // They are never synthesized from a numerical Vec's final length.
        {
            let hubs = self.view().hubs.header();
            for (hub, spokes) in hubs.entries() {
                self.node(hub, true, "hub key")?;
                self.nodes(&spokes, "hub spokes", "hub spoke")?;
            }
        }
        for key in self.graph.owner_preflight_direction_keys(self.receiver) {
            self.node(key, false, "direction key")?;
        }
        {
            let siblings = self.view().common_uncle_siblings.header();
            for (node, siblings) in siblings.entries() {
                self.node(node, true, "common-sibling key")?;
                self.nodes(&siblings, "common siblings", "common sibling")?;
            }
        }
        Ok(())
    }
    fn inventory(&mut self) -> Result<(), Error> {
        self.inventory_roots()?;
        while let Some(entity) = self.queues.iter_mut().find_map(VecDeque::pop_front) {
            match entity {
                Entity::Node(id) => {
                    let node = &self.graph.nodes[id.0 as usize];
                    self.node(node.container, false, "node container")?;
                    let edges = self
                        .graph
                        .owner_node_incident_header(id)
                        .map_err(|e| invalid(e.to_string()))?;
                    self.spare("node edges", edges.len(), edges.capacity())?;
                    for edge in edges.slice.borrowed_slots() {
                        self.edge(edge, true, "node edge")?;
                    }
                    for near in node.nears.raw_keys() {
                        self.node(near, true, "near node")?;
                    }
                    if let Some(requirements) = self
                        .graph
                        .shared_topology
                        .long_distance_requirements
                        .get(&id)
                    {
                        for neighbor in requirements.raw_keys() {
                            self.node(neighbor, true, "long-distance neighbor requirement key")?;
                        }
                    }
                    for (kind, label) in [
                        (OwnerGroupKind::Cluster, "node cluster"),
                        (OwnerGroupKind::Sequence, "node sequence"),
                    ] {
                        let group = self
                            .graph
                            .owner_node_group_id(id, kind)
                            .map_err(|e| invalid(e.to_string()))?;
                        self.queue(group.map(Entity::Group), false, label)?;
                    }
                    self.queue(
                        self.graph
                            .has_herd_assignment(id)
                            .then_some(Entity::Herd(id)),
                        false,
                        "node herd assignment",
                    )?;
                    let hierarchy = self
                        .graph
                        .owner_node_hierarchy(id)
                        .map_err(|error| invalid(error.to_string()))?;
                    self.queue(
                        hierarchy.map(|_| Entity::Hierarchy(id)),
                        false,
                        "node hierarchy",
                    )?;
                }
                Entity::Edge(id) => {
                    let (from, to) = self.endpoints(id);
                    self.node(from, true, "edge source")?;
                    self.node(to, true, "edge target")?;
                    let route = &self.graph.edge_records[id.0 as usize].route;
                    if route.capacity() > MAX_POINTS - self.points {
                        return Err(invalid(format!(
                            "TALA engine route point count exceeds limit {MAX_POINTS}"
                        )));
                    }
                    self.points += route.capacity();
                    let route_header = route.header();
                    for point in route_header.borrowed_slots() {
                        if point.is_none() {
                            return Err(nil("edge route point"));
                        }
                        self.guard.step()?;
                    }
                }
                Entity::Group(id) => {
                    let group = self.graph.owner_group(id).expect("owning group");
                    let fields = group.fields();
                    let label = match id.kind() {
                        OwnerGroupKind::Cluster => "cluster",
                        OwnerGroupKind::Sequence => "sequence",
                    };
                    self.node(fields.vessel, true, &format!("{label} vessel"))?;
                    self.node(fields.container, false, &format!("{label} container"))?;
                    self.nodes(
                        &fields.nodes,
                        &format!("{label} nodes"),
                        &format!("{label} node"),
                    )?;
                    let abductions = match id.kind() {
                        OwnerGroupKind::Cluster => group.policy().edge_abductions,
                        OwnerGroupKind::Sequence => group.sequence_abductions(),
                    };
                    let (len, cap) = abductions
                        .as_ref()
                        .map_or((0, 0), |h| (h.len(), h.capacity()));
                    self.spare(&format!("{label} edge abductions"), len, cap)?;
                    if let Some(abductions) = abductions {
                        for index in 0..len {
                            self.queue(
                                abductions.get(index).flatten().map(Entity::Abduction),
                                true,
                                &format!("{label} edge abduction"),
                            )?;
                        }
                    }
                }
                Entity::Tree(id) => {
                    let tree = self.graph.owner_tree(id).expect("owning tree").fields();
                    self.node(tree.node, true, "tree node")?;
                    self.queue(tree.parent.map(Entity::Tree), false, "tree parent")?;
                    self.edge(tree.sentinel_edge, false, "tree sentinel edge")?;
                    self.spare(
                        "tree children",
                        tree.children.len(),
                        tree.children.capacity(),
                    )?;
                    for index in 0..tree.children.len() {
                        self.queue(
                            tree.children.get(index).flatten().map(Entity::Tree),
                            true,
                            "tree child",
                        )?;
                    }
                }
                Entity::Abduction(id) => {
                    let record = self
                        .graph
                        .owner_edge_abduction(id)
                        .expect("owning abduction");
                    self.edge(record.source_edge, true, "abducted edge")?;
                    for (node, kind) in [
                        (record.originally_from, "abduction original source"),
                        (record.originally_to, "abduction original target"),
                        (record.current_from, "abduction current source"),
                        (record.current_to, "abduction current target"),
                    ] {
                        self.node(node, false, kind)?;
                    }
                }
                Entity::Herd(id) => {
                    if let Some(Some(herd)) = self.graph.shared_topology.node_herds.get(&id) {
                        let fields = herd.fields.read().expect("herd read").clone();
                        for (keys, kind) in [
                            (fields.opposite_side.keys(), "opposite-side herd node"),
                            (fields.same_side.keys(), "same-side herd node"),
                        ] {
                            for node in keys {
                                self.node(node, true, kind)?;
                            }
                        }
                    } else if let Some(herd) = &self.graph.nodes[id.0 as usize].herd_assignment {
                        for (keys, kind) in [
                            (&herd.opposite_side_paired, "opposite-side herd node"),
                            (&herd.same_side_paired, "same-side herd node"),
                        ] {
                            for node in keys {
                                self.node(
                                    (*node >= 0).then_some(NodeId(*node as u32)),
                                    true,
                                    kind,
                                )?;
                            }
                        }
                    }
                }
                Entity::Hierarchy(id) => {
                    let hierarchy = self
                        .graph
                        .owner_node_hierarchy(id)
                        .map_err(|error| invalid(error.to_string()))?
                        .expect("inventoried hierarchy");
                    let fields = self
                        .graph
                        .owner_hierarchy_fields(&hierarchy)
                        .map_err(|error| invalid(error.to_string()))?;
                    for (node, _) in fields.levels.entries() {
                        self.node(node, true, "hierarchy level node")?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Preflight<'_, '_> {
    fn parent(&self, node: NodeId, relation: usize) -> Option<NodeId> {
        let record = &self.graph.nodes[node.0 as usize];
        if relation == 0 {
            return record.container;
        }
        if relation == 2 && record.container.is_some() {
            return record.container;
        }
        for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
            if let Some(group) = self
                .graph
                .owner_node_group_id(node, kind)
                .expect("inventoried node group")
            {
                let fields = self
                    .graph
                    .owner_group(group)
                    .expect("inventoried group")
                    .read_fields();
                if relation == 2 {
                    return fields.vessel;
                }
                // Source Group.isActive reads Vessel.Graph; deactivated groups
                // still retain their vessel and member pointers.
                if let Some(vessel) = fields.vessel {
                    if self.graph.topology_node_owner(vessel).is_some() {
                        return self.graph.nodes[vessel.0 as usize].container;
                    }
                }
            }
        }
        record.container
    }
    fn parents(&mut self) -> Result<(), Error> {
        for (relation, name) in [
            (0, "container parent"),
            (1, "effective container parent"),
            (2, "ancestry parent"),
        ] {
            let mut state = IdHashMap::<NodeId, u8>::default();
            let mut depths = IdHashMap::<NodeId, usize>::default();
            for entity in self.sorted_seen(0) {
                let Entity::Node(node) = entity else {
                    unreachable!()
                };
                if state.get(&node) == Some(&2) {
                    continue;
                }
                let mut path = Vec::new();
                let mut current = Some(node);
                while let Some(node) = current {
                    if state.get(&node).copied().unwrap_or(0) != 0 {
                        break;
                    }
                    self.guard.step()?;
                    state.insert(node, 1);
                    path.push(node);
                    current = self.parent(node, relation);
                }
                if let Some(node) = current {
                    if state.get(&node) == Some(&1) {
                        return Err(invalid(format!(
                            "TALA engine {name} cycle detected at node {}",
                            self.graph.nodes[node.0 as usize].tala_id
                        )));
                    }
                }
                let mut depth = current
                    .and_then(|node| depths.get(&node).copied())
                    .unwrap_or(0);
                for node in path.into_iter().rev() {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err(invalid(format!(
                            "TALA engine {name} depth exceeds limit {MAX_DEPTH}"
                        )));
                    }
                    depths.insert(node, depth);
                    state.insert(node, 2);
                }
            }
        }
        Ok(())
    }
    fn descendant_children(&self, node: Option<NodeId>) -> Result<Vec<NodeId>, Error> {
        let mut children = Vec::new();
        if node.is_none() || node.is_some_and(|id| self.graph.nodes[id.0 as usize].is_container) {
            if let Some(header) = self.graph.containers.get_header(&node) {
                children.extend(header.require_nodes());
            }
        }
        if let Some(node) = node {
            if self.graph.owner_is_cluster_vessel(node) {
                let Some(Some(group)) = self.view().clusters.get(Some(node)) else {
                    return Err(invalid(format!(
                        "TALA engine cluster vessel {} has no cluster record",
                        self.graph.nodes[node.0 as usize].tala_id
                    )));
                };
                children.extend(
                    self.graph
                        .owner_group(group)
                        .expect("owning cluster")
                        .read_fields()
                        .nodes
                        .copied_nodes(),
                );
            }
        }
        if let Some(group) = self.view().sequences.get(node) {
            let group = group.ok_or_else(|| nil("sequence record"))?;
            children.extend(
                self.graph
                    .owner_group(group)
                    .expect("owning sequence")
                    .read_fields()
                    .nodes
                    .copied_nodes(),
            );
        }
        Ok(children)
    }
    fn descendants(&mut self) -> Result<(), Error> {
        struct Frame {
            node: Option<NodeId>,
            children: Vec<NodeId>,
            next: usize,
        }
        let mut starts = Vec::new();
        if self.graph.containers.get_header(&None).is_some() {
            starts.push(None);
        }
        starts.extend(self.sorted_seen(0).into_iter().map(|entity| match entity {
            Entity::Node(node) => Some(node),
            _ => unreachable!(),
        }));
        let mut color = IdHashMap::<Option<NodeId>, u8>::default();
        let mut heights = IdHashMap::<Option<NodeId>, usize>::default();
        for start in starts {
            if color.get(&start).copied().unwrap_or(0) != 0 {
                continue;
            }
            let children = self.descendant_children(start)?;
            color.insert(start, 1);
            let mut stack = vec![Frame {
                node: start,
                children,
                next: 0,
            }];
            while let Some(frame) = stack.last_mut() {
                if frame.next == frame.children.len() {
                    let own = usize::from(frame.node.is_some());
                    let height = frame
                        .children
                        .iter()
                        .map(|child| own + heights.get(&Some(*child)).copied().unwrap_or(0))
                        .max()
                        .unwrap_or(own);
                    if height > MAX_DEPTH {
                        return Err(invalid(format!(
                            "TALA engine descendant depth exceeds limit {MAX_DEPTH}"
                        )));
                    }
                    heights.insert(frame.node, height);
                    color.insert(frame.node, 2);
                    stack.pop();
                    continue;
                }
                let child = frame.children[frame.next];
                frame.next += 1;
                self.guard.step()?;
                match color.get(&Some(child)).copied().unwrap_or(0) {
                    1 => {
                        return Err(invalid(format!(
                            "TALA engine descendant cycle detected at node {}",
                            self.graph.nodes[child.0 as usize].tala_id
                        )));
                    }
                    0 => {
                        let children = self.descendant_children(Some(child))?;
                        color.insert(Some(child), 1);
                        stack.push(Frame {
                            node: Some(child),
                            children,
                            next: 0,
                        });
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    fn tree_fields(&self, tree: OwnerTreeId) -> super::super::owner_tree_map::OwnerTreeFields {
        self.graph
            .owner_tree(tree)
            .expect("inventoried tree")
            .fields()
    }
    fn trees(&mut self) -> Result<(), Error> {
        let trees = self
            .sorted_seen(4)
            .into_iter()
            .map(|entity| match entity {
                Entity::Tree(tree) => tree,
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        let mut color = IdHashMap::<OwnerTreeId, u8>::default();
        let mut heights = IdHashMap::<OwnerTreeId, usize>::default();
        for &start in &trees {
            if color.get(&start).copied().unwrap_or(0) != 0 {
                continue;
            }
            color.insert(start, 1);
            let mut stack = vec![(start, 0)];
            while let Some(&(tree, index)) = stack.last() {
                let record = self.graph.owner_tree(tree).expect("inventoried tree");
                let fields = record.read_fields();
                if index >= fields.children.len() {
                    let height = (0..fields.children.len())
                        .map(|i| {
                            1 + heights
                                .get(
                                    &fields
                                        .children
                                        .get(i)
                                        .flatten()
                                        .expect("validated tree child"),
                                )
                                .copied()
                                .unwrap_or(0)
                        })
                        .max()
                        .unwrap_or(1);
                    if height > MAX_DEPTH {
                        return Err(invalid(format!(
                            "TALA engine tree depth exceeds limit {MAX_DEPTH}"
                        )));
                    }
                    heights.insert(tree, height);
                    color.insert(tree, 2);
                    stack.pop();
                    continue;
                }
                stack.last_mut().unwrap().1 += 1;
                let child = fields
                    .children
                    .get(index)
                    .flatten()
                    .expect("validated tree child");
                drop(fields);
                self.guard.step()?;
                match color.get(&child).copied().unwrap_or(0) {
                    1 => return Err(invalid("TALA engine tree child cycle detected")),
                    0 => {
                        color.insert(child, 1);
                        stack.push((child, 0));
                    }
                    _ => {}
                }
            }
        }
        let mut state = BTreeMap::<OwnerTreeId, u8>::new();
        let mut depths = BTreeMap::<OwnerTreeId, usize>::new();
        for start in trees {
            if state.get(&start) == Some(&2) {
                continue;
            }
            let mut current = Some(start);
            let mut path = Vec::new();
            while let Some(tree) = current {
                if state.get(&tree).copied().unwrap_or(0) != 0 {
                    break;
                }
                self.guard.step()?;
                state.insert(tree, 1);
                path.push(tree);
                current = self
                    .graph
                    .owner_tree(tree)
                    .expect("inventoried tree")
                    .read_fields()
                    .parent;
            }
            if current.is_some_and(|tree| state.get(&tree) == Some(&1)) {
                return Err(invalid("TALA engine tree parent cycle detected"));
            }
            let mut depth = current
                .and_then(|tree| depths.get(&tree).copied())
                .unwrap_or(0);
            for tree in path.into_iter().rev() {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(invalid(format!(
                        "TALA engine tree depth exceeds limit {MAX_DEPTH}"
                    )));
                }
                depths.insert(tree, depth);
                state.insert(tree, 2);
            }
        }
        let mut owned = BTreeMap::<OwnerTreeId, Option<OwnerTreeId>>::new();
        let mut by_node = BTreeMap::<NodeId, OwnerTreeId>::new();
        for (_, roots) in self.view().tree_roots.entries() {
            for index in 0..roots.len() {
                let root = roots.get(index).flatten().expect("validated root");
                let mut stack = vec![(root, None)];
                while let Some((tree, parent)) = stack.pop() {
                    self.guard.step()?;
                    if let Some(previous) = owned.get(&tree) {
                        return Err(invalid(if parent.is_none() {
                            "TALA engine tree is listed as more than one root"
                        } else if *previous == parent {
                            "TALA engine tree is repeated under one parent"
                        } else {
                            "TALA engine tree is shared by multiple parents"
                        }));
                    }
                    owned.insert(tree, parent);
                    let fields = self.tree_fields(tree);
                    let node = fields.node.expect("validated tree node");
                    if by_node.get(&node).is_some_and(|previous| *previous != tree) {
                        return Err(invalid(format!(
                            "TALA engine node {} is owned by multiple trees",
                            self.graph.nodes[node.0 as usize].tala_id
                        )));
                    }
                    by_node.insert(node, tree);
                    for index in (0..fields.children.len()).rev() {
                        let child = fields
                            .children
                            .get(index)
                            .flatten()
                            .expect("validated tree child");
                        if self.tree_fields(child).parent != Some(tree) {
                            return Err(invalid(
                                "TALA engine tree child has an inconsistent parent",
                            ));
                        }
                        stack.push((child, Some(tree)));
                    }
                }
            }
        }
        for (_, roots) in self.view().tree_roots.entries() {
            for index in 0..roots.len() {
                let root = roots.get(index).flatten().expect("validated root");
                let Some(parent) = self.tree_fields(root).parent else {
                    continue;
                };
                if owned.contains_key(&parent) {
                    return Err(invalid(
                        "TALA engine tree root also has an installed parent",
                    ));
                }
                let children = self.tree_fields(parent).children;
                let mut occurrences = 0;
                for index in 0..children.len() {
                    self.guard.step()?;
                    occurrences += usize::from(children.get(index).flatten() == Some(root));
                }
                if occurrences != 1 {
                    return Err(invalid(
                        "TALA engine tree root has an inconsistent placement parent",
                    ));
                }
            }
        }
        let mut aliases = Vec::new();
        self.view().node_to_tree.validate_entries(|node, tree| {
            aliases.push((node, tree));
            Ok::<_, Error>(())
        })?;
        for &(node, tree) in &aliases {
            let tree = tree.expect("validated alias");
            if self.tree_fields(tree).node != node {
                return Err(invalid(
                    "TALA engine node-to-tree alias does not match the tree node",
                ));
            }
            if !owned.is_empty() && !owned.contains_key(&tree) {
                return Err(invalid(
                    "TALA engine node-to-tree alias references a tree outside the installed forest",
                ));
            }
        }
        if !aliases.is_empty() && !owned.is_empty() {
            if aliases.len() != by_node.len() {
                return Err(invalid(
                    "TALA engine node-to-tree aliases do not cover the installed forest",
                ));
            }
            for (node, tree) in by_node {
                if self.view().node_to_tree.get(Some(node)) != Some(Some(tree)) {
                    return Err(invalid(
                        "TALA engine node-to-tree aliases do not match the installed forest",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ArenaGraph {
    pub(in crate::internal) fn validate_owner_graph_topology_with_guard(
        &self,
        receiver: OwnerGraphId,
        guard: &mut WorkGuard<'_>,
    ) -> Result<(), Error> {
        if self.owner_graph_view(receiver).is_none() {
            return Err(invalid("TALA engine requires a graph"));
        }
        let mut state = Preflight {
            graph: self,
            receiver,
            guard,
            references: 0,
            points: 0,
            seen: std::array::from_fn(|_| IdHashSet::default()),
            queues: std::array::from_fn(|_| VecDeque::new()),
            herd_allocations: IdHashSet::default(),
            hierarchy_allocations: IdHashSet::default(),
        };
        state.inventory()?;
        state.parents()?;
        state.descendants()?;
        state.trees()?;
        state.guard.finish()
    }
    pub(in crate::internal) fn validate_owner_graph_topology(
        &self,
        receiver: Option<OwnerGraphId>,
        context: Option<&dyn limits::Context>,
        location: &str,
    ) -> Result<(), Error> {
        let receiver = receiver
            .filter(|id| self.owner_graph_view(*id).is_some())
            .ok_or_else(|| invalid("TALA engine requires a graph"))?;
        let mut guard = WorkGuard::new(context, location, limits::MAX_ENGINE_WORK_UNITS)?;
        guard.set_limit((MAX_REFERENCES * 8) as i64);
        self.validate_owner_graph_topology_with_guard(receiver, &mut guard)
    }
    fn owner_preflight_direction_keys(&self, _receiver: OwnerGraphId) -> Vec<Option<NodeId>> {
        if let Some(directions) = &self.shared_topology.placement_directions {
            return directions.keys().copied().collect();
        }
        self.directions
            .keys()
            .chain(self.shared_topology.direction_extensions.keys())
            .copied()
            .collect()
    }
}
