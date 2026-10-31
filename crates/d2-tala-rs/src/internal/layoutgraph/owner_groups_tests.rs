// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::super::shared_topology::{
    OwnerGraphId, OwnerMemberHeader, OwnerTopologyError, TopologyVessel,
};
use super::super::{ArenaGraph, ClusterArrangement, ClusterState};
use super::*;
use crate::{Graph, Node, Size};
use serde_json::{Value, json};

fn fixture() -> (ArenaGraph, [NodeId; 4], OwnerGraphId) {
    let mut input = Graph::default();
    let nodes = ["A", "B", "C", "D"].map(|name| {
        input.add_node(Node::new(
            name,
            Size {
                width: 10.0,
                height: 20.0,
            },
        ))
    });
    let mut owner = ArenaGraph::from_input(&input);
    owner.initialize_shared_topology();
    owner.cell_size = 10.0;
    let main_nodes = owner
        .allocate_owner_member_header(&[Some(nodes[0]), Some(nodes[1])], 4)
        .unwrap();
    owner
        .set_owner_graph_member_header(OwnerGraphId(0), main_nodes)
        .unwrap();
    let other = owner
        .create_owner_graph_view(Vec::new(), Vec::new())
        .unwrap();
    owner.set_owner_graph_cell_size(other, 20.0).unwrap();
    (owner, nodes, other)
}

#[test]
fn retained_group_map_fields_restore_original_allocations_and_fork_numerical_candidates() {
    for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
        let (mut owner, nodes, other) = fixture();
        let main = OwnerGraphId(0);
        let field = owner
            .retain_owner_graph_group_map_field(main, kind)
            .unwrap();
        let held = field.header();
        owner
            .copy_owner_graph_group_map_from(other, main, kind)
            .unwrap();
        let other_field = owner
            .retain_owner_graph_group_map_field(other, kind)
            .unwrap();
        assert!(!field.same_field(&other_field));
        assert!(held.same_map_allocation(&other_field.header()));
        let snapshot = owner.owner_topology_snapshot();
        let changed = owner.allocate_owner_group_map(kind, true);
        field.replace(changed.clone()).unwrap();
        let group = owner.new_owner_group(kind);
        owner
            .write_owner_group_map_entry(&held, Some(nodes[0]), Some(group))
            .unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(field.header().len(), 0);
        owner.restore_owner_topology_snapshot(snapshot);
        assert!(
            field.same_field(
                &owner
                    .retain_owner_graph_group_map_field(main, kind)
                    .unwrap()
            )
        );
        assert!(field.header().same_map_allocation(&held));
        assert!(other_field.header().same_map_allocation(&held));
        assert_eq!(held.len(), 0);
        let fork = owner.clone();
        let copied = fork.retain_owner_graph_group_map_field(main, kind).unwrap();
        let copied_other = fork
            .retain_owner_graph_group_map_field(other, kind)
            .unwrap();
        assert!(!field.same_field(&copied));
        assert!(!field.header().same_map_allocation(&copied.header()));
        assert!(copied.header().same_map_allocation(&copied_other.header()));
        assert!(copied.replace(field.header()).is_err());
        let wrong_kind = owner.allocate_owner_group_map(
            if kind == OwnerGroupKind::Cluster {
                OwnerGroupKind::Sequence
            } else {
                OwnerGroupKind::Cluster
            },
            true,
        );
        assert!(field.replace(wrong_kind).is_err());
    }
}
fn observe_header(h: Option<&OwnerMemberHeader>, nodes: &[NodeId; 4]) -> Value {
    let Some(h) = h else {
        return json!({"present":false,"nil":true,"len":0,"cap":0,"slots":[]});
    };
    let slots: Vec<_> = (0..h.capacity())
        .map(|i| {
            h.read_capacity_slot(i)
                .unwrap()
                .map(|node| nodes.iter().position(|&known| known == node).unwrap())
        })
        .collect();
    json!({"present":true,"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":slots})
}
#[test]
fn owner_groups_actual_source_controls() {
    for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
        for mode in [
            "new",
            "nullable",
            "copy-live",
            "retained",
            "restore",
            "restore-nil",
        ] {
            let (mut owner, nodes, other) = fixture();
            let main = OwnerGraphId(0);
            let groups = [owner.new_owner_group(kind), owner.new_owner_group(kind)];
            let original = owner.owner_graph_group_map(main, kind).unwrap();
            let mut retained = None;
            let mut replacement = None;
            if mode == "nullable" {
                owner
                    .write_owner_group_map_entry(&original, None, None)
                    .unwrap();
                owner
                    .write_owner_group_map_entry(&original, Some(nodes[0]), Some(groups[0]))
                    .unwrap();
                owner
                    .write_owner_group_map_entry(&original, Some(nodes[1]), None)
                    .unwrap();
            }
            if mode == "copy-live" || mode == "retained" || mode == "restore" {
                let mut f = owner.owner_group_fields(groups[0]).unwrap();
                f.nodes = owner.owner_graph_member_header(main).unwrap();
                f.vessel = Some(nodes[2]);
                f.graph = Some(other);
                f.container = Some(nodes[1]);
                owner.set_owner_group_fields(groups[0], f).unwrap();
                let mut f = owner.owner_group_fields(groups[1]).unwrap();
                f.nodes = owner
                    .owner_graph_member_header(main)
                    .unwrap()
                    .reslice_with_capacity(1, 2, 4)
                    .unwrap();
                owner.set_owner_group_fields(groups[1], f).unwrap();
                owner
                    .write_owner_group_map_entry(&original, Some(nodes[0]), Some(groups[0]))
                    .unwrap();
                owner
                    .copy_owner_graph_group_map_from(other, main, kind)
                    .unwrap();
                let copied = owner.owner_graph_group_map(other, kind).unwrap();
                owner
                    .write_owner_group_map_entry(&copied, Some(nodes[1]), Some(groups[1]))
                    .unwrap();
            }
            if mode == "retained" || mode == "restore" {
                retained = Some(owner.owner_group_fields(groups[0]).unwrap().nodes);
                let snapshot = (mode == "restore").then(|| owner.owner_topology_snapshot());
                let old = retained.as_ref().unwrap();
                owner
                    .write_owner_member_capacity_slot(old, 1, None)
                    .unwrap();
                owner
                    .write_owner_member_capacity_slot(old, 2, Some(nodes[3]))
                    .unwrap();
                let mut fields = owner.owner_group_fields(groups[0]).unwrap();
                fields.nodes = owner
                    .allocate_owner_member_header(&[Some(nodes[1])], 1)
                    .unwrap();
                fields.vessel = Some(nodes[3]);
                owner.set_owner_group_fields(groups[0], fields).unwrap();
                if let Some(snapshot) = snapshot {
                    owner
                        .write_owner_group_map_entry(&original, None, None)
                        .unwrap();
                    owner
                        .delete_owner_group_map_entry(&original, Some(nodes[0]))
                        .unwrap();
                    let changed = owner.allocate_owner_group_map(kind, true);
                    owner
                        .write_owner_group_map_entry(&changed, Some(nodes[2]), Some(groups[1]))
                        .unwrap();
                    owner
                        .set_owner_graph_group_map(main, changed.clone())
                        .unwrap();
                    replacement = Some(changed);
                    owner.restore_owner_topology_snapshot(snapshot);
                }
            }
            if mode == "restore-nil" {
                let nil = owner.allocate_owner_group_map(kind, false);
                owner.set_owner_graph_group_map(main, nil).unwrap();
                let snapshot = owner.owner_topology_snapshot();
                let changed = owner.allocate_owner_group_map(kind, true);
                owner
                    .write_owner_group_map_entry(&changed, Some(nodes[2]), Some(groups[1]))
                    .unwrap();
                owner
                    .set_owner_graph_group_map(main, changed.clone())
                    .unwrap();
                replacement = Some(changed);
                owner.restore_owner_topology_snapshot(snapshot);
            }
            let map = owner.owner_graph_group_map(main, kind).unwrap();
            let other_map = owner.owner_graph_group_map(other, kind).unwrap();
            let shares = if map.is_nil() {
                false
            } else {
                owner
                    .write_owner_group_map_entry(&map, Some(nodes[3]), None)
                    .unwrap();
                let visible = owner
                    .owner_graph_group_entry(other, kind, Some(nodes[3]))
                    .unwrap()
                    .is_some();
                owner
                    .delete_owner_group_map_entry(&map, Some(nodes[3]))
                    .unwrap();
                visible
            };
            let entries:Vec<_>=[None,Some(nodes[0]),Some(nodes[1]),Some(nodes[2])].into_iter().map(|key|{let entry=owner.owner_group_map_entry(&map,key).unwrap();json!({"present":entry.is_some(),"group":entry.flatten().map(|group|groups.iter().position(|&known|known==group).unwrap())})}).collect();
            let fields = groups.map(|group| owner.owner_group_fields(group).unwrap());
            let headers = [
                Some(owner.owner_graph_member_header(main).unwrap()),
                Some(fields[0].nodes.clone()),
                Some(fields[1].nodes.clone()),
                retained,
            ];
            let mut overlaps = Vec::new();
            for left in 0..4 {
                if let Some(a) = &headers[left] {
                    for ai in 0..a.capacity() {
                        for right in left + 1..4 {
                            if let Some(b) = &headers[right] {
                                for bi in 0..b.capacity() {
                                    if a.slot_address(ai) == b.slot_address(bi) {
                                        overlaps.push([left, ai, right, bi])
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let group_fields:Vec<_>=fields.iter().map(|f|json!({"vessel":f.vessel.map(|node|nodes.iter().position(|&n|n==node).unwrap()),"graph":f.graph.map(|g|g.0),"container":f.container.map(|node|nodes.iter().position(|&n|n==node).unwrap())})).collect();
            let replacement_entry = replacement.as_ref().map(|m| {
                owner
                    .owner_group_map_entry(m, Some(nodes[2]))
                    .unwrap()
                    .flatten()
                    .map(|g| groups.iter().position(|&known| known == g).unwrap())
            });
            let label = match kind {
                OwnerGroupKind::Cluster => "cluster",
                OwnerGroupKind::Sequence => "sequence",
            };
            let row = json!({"case":format!("{label}-{mode}"),"map_nil":map.is_nil(),"map_len":map.len(),"other_nil":other_map.is_nil(),"other_len":other_map.len(),"shares_map":shares,"entries":entries,"headers":headers.iter().map(|h|observe_header(h.as_ref(),&nodes)).collect::<Vec<_>>(),"overlaps":overlaps,"group_fields":group_fields,"cells":[owner.owner_graph_cell_size(main).unwrap(),owner.owner_graph_cell_size(other).unwrap()],"node_owners":nodes.map(|node|owner.topology_node_owner(node).map(|id|id.0)),"replacement_len":replacement.as_ref().map(|m|m.len()),"replacement_entry":replacement_entry});
            println!("OWNER_GROUPS_CONTROL {row}");
        }
    }
}

#[test]
fn group_nodes_and_graph_members_detach_together_in_numerical_fork() {
    let (mut owner, nodes, other) = fixture();
    let group = owner.new_owner_group(OwnerGroupKind::Cluster);
    let mut fields = owner.owner_group_fields(group).unwrap();
    fields.nodes = owner
        .owner_graph_member_header(OwnerGraphId(0))
        .unwrap()
        .reslice_with_capacity(1, 2, 4)
        .unwrap();
    owner.set_owner_group_fields(group, fields).unwrap();
    let original = owner
        .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
        .unwrap();
    owner
        .write_owner_group_map_entry(&original, Some(nodes[0]), Some(group))
        .unwrap();
    owner
        .copy_owner_graph_group_map_from(other, OwnerGraphId(0), OwnerGroupKind::Cluster)
        .unwrap();
    let mut fork = owner.clone();
    let fork_members = fork.owner_graph_member_header(OwnerGraphId(0)).unwrap();
    let mut fields = fork.owner_group_fields(group).unwrap();
    assert_eq!(fork_members.slot_address(1), fields.nodes.slot_address(0));
    assert_ne!(
        fork_members.slot_address(1),
        owner
            .owner_graph_member_header(OwnerGraphId(0))
            .unwrap()
            .slot_address(1)
    );
    assert_ne!(
        fork.owner_group(group).unwrap() as *const _,
        owner.owner_group(group).unwrap() as *const _
    );
    assert!(
        fork.owner_graph_group_map(other, OwnerGroupKind::Cluster)
            .unwrap()
            .same_map_allocation(
                &fork
                    .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
                    .unwrap()
            )
    );
    assert!(
        !original.same_map_allocation(
            &fork
                .owner_graph_group_map(other, OwnerGroupKind::Cluster)
                .unwrap()
        )
    );
    fork.write_owner_member_capacity_slot(&fields.nodes, 0, None)
        .unwrap();
    assert_eq!(fork_members.get(1), Some(None));
    assert_eq!(
        owner.owner_group_fields(group).unwrap().nodes.get(0),
        Some(Some(nodes[1]))
    );
    fields.nodes = owner.owner_group_fields(group).unwrap().nodes;
    assert_eq!(
        fork.set_owner_group_fields(group, fields),
        Err(OwnerTopologyError::ForeignMemberScope)
    );
    assert_eq!(
        fork.set_owner_graph_group_map(other, original),
        Err(OwnerTopologyError::ForeignGroupScope)
    );
}
#[test]
fn canonical_group_vessel_is_the_bound_references_single_live_authority() {
    let (mut owner, nodes, _) = fixture();
    owner.clusters.push(ClusterState {
        members: vec![nodes[0]],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 0.0,
        vessel_tala_id: owner.nodes[nodes[2].0 as usize].tala_id,
        fixed_size: false,
    });
    owner.nodes[nodes[0].0 as usize].cluster = Some(0);
    owner.nodes[nodes[2].0 as usize].container = Some(nodes[1]);
    let group = owner.new_owner_group(OwnerGroupKind::Cluster);
    let mut fields = owner.owner_group_fields(group).unwrap();
    fields.vessel = Some(nodes[2]);
    owner.set_owner_group_fields(group, fields).unwrap();
    owner
        .bind_owner_group_reference(TopologyVessel::Cluster(0), group)
        .unwrap();
    assert_eq!(
        super::super::owner_placement::owner_owning_container(&owner, nodes[0]),
        Ok(Some(nodes[1]))
    );
    owner
        .set_topology_vessel_reference(TopologyVessel::Cluster(0), Some(nodes[3]))
        .unwrap();
    assert_eq!(
        owner.owner_group_fields(group).unwrap().vessel,
        Some(nodes[3])
    );
    let mut fields = owner.owner_group_fields(group).unwrap();
    fields.vessel = None;
    owner.set_owner_group_fields(group, fields).unwrap();
    assert_eq!(
        owner.topology_vessel_reference(TopologyVessel::Cluster(0)),
        Some(None)
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::super::owner_placement::owner_owning_container(&owner, nodes[0])
        }))
        .is_err()
    );
}
#[test]
fn retained_group_records_survive_map_restore_and_reject_wrong_kind() {
    let (mut owner, nodes, _) = fixture();
    let main = OwnerGraphId(0);
    let map = owner
        .owner_graph_group_map(main, OwnerGroupKind::Cluster)
        .unwrap();
    let snapshot = owner.owner_topology_snapshot();
    let group = owner.new_owner_group(OwnerGroupKind::Cluster);
    let ptr = owner.owner_group(group).unwrap() as *const _;
    owner
        .write_owner_group_map_entry(&map, Some(nodes[0]), Some(group))
        .unwrap();
    owner.restore_owner_topology_snapshot(snapshot);
    assert_eq!(owner.owner_group(group).unwrap() as *const _, ptr);
    assert_eq!(owner.owner_group_map_entry(&map, Some(nodes[0])), Ok(None));
    assert_ne!(owner.new_owner_group(OwnerGroupKind::Cluster), group);
    let sequence = owner
        .owner_graph_group_map(main, OwnerGroupKind::Sequence)
        .unwrap();
    assert_eq!(
        owner.write_owner_group_map_entry(&sequence, None, Some(group)),
        Err(OwnerTopologyError::GroupKindMismatch)
    );
    let nil = owner.allocate_owner_group_map(OwnerGroupKind::Cluster, false);
    assert_eq!(
        owner.write_owner_group_map_entry(&nil, None, Some(group)),
        Err(OwnerTopologyError::NilGroupMapAssignment)
    );
}

#[test]
fn bound_group_descendants_use_replaced_live_members_instead_of_dense_members() {
    let (mut owner, nodes, _) = fixture();
    owner.clusters.push(ClusterState {
        members: vec![nodes[3]],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 0.0,
        vessel_tala_id: owner.nodes[nodes[2].0 as usize].tala_id,
        fixed_size: false,
    });
    let group = owner.new_owner_group(OwnerGroupKind::Cluster);
    let mut fields = owner.owner_group_fields(group).unwrap();
    fields.vessel = Some(nodes[2]);
    fields.nodes = owner
        .allocate_owner_member_header(&[Some(nodes[0]), None, Some(nodes[1])], 4)
        .unwrap();
    owner.set_owner_group_fields(group, fields).unwrap();
    owner
        .bind_owner_group_reference(TopologyVessel::Cluster(0), group)
        .unwrap();
    assert_eq!(
        owner.topology_retained_closure(&[nodes[2]]).unwrap(),
        [nodes[2], nodes[0], nodes[1]]
    );
    let mut fields = owner.owner_group_fields(group).unwrap();
    fields.nodes = owner
        .allocate_owner_member_header(&[None, Some(nodes[3])], 2)
        .unwrap();
    owner.set_owner_group_fields(group, fields).unwrap();
    assert_eq!(
        owner.topology_retained_closure(&[nodes[2]]).unwrap(),
        [nodes[2], nodes[3]]
    );
}

#[test]
fn preferred_arrangement_updates_the_retained_group_without_replacing_other_policy() {
    let (mut owner, nodes, other) = fixture();
    owner.clusters.push(ClusterState {
        members: vec![nodes[0], nodes[1]],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: owner.nodes[nodes[2].0 as usize].tala_id,
        fixed_size: false,
    });
    let group = owner.new_owner_group(OwnerGroupKind::Cluster);
    let mut fields = owner.owner_group_fields(group).unwrap();
    fields.vessel = Some(nodes[2]);
    owner.set_owner_group_fields(group, fields).unwrap();
    owner
        .bind_owner_group_reference(TopologyVessel::Cluster(0), group)
        .unwrap();
    owner
        .set_owner_cluster_policy(
            group,
            OwnerClusterPolicy {
                arrangement: "Column".into(),
                desired_arrangement: "Column".into(),
                padding: 37.5,
                fixed_size: true,
                edge_abductions: None,
            },
        )
        .unwrap();
    let map = owner
        .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
        .unwrap();
    owner
        .write_owner_group_map_entry(&map, Some(nodes[2]), Some(group))
        .unwrap();
    owner
        .copy_owner_graph_group_map_from(other, OwnerGraphId(0), OwnerGroupKind::Cluster)
        .unwrap();
    // Retaining a map alias and then deleting its entry does not invalidate
    // the separate Cluster pointer held by the algorithm's carrier binding.
    owner
        .delete_owner_group_map_entry(&map, Some(nodes[2]))
        .unwrap();
    owner.set_cluster_desired_arrangement(0, ClusterArrangement::Row);
    assert_eq!(
        owner.owner_group_binding(TopologyVessel::Cluster(0)),
        Some(group)
    );
    assert_eq!(
        owner
            .owner_graph_group_entry(other, OwnerGroupKind::Cluster, Some(nodes[2]))
            .unwrap(),
        None
    );
    let policy = owner.owner_cluster_policy(group).unwrap();
    assert_eq!(policy.desired_arrangement, "Row");
    assert_eq!(policy.arrangement, "Column");
    assert_eq!(policy.padding, 37.5);
    assert!(policy.fixed_size);
    assert!(policy.edge_abductions.is_none());
    assert_eq!(owner.clusters[0].arrangement, ClusterArrangement::Column);
    assert_eq!(
        owner.clusters[0].desired_arrangement,
        ClusterArrangement::Row
    );
    let sequence = owner.new_owner_group(OwnerGroupKind::Sequence);
    assert!(matches!(
        owner.set_owner_cluster_desired_arrangement(sequence, "Row".into()),
        Err(OwnerTopologyError::GroupKindMismatch)
    ));
}
