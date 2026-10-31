// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned ovg_resource.go containerRDFSOrder and ovg.go final construction.
use super::ovg_intersections::{Budget, check, rect, step};
use super::ovg_ports::PortBuild;
use super::{ArenaGraph, NodeId};
use crate::internal::{OwnerGraphId, owner_groups::OwnerGroupKind};
use std::collections::{BTreeSet, VecDeque};

// Each frame retains the source range header, reading a slot before its step.
// An explicit stack also allows the guard to interrupt cyclic/deep hierarchies.
enum Frame {
    Children(Box<dyn Iterator<Item = Option<NodeId>>>, bool),
    Append(NodeId),
    Exit,
}
fn enter(graph: &ArenaGraph, root: Option<NodeId>, stack: &mut Vec<Frame>) {
    stack.push(Frame::Exit);
    if let Some(header) = graph.containers.get_header(&root) {
        stack.push(Frame::Children(
            Box::new(
                (0..header.len())
                    .rev()
                    .map(move |i| header.get(i).expect("container range bound")),
            ),
            false,
        ));
    }
}
pub(super) fn container_order(
    graph: &ArenaGraph,
    root: Option<NodeId>,
    budget: Budget<'_, '_>,
) -> Result<Vec<NodeId>, ()> {
    let mut order = Vec::new();
    if root.is_some_and(|node| !graph.nodes[node.0 as usize].is_container) {
        check(budget)?;
        return Ok(order);
    }
    let mut stack = Vec::new();
    enter(graph, root, &mut stack);
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Exit => check(budget)?,
            Frame::Append(node) => order.push(node),
            Frame::Children(mut children, in_cluster) => {
                let Some(child) = children.next() else {
                    continue;
                };
                step(budget)?;
                stack.push(Frame::Children(children, in_cluster));
                // Both Go IsContainer and IsClusterVessel accept nil receivers.
                let Some(child) = child else {
                    continue;
                };
                if graph.nodes[child.0 as usize].is_container {
                    stack.push(Frame::Append(child));
                    enter(graph, Some(child), &mut stack);
                } else if !in_cluster {
                    if graph.shared_topology_initialized() {
                        if !graph.owner_is_cluster_vessel(child) {
                            continue;
                        }
                        // Graph.Clusters[child], not the child's Cluster field.
                        let group = graph
                            .owner_graph_group_entry(
                                OwnerGraphId(0),
                                OwnerGroupKind::Cluster,
                                Some(child),
                            )
                            .expect("OVG cluster map")
                            .flatten()
                            .expect(
                                "runtime error: invalid memory address or nil pointer dereference",
                            );
                        let header = graph
                            .owner_group_fields(group)
                            .expect("OVG cluster fields")
                            .nodes;
                        stack.push(Frame::Children(
                            Box::new(
                                (0..header.len())
                                    .rev()
                                    .map(move |i| header.get(i).expect("cluster range bound")),
                            ),
                            true,
                        ));
                    } else if let Some(cluster) = graph.clusters.iter().find(|cluster| {
                        cluster.vessel_tala_id == graph.nodes[child.0 as usize].tala_id
                    }) {
                        // Numerical scopes preceding owner binding retain their
                        // own vessel map; explicit raw nil never falls back.
                        stack.push(Frame::Children(
                            Box::new(cluster.members.clone().into_iter().rev().map(Some)),
                            true,
                        ));
                    }
                }
            }
        }
    }
    Ok(order)
}

impl PortBuild {
    pub(super) fn map_containers(
        &mut self,
        graph: &ArenaGraph,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        let order = container_order(graph, None, budget)?;
        for &node in &self.members {
            step(budget)?;
            for &container in &order {
                step(budget)?;
                let bounds = rect(graph, container);
                let point = self.nodes[node].point();
                if bounds.origin.x <= point.x
                    && bounds.origin.x + bounds.size.width >= point.x
                    && bounds.origin.y <= point.y
                    && bounds.origin.y + bounds.size.height >= point.y
                {
                    self.nodes[node].write().container = Some(container);
                    break;
                }
            }
        }
        Ok(()) // Source has no final check or clearing of unmatched containers.
    }

    pub(super) fn flag_near_ports(&mut self, budget: Budget<'_, '_>) -> Result<(), ()> {
        check(budget)?;
        for (&owner, ports) in &self.ports {
            step(budget)?;
            for &(port, _) in ports {
                step(budget)?;
                let port = self.nodes[port].clone();
                let mut seen = BTreeSet::from([port.allocation_key()]);
                let mut queue = VecDeque::with_capacity(5);
                let edges = port.read().edges.clone();
                for edge in edges.iter_slots() {
                    step(budget)?;
                    let edge = edge
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    let adjacent = edge.adjacent(&port);
                    let point = adjacent.point();
                    let origin = port.point();
                    if point.x != origin.x && point.y != origin.y {
                        continue;
                    }
                    queue.push_back(adjacent);
                }
                while !queue.is_empty() {
                    step(budget)?;
                    let current = queue.pop_front().unwrap();
                    if !seen.insert(current.allocation_key()) {
                        continue;
                    }
                    if !current.read().owners.contains_key(&owner) {
                        if (current.point().y - port.point().y).abs() >= 30.0 {
                            continue;
                        }
                        if (current.point().x - port.point().x).abs() >= 30.0 {
                            continue;
                        }
                        current
                            .write()
                            .near_ports
                            .get_or_insert_with(Box::default)
                            .insert(owner);
                    }
                    let edges = current.read().edges.clone();
                    for edge in edges.iter_slots() {
                        step(budget)?;
                        let edge = edge.expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        );
                        let adjacent = edge.adjacent(&current);
                        let point = adjacent.point();
                        let origin = port.point();
                        if point.x != origin.x && point.y != origin.y {
                            continue;
                        }
                        queue.push_back(adjacent);
                    }
                }
            }
        }
        check(budget)
    }
    pub(super) fn assign_indices(&mut self, budget: Budget<'_, '_>) -> Result<(), ()> {
        for (index, &node) in self.members.iter().enumerate() {
            step(budget)?;
            self.nodes[node].write().index = index;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Point, Port, PortSide, route_search_guard::OvgBuildBudget};
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;
    struct Caller<'a> {
        polls: Cell<u64>,
        cancel: u64,
        mutate: Option<&'a dyn Fn(u64)>,
    }
    impl Context for Caller<'_> {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let n = self.polls.get() + 1;
            self.polls.set(n);
            if let Some(mutate) = self.mutate {
                mutate(n);
            }
            (self.cancel != 0 && n >= self.cancel).then_some(ContextError::Cancelled)
        }
    }
    fn graph() -> ArenaGraph {
        let input = serde_json::json!({"nodes":[
            {"id":1,"width":100,"height":100,"topLeft":{"x":0,"y":0}},
            {"id":2,"width":40,"height":40,"topLeft":{"x":10,"y":10}},
            {"id":3,"width":1,"height":1,"topLeft":{"x":20,"y":20}}
        ],"edges":[],"containers":{"0":[1],"1":[2],"2":[3]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&input).unwrap()).unwrap(),
        )
    }
    #[test]
    fn traversal_reads_each_retained_slot_before_its_callback_and_accepts_nil() {
        let graph = graph();
        let header = graph.containers.get_header(&None).unwrap();
        let mutate = |n| {
            if n == 1 {
                header.write_capacity_slot(0, None).unwrap();
            }
        };
        let context = Caller {
            polls: Cell::new(0),
            cancel: 0,
            mutate: Some(&mutate),
        };
        let budget = OvgBuildBudget::with_context(&context);
        assert_eq!(
            container_order(&graph, None, Some(&budget)).unwrap(),
            [NodeId(1), NodeId(0)]
        );
        assert_eq!(context.polls.get(), 6);
        assert!(container_order(&graph, None, None).unwrap().is_empty());
    }
    #[test]
    fn container_mapping_reads_live_geometry_and_retains_unmatched_state() {
        let mut graph = graph();
        graph.materialize_owner_node_point(NodeId(1)).unwrap();
        let point = graph
            .owner_node_point(NodeId(1))
            .unwrap()
            .shared_point()
            .unwrap();
        let mutate = |n| {
            if n == 9 {
                point.set(Point { x: 500.0, y: 500.0 });
            }
        };
        let context = Caller {
            polls: Cell::new(0),
            cancel: 0,
            mutate: Some(&mutate),
        };
        let budget = OvgBuildBudget::with_context(&context);
        let mut build = PortBuild::default();
        let mut node = build.add_point(Point { x: 20.0, y: 20.0 }, None).unwrap();
        let outside = build.add_point(Point { x: -1.0, y: -1.0 }, None).unwrap();
        build.nodes[outside].write().container = Some(NodeId(1));
        build.map_containers(&graph, Some(&budget)).unwrap();
        assert_eq!(build.nodes[node].read().container, Some(NodeId(0)));
        assert_eq!(build.nodes[outside].read().container, Some(NodeId(1)));
        assert_eq!(context.polls.get(), 13); // No extra final poll.
    }
    #[test]
    fn cluster_traversal_uses_map_entry_even_when_child_group_pointer_is_nil() {
        let mut graph = graph();
        let vessel = NodeId(0);
        graph.nodes[0].is_container = false;
        graph.set_owner_cluster_vessel_flag(vessel, true).unwrap();
        let group = graph.new_owner_group(OwnerGroupKind::Cluster);
        let mut fields = graph.owner_group_fields(group).unwrap();
        fields.nodes = graph
            .allocate_owner_member_header(&[Some(NodeId(1)), None], 2)
            .unwrap();
        graph.set_owner_group_fields(group, fields).unwrap();
        let map = graph
            .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
            .unwrap();
        graph
            .write_owner_group_map_entry(&map, Some(vessel), Some(group))
            .unwrap();
        graph
            .set_owner_node_group(vessel, OwnerGroupKind::Cluster, None)
            .unwrap();
        assert_eq!(container_order(&graph, None, None).unwrap(), [NodeId(1)]);
        graph
            .write_owner_group_map_entry(&map, Some(vessel), None)
            .unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| container_order(
                &graph, None, None
            )))
            .is_err()
        );
    }
    fn near_graph() -> (PortBuild, usize, usize) {
        let mut build = PortBuild::default();
        let point = Point { x: 0.0, y: 0.0 };
        build
            .import_ports(
                &[(
                    NodeId(0),
                    Port {
                        point,
                        side: PortSide::Top,
                        direction: PortSide::Top,
                        index: 0,
                        is_center: false,
                        tunnel: None,
                    },
                )],
                None,
            )
            .unwrap();
        let close = build.add_point(Point { x: 29.0, y: 0.0 }, None).unwrap();
        let boundary = build.add_point(Point { x: 30.0, y: 0.0 }, None).unwrap();
        build.connect(0, close, None).unwrap();
        build.connect(close, boundary, None).unwrap();
        (build, close, boundary)
    }
    #[test]
    fn near_flags_publish_before_followed_edge_poll_and_use_strict_cutoff() {
        let (mut build, close, boundary) = near_graph();
        let context = Caller {
            polls: Cell::new(0),
            cancel: 6,
            mutate: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        assert!(build.flag_near_ports(Some(&budget)).is_err());
        assert_eq!(
            build.nodes[close].read().near_ports.as_deref().cloned(),
            Some(BTreeSet::from([NodeId(0)]))
        );
        assert!(build.nodes[boundary].read().near_ports.is_none());
        build.flag_near_ports(None).unwrap();
        assert!(build.nodes[boundary].read().near_ports.is_none());
    }
    #[test]
    fn near_flags_follow_ports_retained_outside_current_membership() {
        let (mut build, close, _) = near_graph();
        build.members.retain(|&node| node != 0);
        build.flag_near_ports(None).unwrap();
        assert_eq!(
            build.nodes[close].read().near_ports.as_deref().cloned(),
            Some(BTreeSet::from([NodeId(0)]))
        );
    }
    #[test]
    fn indices_update_shared_identities_incrementally_without_exit_check() {
        let mut build = PortBuild::default();
        let mut node = build
            .add_point(
                Point {
                    x: f64::NAN,
                    y: 0.0,
                },
                None,
            )
            .unwrap();
        build.add_existing_node(node, None).unwrap();
        build.add_existing_node(node, None).unwrap();
        let context = Caller {
            polls: Cell::new(0),
            cancel: 3,
            mutate: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        assert!(build.assign_indices(Some(&budget)).is_err());
        assert_eq!(build.nodes[node].read().index, 1);
        let context = Caller {
            polls: Cell::new(0),
            cancel: 4,
            mutate: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        build.assign_indices(Some(&budget)).unwrap();
        assert_eq!(build.nodes[node].read().index, 2);
        assert_eq!(context.polls.get(), 3);
    }
}
