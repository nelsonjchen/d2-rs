// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned routing/ovg.go addTreeNodes and tree_routes.go.
use super::ovg_intersections::{Budget, check, rect, step};
use super::ovg_ports::PortBuild;
use super::{ArenaGraph, NodeId, Point, Port, PortSide};
use crate::internal::OwnerGraphId;
use crate::internal::nodeshape::ports::{shape_center_port_index, shape_port_indices};
use crate::internal::owner_tree_map::{OwnerTreeFields, OwnerTreeId, OwnerTreeSlice};
use std::collections::{BTreeMap, BTreeSet};

const NIL: &str = "runtime error: invalid memory address or nil pointer dereference";

#[derive(Clone, Copy)]
enum Tree {
    Owned(Option<OwnerTreeId>),
    Numerical(NodeId),
}

#[cfg(test)]
mod tests {
    use super::super::route_search_guard::OvgBuildBudget;
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;

    fn graph(width: f64, reverse: bool) -> ArenaGraph {
        let parent = serde_json::json!({"id":1,"width":52,"height":40,"topLeft":{"x":0,"y":0}});
        let child = serde_json::json!({"id":2,"width":width,"height":40,"topLeft":{"x":0,"y":200}});
        let edge = serde_json::json!({"id":11,"from":if reverse {2} else {1},"to":if reverse {1} else {2},"points":[]});
        let value = serde_json::json!({"nodes":[parent,child],"edges":[edge],"containers":{"0":[1,2]},
            "trees":{"1":[{"Node":child,"SentinelEdge":edge,"Children":[],"Orientation":6}]}});
        let input =
            crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap();
        ArenaGraph::from_input(&input)
    }
    struct Caller<'a> {
        polls: Cell<u64>,
        mutate_at: u64,
        tree: Option<&'a crate::internal::owner_tree_map::OwnerTreeRecord>,
        orientation: Option<i64>,
    }
    impl Context for Caller<'_> {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            if poll == self.mutate_at
                && let Some(tree) = self.tree
            {
                let mut fields = tree.fields();
                if let Some(raw) = self.orientation {
                    fields.orientation = raw;
                } else {
                    fields.node = Some(NodeId(0));
                }
                tree.set_fields(fields);
            }
            None
        }
    }
    #[test]
    fn aligned_tree_port_is_canonical_and_temporary_allocation_is_separate() {
        for reverse in [false, true] {
            let graph = graph(53.0, reverse);
            let context = Caller {
                polls: Cell::new(0),
                mutate_at: 0,
                tree: None,
                orientation: None,
            };
            let budget = OvgBuildBudget::with_context(&context);
            let mut ovg = PortBuild::default();
            ovg.add_ports(&graph, &graph.graph_node_order(), Some(&budget))
                .unwrap();
            let before = budget.observed_resources();
            let before_nodes = ovg.nodes.len();
            let added = ovg
                .add_tree_nodes(&graph, &graph.graph_node_order(), Some(&budget))
                .unwrap();
            assert_eq!(added[&NodeId(1)].len(), 1);
            assert_eq!(added[&NodeId(1)][0].point, Point { x: 26.0, y: 200.0 });
            assert_eq!(added[&NodeId(1)][0].direction, PortSide::Top);
            assert!(!added[&NodeId(1)][0].is_center);
            assert_eq!(
                budget.observed_resources()[1] - before[1],
                (ovg.nodes.len() - before_nodes) as u64 + 1
            );
            let before = budget.observed_resources();
            assert!(
                ovg.add_tree_nodes(&graph, &graph.graph_node_order(), Some(&budget))
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                budget.observed_resources()[1] - before[1],
                1,
                "the discarded aligned node is still allocated on a repeated lookup"
            );
        }
    }
    #[test]
    fn actual_tree_map_absence_and_present_nil_override_numerical_projection() {
        let mut graph = graph(53.0, false);
        graph.tree_routing_nodes.insert(
            NodeId(1),
            crate::internal::TreeRoutingNode {
                parent: NodeId(0),
                sentinel_edge: crate::EdgeId(0),
                orientation: super::super::Orientation::Bottom,
                extraction_order: 0,
            },
        );
        let map = graph.allocate_owner_node_to_tree_map(true);
        graph
            .set_owner_graph_node_to_tree_map(OwnerGraphId(0), map.clone())
            .unwrap();
        let mut ovg = PortBuild::default();
        ovg.add_ports(&graph, &graph.graph_node_order(), None)
            .unwrap();
        assert!(
            ovg.add_tree_nodes(&graph, &graph.graph_node_order(), None)
                .unwrap()
                .is_empty()
        );
        graph
            .write_owner_node_to_tree_entry(&map, Some(NodeId(1)), None)
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ovg.add_tree_nodes(&graph, &graph.graph_node_order(), None)
        }));
        assert!(result.is_err());
    }
    #[test]
    fn publication_uses_graph_loop_owner_when_tree_node_changes_after_path_selection() {
        let graph = graph(53.0, false);
        let tree = graph
            .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(NodeId(1)))
            .unwrap()
            .unwrap()
            .unwrap();
        let context = Caller {
            polls: Cell::new(0),
            mutate_at: 0,
            tree: None,
            orientation: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        let mut ovg = PortBuild::default();
        ovg.add_ports(&graph, &graph.graph_node_order(), None)
            .unwrap();
        ovg.add_tree_nodes(&graph, &graph.graph_node_order(), Some(&budget))
            .unwrap();
        // Change tree.Node at the final addNode check: source has already
        // selected its branch and point, but publishes on the loop's owner.
        let mutate_at = context.polls.get() - 1;
        let context = Caller {
            polls: Cell::new(0),
            mutate_at,
            tree: graph.owner_tree(tree),
            orientation: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        let mut ovg = PortBuild::default();
        ovg.add_ports(&graph, &graph.graph_node_order(), None)
            .unwrap();
        let added = ovg
            .add_tree_nodes(&graph, &graph.graph_node_order(), Some(&budget))
            .unwrap();
        assert_eq!(
            graph.owner_tree(tree).unwrap().fields().node,
            Some(NodeId(0))
        );
        assert_eq!(added[&NodeId(1)][0].point, Point { x: 26.0, y: 200.0 });
    }
    #[test]
    fn late_orientation_change_keeps_all_source_direction_bits() {
        for (raw, expected) in [(0, 1 << 3), (1, 1 << 2), (8, 1 << 8), (-1, 0), (10, 0)] {
            let graph = graph(53.0, false);
            let tree = graph
                .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(NodeId(1)))
                .unwrap()
                .unwrap()
                .unwrap();
            let context = Caller {
                polls: Cell::new(0),
                mutate_at: 7,
                tree: graph.owner_tree(tree),
                orientation: Some(raw),
            };
            let budget = OvgBuildBudget::with_context(&context);
            let mut ovg = PortBuild::default();
            ovg.add_ports(&graph, &graph.graph_node_order(), None)
                .unwrap();
            let added = ovg
                .add_tree_nodes(&graph, &graph.graph_node_order(), Some(&budget))
                .unwrap();
            assert_eq!(added[&NodeId(1)][0].point, Point { x: 26.0, y: 200.0 });
            let index = ovg.ports[&NodeId(1)].last().unwrap().0;
            assert_eq!(ovg.port_directions(index, NodeId(1)), expected);
            assert!(
                ovg.nodes
                    .iter()
                    .any(|node| node.read().point() == Point { x: 0.0, y: 0.0 })
            );
        }
    }
}
impl Tree {
    fn fields(self, graph: &ArenaGraph) -> OwnerTreeFields {
        match self {
            Self::Owned(tree) => graph.owner_tree(tree.expect(NIL)).expect(NIL).fields(),
            Self::Numerical(node) => {
                let tree = graph.tree_routing_nodes[&node];
                OwnerTreeFields {
                    node: Some(node),
                    parent: None,
                    children: OwnerTreeSlice::allocated_empty(),
                    sentinel_edge: Some(tree.sentinel_edge),
                    orientation: ArenaGraph::herd_orientation_raw(tree.orientation),
                }
            }
        }
    }
    fn child_is_source(self, graph: &ArenaGraph) -> bool {
        let tree = self.fields(graph);
        tree.sentinel_edge.is_some_and(|edge| {
            graph.owner_edge_endpoint(edge, false).expect("tree edge") == tree.node
        })
    }
    fn parent(self, graph: &ArenaGraph) -> Option<NodeId> {
        let tree = self.fields(graph);
        let edge = tree.sentinel_edge.expect(NIL);
        let from = graph.owner_edge_endpoint(edge, false).expect("tree source");
        if from == tree.node {
            graph.owner_edge_endpoint(edge, true).expect("tree target")
        } else {
            from
        }
    }
    fn arrowhead(self, graph: &ArenaGraph, target: bool) -> String {
        let edge = self.fields(graph).sentinel_edge.expect(NIL);
        let edge = &graph.edge_records[edge.0 as usize];
        if target {
            &edge.target_arrowhead
        } else {
            &edge.source_arrowhead
        }
        .clone()
        .unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
enum TreePort {
    Existing(usize),
    Derived(Point),
}
impl TreePort {
    fn point(self, ovg: &PortBuild) -> Point {
        match self {
            Self::Existing(i) => ovg.nodes[i].point(),
            Self::Derived(point) => point,
        }
    }
}
struct TreePath {
    source: TreePort,
    target: TreePort,
    source_midpoint: Point,
    target_midpoint: Point,
    direction: i64,
    child_side: PortSide,
}
fn side(raw: i64) -> Option<PortSide> {
    match raw {
        4 => Some(PortSide::Top),
        5 => Some(PortSide::Right),
        6 => Some(PortSide::Bottom),
        7 => Some(PortSide::Left),
        _ => None,
    }
}
fn opposite(raw: i64) -> i64 {
    match raw {
        0 => 3,
        1 => 2,
        2 => 1,
        3 => 0,
        4 => 6,
        5 => 7,
        6 => 4,
        7 => 5,
        _ => raw,
    }
}
fn center_index(
    graph: &ArenaGraph,
    node: Option<NodeId>,
    direction: Option<PortSide>,
) -> Option<usize> {
    let node = &graph.nodes[node.expect(NIL).0 as usize];
    direction.and_then(|s| shape_center_port_index(node.shape, s, node.table_column_count))
}
impl PortBuild {
    fn indexed_port(&self, node: NodeId, index: Option<usize>) -> usize {
        let index = index.expect("runtime error: index out of range [-1]");
        let ports = self.ports.get(&node).map(Vec::as_slice).unwrap_or_default();
        ports
            .get(index)
            .unwrap_or_else(|| {
                panic!(
                    "runtime error: index out of range [{index}] with length {}",
                    ports.len()
                )
            })
            .0
    }
    fn tree_path(
        &self,
        graph: &ArenaGraph,
        tree: Tree,
        arrowheads: &BTreeMap<usize, BTreeSet<String>>,
        budget: Budget<'_, '_>,
    ) -> Result<TreePath, ()> {
        step(budget)?;
        let parent = tree.parent(graph);
        let child = tree.fields(graph).node;
        let direction = side(tree.fields(graph).orientation);
        let parent_index = center_index(graph, parent, direction);
        let child_index = center_index(graph, child, direction.map(super::opposite));
        let parent = parent.expect(NIL);
        let child = child.expect(NIL);
        let mut parent_port = self.indexed_port(parent, parent_index);
        let mut child_port = TreePort::Existing(self.indexed_port(child, child_index));
        // A non-cardinal orientation has already failed the source index read.
        let direction = direction.expect("validated tree orientation");
        if let Some(used) = arrowheads.get(&parent_port) {
            let arrow = tree.arrowhead(graph, tree.child_is_source(graph));
            if !used.is_empty() && !used.contains(&arrow) {
                let node = &graph.nodes[parent.0 as usize];
                let mut closest = None;
                let mut distance = f64::INFINITY;
                for index in shape_port_indices(node.shape, direction, node.table_column_count) {
                    step(budget)?;
                    if Some(index) == parent_index {
                        continue;
                    }
                    let candidate = self.indexed_port(parent, Some(index));
                    if arrowheads
                        .get(&candidate)
                        .is_none_or(|used| used.contains(&arrow))
                    {
                        let next = super::euclidean_distance(
                            self.nodes[candidate].point(),
                            child_port.point(self),
                        );
                        if next < distance {
                            closest = Some(candidate);
                            distance = next;
                        }
                    }
                }
                if let Some(closest) = closest {
                    parent_port = closest;
                }
            }
        }
        // alignedPortNodeForBuild reacquires the tree's parent/child fields
        // after callbacks in the alternative-port scan.
        let aligned_parent = rect(graph, tree.parent(graph).expect(NIL));
        let aligned_child = rect(graph, tree.fields(graph).node.expect(NIL));
        let aligned_direction = side(tree.fields(graph).orientation);
        let parent_point = self.nodes[parent_port].point();
        let child_point = child_port.point(self);
        let vertical = (aligned_parent.center().x - aligned_child.center().x).abs() <= 10.0;
        let horizontal = (aligned_parent.center().y - aligned_child.center().y).abs() <= 10.0;
        let derived = if matches!(aligned_direction, Some(PortSide::Top | PortSide::Bottom))
            && vertical
            && parent_point.x != child_point.x
            && aligned_child.origin.x + 5.0 < parent_point.x
            && parent_point.x < aligned_child.origin.x + aligned_child.size.width - 5.0
        {
            Some(Point {
                x: parent_point.x,
                y: child_point.y,
            })
        } else if matches!(aligned_direction, Some(PortSide::Left | PortSide::Right))
            && horizontal
            && parent_point.y != child_point.y
            && aligned_child.origin.y + 5.0 < parent_point.y
            && parent_point.y < aligned_child.origin.y + aligned_child.size.height - 5.0
        {
            Some(Point {
                x: child_point.x,
                y: parent_point.y,
            })
        } else {
            None
        };
        if let Some(point) = derived {
            step(budget)?;
            if let Some(b) = budget {
                b.reserve_nodes(1)?;
            }
            child_port = TreePort::Derived(point);
        }
        for &(index, _) in self.ports.get(&child).into_iter().flatten() {
            step(budget)?;
            if self.nodes[index].point() == child_port.point(self) {
                child_port = TreePort::Existing(index);
                break;
            }
        }
        let child_point = child_port.point(self);
        let mut parent_midpoint = Point { x: 0.0, y: 0.0 };
        let mut child_midpoint = parent_midpoint;
        let midpoint_direction = side(tree.fields(graph).orientation);
        match midpoint_direction {
            Some(PortSide::Left | PortSide::Right) => {
                let r = rect(graph, parent);
                let x = if midpoint_direction == Some(PortSide::Left) {
                    r.origin.x - 50.0
                } else {
                    r.origin.x + r.size.width + 50.0
                };
                parent_midpoint = Point {
                    x,
                    y: parent_point.y,
                };
                child_midpoint = Point {
                    x,
                    y: child_point.y,
                };
            }
            Some(PortSide::Top | PortSide::Bottom) => {
                let r = rect(graph, parent);
                let y = if midpoint_direction == Some(PortSide::Top) {
                    r.origin.y - 50.0
                } else {
                    r.origin.y + r.size.height + 50.0
                };
                parent_midpoint = Point {
                    x: parent_point.x,
                    y,
                };
                child_midpoint = Point {
                    x: child_point.x,
                    y,
                };
            }
            None => {}
        }
        // No callback occurs between these reads and returning the path.
        let child_side = super::opposite(direction);
        let direction = tree.fields(graph).orientation;
        Ok(if tree.child_is_source(graph) {
            TreePath {
                source: child_port,
                target: TreePort::Existing(parent_port),
                source_midpoint: child_midpoint,
                target_midpoint: parent_midpoint,
                direction,
                child_side,
            }
        } else {
            TreePath {
                source: TreePort::Existing(parent_port),
                target: child_port,
                source_midpoint: parent_midpoint,
                target_midpoint: child_midpoint,
                direction: opposite(direction),
                child_side,
            }
        })
    }
    pub(super) fn add_tree_nodes(
        &mut self,
        graph: &ArenaGraph,
        nodes: &[NodeId],
        budget: Budget<'_, '_>,
    ) -> Result<BTreeMap<NodeId, Vec<Port>>, ()> {
        check(budget)?;
        let mut arrowheads = BTreeMap::<usize, BTreeSet<String>>::new();
        let mut added = BTreeMap::<NodeId, Vec<Port>>::new();
        for &node in nodes {
            step(budget)?;
            let tree = if graph.shared_topology_initialized() {
                graph
                    .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(node))
                    .expect("routing graph NodeToTree")
                    .map(Tree::Owned)
            } else {
                graph
                    .tree_routing_nodes
                    .contains_key(&node)
                    .then_some(Tree::Numerical(node))
            };
            let Some(tree) = tree else {
                continue;
            };
            let path = self.tree_path(graph, tree, &arrowheads, budget)?;
            self.add_point(path.source_midpoint, budget)?;
            self.add_point(path.target_midpoint, budget)?;
            let child_is_source = tree.child_is_source(graph);
            let (child, parent, direction) = if child_is_source {
                (path.source, path.target, opposite(path.direction))
            } else {
                (path.target, path.source, path.direction)
            };
            let port = match child {
                TreePort::Existing(index) => self.add_existing_node(index, budget)?,
                TreePort::Derived(point) => self.add_point(point, budget)?,
            };
            if let Some(port) = self.add_tree_port_owner(port, node, direction, path.child_side) {
                added.entry(node).or_default().push(port);
            }
            // Source chooses the branch before addNode's callback, then reads
            // the current sentinel edge's matching arrowhead after publishing.
            if let TreePort::Existing(parent) = parent {
                arrowheads
                    .entry(parent)
                    .or_default()
                    .insert(tree.arrowhead(graph, child_is_source));
            }
        }
        check(budget)?;
        Ok(added)
    }
}
