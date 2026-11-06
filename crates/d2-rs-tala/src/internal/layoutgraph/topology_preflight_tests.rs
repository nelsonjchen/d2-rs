// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;

struct Background;
impl limits::Context for Background {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<limits::ContextError> {
        None
    }
}

fn fixture() -> ArenaGraph {
    let mut input = Graph::default();
    for name in ["a", "b"] {
        input.add_node(crate::Node::new(
            name,
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
    }
    input.add_edge(crate::Edge {
        source: NodeId(0),
        target: NodeId(1),
    });
    let mut graph = ArenaGraph::from_input(&input);
    graph.initialize_shared_topology();
    graph.containers.share_map().clear();
    graph.shared_topology.views[0]
        .as_mut()
        .unwrap()
        .nodes
        .replace(SharedNodeSlice::from_nodes(&[NodeId(0), NodeId(1)]).unwrap());
    let edges = graph
        .allocate_owner_incident_header(&[Some(EdgeId(0))], 1, true)
        .unwrap();
    graph
        .set_owner_graph_edge_header(OwnerGraphId(0), edges.clone())
        .unwrap();
    for id in [NodeId(0), NodeId(1)] {
        graph
            .set_owner_node_incident_header(id, edges.clone())
            .unwrap();
    }
    graph
}

fn validate(graph: &ArenaGraph, limit: i64) -> (Result<(), Error>, i64) {
    let mut guard = WorkGuard::new(
        Some(&Background),
        "SimplifyEdgeRoutes",
        limits::MAX_ENGINE_WORK_UNITS,
    )
    .unwrap();
    guard.set_limit(limit);
    let result = graph.validate_owner_graph_topology_with_guard(OwnerGraphId(0), &mut guard);
    (result, guard.used())
}

#[test]
fn preflight_charges_duplicate_references_and_reserves_real_capacity() {
    let mut graph = fixture();
    assert_eq!(validate(&graph, 100).1, 13);
    graph.shared_topology.views[0]
        .as_mut()
        .unwrap()
        .nodes
        .replace(SharedNodeSlice::from_nodes(&[NodeId(0), NodeId(0), NodeId(1)]).unwrap());
    assert_eq!(validate(&graph, 100).1, 14);
    graph.shared_topology.views[0]
        .as_mut()
        .unwrap()
        .nodes
        .replace(SharedNodeSlice::with_len_cap(0, 1_000_001).unwrap());
    let (error, used) = validate(&graph, 100);
    assert_eq!(used, 0);
    assert_eq!(
        error.unwrap_err().to_string(),
        "TALA engine topology references exceed limit 1000000 while reserving graph nodes capacity"
    );
}

#[test]
fn preflight_discovers_hidden_incident_edges_and_rejects_nil_before_point_charge() {
    let mut graph = fixture();
    let empty = graph.allocate_owner_incident_header(&[], 0, true).unwrap();
    graph
        .set_owner_graph_edge_header(OwnerGraphId(0), empty)
        .unwrap();
    graph.edge_records[0]
        .route
        .replace(super::super::super::route_points::RoutePoints::pointer_literal(&[None]));
    let (error, used) = validate(&graph, 6);
    assert_eq!(used, 6);
    assert_eq!(
        error.unwrap_err().to_string(),
        "TALA engine topology contains nil edge route point"
    );
    let (error, used) = validate(&graph, 5);
    assert_eq!(used, 6);
    assert_eq!(
        error.unwrap_err().to_string(),
        "TALA SimplifyEdgeRoutes work exceeds limit 5"
    );
}

#[test]
fn preflight_rejects_descendant_cycle_and_inconsistent_tree_backlink() {
    let mut graph = fixture();
    graph.nodes[0].is_container = true;
    graph.containers.replace_header(
        Some(NodeId(0)),
        SharedNodeSlice::from_nodes(&[NodeId(0)]).unwrap(),
    );
    assert!(
        validate(&graph, 100)
            .0
            .unwrap_err()
            .to_string()
            .contains("descendant cycle")
    );
    graph.containers.share_map().clear();
    let parent = graph.new_owner_tree(Some(NodeId(0))).unwrap();
    let child = graph.new_owner_tree(Some(NodeId(1))).unwrap();
    let mut fields = graph.owner_tree(parent).unwrap().fields();
    fields.children = OwnerTreeSlice::from_slots(&[Some(child)], 1).unwrap();
    graph.set_owner_tree_fields(parent, fields).unwrap();
    graph.shared_topology.views[0]
        .as_ref()
        .unwrap()
        .tree_roots
        .insert(
            None,
            OwnerTreeSlice::from_slots(&[Some(parent)], 1).unwrap(),
        )
        .unwrap();
    assert_eq!(
        validate(&graph, 100).0.unwrap_err().to_string(),
        "TALA engine tree child has an inconsistent parent"
    );
    let mut fields = graph.owner_tree(child).unwrap().fields();
    fields.parent = Some(parent);
    graph.set_owner_tree_fields(child, fields).unwrap();
    assert!(validate(&graph, 100).0.is_ok());
    graph.shared_topology.views[0]
        .as_mut()
        .unwrap()
        .node_to_tree = SharedOwnerTreeMap::allocated();
    graph.shared_topology.views[0]
        .as_ref()
        .unwrap()
        .node_to_tree
        .insert(Some(NodeId(0)), Some(parent))
        .unwrap();
    assert_eq!(
        validate(&graph, 100).0.unwrap_err().to_string(),
        "TALA engine node-to-tree aliases do not cover the installed forest"
    );
}

#[test]
fn hierarchy_level_maps_have_independent_nil_and_record_identity() {
    let mut graph = fixture();
    let hierarchy = graph.new_owner_hierarchy();
    let original = graph.owner_hierarchy_levels(&hierarchy).unwrap();
    graph
        .set_owner_node_hierarchy(NodeId(0), Some(hierarchy.clone()))
        .unwrap();
    graph
        .set_owner_node_hierarchy(NodeId(1), Some(hierarchy.clone()))
        .unwrap();
    graph
        .write_owner_hierarchy_level(&original, Some(NodeId(0)), -4)
        .unwrap();
    assert_eq!(graph.owner_hierarchy_level(NodeId(0)), Ok(-4));
    assert_eq!(graph.owner_hierarchy_level(NodeId(1)), Ok(0));
    let nil = graph.allocate_owner_hierarchy_map(false);
    graph
        .set_owner_hierarchy_fields(
            &hierarchy,
            OwnerHierarchyFields {
                levels: nil,
                level_count: -9,
            },
        )
        .unwrap();
    assert_eq!(graph.owner_hierarchy_level(NodeId(0)), Ok(0));
    let allocated = graph.owner_hierarchy_levels(&hierarchy).unwrap();
    assert!(!allocated.is_nil());
    assert!(!allocated.same_allocation(&original));
    assert_eq!(original.entries(), vec![(Some(NodeId(0)), -4)]);
    graph
        .write_owner_hierarchy_level(&allocated, None, i64::MIN)
        .unwrap();
    assert_eq!(
        validate(&graph, 100).0.unwrap_err().to_string(),
        "TALA engine topology contains nil hierarchy level node"
    );
    graph
        .delete_owner_hierarchy_level(&allocated, None)
        .unwrap();
    assert!(validate(&graph, 100).0.is_ok());
}

#[test]
fn numerical_clone_detaches_hierarchy_records_and_preserves_cross_record_map_aliases() {
    let mut graph = fixture();
    let first = graph.new_owner_hierarchy();
    let second = graph.new_owner_hierarchy();
    let fields = graph.owner_hierarchy_fields(&first).unwrap();
    graph
        .write_owner_hierarchy_level(&fields.levels, Some(NodeId(0)), 7)
        .unwrap();
    graph
        .set_owner_hierarchy_fields(&second, fields.clone())
        .unwrap();
    graph
        .set_owner_node_hierarchy(NodeId(0), Some(first.clone()))
        .unwrap();
    graph
        .set_owner_node_hierarchy(NodeId(1), Some(second))
        .unwrap();
    let cloned = graph.clone();
    let cloned_first = cloned.owner_node_hierarchy(NodeId(0)).unwrap().unwrap();
    let cloned_second = cloned.owner_node_hierarchy(NodeId(1)).unwrap().unwrap();
    assert!(!cloned_first.same_allocation(&first));
    let a = cloned.owner_hierarchy_fields(&cloned_first).unwrap().levels;
    let b = cloned
        .owner_hierarchy_fields(&cloned_second)
        .unwrap()
        .levels;
    assert!(a.same_allocation(&b));
    assert!(!a.same_allocation(&fields.levels));
    cloned
        .write_owner_hierarchy_level(&a, Some(NodeId(0)), 99)
        .unwrap();
    assert_eq!(graph.owner_hierarchy_level(NodeId(0)), Ok(7));
    assert_eq!(cloned.owner_hierarchy_level(NodeId(0)), Ok(99));
    assert!(matches!(
        cloned.owner_hierarchy_fields(&first),
        Err(OwnerTopologyError::ForeignHierarchyScope)
    ));
}

#[test]
fn hierarchy_topology_rollback_repairs_retained_maps_and_node_pointers() {
    let mut graph = fixture();
    let hierarchy = graph.new_owner_hierarchy();
    let mut fields = graph.owner_hierarchy_fields(&hierarchy).unwrap();
    fields.level_count = 3;
    let retained = fields.levels.clone();
    graph
        .write_owner_hierarchy_level(&retained, Some(NodeId(0)), -7)
        .unwrap();
    graph
        .set_owner_hierarchy_fields(&hierarchy, fields)
        .unwrap();
    graph
        .set_owner_node_hierarchy(NodeId(0), Some(hierarchy.clone()))
        .unwrap();
    let snapshot = graph.owner_topology_snapshot();
    graph
        .write_owner_hierarchy_level(&retained, None, 99)
        .unwrap();
    graph
        .delete_owner_hierarchy_level(&retained, Some(NodeId(0)))
        .unwrap();
    let replacement = graph.allocate_owner_hierarchy_map(true);
    graph
        .write_owner_hierarchy_level(&replacement, Some(NodeId(1)), 20)
        .unwrap();
    graph
        .set_owner_hierarchy_fields(
            &hierarchy,
            OwnerHierarchyFields {
                levels: replacement.clone(),
                level_count: 99,
            },
        )
        .unwrap();
    graph.set_owner_node_hierarchy(NodeId(0), None).unwrap();
    graph.restore_owner_topology_snapshot(snapshot);
    assert!(
        graph
            .owner_node_hierarchy(NodeId(0))
            .unwrap()
            .unwrap()
            .same_allocation(&hierarchy)
    );
    let fields = graph.owner_hierarchy_fields(&hierarchy).unwrap();
    assert_eq!(fields.level_count, 3);
    assert!(fields.levels.same_allocation(&retained));
    assert_eq!(retained.entries(), vec![(Some(NodeId(0)), -7)]);
    assert_eq!(replacement.entries(), vec![(Some(NodeId(1)), 20)]);
}

#[test]
fn topology_rollback_repairs_retained_hub_headers_and_requirement_keys() {
    let mut graph = fixture();
    let map = SharedSliceMap::new();
    let header = SharedNodeSlice::with_len_cap(1, 3).unwrap();
    header.write_capacity_slot(0, Some(NodeId(0))).unwrap();
    header.write_capacity_slot(2, Some(NodeId(1))).unwrap();
    map.replace(Some(NodeId(0)), header.clone()).unwrap();
    let view = graph.shared_topology.views[0].as_mut().unwrap();
    view.hubs.replace_header(map.clone());
    view.common_uncle_siblings.replace_header(map.clone());
    let requirements =
        super::node_requirements_map::NodeRequirementsMap::from_raw_keys([Some(NodeId(1))]);
    graph
        .shared_topology
        .long_distance_requirements
        .insert(NodeId(0), requirements.retained());
    let snapshot = graph.owner_topology_snapshot();
    header.write_capacity_slot(2, None).unwrap();
    map.clear();
    requirements
        .set(
            None,
            super::model::LongDistanceNeighborRequirements::default(),
        )
        .unwrap();
    graph.restore_owner_topology_snapshot(snapshot);
    assert_eq!(header.read_capacity_slot(2).unwrap(), Some(NodeId(1)));
    assert_eq!(map.entries().len(), 1);
    assert_eq!(requirements.raw_keys(), vec![Some(NodeId(1))]);
}

#[test]
fn nullable_abduction_edge_is_rejected_by_preflight_and_dereferenced_only_if_used() {
    let mut graph = fixture();
    let record = OwnerEdgeAbduction {
        source_edge: None,
        originally_from: Some(NodeId(0)),
        originally_to: None,
        current_from: None,
        current_to: None,
    };
    let records = graph
        .register_owner_edge_abductions(vec![record], true)
        .unwrap();
    graph.apply_owner_edge_abductions(&records);
    let group = graph.new_owner_group(OwnerGroupKind::Cluster);
    graph
        .owner_group(group)
        .unwrap()
        .set_fields(OwnerGroupFields {
            vessel: Some(NodeId(0)),
            nodes: SharedNodeSlice::from_nodes(&[NodeId(1)]).unwrap(),
            graph: Some(OwnerGraphId(0)),
            container: None,
        });
    let header = graph
        .allocate_owner_abduction_header(&[Some(records.records[0])], 1, true)
        .unwrap();
    let mut policy = graph.owner_group(group).unwrap().policy();
    policy.edge_abductions = Some(header);
    graph.owner_group(group).unwrap().set_policy(policy);
    graph
        .set_owner_node_group(NodeId(1), OwnerGroupKind::Cluster, Some(group))
        .unwrap();
    assert_eq!(
        validate(&graph, 100).0.unwrap_err().to_string(),
        "TALA engine topology contains nil abducted edge"
    );
    graph
        .set_owner_edge_abduction(
            records.records[0],
            OwnerEdgeAbduction {
                current_from: Some(NodeId(1)),
                ..record
            },
        )
        .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        graph.apply_owner_edge_abductions(&records)
    }))
    .unwrap_err();
    assert_eq!(
        panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied()),
        Some("runtime error: invalid memory address or nil pointer dereference")
    );
}

#[test]
fn production_hierarchy_members_read_live_record_levels_and_count() {
    let mut graph = fixture();
    graph.assign_hierarchy_member(NodeId(0), 0, None, 2, 4);
    graph.assign_hierarchy_member(NodeId(1), 0, None, 3, 4);
    let record = graph.owner_node_hierarchy(NodeId(0)).unwrap().unwrap();
    assert!(record.same_allocation(&graph.owner_node_hierarchy(NodeId(1)).unwrap().unwrap()));
    let mut fields = graph.owner_hierarchy_fields(&record).unwrap();
    graph
        .write_owner_hierarchy_level(&fields.levels, Some(NodeId(0)), i64::MIN)
        .unwrap();
    fields.level_count = i64::MAX;
    graph.set_owner_hierarchy_fields(&record, fields).unwrap();
    let member = graph.nodes[0].hierarchy.as_ref().unwrap();
    assert_eq!(member.level(), i64::MIN);
    assert_eq!(member.level_count(), i64::MAX);
    assert!(validate(&graph, 100).0.is_ok());
}

#[test]
fn production_hierarchy_cleanup_repairs_retained_map_and_uses_live_incidents() {
    let mut graph = fixture();
    graph.assign_hierarchy_member(NodeId(0), 0, None, 0, 2);
    graph.assign_hierarchy_member(NodeId(1), 0, None, 1, 2);
    let record = graph.owner_node_hierarchy(NodeId(0)).unwrap().unwrap();
    let retained = graph.owner_hierarchy_levels(&record).unwrap();
    let empty = graph.allocate_owner_incident_header(&[], 0, true).unwrap();
    graph
        .set_owner_node_incident_header(NodeId(0), empty)
        .unwrap();
    // The ordinary field and raw API now expose the same empty header.
    // Cleanup must repair the exact hierarchy map retained by this observer.
    assert!(graph.nodes[0].edges.is_empty());
    graph.remove_isolated_hierarchy_memberships();
    assert!(graph.owner_node_hierarchy(NodeId(0)).unwrap().is_none());
    assert_eq!(retained.entries(), vec![(Some(NodeId(1)), 1)]);
    assert!(record.same_allocation(&graph.owner_node_hierarchy(NodeId(1)).unwrap().unwrap()));
}

#[test]
fn forced_assignment_publishes_records_and_recompute_clears_retained_levels() {
    let mut input = Graph {
        root_hierarchy: true,
        ..Graph::default()
    };
    for name in ["a", "b"] {
        input.add_node(crate::Node::new(
            name,
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
    }
    input.add_edge(crate::Edge {
        source: NodeId(0),
        target: NodeId(1),
    });
    let mut graph = ArenaGraph::from_input(&input);
    let original = graph.owner_node_hierarchy(NodeId(0)).unwrap().unwrap();
    let retained = graph.owner_hierarchy_levels(&original).unwrap();
    assert_eq!(retained.entries().len(), 2);
    graph.assign_forced_hierarchies();
    let replacement = graph.owner_node_hierarchy(NodeId(0)).unwrap().unwrap();
    assert!(!replacement.same_allocation(&original));
    assert!(retained.entries().is_empty());
    assert_eq!(
        graph
            .owner_hierarchy_levels(&replacement)
            .unwrap()
            .entries()
            .len(),
        2
    );
}

#[test]
fn recompute_clears_owned_keys_even_after_node_changes_record() {
    let mut graph = fixture();
    let first = graph.new_owner_hierarchy();
    let second = graph.new_owner_hierarchy();
    let a = graph.owner_hierarchy_levels(&first).unwrap();
    let b = graph.owner_hierarchy_levels(&second).unwrap();
    for map in [&a, &b] {
        for node in [NodeId(0), NodeId(1)] {
            graph
                .write_owner_hierarchy_level(map, Some(node), 9)
                .unwrap();
        }
    }
    graph
        .set_owner_node_hierarchy(NodeId(0), Some(first))
        .unwrap();
    graph
        .set_owner_node_hierarchy(NodeId(1), Some(second))
        .unwrap();
    graph.assign_forced_hierarchies();
    assert!(a.entries().is_empty());
    assert!(b.entries().is_empty());
    assert!(graph.nodes.iter().all(|node| node.hierarchy.is_none()));
}
