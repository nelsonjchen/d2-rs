// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Herd assignment for related nodes across container boundaries.
//!
//! Edge abductions group cousins by their shared outside container, then
//! propagate compatible directional assignments through each group.

use crate::internal::*;

impl Pipeline {
    /// Source GroupSheep consumes actual parent records, child-major. A used
    /// record is claimed once; repeated records/connections remain repeated.
    pub(in crate::internal) fn group_owner_sheep(
        &self,
        scope: NodeId,
        receiver: OwnerGraphId,
        abductions: &OwnerAbductionSlice,
    ) -> Result<Vec<(NodeId, Vec<(NodeId, Vec<NodeId>)>)>, crate::LayoutError> {
        let children = self
            .graph
            .owner_container_map(receiver)
            .expect("actual herd graph")
            .get(Some(scope));
        let mut used = vec![false; abductions.records.len()];
        let mut grouped = BTreeMap::<NodeId, Vec<(NodeId, Vec<NodeId>)>>::new();
        if let Some(children) = children {
            for child_index in 0..children.len() {
                herd_poll()?;
                let child = children
                    .get(child_index)
                    .expect("logical child slot")
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                for (index, &id) in abductions.records.iter().enumerate() {
                    herd_poll()?;
                    if used[index] {
                        continue;
                    }
                    let record = *self
                        .graph
                        .owner_edge_abduction(id)
                        .expect("actual parent abduction");
                    let from = herd_group_vessel(&self.graph, record.originally_from)?;
                    let to = herd_group_vessel(&self.graph, record.originally_to)?;
                    let candidate = if record.originally_to.is_some()
                        && herd_descendant(&self.graph, from, child)?
                    {
                        if record.current_to.is_some_and(|node| {
                            !self
                                .graph
                                .topology_node(node)
                                .expect("current endpoint")
                                .is_container
                        }) {
                            continue;
                        }
                        if to.is_none()
                            || herd_owning_container(&self.graph, to.unwrap())?.is_none()
                        {
                            continue;
                        }
                        Some((to.unwrap(), record.current_to))
                    } else if record.originally_from.is_some()
                        && herd_descendant(&self.graph, to, child)?
                    {
                        if record.current_from.is_some_and(|node| {
                            !self
                                .graph
                                .topology_node(node)
                                .expect("current endpoint")
                                .is_container
                        }) {
                            continue;
                        }
                        if from.is_none()
                            || herd_owning_container(&self.graph, from.unwrap())?.is_none()
                        {
                            continue;
                        }
                        Some((from.unwrap(), record.current_from))
                    } else {
                        None
                    };
                    let Some((mut cousin, current)) = candidate else {
                        continue;
                    };
                    used[index] = true;
                    while herd_owning_container(&self.graph, cousin)? != current {
                        let group = herd_group_vessel(&self.graph, Some(cousin))?;
                        cousin = if group != Some(cousin) {
                            group
                        } else {
                            herd_owning_container(&self.graph, cousin)?
                        }
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    }
                    let Some(uncle) = herd_owning_container(&self.graph, cousin)? else {
                        continue;
                    };
                    if !self
                        .graph
                        .topology_node(uncle)
                        .expect("herd uncle")
                        .is_container
                    {
                        continue;
                    }
                    let nodes = grouped.entry(uncle).or_default();
                    if let Some((_, cousins)) = nodes.iter_mut().find(|(node, _)| *node == child) {
                        cousins.push(cousin);
                    } else {
                        nodes.push((child, vec![cousin]));
                    }
                }
            }
        }
        Ok(grouped.into_iter().collect())
    }

    pub(in crate::internal) fn assign_herds_for_scope(
        &mut self,
        scope: NodeId,
        receiver: OwnerGraphId,
        abductions: &OwnerAbductionSlice,
    ) -> Result<(), crate::LayoutError> {
        herd_poll()?;
        #[cfg(feature = "diagnostic-traces")]
        let trace = crate::internal::trace_env_enabled("WEFTAN_TRACE_HERD_SCOPE");
        let mut groups = self
            .group_owner_sheep(scope, receiver, abductions)?
            .into_iter()
            .filter(|(_, nodes)| nodes.len() > 1)
            .collect::<Vec<_>>();
        #[cfg(feature = "diagnostic-traces")]
        if trace {
            eprintln!(
                "HERD_SCOPE_RUST root={} groups={groups:?}",
                self.graph.nodes[scope.0 as usize].tala_id
            );
            for cousins in groups
                .iter()
                .flat_map(|(_, nodes)| nodes.iter().map(|(_, cousins)| cousins))
            {
                for cousin in cousins {
                    eprintln!(
                        "HERD_SCOPE_RUST cousin={} assignment={:?}",
                        self.graph.nodes[cousin.0 as usize].tala_id,
                        self.graph.herd_assignment_snapshot(*cousin)
                    );
                }
            }
        }
        groups.sort_by_key(|(uncle, _)| self.graph.nodes[uncle.0 as usize].tala_id);

        let herd_groups = groups
            .iter()
            .map(|(_, nodes)| nodes.iter().map(|(node, _)| *node).collect::<Vec<_>>())
            .collect::<Vec<_>>();

        // Match Go's connectedHerds: groups that share a local node are one
        // equality component and therefore receive one common preferred side.
        // Assigning each uncle independently lets a later group flip a node's
        // side; that was the source of the ui.entry_list divergence.
        let mut groups_by_node = BTreeMap::<NodeId, Vec<usize>>::new();
        for (index, (_, nodes)) in groups.iter().enumerate() {
            for node in nodes.iter().map(|(node, _)| *node) {
                groups_by_node.entry(node).or_default().push(index);
            }
        }
        let mut seen_groups = BTreeSet::new();
        let mut components = Vec::<(Vec<usize>, Vec<NodeId>)>::new();
        for start in 0..groups.len() {
            if !seen_groups.insert(start) {
                continue;
            }
            let mut pending = VecDeque::from([start]);
            let mut component_groups = Vec::new();
            let mut component_nodes = Vec::new();
            let mut seen_nodes = BTreeSet::new();
            while let Some(group_index) = pending.pop_front() {
                herd_poll()?;
                component_groups.push(group_index);
                for node in groups[group_index].1.iter().map(|(node, _)| *node) {
                    if seen_nodes.insert(node) {
                        component_nodes.push(node);
                        for related in groups_by_node.get(&node).into_iter().flatten().copied() {
                            if seen_groups.insert(related) {
                                pending.push_back(related);
                            }
                        }
                    }
                }
            }
            components.push((component_groups, component_nodes));
        }
        #[cfg(feature = "diagnostic-traces")]
        if trace {
            eprintln!(
                "HERD_COMPONENTS_RUST root={} components={components:?}",
                self.graph.nodes[scope.0 as usize].tala_id
            );
        }

        let seed_sides = [
            Orientation::Top,
            Orientation::Right,
            Orientation::Bottom,
            Orientation::Left,
        ];
        let mut unbiased_side = 0usize;
        for (component_groups, component_nodes) in &components {
            let mut sides = seed_sides.to_vec();
            let mut preferred = None;
            for group_index in component_groups {
                let (uncle, nodes) = &groups[*group_index];
                herd_poll()?;
                let children = self
                    .graph
                    .owner_container_map(receiver)
                    .expect("actual herd graph")
                    .get(Some(*uncle));
                let Some(children) = children.filter(|children| children.len() > 0) else {
                    return Err(herd_invariant(format!(
                        "herding uncle {} has no children",
                        self.graph.topology_node(*uncle).expect("uncle").external_id
                    )));
                };
                if self
                    .graph
                    .position(children.get(0).unwrap().expect("first child"))
                    .is_none()
                {
                    continue;
                }
                let uncle_size = self.graph.nodes[uncle.0 as usize].rect.size;
                for cousins in nodes.iter().map(|(_, cousins)| cousins) {
                    for cousin in cousins {
                        let Some(assignment) = self.graph.herd_assignment_snapshot(*cousin) else {
                            continue;
                        };
                        if !matches!(
                            assignment.orientation,
                            Orientation::Top
                                | Orientation::Right
                                | Orientation::Bottom
                                | Orientation::Left
                        ) {
                            return Err(herd_invariant(format!(
                                "cousin {} has an invalid herd orientation",
                                self.graph
                                    .topology_node(*cousin)
                                    .expect("cousin")
                                    .external_id
                            )));
                        }
                        let can_use_both_sides = match assignment.orientation {
                            Orientation::Top | Orientation::Bottom => {
                                uncle_size.width >= uncle_size.height * 2.0
                            }
                            Orientation::Left | Orientation::Right => {
                                uncle_size.height >= uncle_size.width * 2.0
                            }
                            _ => false,
                        };
                        sides.retain(|side| {
                            *side == assignment.orientation.opposite()
                                || (can_use_both_sides && *side == assignment.orientation)
                        });
                        if preferred.is_none() {
                            let side = if can_use_both_sides
                                && assignment.same_side_paired.len()
                                    < assignment.opposite_side_paired.len()
                            {
                                assignment.orientation
                            } else {
                                assignment.orientation.opposite()
                            };
                            preferred = Some(side);
                        }
                    }
                }
            }
            if sides.is_empty() {
                for node in component_nodes {
                    self.graph
                        .set_owner_node_herd(*node, None)
                        .expect("clear infeasible herd");
                }
                continue;
            }
            let selected = match preferred {
                None => {
                    let side = sides[unbiased_side % sides.len()];
                    unbiased_side += 1;
                    side
                }
                Some(side) if sides.contains(&side) => side,
                Some(_) => sides[0],
            };
            for node in component_nodes {
                self.graph.create_node_herd(*node, selected);
            }
            // Pair counts influence later components. Keep the same per-uncle
            // bookkeeping as Go after the component side has been chosen.
            for group_index in component_groups {
                let (uncle, nodes) = &groups[*group_index];
                let children = self
                    .graph
                    .owner_container_map(receiver)
                    .expect("actual herd graph")
                    .get(Some(*uncle))
                    .expect("checked uncle children");
                if self
                    .graph
                    .position(children.get(0).unwrap().expect("first child"))
                    .is_none()
                {
                    continue;
                }
                for (node, cousins) in nodes {
                    let node = *node;
                    let Some(node_orientation) = self
                        .graph
                        .herd_assignment_snapshot(node)
                        .map(|assignment| assignment.orientation)
                    else {
                        continue;
                    };
                    for cousin in cousins {
                        let Some(cousin_orientation) = self
                            .graph
                            .herd_assignment_snapshot(*cousin)
                            .map(|assignment| assignment.orientation)
                        else {
                            continue;
                        };
                        if node_orientation == cousin_orientation {
                            self.graph.record_node_herd_pair(node, *uncle, false);
                            self.graph.record_node_herd_pair(*cousin, *uncle, false);
                        } else {
                            self.graph.record_node_herd_pair(node, *uncle, true);
                            self.graph.record_node_herd_pair(*cousin, *uncle, true);
                        }
                    }
                }
            }
        }

        let propagate = |graph: &mut ArenaGraph| -> Result<(), crate::LayoutError> {
            loop {
                herd_poll()?;
                let mut changed = false;
                for nodes in &herd_groups {
                    herd_poll()?;
                    let Some(source) = nodes.iter().find_map(|node| {
                        graph
                            .herd_assignment_snapshot(*node)
                            .filter(|assignment| assignment.orientation != Orientation::None)
                            .map(|_| *node)
                    }) else {
                        continue;
                    };
                    for node in nodes {
                        if graph.herd_assignment_snapshot(*node).is_none() {
                            graph.copy_node_herd(source, *node);
                            changed = true;
                        } else if let Some(assignment) = graph.herd_assignment_snapshot(*node) {
                            let expected = graph
                                .herd_assignment_snapshot(source)
                                .expect("source herd")
                                .orientation;
                            if assignment.orientation != Orientation::None
                                && assignment.orientation != expected
                            {
                                return Err(herd_invariant(format!(
                                    "node {} has herd orientation {:?}; expected {:?}",
                                    graph.topology_node(*node).expect("herd node").external_id,
                                    assignment.orientation,
                                    expected
                                )));
                            }
                        }
                    }
                }
                if !changed {
                    return Ok(());
                }
            }
        };
        propagate(&mut self.graph)?;

        // Go keeps cluster vessels in the herd groups and lets their chosen
        // fence determine the vessel axis before the next placement scope is
        // built.  The stable arena represents a vessel by its first member,
        // so update that owning ClusterState when the propagated assignment
        // reaches an aggregate carrier.  Omitting this step leaves wide
        // horizontal clusters in a column (or vice versa) and changes the
        // very first initialized placement snapshot.
        for (_, nodes) in &groups {
            for node in nodes.iter().map(|(node, _)| *node) {
                let Some(assignment) = self
                    .graph
                    .herd_assignment_snapshot(node)
                    .map(|assignment| assignment.orientation)
                else {
                    continue;
                };
                // Source herding.go:128-136 looks up Graph.Clusters[vessel],
                // not vessel.Cluster (which belongs only to original members).
                if self.graph.topology_node_vessel(node).is_some()
                    && !self.graph.owner_is_cluster_vessel(node)
                {
                    continue;
                }
                let group = if self.graph.owner_is_cluster_vessel(node) {
                    self.graph
                        .owner_graph_group_entry(
                            receiver,
                            owner_groups::OwnerGroupKind::Cluster,
                            Some(node),
                        )
                        .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?
                        .flatten()
                        .or_else(|| {
                            panic!(
                                "runtime error: invalid memory address or nil pointer dereference"
                            )
                        })
                } else {
                    None
                };
                let cluster_index = match self.graph.topology_node_vessel(node) {
                    Some(TopologyVessel::Cluster(index)) => Some(index),
                    _ => self.graph.nodes[node.0 as usize].cluster.filter(|index| {
                        self.graph
                            .topology_vessel_node(TopologyVessel::Cluster(*index))
                            .is_none()
                    }),
                };
                let arrangement = if let Some(group) = group {
                    self.graph
                        .owner_cluster_policy(group)
                        .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?
                        .arrangement
                } else if let Some(index) = cluster_index {
                    match self.graph.clusters[index].arrangement {
                        ClusterArrangement::Row => "Row".to_owned(),
                        ClusterArrangement::Column => "Column".to_owned(),
                    }
                } else {
                    continue;
                };
                let next = match (assignment, arrangement.as_str()) {
                    (Orientation::Top | Orientation::Bottom, "Column") => {
                        Some(ClusterArrangement::Row)
                    }
                    (Orientation::Left | Orientation::Right, "Row") => {
                        Some(ClusterArrangement::Column)
                    }
                    // The canonical field can already have the selected
                    // arrangement; keep the compact projection synchronized.
                    (_, "Row") => Some(ClusterArrangement::Row),
                    (_, "Column") => Some(ClusterArrangement::Column),
                    _ => None,
                };
                if let Some(next) = next {
                    if let Some(index) = cluster_index {
                        self.graph.clusters[index].arrangement = next;
                    }
                    if let Some(group) = group {
                        let mut policy = self
                            .graph
                            .owner_cluster_policy(group)
                            .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
                        policy.arrangement = match next {
                            ClusterArrangement::Row => "Row".to_owned(),
                            ClusterArrangement::Column => "Column".to_owned(),
                        };
                        self.graph
                            .set_owner_cluster_policy(group, policy)
                            .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
                    }
                }
            }
        }

        #[cfg(feature = "diagnostic-traces")]
        if trace {
            for nodes in &herd_groups {
                for node in nodes {
                    eprintln!(
                        "HERD_SCOPE_RUST node={} assignment={:?}",
                        self.graph.nodes[node.0 as usize].tala_id,
                        self.graph.herd_assignment_snapshot(*node),
                    );
                }
            }
        }
        Ok(())
    }
}

fn herd_poll() -> Result<(), crate::LayoutError> {
    match crate::internal::cancellation_error() {
        Some(error) => Err(crate::LayoutError::PlacementTransaction(format!(
            "AssignHerds: {error}"
        ))),
        None => Ok(()),
    }
}
fn herd_invariant(message: String) -> crate::LayoutError {
    crate::LayoutError::PlacementTransaction(format!("layout invariant violated: {message}"))
}
fn herd_group_vessel(
    graph: &ArenaGraph,
    node: Option<NodeId>,
) -> Result<Option<NodeId>, crate::LayoutError> {
    let Some(node) = node else { return Ok(None) };
    for kind in [
        owner_groups::OwnerGroupKind::Cluster,
        owner_groups::OwnerGroupKind::Sequence,
    ] {
        if let Some(vessel) = graph
            .owner_node_group_vessel(node, kind)
            .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?
        {
            return Ok(vessel);
        }
    }
    Ok(Some(node))
}
fn herd_owning_container(
    graph: &ArenaGraph,
    node: NodeId,
) -> Result<Option<NodeId>, crate::LayoutError> {
    for kind in [
        owner_groups::OwnerGroupKind::Cluster,
        owner_groups::OwnerGroupKind::Sequence,
    ] {
        if let Some(vessel) = graph
            .owner_node_group_vessel(node, kind)
            .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?
        {
            let vessel =
                vessel.expect("runtime error: invalid memory address or nil pointer dereference");
            if graph.topology_node_owner(vessel).is_some() {
                return Ok(graph.topology_node(vessel).expect("group vessel").container);
            }
        }
    }
    Ok(graph.topology_node(node).expect("herd node").container)
}
fn herd_descendant(
    graph: &ArenaGraph,
    mut node: Option<NodeId>,
    ancestor: NodeId,
) -> Result<bool, crate::LayoutError> {
    while let Some(current) = node {
        if current == ancestor {
            return Ok(true);
        }
        let live = graph.topology_node(current).expect("descendant");
        node = if live.container.is_some() {
            live.container
        } else {
            let vessel = herd_group_vessel(graph, Some(current))?;
            if vessel == Some(current) {
                None
            } else {
                vessel
            }
        };
    }
    Ok(false)
}

impl ArenaGraph {
    fn herd_sync_member_order(&self) -> Vec<NodeId> {
        // An initialized root owns an actual Graph.Nodes header. Otherwise
        // this numerical graph's active order is its current member surface.
        if let Some(view) = self.owner_graph_view(OwnerGraphId(0)) {
            view.nodes.require_nodes().collect()
        } else {
            self.graph_node_order()
        }
    }

    /// Recovered `Graph.syncHerdFences`: each assigned node is attracted to
    /// the current fixed bounding-box face of its optimization scope.
    pub(in crate::internal) fn sync_herd_fences(&mut self) {
        let all_nodes = self.herd_sync_member_order();
        let Some((top_left, bottom_right)) = self.fixed_node_bounds(&all_nodes) else {
            return;
        };
        let (top_left, bottom_right) = self.include_graph_edge_bounds(top_left, bottom_right);
        self.sync_herd_fences_at_bounds(top_left, bottom_right);
    }

    pub(in crate::internal) fn sync_herd_fences_at_bounds(
        &mut self,
        top_left: Point,
        bottom_right: Point,
    ) {
        // Graph.bounds applies a final math.Round after fixed-origin selection.
        let top_left = Point {
            x: top_left.x.round(),
            y: top_left.y.round(),
        };
        let bottom_right = Point {
            x: bottom_right.x.round(),
            y: bottom_right.y.round(),
        };
        for node_id in self.herd_sync_member_order() {
            if self.nodes[node_id.0 as usize].fixed_top_left.is_some() {
                continue;
            }
            // Source reads a raw orientation and changes only Val. Avoid a
            // numerical projection of actual records: unknown orientations
            // are valid no-ops and pairing-map allocations must stay intact.
            let binding = match self.owner_node_herd(node_id) {
                Ok(value) => Some(value),
                Err(OwnerTopologyError::UnboundHerdAssignment(_)) => None,
                Err(error) => panic!("herd member binding: {error}"),
            };
            if let Some(binding) = binding {
                let Some(herd) = binding else {
                    continue;
                };
                let mut fields = self.owner_herd_fields(&herd).expect("bound herd fields");
                fields.value = match fields.orientation {
                    4 => top_left.y,
                    5 => bottom_right.x,
                    6 => bottom_right.y,
                    7 => top_left.x,
                    _ => continue,
                };
                self.set_owner_herd_fields(&herd, fields)
                    .expect("bound herd scalar update");
                continue;
            }
            let Some(assignment) = self.nodes[node_id.0 as usize].herd_assignment.as_mut() else {
                continue;
            };
            assignment.value = match assignment.orientation {
                Orientation::Top => top_left.y,
                Orientation::Right => bottom_right.x,
                Orientation::Bottom => bottom_right.y,
                Orientation::Left => top_left.x,
                _ => continue,
            };
        }
    }

    pub(in crate::internal) fn herd_fence_distance(&self, node: NodeId) -> Option<f64> {
        // assignHerds can assign a fence to a cluster vessel too. Score its
        // active box, rather than suppressing the assignment or measuring
        // the stable member that carries the vessel in the arena.
        // The numerical graph keeps a first-member carrier, but Source
        // NodeEdgeLength reads the visible vessel's own HerdAssignment.
        // Prefer the actual bound vessel only when it is materialized in
        // this arena; induced workspaces retain their local field projection.
        let actual_vessel = self
            .active_cluster_index(node)
            .filter(|&index| self.clusters[index].members.first() == Some(&node))
            .and_then(|index| {
                let vessel = self.topology_vessel_node(TopologyVessel::Cluster(index))?;
                self.topology_node(vessel).map(|_| vessel)
            });
        let assignment_node = actual_vessel.unwrap_or(node);
        let Some(assignment) = self.herd_assignment_snapshot(assignment_node) else {
            return None;
        };
        let position = actual_vessel
            .and_then(|vessel| self.position(vessel))
            .or_else(|| self.active_node_position(node))
            .unwrap_or_default();
        let size = actual_vessel
            .map(|vessel| self.nodes[vessel.0 as usize].rect.size)
            .unwrap_or_else(|| self.active_node_size(node));
        let distance = match assignment.orientation {
            Orientation::Top => position.y - assignment.value,
            Orientation::Right => assignment.value - (position.x + size.width),
            Orientation::Bottom => assignment.value - (position.y + size.height),
            Orientation::Left => position.x - assignment.value,
            _ => 0.0,
        };
        Some(distance)
    }

    pub(in crate::internal) fn herd_penalty(&self, node: NodeId) -> f64 {
        match self.herd_fence_distance(node) {
            Some(distance) if distance >= 0.0 => distance,
            Some(distance) => self.cell_size - distance,
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod production_sheep_tests {
    use super::*;

    #[test]
    fn carrier_herd_penalty_reads_distinct_vessel_field_and_explicit_nil() {
        let mut input = crate::Graph::default();
        let first = input.add_node(crate::Node::new(
            "first",
            Size {
                width: 30.0,
                height: 10.0,
            },
        ));
        let second = input.add_node(crate::Node::new(
            "second",
            Size {
                width: 30.0,
                height: 10.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 10001,
            fixed_size: false,
        });
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.node_order.retain(|node| *node != second);
        let mut entity = graph.nodes[first.0 as usize].clone();
        entity.tala_id = 10001;
        entity.position = Some(Point {
            x: 120.25,
            y: 230.75,
        });
        entity.rect.size = Size {
            width: 50.0,
            height: 80.0,
        };
        let vessel = graph.retain_topology_node(entity);
        graph
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        graph.set_position(
            first,
            Point {
                x: -400.0,
                y: -500.0,
            },
        );
        graph.create_node_herd(first, Orientation::Bottom);
        graph.create_node_herd(vessel, Orientation::Top);
        graph.refresh_node_order_membership();
        assert_eq!(graph.herd_penalty(first), 230.75);
        // A genuinely nil vessel field must not fall back to the hidden
        // member's independently retained assignment.
        graph.set_owner_node_herd(vessel, None).unwrap();
        assert!(graph.has_herd_assignment(first));
        assert_eq!(graph.herd_penalty(first), 0.0);
        // Direct Source field replacement may alias another node. Follow
        // the actual pointer rather than the initial vessel's EntityID.
        graph.set_position(
            second,
            Point {
                x: 600.5,
                y: 700.25,
            },
        );
        graph.create_node_herd(second, Orientation::Left);
        graph
            .set_topology_vessel_reference(TopologyVessel::Cluster(0), Some(second))
            .unwrap();
        assert_eq!(graph.herd_penalty(first), 600.5);
    }

    #[test]
    fn herding_changes_the_distinct_vessel_group_without_a_member_cluster_field() {
        let mut input = crate::Graph::default();
        let local = input.add_node(crate::Node::new(
            "local",
            Size {
                width: 200.0,
                height: 120.0,
            },
        ));
        let uncle = input.add_node(crate::Node::new(
            "uncle",
            Size {
                width: 200.0,
                height: 120.0,
            },
        ));
        let mut child = |name: &str, parent| {
            let mut node = crate::Node::new(
                name,
                Size {
                    width: 40.0,
                    height: 20.0,
                },
            );
            node.parent = Some(parent);
            input.add_node(node)
        };
        let a = child("a", local);
        let b = child("b", local);
        let vessel = child("vessel", local);
        let peer = child("peer", local);
        let cousin = child("cousin", uncle);
        let edge = input.add_edge(crate::Edge {
            source: vessel,
            target: cousin,
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, false, false);
        pipeline.graph.initialize_shared_topology();
        pipeline.graph.clusters.push(ClusterState {
            members: vec![a, b],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 10.0,
            vessel_tala_id: pipeline.graph.nodes[vessel.0 as usize].tala_id,
            fixed_size: true,
        });
        for member in [a, b] {
            pipeline.graph.nodes[member.0 as usize].cluster = Some(0);
        }
        let group = pipeline
            .graph
            .new_owner_group(owner_groups::OwnerGroupKind::Cluster);
        let mut fields = pipeline.graph.owner_group_fields(group).unwrap();
        fields.vessel = Some(vessel);
        fields.container = Some(local);
        fields.graph = Some(OwnerGraphId(0));
        pipeline
            .graph
            .set_owner_group_fields(group, fields)
            .unwrap();
        pipeline
            .graph
            .bind_owner_group_reference(TopologyVessel::Cluster(0), group)
            .unwrap();
        pipeline
            .graph
            .set_owner_cluster_vessel_flag(vessel, true)
            .unwrap();
        let map = pipeline
            .graph
            .allocate_owner_group_map(owner_groups::OwnerGroupKind::Cluster, true);
        pipeline
            .graph
            .write_owner_group_map_entry(&map, Some(vessel), Some(group))
            .unwrap();
        pipeline
            .graph
            .set_owner_graph_group_map(OwnerGraphId(0), map)
            .unwrap();
        let mut policy = pipeline.graph.owner_cluster_policy(group).unwrap();
        policy.arrangement = "Column".to_owned();
        policy.desired_arrangement = "Column".to_owned();
        pipeline
            .graph
            .set_owner_cluster_policy(group, policy)
            .unwrap();
        let header = pipeline
            .graph
            .allocate_owner_container_header(&[vessel, peer], 2)
            .unwrap();
        pipeline
            .graph
            .set_owner_container_header(Some(local), header)
            .unwrap();
        let records = pipeline
            .graph
            .register_owner_edge_abductions(
                vec![vessel, peer]
                    .into_iter()
                    .map(|node| OwnerEdgeAbduction {
                        source_edge: Some(edge),
                        originally_from: Some(node),
                        originally_to: Some(cousin),
                        current_from: Some(local),
                        current_to: Some(uncle),
                    })
                    .collect(),
                true,
            )
            .unwrap();
        assert_eq!(pipeline.graph.nodes[vessel.0 as usize].cluster, None);
        pipeline
            .graph
            .set_owner_cluster_vessel_flag(vessel, false)
            .unwrap();
        pipeline
            .assign_herds_for_scope(local, OwnerGraphId(0), &records)
            .unwrap();
        assert_eq!(
            pipeline.graph.clusters[0].arrangement,
            ClusterArrangement::Column
        );
        assert_eq!(
            pipeline
                .graph
                .owner_cluster_policy(group)
                .unwrap()
                .arrangement,
            "Column"
        );
        pipeline
            .graph
            .set_owner_cluster_vessel_flag(vessel, true)
            .unwrap();
        pipeline
            .assign_herds_for_scope(local, OwnerGraphId(0), &records)
            .unwrap();
        let orientation = pipeline
            .graph
            .herd_assignment_snapshot(vessel)
            .unwrap()
            .orientation;
        assert!(matches!(
            orientation,
            Orientation::Top | Orientation::Bottom
        ));
        assert_eq!(
            pipeline.graph.clusters[0].arrangement,
            ClusterArrangement::Row
        );
        let policy = pipeline.graph.owner_cluster_policy(group).unwrap();
        assert_eq!(policy.arrangement, "Row");
        assert_eq!(policy.desired_arrangement, "Column");
        assert_eq!(pipeline.graph.nodes[vessel.0 as usize].cluster, None);
        assert!(pipeline.graph.herd_assignment_snapshot(a).is_none());
        let placement = pipeline.placement_scope_graph(Some(local)).unwrap();
        let projected = placement.old_to_new[&a];
        let metadata = placement
            .scoring_node_metadata
            .iter()
            .find(|metadata| metadata.node == projected)
            .unwrap();
        let actual = pipeline.graph.herd_assignment_snapshot(vessel).unwrap();
        let copied = metadata
            .herd_assignment
            .as_ref()
            .expect("vessel herd survives projection");
        assert_eq!(copied.orientation, actual.orientation);
    }

    #[test]
    fn production_group_sheep_source_controls() {
        for case in [
            "ordered-duplicates",
            "current-leaf",
            "invalid-orientation",
            "empty-uncle",
        ] {
            let mut input = crate::Graph::default();
            let mut add = |name: &str, parent| {
                let mut node = crate::Node::new(
                    name,
                    Size {
                        width: 200.0,
                        height: 120.0,
                    },
                );
                node.parent = parent;
                input.add_node(node)
            };
            let local = add("local", None);
            let uncle = add("uncle", None);
            let first = add("first", Some(local));
            let second = add("second", Some(local));
            let cousin = add("cousin", Some(uncle));
            let edge = input.add_edge(crate::Edge {
                source: first,
                target: cousin,
            });
            let mut pipeline = Pipeline::new_untraced(&input, 1, false, false);
            pipeline.graph.initialize_shared_topology();
            // Header order deliberately differs from immutable input/arena order.
            let children = pipeline
                .graph
                .allocate_owner_container_header(&[second, first], 2)
                .unwrap();
            pipeline
                .graph
                .set_owner_container_header(Some(local), children)
                .unwrap();
            let records = pipeline
                .graph
                .register_owner_edge_abductions(
                    vec![
                        OwnerEdgeAbduction {
                            source_edge: Some(edge),
                            originally_from: Some(first),
                            originally_to: Some(cousin),
                            current_from: Some(local),
                            current_to: Some(uncle),
                        },
                        OwnerEdgeAbduction {
                            source_edge: Some(edge),
                            originally_from: Some(second),
                            originally_to: Some(cousin),
                            current_from: Some(local),
                            current_to: Some(uncle),
                        },
                    ],
                    true,
                )
                .unwrap();
            let mut records = records;
            records.records.push(records.records[0]); // Same actual record, separate slice index.
            if case == "current-leaf" {
                records = pipeline
                    .graph
                    .register_owner_edge_abductions(
                        vec![
                            OwnerEdgeAbduction {
                                source_edge: Some(edge),
                                originally_from: Some(first),
                                originally_to: Some(cousin),
                                current_from: Some(local),
                                current_to: Some(cousin),
                            },
                            OwnerEdgeAbduction {
                                source_edge: Some(edge),
                                originally_from: Some(second),
                                originally_to: Some(cousin),
                                current_from: Some(local),
                                current_to: Some(uncle),
                            },
                        ],
                        true,
                    )
                    .unwrap();
            }
            let groups = pipeline
                .group_owner_sheep(local, OwnerGraphId(0), &records)
                .unwrap();
            let names = groups.iter().map(|(uncle, nodes)| serde_json::json!({
                "uncle": pipeline.graph.topology_node(*uncle).unwrap().external_id,
                "nodes": nodes.iter().map(|(node, cousins)| serde_json::json!({
                    "node": pipeline.graph.topology_node(*node).unwrap().external_id,
                    "cousins": cousins.iter().map(|node| pipeline.graph.topology_node(*node).unwrap().external_id.clone()).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>();
            if case == "invalid-orientation" {
                pipeline.graph.set_position(cousin, Point::default());
                pipeline.graph.create_node_herd(cousin, Orientation::None);
            } else if case == "empty-uncle" {
                let empty = pipeline
                    .graph
                    .allocate_owner_container_header(&[], 0)
                    .unwrap();
                pipeline
                    .graph
                    .set_owner_container_header(Some(uncle), empty)
                    .unwrap();
            }
            let error = pipeline
                .assign_herds_for_scope(local, OwnerGraphId(0), &records)
                .err()
                .map(|error| match error {
                    crate::LayoutError::PlacementTransaction(message) => message,
                    other => other.to_string(),
                });
            let value = serde_json::json!({"case": case, "groups": names, "error": error});
            match case {
                "current-leaf" => assert_eq!(
                    groups[0]
                        .1
                        .iter()
                        .map(|(node, _)| *node)
                        .collect::<Vec<_>>(),
                    vec![second]
                ),
                _ => {
                    assert_eq!(
                        groups[0]
                            .1
                            .iter()
                            .map(|(node, _)| *node)
                            .collect::<Vec<_>>(),
                        vec![second, first]
                    );
                    assert_eq!(groups[0].1[1].1, vec![cousin, cousin]);
                }
            }
            eprintln!("PRODUCTION_SHEEP_CONTROL {value}");
        }
    }
}

