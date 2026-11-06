// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use crate::internal::owner_groups::OwnerGroupId;
#[cfg(feature = "diagnostic-traces")]
use crate::internal::owner_groups::{OwnerClusterPolicy, OwnerGroupKind};
#[cfg(feature = "diagnostic-traces")]
use crate::internal::shared_topology::OwnerGroupFieldView;
use crate::internal::shared_topology::OwnerMemberHeader;
struct GroupObserve {
    polls: Cell<u64>,
    cancel: u64,
    panic_at: u64,
    mutation_poll: u64,
    mutated: Cell<bool>,
    cluster: OwnerGroupId,
    replacement_group: OwnerGroupId,
    original_members: OwnerMemberHeader,
    point: OwnerNodePoint,
    nodes: Vec<NodeId>,
}
impl Context for GroupObserve {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        if poll == self.panic_at {
            panic!("caller panic")
        };
        (poll == self.cancel).then_some(ContextError::Cancelled)
    }
    #[cfg(feature = "diagnostic-traces")]
    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        if self.polls.get() + 1 == self.mutation_poll {
            self.mutated.set(true);
            let mut policy = owner.owner_cluster_policy(self.cluster).unwrap();
            policy.arrangement = "mutated".into();
            policy.desired_arrangement = "changed".into();
            policy.padding = 99.;
            policy.fixed_size = true;
            policy.edge_abductions = Some(
                owner
                    .allocate_owner_abduction_header(&[None], 1, true)
                    .unwrap(),
            );
            owner
                .set_owner_cluster_policy(self.cluster, policy)
                .unwrap();
            if self.original_members.len() > 1 {
                owner
                    .write_owner_member_capacity_slot(
                        &self.original_members,
                        if self.original_members.len() > 64 {
                            64
                        } else {
                            1
                        },
                        Some(self.nodes[0]),
                    )
                    .unwrap();
            }
            let mut fields = owner.owner_group_fields(self.cluster).unwrap();
            fields.vessel = Some(self.nodes[1]);
            fields.container = Some(self.nodes[3]);
            fields.graph = None;
            fields.nodes = owner
                .allocate_owner_member_header(&[Some(self.nodes[1])], 1)
                .unwrap();
            owner.set_owner_group_fields(self.cluster, fields).unwrap();
            let map = owner.allocate_owner_group_map(OwnerGroupKind::Cluster, true);
            owner
                .write_owner_group_map_entry(
                    &map,
                    Some(self.nodes[0]),
                    Some(self.replacement_group),
                )
                .unwrap();
            owner
                .set_owner_graph_group_map(OwnerGraphId(0), map)
                .unwrap();
            owner.containers = Default::default();
            owner
                .containers
                .insert(Some(self.nodes[0]), vec![self.nodes[3]]);
            owner
                .write_owner_point(&self.point, Point { x: 99., y: 98. })
                .unwrap();
            owner.set_position(self.nodes[2], Point { x: 77., y: 76. });
            owner.nodes[self.nodes[2].0 as usize].rect.size = Size {
                width: 88.,
                height: 87.,
            };
        }
        self.error()
    }
}
fn observe_group(owner: &ArenaGraph, id: OwnerGroupId) -> Value {
    let p = owner.owner_cluster_policy(id).unwrap();
    let f = owner.owner_group_fields(id).unwrap();
    json!({"arrangement":p.arrangement,"desired":p.desired_arrangement,"padding":format!("{:016x}",p.padding.to_bits()),"fixed":p.fixed_size,"abductions":p.edge_abductions.as_ref().map_or(0,|h|h.len()),"vessel":f.vessel.map(|n|owner.nodes[n.0 as usize].external_id.clone()),"container":f.container.map(|n|owner.nodes[n.0 as usize].external_id.clone()),"graph":f.graph.map(|g|g.0),"members":(0..f.nodes.len()).map(|i|f.nodes.get(i).unwrap().map(|n|owner.nodes[n.0 as usize].external_id.clone())).collect::<Vec<_>>()})
}
#[cfg(feature = "diagnostic-traces")]
#[test]
fn complete_group_optimizer_source_controls() {
    let mut cases = [
        "ordinary",
        "node-membership",
        "oversized-cluster",
        "oversized-sequence",
        "nil-cluster-map",
        "nil-sequence-map",
        "outside-members",
        "alias-cluster",
        "duplicate-mutate-cancel-3",
        "duplicate-mutate-cancel-4",
        "duplicate-mutate-cancel-5",
        "metadata-mutate-cancel-4",
        "metadata-mutate-panic-4",
        "nil-cluster",
        "nil-sequence",
        "nil-cluster-member",
        "nil-sequence-member",
        "duplicate-members",
        "empty-cluster",
        "empty-sequence",
        "mutate-success-60",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    cases.extend((0..=700).map(|n| format!("budget-{n}")));
    cases.extend((1..=200).map(|n| format!("cancel-{n}")));
    cases.extend((1..=40).map(|n| format!("mutate-cancel-{n}")));
    cases.extend([60, 120, 200, 500].map(|n| format!("mutate-cancel-{n}")));
    cases.extend([2, 7, 11, 12, 20, 60, 120, 500].map(|n| format!("mutate-panic-{n}")));
    for name in cases {
        let coords = [
            Point { x: 0., y: 0. },
            Point { x: 4., y: 0. },
            Point { x: 0., y: 4. },
            Point { x: 5., y: 5. },
        ];
        let pairs = [(0, 1), (0, 2), (1, 3), (2, 3)];
        let mut input = Graph::default();
        let nodes = (0..4)
            .map(|i| {
                input.add_node(Node::new(
                    ((b'A' + i) as char).to_string(),
                    Size {
                        width: 10.,
                        height: 20.,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let edges = pairs
            .iter()
            .map(|&(a, b)| {
                input.add_edge(Edge {
                    source: nodes[a],
                    target: nodes[b],
                })
            })
            .collect::<Vec<_>>();
        let mut owner = ArenaGraph::from_input(&input);
        let mut workspace = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.cell_size = 10.;
        for (i, &node) in nodes.iter().enumerate() {
            owner.nodes[node.0 as usize].tala_id = i as i64 + 1;
            workspace.nodes[node.0 as usize].tala_id = i as i64 + 1;
            let p = owner.allocate_owner_node_point(Some(coords[i]));
            owner.bind_owner_node_point(node, p).unwrap();
            let incident = pairs
                .iter()
                .enumerate()
                .filter(|(_, (a, b))| *a == i || *b == i)
                .map(|(e, _)| Some(edges[e]))
                .collect::<Vec<_>>();
            let h = owner
                .allocate_owner_incident_header(&incident, incident.len(), true)
                .unwrap();
            owner.set_owner_node_incident_header(node, h).unwrap();
        }
        let seed = name
            .strip_prefix("seed-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let rng = OwnerOptimizerRng::new(GoRng::new(seed));
        if name == "outside-members" {
            let header = owner
                .allocate_owner_member_header(&[Some(nodes[0]), Some(nodes[1])], 2)
                .unwrap();
            owner
                .set_owner_graph_member_header(OwnerGraphId(0), header)
                .unwrap();
        }
        let optim = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(OwnerGraphId(0)),
            (name != "nil-rng").then(|| rng.clone()),
            Some(&Quiet),
        )
        .unwrap();
        let cluster = owner.new_owner_group(OwnerGroupKind::Cluster);
        let replacement_group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let sequence = owner.new_owner_group(OwnerGroupKind::Sequence);
        let members = if name == "nil-cluster-member" {
            vec![None]
        } else if name == "oversized-cluster" {
            vec![Some(nodes[2]); 10001]
        } else if name.contains("duplicate") {
            vec![Some(nodes[2]); 129]
        } else if name.starts_with("metadata-") {
            vec![Some(nodes[2]); 57]
        } else if name == "empty-cluster" {
            vec![]
        } else {
            vec![Some(nodes[2]), Some(nodes[3])]
        };
        let original_members = owner
            .allocate_owner_member_header(&members, members.len().max(2))
            .unwrap();
        let sequence_members = if name == "nil-sequence-member" {
            vec![None]
        } else if name == "oversized-sequence" {
            vec![Some(nodes[3]); 10001]
        } else if name == "empty-sequence" {
            vec![]
        } else {
            vec![Some(nodes[3]), Some(nodes[2])]
        };
        let seq_header = owner
            .allocate_owner_member_header(&sequence_members, sequence_members.len())
            .unwrap();
        let replacement_header = owner
            .allocate_owner_member_header(&[Some(nodes[1])], 1)
            .unwrap();
        for (id, vessel, header) in [
            (cluster, nodes[0], original_members.clone()),
            (sequence, nodes[1], seq_header),
            (replacement_group, nodes[1], replacement_header),
        ] {
            owner
                .set_owner_group_fields(
                    id,
                    OwnerGroupFieldView {
                        vessel: Some(vessel),
                        nodes: header,
                        graph: Some(OwnerGraphId(0)),
                        container: None,
                    },
                )
                .unwrap();
        }
        owner
            .set_owner_cluster_policy(
                cluster,
                OwnerClusterPolicy {
                    arrangement: "Row".into(),
                    desired_arrangement: "Column".into(),
                    padding: 3.,
                    ..Default::default()
                },
            )
            .unwrap();
        owner
            .set_owner_cluster_policy(
                replacement_group,
                OwnerClusterPolicy {
                    arrangement: "replacement".into(),
                    desired_arrangement: "".into(),
                    padding: 7.,
                    ..Default::default()
                },
            )
            .unwrap();
        let cluster_map = owner
            .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
            .unwrap();
        owner
            .write_owner_group_map_entry(
                &cluster_map,
                Some(nodes[0]),
                (name != "nil-cluster").then_some(cluster),
            )
            .unwrap();
        if name == "alias-cluster" {
            owner
                .write_owner_group_map_entry(&cluster_map, Some(nodes[2]), Some(cluster))
                .unwrap();
        }
        let seq_map = owner
            .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Sequence)
            .unwrap();
        owner
            .write_owner_group_map_entry(
                &seq_map,
                Some(nodes[1]),
                (name != "nil-sequence").then_some(sequence),
            )
            .unwrap();
        if name == "node-membership" {
            owner
                .set_owner_node_group(nodes[2], OwnerGroupKind::Cluster, Some(cluster))
                .unwrap();
            owner
                .set_owner_node_group(nodes[3], OwnerGroupKind::Sequence, Some(sequence))
                .unwrap();
        }
        if name == "nil-cluster-map" {
            let map = owner.allocate_owner_group_map(OwnerGroupKind::Cluster, false);
            owner
                .set_owner_graph_group_map(OwnerGraphId(0), map)
                .unwrap();
        }
        if name == "nil-sequence-map" {
            let map = owner.allocate_owner_group_map(OwnerGroupKind::Sequence, false);
            owner
                .set_owner_graph_group_map(OwnerGraphId(0), map)
                .unwrap();
        }
        let retained = nodes
            .iter()
            .map(|&n| owner.owner_node_point(n).unwrap())
            .collect::<Vec<_>>();
        let occupied = optim.occupied().borrow().clone();
        let mutation_poll = if name.starts_with("metadata-") {
            3
        } else if name.contains("mutate-") {
            name.rsplit('-').next().unwrap().parse().unwrap()
        } else {
            0
        };
        let context = GroupObserve {
            polls: Cell::new(0),
            cancel: if name.starts_with("metadata-mutate-cancel-") {
                4
            } else if name.contains("mutate-cancel-") {
                mutation_poll
            } else {
                name.strip_prefix("cancel-")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0)
            },
            panic_at: if name.starts_with("metadata-mutate-panic-") {
                4
            } else if name.contains("mutate-panic-") {
                mutation_poll
            } else {
                0
            },
            mutation_poll,
            mutated: Cell::new(false),
            cluster,
            replacement_group,
            original_members: original_members.clone(),
            point: retained[2].clone(),
            nodes: nodes.clone(),
        };
        let limit = name
            .strip_prefix("budget-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100000);
        let temp = if name == "temperature" { 1.0 } else { 0.0 };
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            nodes.clone(),
            edges.clone(),
        );
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| {
            optim.optimize_with_limit(&workspace, &mut session, &context, temp, limit)
        }));
        drop(session);
        let mut error = Value::Null;
        let mut panic_value = Value::Null;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error = json!(e.to_string()),
            Err(p) => {
                panic_value = json!(
                    p.downcast_ref::<&str>()
                        .copied()
                        .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown")
                )
            }
        };
        let same = nodes
            .iter()
            .zip(&retained)
            .map(|(&n, p)| owner.owner_node_point(n).unwrap().same_allocation(p))
            .collect::<Vec<_>>();
        let entries = map_entries(&optim.occupied().borrow(), &owner);
        let scratch = optim.take_mutation_scratch();
        let released = scratch.captured_nodes().len() == 0
            && !scratch.is_complete()
            && scratch.cost_snapshot().is_none();
        assert!(released);
        optim.retain_mutation_scratch(scratch);
        println!(
            "OWNER_GROUP_LOOP_CONTROL {}",
            json!({"case":name,"groups":[observe_group(&owner,cluster),observe_group(&owner,replacement_group)],"original_members":(0..original_members.len()).map(|i|original_members.get(i).unwrap().map(|n|owner.nodes[n.0 as usize].external_id.clone())).collect::<Vec<_>>(),"current_group_is_original":owner.owner_graph_group_entry(OwnerGraphId(0),OwnerGroupKind::Cluster,Some(nodes[0])).unwrap().flatten()==Some(cluster),"mutated":context.mutated.get(),"old_occupied":map_entries(&occupied,&owner),"replacement_occupied":Value::Null,"sizes":nodes.iter().map(|n|[format!("{:016x}",owner.nodes[n.0 as usize].rect.size.width.to_bits()),format!("{:016x}",owner.nodes[n.0 as usize].rect.size.height.to_bits())]).collect::<Vec<_>>(),"error":error,"panic":panic_value,"polls":context.polls.get(),"points":nodes.iter().map(|&n|bits(owner.position(n))).collect::<Vec<_>>(),"retained":retained.iter().map(|p|bits(p.get())).collect::<Vec<_>>(),"same_retained":same,"occupied":entries,"same_map":optim.occupied().borrow().same_map_allocation(&occupied),"nil_map":optim.occupied().borrow().is_nil(),"scratch_released":released,"next_rng":format!("{:016x}",rng.stream().borrow_mut().float64().to_bits())})
        );
    }
}
