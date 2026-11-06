// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use serde_json::{Value, json};
fn decoded(value: Value) -> ArenaGraph {
    let input =
        crate::internal::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap())
            .unwrap();
    ArenaGraph::from_input(&input)
}
fn basic() -> Value {
    json!({"nodes":[{"id":1},{"id":2},{"id":3}],"edges":[{"id":11,"from":1,"to":2}],"containers":{"0":[1,2,3]}})
}

#[test]
fn decoded_group_fields_headers_and_backlinks_are_actual_shared_records() {
    let mut input = basic();
    input["clusters"] = json!({"99":{"Nodes":[1,2,3],"Arrangement":"odd","DesiredArrangement":"Column","Padding":1.25,"FixedSize":true,
        "EdgeAbductions":[{"Edge":11,"CurrentFromNode":99},{"Edge":11,"CurrentFromNode":99},{"Edge":11,"CurrentFromNode":99}]}});
    input["clusterVessels"] = json!({"99":{"id":99}});
    let g = decoded(input);
    let group = g.owner_group_binding(TopologyVessel::Cluster(0)).unwrap();
    let fields = g.owner_group_fields(group).unwrap();
    assert_eq!((fields.nodes.len(), fields.nodes.capacity()), (3, 4));
    assert_eq!(fields.nodes.read_capacity_slot(3), Some(None));
    assert_eq!(fields.vessel, Some(NodeId(3)));
    assert_eq!(fields.graph, Some(OwnerGraphId(0)));
    assert_eq!(g.topology_node_owner(NodeId(3)), None);
    let map = g
        .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
        .unwrap();
    assert_eq!(
        g.owner_group_map_entry(&map, Some(NodeId(3))).unwrap(),
        Some(Some(group))
    );
    for member in [NodeId(0), NodeId(1), NodeId(2)] {
        assert_eq!(
            g.owner_node_group_id(member, OwnerGroupKind::Cluster)
                .unwrap(),
            Some(group)
        );
    }
    let policy = g.owner_cluster_policy(group).unwrap();
    assert_eq!(policy.arrangement, "odd");
    assert_eq!(policy.desired_arrangement, "Column");
    assert!(policy.fixed_size);
    assert_eq!(policy.padding, 1.25);
    let header = policy.edge_abductions.unwrap();
    assert_eq!((header.len(), header.capacity()), (3, 3));
    assert_ne!(header.get(0), header.get(1));
    assert_eq!(
        g.owner_edge_abduction(header.get(0).unwrap().unwrap())
            .unwrap()
            .current_from,
        Some(NodeId(3))
    );
}

#[test]
fn final_group_pointer_controls_decoded_graph_membership() {
    for (field, vessels, kind) in [
        ("clusters", "clusterVessels", OwnerGroupKind::Cluster),
        ("sequences", "sequenceVessels", OwnerGroupKind::Sequence),
    ] {
        let mut input = basic();
        input["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":90}));
        input[field] = json!({"90":{"Nodes":[1,2]},"99":{"Nodes":[2,3]}});
        input[vessels] = json!({"99":{"id":99}});
        let g = decoded(input);
        assert_eq!(g.node_order, vec![NodeId(1), NodeId(2), NodeId(3)]);
        let last = g.owner_node_group_id(NodeId(1), kind).unwrap().unwrap();
        assert_eq!(g.owner_group_fields(last).unwrap().vessel, Some(NodeId(4)));
        assert_eq!(
            g.owner_graph_member_header(OwnerGraphId(0))
                .unwrap()
                .capacity(),
            4
        );
    }
}

#[test]
fn effective_zero_ids_do_not_merge_tree_edges_or_abduction_references() {
    let mut input = basic();
    input["edges"] =
        json!([{"id":-1,"originalId":0,"from":1,"to":2},{"id":-2,"originalId":0,"from":2,"to":3}]);
    input["clusters"] = json!({"99":{"Nodes":[1,2],"EdgeAbductions":[{"Edge":-1},{"Edge":-2}]}});
    input["clusterVessels"] = json!({"99":{"id":99}});
    input["trees"] = json!({"0":[{"Node":{"id":1},"SentinelEdge":input["edges"][0],"Orientation":9223372036854775807_i64},{"Node":{"id":2},"SentinelEdge":input["edges"][1],"Orientation":-9223372036854775808_i64}]});
    let g = decoded(input);
    let group = g.owner_group_binding(TopologyVessel::Cluster(0)).unwrap();
    let header = g
        .owner_cluster_policy(group)
        .unwrap()
        .edge_abductions
        .unwrap();
    for i in 0..2 {
        assert_eq!(
            g.owner_edge_abduction(header.get(i).unwrap().unwrap())
                .unwrap()
                .source_edge,
            Some(EdgeId(i as u32))
        );
    }
    let roots = g.shared_topology.views[0]
        .as_ref()
        .unwrap()
        .tree_roots
        .get(None)
        .unwrap();
    for i in 0..2 {
        let tree = roots.get(i).unwrap().unwrap();
        let fields = g.owner_tree(tree).unwrap().fields();
        assert_eq!(fields.sentinel_edge, Some(EdgeId(i as u32)));
        assert_eq!(fields.orientation, [i64::MAX, i64::MIN][i]);
        assert!(fields.children.is_nil());
        assert_eq!(
            g.shared_topology.views[0]
                .as_ref()
                .unwrap()
                .node_to_tree
                .get(Some(NodeId(i as u32))),
            Some(Some(tree))
        );
    }
    assert_eq!(
        g.serialized_cluster_edge_abductions[&99][0].edge,
        Some(EdgeId(0))
    );
    assert_eq!(
        g.serialized_cluster_edge_abductions[&99][1].edge,
        Some(EdgeId(1))
    );
}

#[test]
fn decoded_hierarchies_share_maps_and_snapshot_retains_later_overlapping_records() {
    let mut input = basic();
    input["hierarchies"] = json!([{"Levels":{"1":-7,"2":99}},{"Levels":{"3":17}}]);
    let mut g = decoded(input);
    let a = g.owner_node_hierarchy(NodeId(0)).unwrap().unwrap();
    let b = g.owner_node_hierarchy(NodeId(2)).unwrap().unwrap();
    assert!(!a.same_allocation(&b));
    assert!(a.same_allocation(&g.owner_node_hierarchy(NodeId(1)).unwrap().unwrap()));
    let am = g.owner_hierarchy_levels(&a).unwrap();
    assert_eq!(
        am.values
            .as_ref()
            .unwrap()
            .read()
            .unwrap()
            .get(&Some(NodeId(1))),
        Some(&99)
    );
    assert_eq!(g.owner_hierarchy_fields(&a).unwrap().level_count, 0);
    // Preflight rejects overlapping decoded hierarchies. A later pointer/map
    // edit can leave a retained key in an older allocation; snapshots preserve it.
    let bm = g.owner_hierarchy_levels(&b).unwrap();
    g.write_owner_hierarchy_level(&bm, Some(NodeId(1)), 8)
        .unwrap();
    g.set_owner_node_hierarchy(NodeId(1), Some(b.clone()))
        .unwrap();
    assert_eq!(g.owner_hierarchy_level(NodeId(1)).unwrap(), 8);
    let snapshot = g.owner_topology_snapshot();
    g.write_owner_hierarchy_level(&am, Some(NodeId(1)), 100)
        .unwrap();
    g.set_owner_node_hierarchy(NodeId(1), Some(a.clone()))
        .unwrap();
    g.restore_owner_topology_snapshot(snapshot);
    assert!(
        g.owner_node_hierarchy(NodeId(1))
            .unwrap()
            .unwrap()
            .same_allocation(&b)
    );
    assert_eq!(
        am.values
            .as_ref()
            .unwrap()
            .read()
            .unwrap()
            .get(&Some(NodeId(1))),
        Some(&99)
    );
}

#[test]
fn regrouping_keeps_record_identity_and_obeys_map_replacement_policy() {
    let mut input = basic();
    input["clusters"] = json!({"90":{"Nodes":[1,2]}});
    input["clusterVessels"] = json!({"90":{"id":90}});
    input["sequences"] = json!({"99":{"Nodes":[2,3]}});
    input["sequenceVessels"] = json!({"99":{"id":99}});
    let mut g = decoded(input);
    for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
        let retained = g.owner_graph_group_map(OwnerGraphId(0), kind).unwrap();
        let key = g.owner_group_map_keys(&retained).unwrap()[0];
        let group = g
            .owner_group_map_entry(&retained, key)
            .unwrap()
            .unwrap()
            .unwrap();
        g.begin_owner_group_discovery(kind);
        let current = g.owner_graph_group_map(OwnerGraphId(0), kind).unwrap();
        assert_eq!(current.len(), 0);
        assert_eq!(
            retained.same_map_allocation(&current),
            kind == OwnerGroupKind::Cluster
        );
        assert_eq!(
            retained.len(),
            if kind == OwnerGroupKind::Cluster {
                0
            } else {
                1
            }
        );
        assert_eq!(g.owner_group_fields(group).unwrap().vessel, key);
        assert_eq!(g.owner_group_fields(group).unwrap().nodes.len(), 2);
        let fresh = g.new_owner_group(kind);
        assert_ne!(fresh, group);
        let reference = if kind == OwnerGroupKind::Cluster {
            TopologyVessel::Cluster(0)
        } else {
            TopologyVessel::Sequence(0)
        };
        g.bind_owner_group_reference(reference, fresh).unwrap();
        assert_eq!(g.owner_group_binding(reference), Some(fresh));
    }
}

#[test]
fn invalid_remembered_sequence_cleanup_does_not_clear_a_later_group_pointer() {
    let mut input = basic();
    for node in input["nodes"].as_array_mut().unwrap() {
        node["shapeType"] = json!("Step");
    }
    input["sequences"] = json!({"90":{"Nodes":[1,2]},"99":{"Nodes":[2,3]}});
    input["sequenceVessels"] = json!({"90":{"id":90},"99":{"id":99}});
    let mut g = decoded(input);
    let valid = g.owner_group_binding(TopologyVessel::Sequence(1)).unwrap();
    assert_eq!(
        g.owner_remembered_sequence_validity(),
        BTreeMap::from([(0, false), (1, true)])
    );
    assert_eq!(
        g.owner_node_group_id(NodeId(0), OwnerGroupKind::Sequence)
            .unwrap(),
        None
    );
    assert_eq!(
        g.owner_node_group_id(NodeId(1), OwnerGroupKind::Sequence)
            .unwrap(),
        Some(valid)
    );
    assert_eq!(
        g.owner_node_group_id(NodeId(2), OwnerGroupKind::Sequence)
            .unwrap(),
        Some(valid)
    );
}
