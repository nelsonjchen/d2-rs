// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::super::limits::{Context, ContextError};
use super::*;
use crate::{Graph, Node, Size};
use serde_json::{Value, json};
use std::cell::Cell;

struct Quiet;
impl Context for Quiet {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        None
    }
}
struct Observe<'a> {
    name: &'a str,
    polls: Cell<u64>,
    nodes: [NodeId; 5],
}
impl Context for Observe<'_> {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        self.polls.set(self.polls.get() + 1);
        None
    }
    #[cfg(feature = "diagnostic-traces")]
    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        let p = self.polls.get() + 1;
        self.polls.set(p);
        if p == 2 {
            match self.name {
                "live-capture" => {
                    owner.set_position(self.nodes[0], Point { x: 99.0, y: 100.0 });
                    owner.set_position(self.nodes[1], Point { x: 5.0, y: 6.0 });
                }
                "live-replay" => owner.clear_position(self.nodes[1]),
                "cancel-replay" => return Some(ContextError::Cancelled),
                _ => (),
            }
        }
        None
    }
}
fn bits(p: Option<Point>) -> Value {
    json!(p.map(|p| [
        format!("{:016x}", p.x.to_bits()),
        format!("{:016x}", p.y.to_bits())
    ]))
}
fn points(owner: &ArenaGraph, nodes: &[NodeId]) -> Value {
    json!(
        nodes
            .iter()
            .map(|&n| bits(owner.position(n)))
            .collect::<Vec<_>>()
    )
}

#[cfg(feature = "diagnostic-traces")]
#[test]
fn actual_owner_swap_motion_controls() {
    let mut cases = [
        "commit-leaf",
        "trial-leaf",
        "scored-trial",
        "trial-same",
        "commit-alias",
        "trial-alias",
        "commit-overlap",
        "trial-overlap",
        "nil-descendant",
        "nil-member",
        "commit-root-nil",
        "trial-root-nil",
        "commit-nil-argument",
        "trial-nil-argument",
        "callback-error",
        "callback-panic",
        "callback-replace",
        "live-capture",
        "live-replay",
        "cancel-replay",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for limit in 0..=40 {
        cases.push(format!("commit-budget-{limit}"));
        cases.push(format!("trial-budget-{limit}"));
    }
    for name in cases {
        let mut input = Graph::default();
        let nodes = ["A", "B", "C", "D", "E"].map(|id| {
            input.add_node(Node::new(
                id,
                Size {
                    width: 10.0,
                    height: 20.0,
                },
            ))
        });
        let edges = [1, 2, 3, 4].map(|i| {
            input.add_edge(Edge {
                source: nodes[0],
                target: nodes[i],
            })
        });
        let workspace = ArenaGraph::from_input(&input);
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.cell_size = 10.0;
        for (i, &n) in nodes.iter().enumerate() {
            let slots = if i == 0 {
                edges.iter().copied().map(Some).collect::<Vec<_>>()
            } else {
                vec![Some(edges[i - 1])]
            };
            let h = owner
                .allocate_owner_incident_header(&slots, slots.len(), true)
                .unwrap();
            owner.set_owner_node_incident_header(n, h).unwrap();
            let p = owner.allocate_owner_node_point(Some(Point {
                x: i as f64 * 2.0,
                y: i as f64 * -3.0,
            }));
            owner.bind_owner_node_point(n, p).unwrap();
        }
        let optim =
            OwnerSizelessOptimizer::new(&mut owner, Some(OwnerGraphId(0)), None, Some(&Quiet))
                .unwrap();
        if name.ends_with("alias") {
            let p = owner.owner_node_point(nodes[0]).unwrap();
            owner.bind_owner_node_point(nodes[1], p).unwrap();
        }
        if name.contains("overlap")
            || name.contains("budget")
            || matches!(name.as_str(), "nil-descendant" | "nil-member")
        {
            for root in [nodes[0], nodes[1]] {
                let group = owner.new_owner_group(OwnerGroupKind::Cluster);
                let mut f = owner.owner_group_fields(group).unwrap();
                let slots = if name == "nil-member" {
                    vec![None]
                } else {
                    vec![Some(nodes[2]), Some(nodes[3])]
                };
                f.nodes = owner
                    .allocate_owner_member_header(&slots, slots.len())
                    .unwrap();
                owner.set_owner_group_fields(group, f).unwrap();
                owner.set_owner_cluster_vessel_flag(root, true).unwrap();
                let map = owner
                    .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
                    .unwrap();
                owner
                    .write_owner_group_map_entry(&map, Some(root), Some(group))
                    .unwrap();
            }
        }
        if name == "nil-descendant" {
            owner.clear_position(nodes[3]);
        }
        if name.ends_with("root-nil") {
            owner.clear_position(nodes[0]);
        }
        if name == "scored-trial" {
            assert_eq!(
                owner
                    .owner_graph_direction_entry(OwnerGraphId(0), None)
                    .unwrap(),
                Some(super::super::Orientation::Bottom)
            );
            assert!(
                owner
                    .edge_records
                    .iter()
                    .all(|e| !e.source_arrow && !e.target_arrow)
            );
        }
        let retained: Vec<OwnerNodePoint> = nodes
            .iter()
            .map(|&n| owner.owner_node_point(n).unwrap())
            .collect();
        let context = Observe {
            name: &name,
            polls: Cell::new(0),
            nodes,
        };
        let limit = name
            .rsplit('-')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100000);
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "control", limit, &mut owner)
                .unwrap();
        if name == "live-capture" {
            guard.add_with_owner(62, &mut owner).unwrap();
        }
        if matches!(name.as_str(), "live-replay" | "cancel-replay") {
            guard.add_with_owner(60, &mut owner).unwrap();
        }
        let a = if name.ends_with("nil-argument") {
            None
        } else {
            Some(nodes[0])
        };
        let b = Some(if name == "trial-same" {
            nodes[0]
        } else {
            nodes[1]
        });
        let mut callback = Value::Null;
        let mut replacement = Value::Null;
        let mut scores = Value::Null;
        let result = catch_unwind(AssertUnwindSafe(|| {
            if name.starts_with("commit-") {
                optim.commit_swap_guarded(&mut owner, a, b, &mut guard)
            } else {
                optim.with_swap_trial_guarded(&mut owner, a, b, &mut guard, |owner, _guard| {
                    callback = points(owner, &nodes);
                    if name == "scored-trial" {
                        use super::super::OwnerScoreFrame;
                        use super::super::scoring::OwnerSizelessEdgeLengthScorer;
                        use super::super::scoring_context::ScoreContext;
                        let mut values = Vec::new();
                        for node in [nodes[0], nodes[1]] {
                            let edges = owner.owner_node_incident_count(node) as u64 + 1;
                            let receiver = owner.topology_node_owner(node).unwrap();
                            let count =
                                owner.owner_graph_member_header(receiver).unwrap().len() as u64 + 1;
                            _guard.add_product_with_owner(edges, count, owner)?;
                            let frame = OwnerScoreFrame::new(
                                owner,
                                owner.topology_arena_id().unwrap(),
                                OwnerGraphId(0),
                                &nodes,
                            )
                            .unwrap();
                            let mut scorer = OwnerSizelessEdgeLengthScorer::new(node, true);
                            let value = scorer
                                .score(&workspace, &frame, ScoreContext::none())
                                .unwrap();
                            scorer.close();
                            values.push(format!("{:016x}", value.to_bits()));
                        }
                        scores = json!(values);
                    }
                    match name.as_str() {
                        "callback-error" => {
                            return Err(OptimizationError::Invalid("callback error".into()));
                        }
                        "callback-panic" => panic!("callback boom"),
                        "callback-replace" => {
                            let p = owner.allocate_owner_node_point(Some(Point { x: 7.0, y: 8.0 }));
                            owner.bind_owner_node_point(nodes[0], p.clone()).unwrap();
                            owner
                                .write_owner_point(&retained[0], Point { x: 90.0, y: 91.0 })
                                .unwrap();
                            replacement = bits(p.get());
                        }
                        _ => (),
                    }
                    Ok(())
                })
            }
        }));
        let (error, panic) = match result {
            Ok(Ok(())) => (Value::Null, Value::Null),
            Ok(Err(e)) => (json!(e.to_string()), Value::Null),
            Err(p) => (
                Value::Null,
                json!(
                    p.downcast_ref::<&str>()
                        .copied()
                        .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown")
                ),
            ),
        };
        let aliases = nodes
            .iter()
            .zip(&retained)
            .map(|(&n, p)| owner.owner_node_point(n).unwrap().same_allocation(p))
            .collect::<Vec<_>>();
        let row = json!({"case":name,"error":error,"panic":panic,"used":guard.used(),"polls":context.polls.get(),"callback":callback,"points":points(&owner,&nodes),"retained":retained.iter().map(|p|bits(p.get())).collect::<Vec<_>>(),"same_retained":aliases,"alias_ab":owner.owner_node_point(nodes[0]).unwrap().same_allocation(&owner.owner_node_point(nodes[1]).unwrap()),"replacement":replacement,"scores":scores});
        println!("OWNER_SWAP_MOTION_CONTROL {row}");
    }
}

#[test]
fn selected_container_headers_preserve_copy_fork_and_rollback_contracts() {
    let mut input = Graph::default();
    for i in 0..3 {
        input.add_node(Node::new(
            format!("n{i}"),
            Size {
                width: 1.,
                height: 1.,
            },
        ));
    }
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    let first = owner.create_owner_graph_view(vec![], vec![]).unwrap();
    let second = owner.create_owner_graph_view(vec![], vec![]).unwrap();
    let map = owner.allocate_owner_container_map(true);
    owner
        .write_owner_container_map_entry(
            &map,
            Some(NodeId(0)),
            &[Some(NodeId(1)), None, Some(NodeId(2))],
        )
        .unwrap();
    owner
        .set_owner_graph_container_map(first, map.clone())
        .unwrap();
    owner
        .set_owner_graph_container_map(second, map.clone())
        .unwrap();
    let snapshot = owner.owner_topology_snapshot();
    let replacement = owner.allocate_owner_container_map(true);
    owner
        .set_owner_graph_container_map(first, replacement.clone())
        .unwrap();
    owner
        .write_owner_container_map_entry(&map, Some(NodeId(0)), &[Some(NodeId(2))])
        .unwrap();
    assert!(
        !owner
            .owner_container_map(first)
            .unwrap()
            .same_map_allocation(&owner.owner_container_map(second).unwrap())
    );
    owner.restore_owner_topology_snapshot(snapshot);
    for id in [first, second] {
        let restored = owner.owner_container_map(id).unwrap();
        assert!(restored.same_map_allocation(&map));
        let values = restored.get(Some(NodeId(0))).unwrap();
        assert_eq!(
            (0..values.len())
                .map(|i| values.get(i).unwrap())
                .collect::<Vec<_>>(),
            [Some(NodeId(1)), None, Some(NodeId(2))]
        );
    }
    let mut fork = owner.clone();
    let a = fork.owner_container_map(first).unwrap();
    let b = fork.owner_container_map(second).unwrap();
    assert!(a.same_map_allocation(&b));
    assert!(!a.same_map_allocation(&map));
    assert!(
        fork.set_owner_graph_container_map(first, map.clone())
            .is_err()
    );
    assert!(
        fork.write_owner_container_map_entry(&map, None, &[])
            .is_err()
    );
    fork.write_owner_container_map_entry(&a, Some(NodeId(0)), &[])
        .unwrap();
    assert_eq!(
        owner
            .owner_container_map(first)
            .unwrap()
            .get(Some(NodeId(0)))
            .unwrap()
            .len(),
        3
    );
    let nil = owner.allocate_owner_container_map(false);
    owner.set_owner_graph_container_map(first, nil).unwrap();
    assert_eq!(owner.owner_container_map(first).unwrap().len(), 0);
    assert!(
        owner
            .owner_container_map(first)
            .unwrap()
            .get(Some(NodeId(0)))
            .is_none()
    );
}
