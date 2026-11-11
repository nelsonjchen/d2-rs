// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use crate::{
    ContentAlignment, Direction, Edge, EdgeArrows, ExternalAlignment, ExternalLabel, ExternalSide,
    Insets, LabelPosition, Node, Size,
};

fn node(name: &str, width: f64, height: f64) -> Node {
    Node {
        d2_near_key: None,
        external_id: name.into(),
        size: Size { width, height },
        declared_size: None,
        d2_authored_geometry: None,
        label_size: None,
        font_size: None,
        label_position: LabelPosition::Unset,
        parent: None,
        locked_position: None,
        constrained_x: None,
        constrained_y: None,
        near: None,
        fixed_width: false,
        fixed_height: false,
        direction: None,
        force_hierarchy: false,
        grid_rows: None,
        grid_columns: None,
        canvas_position: None,
        content_insets: Insets::uniform(60.0),
        layout_margins: Insets::uniform(0.0),
        external_label: None,
        icon_position: None,
        has_icon: false,
        label_aware_grid: false,
        packed_grid: false,
        content_alignment: ContentAlignment::Padding,
        port_spread: 0.0,
        person: false,
        is_3d: false,
        is_multiple: false,
        shape: ShapeKind::Rectangle,
    }
}

#[test]
fn root_singleton_multiple_shape_skips_combine_subgraphs() {
    let mut input = Graph::default();
    let mut person = node("person", 44.0, 66.0);
    person.shape = ShapeKind::Person;
    person.person = true;
    person.is_multiple = true;
    person.label_size = Some(Size {
        width: 19.0,
        height: 21.0,
    });
    person.label_position = LabelPosition::OutsideBottomCenter;
    person.external_label = Some(ExternalLabel {
        size: Size {
            width: 19.0,
            height: 21.0,
        },
        side: ExternalSide::Bottom,
        alignment: ExternalAlignment::Center,
        automatic: false,
        reserve_space: true,
    });
    let person = input.add_node(person);

    let snapshot = layout_snapshot(&input, 1, LayoutStage::NodePlacement);

    assert_eq!(
        snapshot.nodes[person.0 as usize].position,
        Some(Point::default())
    );
}

#[test]
fn connected_views_preserve_ordinary_exclusion_traversal() {
    // Mirror upstream connectedMembershipFixture and its exclusion matrix.
    for count in [4, 12, 128] {
        let mut input = Graph::default();
        let root = input.add_node(node("root", 100.0, 100.0));
        let mut children = Vec::new();
        for i in 0..count {
            let mut child = node(&format!("root.child{i}"), 10.0, 10.0);
            child.parent = Some(root);
            let child = input.add_node(child);
            if let Some(&previous) = children.last() {
                input.add_edge(Edge {
                    source: previous,
                    target: child,
                });
            }
            children.push(child);
        }
        let graph = ArenaGraph::from_input(&input);
        for excluded in [
            vec![],
            vec![root],
            vec![children[0]],
            vec![children[count - 1]],
            vec![children[1], children[2]],
        ] {
            let views = graph.connected_node_views(
                ConnectedNode::Active(children[0]),
                &excluded
                    .iter()
                    .copied()
                    .map(ConnectedNode::Active)
                    .collect::<Vec<_>>(),
            );
            let ordinary = graph.connected_nodes_excluding_list(children[0], &excluded);
            assert_eq!(
                views,
                ordinary
                    .into_iter()
                    .map(ConnectedNode::Active)
                    .collect::<Vec<_>>(),
                "count={count}, excluded={excluded:?}"
            );
        }
    }
}

#[test]
fn connected_views_follow_retained_source_order_when_it_covers_active_children() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 100.0, 100.0));
    let mut children = Vec::new();
    for name in ["first", "second", "third", "fourth"] {
        let mut child = node(name, 10.0, 10.0);
        child.parent = Some(parent);
        children.push(input.add_node(child));
    }
    input.set_hierarchy_children_order(BTreeMap::from([(Some(parent), children.clone())]));

    let graph = ArenaGraph::from_input(&input);
    graph.containers.insert(
        Some(parent),
        vec![children[0], children[2], children[3], children[1]],
    );

    let reached = graph.connected_node_views(ConnectedNode::Active(parent), &[]);
    let expected = std::iter::once(ConnectedNode::Active(parent))
        .chain(children.into_iter().map(ConnectedNode::Active))
        .collect::<Vec<_>>();
    assert_eq!(reached, expected);
}

#[test]
fn node_placement_refreshes_connected_order_from_restored_owner_children() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 100.0, 100.0));
    let mut children = Vec::new();
    for name in ["gen", "adapter", "pathom"] {
        let mut child = node(name, 10.0, 10.0);
        child.parent = Some(parent);
        children.push(input.add_node(child));
    }
    input.set_hierarchy_children_order(BTreeMap::from([(Some(parent), children.clone())]));

    let mut pipeline = Pipeline::new(&input, 1, false, false);
    let live_children = vec![children[2], children[0], children[1]];
    pipeline
        .graph
        .containers
        .insert(Some(parent), live_children.clone());
    let parent_tala_id = pipeline.graph.nodes[parent.0 as usize].tala_id;
    let live_order = live_children
        .iter()
        .map(|child| pipeline.graph.nodes[child.0 as usize].tala_id)
        .collect::<Vec<_>>();

    pipeline.refresh_connected_container_child_order_from_live_slices();

    assert_eq!(
        pipeline.graph.connected_container_child_order[&parent_tala_id],
        live_order
    );
    let reached = pipeline
        .graph
        .connected_node_views(ConnectedNode::Active(parent), &[]);
    let expected = std::iter::once(ConnectedNode::Active(parent))
        .chain(live_children.into_iter().map(ConnectedNode::Active))
        .collect::<Vec<_>>();
    assert_eq!(reached, expected);
}

#[test]
fn connected_views_ignore_stale_partial_source_order() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 100.0, 100.0));
    let mut children = Vec::new();
    for name in ["first", "second", "third", "fourth"] {
        let mut child = node(name, 10.0, 10.0);
        child.parent = Some(parent);
        children.push(input.add_node(child));
    }
    input.set_hierarchy_children_order(BTreeMap::from([(Some(parent), children.clone())]));

    let mut graph = ArenaGraph::from_input(&input);
    graph.containers.insert(
        Some(parent),
        vec![children[0], children[2], children[3], children[1]],
    );
    let parent_id = graph.nodes[parent.0 as usize].tala_id;
    let child_id = |child: NodeId| graph.nodes[child.0 as usize].tala_id;
    graph.connected_container_child_order.insert(
        parent_id,
        vec![
            i64::MAX,
            child_id(children[1]),
            i64::MIN,
            child_id(children[0]),
            child_id(children[2]),
        ],
    );

    let reached = graph.connected_node_views(ConnectedNode::Active(parent), &[]);
    let expected = std::iter::once(ConnectedNode::Active(parent))
        .chain(
            [children[0], children[2], children[3], children[1]]
                .into_iter()
                .map(ConnectedNode::Active),
        )
        .collect::<Vec<_>>();
    assert_eq!(reached, expected);
}

#[test]
fn connected_views_restore_extracted_children_from_source_order() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 100.0, 100.0));
    let mut children = Vec::new();
    for name in ["first", "second", "third", "fourth"] {
        let mut child = node(name, 10.0, 10.0);
        child.parent = Some(parent);
        children.push(input.add_node(child));
    }
    input.set_hierarchy_children_order(BTreeMap::from([(Some(parent), children.clone())]));

    let graph = ArenaGraph::from_input(&input);
    graph
        .containers
        .insert(Some(parent), vec![children[0], children[2], children[3]]);

    let reached = graph.connected_node_views(ConnectedNode::Active(parent), &[]);
    let expected = std::iter::once(ConnectedNode::Active(parent))
        .chain(children.into_iter().map(ConnectedNode::Active))
        .collect::<Vec<_>>();
    assert_eq!(reached, expected);
}

#[test]
fn connected_child_order_survives_induced_arena_reindexing() {
    let mut input = Graph::default();
    let unrelated = input.add_node(node("unrelated", 10.0, 10.0));
    let parent = input.add_node(node("parent", 100.0, 100.0));
    let mut children = Vec::new();
    for name in ["first", "second", "third", "fourth"] {
        let mut child = node(name, 10.0, 10.0);
        child.parent = Some(parent);
        children.push(input.add_node(child));
    }
    input.set_hierarchy_children_order(BTreeMap::from([(Some(parent), children.clone())]));

    let graph = ArenaGraph::from_input(&input);
    // Put the parent at a different dense index in the induced arena. The
    // connectivity snapshot is carried by TALA identity, not that index.
    let members = vec![
        children[2],
        unrelated,
        children[0],
        parent,
        children[3],
        children[1],
    ];
    let (induced, old_by_new) = graph.induced_placement_subgraph(&members);
    let local = |old: NodeId| {
        NodeId(
            old_by_new
                .iter()
                .position(|candidate| *candidate == old)
                .expect("induced node") as u32,
        )
    };
    let local_parent = local(parent);
    let local_children = children.iter().copied().map(local).collect::<Vec<_>>();
    induced.containers.insert(
        Some(local_parent),
        vec![
            local_children[0],
            local_children[2],
            local_children[3],
            local_children[1],
        ],
    );

    let reached = induced.connected_node_views(ConnectedNode::Active(local_parent), &[]);
    let expected = std::iter::once(ConnectedNode::Active(local_parent))
        .chain(local_children.into_iter().map(ConnectedNode::Active))
        .collect::<Vec<_>>();
    assert_eq!(reached, expected);
}

#[test]
fn connected_traversal_retains_cluster_container_pointer_views() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let mut a = node("first.child", 20.0, 20.0);
    a.parent = Some(first);
    let a = input.add_node(a);
    let mut b = node("second.child", 20.0, 20.0);
    b.parent = Some(second);
    let b = input.add_node(b);
    let mut graph = ArenaGraph::from_input(&input);
    graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    graph.nodes[first.0 as usize].cluster = Some(0);
    graph.nodes[second.0 as usize].cluster = Some(0);
    graph.set_position(first, Point { x: 100.0, y: 40.0 });
    graph.set_position(second, Point { x: 220.0, y: 40.0 });
    graph
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 80.0, y: 20.0 });
    graph.rebuild_active_aggregate_node_order();
    use ConnectedNode::{Active, ClusterMember};
    let reached = graph.connected_node_views(Active(first), &[]);
    assert_eq!(graph.connected_node_views(Active(second), &[]), reached);
    assert_eq!(
        reached,
        vec![
            Active(first),
            Active(a),
            Active(b),
            ClusterMember(first),
            ClusterMember(second)
        ]
    );
    assert_ne!(
        graph.connected_node_id(Active(first)),
        graph.connected_node_id(ClusterMember(first))
    );
    assert_eq!(
        graph.connected_node_box(Active(first)).unwrap().0,
        Point { x: 80.0, y: 20.0 }
    );
    assert_eq!(
        graph.connected_node_box(ClusterMember(first)).unwrap().0,
        Point { x: 100.0, y: 40.0 }
    );
    assert!(graph.connected_is_descendant(Active(a), ClusterMember(first)));
    assert!(graph.connected_is_descendant(Active(a), Active(first)));
    let projected = graph.projected_transaction_state();
    let child = projected
        .nodes
        .iter()
        .find(|entry| entry.node.tala_id == graph.nodes[a.0 as usize].tala_id)
        .unwrap();
    assert_eq!(
        child.structural_ancestor_tala_ids,
        vec![graph.nodes[first.0 as usize].tala_id, 10_001]
    );
    let child = projected
        .nodes
        .iter()
        .find(|entry| entry.node.tala_id == graph.nodes[b.0 as usize].tala_id)
        .unwrap();
    assert_eq!(
        child.structural_ancestor_tala_ids,
        vec![graph.nodes[second.0 as usize].tala_id, 10_001]
    );
    assert!(!graph.connected_is_descendant(Active(first), ClusterMember(first)));
    assert!(
        graph
            .connected_node_views(Active(a), &[Active(first)])
            .is_empty()
    );
    assert!(
        graph
            .connected_node_views(Active(a), &[ClusterMember(first)])
            .is_empty()
    );
    let blocked = graph.connected_node_views(Active(first), &[Active(a)]);
    assert!(!blocked.contains(&Active(a)));
    assert!(!blocked.contains(&ClusterMember(first)));

    graph.set_position(a, Point { x: 110.0, y: 50.0 });
    // A child entering its vessel after an arrangement change is not a new
    // unrelated-node overlap, even if it was outside the vessel at snapshot.
    graph.set_position(b, Point { x: 400.0, y: 50.0 });
    let before_arrangement = graph.projected_transaction_state();
    graph.set_position(b, Point { x: 240.0, y: 50.0 });
    assert!(graph.projected_transaction_nodes_are_valid(&before_arrangement));
    graph.translate_connected_node_boxes(
        &[Active(first), ClusterMember(first)],
        Point { x: 10.0, y: 20.0 },
    );
    assert_eq!(
        graph.connected_node_box(Active(first)).unwrap().0,
        Point { x: 90.0, y: 40.0 }
    );
    assert_eq!(graph.position(first), Some(Point { x: 110.0, y: 60.0 }));
    assert_eq!(graph.position(second), Some(Point { x: 220.0, y: 40.0 }));
    assert_eq!(graph.position(a), Some(Point { x: 110.0, y: 50.0 }));
}

#[test]
fn retained_member_spacing_does_not_inherit_vessel_edges() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 100.0, 100.0));
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    input.add_edge(Edge {
        source,
        target: first,
    });
    let mut graph = ArenaGraph::from_input(&input);
    graph.set_position(source, Point { x: 0.0, y: 0.0 });
    graph.set_position(first, Point { x: 1000.0, y: 0.0 });
    graph.set_position(second, Point { x: 2000.0, y: 0.0 });
    graph.refresh_node_order_membership();
    // node.go:799-817 reads each current incident edge, including its live
    // endpoints and dimensions. Both numerical owner-table paths must agree
    // after either kind of edge mutation, in both adjacent branches.
    for reversed in [false, true] {
        let edge = &mut graph.edge_records[0];
        (edge.from, edge.to) = if reversed {
            (first, source)
        } else {
            (source, first)
        };
        for width in [90.0, 140.0] {
            graph.edge_records[0].min_width = width;
            for valid in [true, false] {
                graph.node_order_membership_valid = valid;
                for (a, b, expected) in [
                    (source, first, width),
                    (first, source, width),
                    (source, second, 20.0),
                ] {
                    assert_eq!(
                        graph.spacing_delta(a, b, graph.position(a).unwrap()),
                        expected,
                        "reversed={reversed}, table={valid}, {a:?}->{b:?}"
                    );
                }
            }
        }
    }
    graph.edge_records[0].from = source;
    graph.edge_records[0].to = first;
    graph.edge_records[0].min_width = 0.0;
    graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    graph.nodes[first.0 as usize].cluster = Some(0);
    graph.nodes[second.0 as usize].cluster = Some(0);
    graph.set_position(source, Point { x: 0.0, y: 0.0 });
    graph.set_position(first, Point { x: 1000.0, y: 0.0 });
    graph.set_position(second, Point { x: 1120.0, y: 0.0 });
    graph
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 980.0, y: -20.0 });
    graph.rebuild_active_aggregate_node_order();
    use ConnectedNode::{Active, ClusterMember};
    assert_eq!(
        graph.connected_spacing_delta(Active(source), Active(first)),
        60.0
    );
    assert_eq!(
        graph.connected_spacing_delta(Active(source), ClusterMember(first)),
        20.0
    );

    // node.go deltaToGuarded consumes the current node's LoopOffsets. An
    // active vessel has no member-loop offsets, while the ordinary endpoint
    // retains its own offsets. Exercise nil, empty and nonempty maps.
    for offsets in [
        None,
        Some(LoopOffsetsMap::new()),
        Some(LoopOffsetsMap::from([(7, 700.0)])),
    ] {
        graph.nodes[first.0 as usize].loop_offsets = offsets;
        assert_eq!(
            graph.spacing_delta_with_loops(source, first, graph.position(source).unwrap()),
            60.0
        );
        assert_eq!(
            graph.spacing_delta_with_loops(
                first,
                source,
                graph.active_node_position(first).unwrap()
            ),
            60.0
        );
    }
    graph.nodes[source.0 as usize].loop_offsets = Some(LoopOffsetsMap::from([(5, 150.0)]));
    assert_eq!(
        graph.spacing_delta_with_loops(source, first, graph.position(source).unwrap()),
        170.0
    );
    assert_eq!(
        graph.spacing_delta_with_loops(first, source, graph.active_node_position(first).unwrap()),
        170.0
    );
}

#[test]
fn herd_penalty_uses_active_cluster_box() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 60.0));
    let second = input.add_node(node("second", 100.0, 60.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    arena.set_position(first, Point { x: 100.0, y: 700.0 });
    arena.set_position(second, Point { x: 220.0, y: 700.0 });
    arena
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 80.0, y: 680.0 });
    arena.rebuild_active_aggregate_node_order();
    assert_eq!(arena.herd_penalty(first), 0.0);
    let size = arena.active_node_size(first);
    for (orientation, value, expected) in [
        (Orientation::Top, 0.0, 680.0),
        (Orientation::Left, 0.0, 80.0),
        (Orientation::Right, 80.0 + size.width + 25.0, 25.0),
        (Orientation::Bottom, 680.0 + size.height + 25.0, 25.0),
        (Orientation::Top, 700.0, arena.cell_size + 20.0),
    ] {
        arena.nodes[first.0 as usize].herd_assignment = Some(HerdAssignment {
            orientation,
            value,
            same_side_paired: BTreeSet::new(),
            opposite_side_paired: BTreeSet::new(),
        });
        assert_eq!(arena.herd_penalty(first), expected, "{orientation:?}");
    }
}

#[test]
fn container_topology_does_not_mint_hierarchy_metadata() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 100.0, 100.0));
    let mut first_node = node("first", 40.0, 40.0);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 40.0, 40.0);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);
    input.add_edge(Edge {
        source: first,
        target: second,
    });

    let arena = ArenaGraph::from_input(&input);
    assert!(arena.nodes.iter().all(|node| node.hierarchy.is_none()));
    assert!(!arena.first_node_owns_hierarchy());
}

#[test]
fn graphjson_cluster_ownership_is_restored_before_layout() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 80.0, 40.0));
    let second = input.add_node(node("second", 80.0, 40.0));
    let edge = input.add_edge(Edge {
        source: first,
        target: second,
    });
    input.tala_id_overrides = vec![Some(1), Some(2)];
    input.tala_edge_ids[edge.0 as usize] = Some(-8);
    input.serialized_clusters.push(SerializedClusterState {
        vessel_tala_id: 99,
        member_tala_ids: vec![1, 2],
        arrangement: "Row".to_owned(),
        desired_arrangement: "Row".to_owned(),
        edge_abductions: vec![SerializedAggregateEdgeAbduction {
            edge_id: -8,
            originally_from_node: 1,
            originally_to_node: 0,
            current_from_node: 99,
            current_to_node: 2,
        }],
        container_tala_id: 0,
        fixed_size: false,
        padding: 10.0,
        vessel_position: Some(Point { x: 12.0, y: 24.0 }),
        vessel_size: Size {
            width: 180.0,
            height: 100.0,
        },
    });
    input.tala_id_overrides = vec![Some(1), Some(2)];

    let arena = ArenaGraph::from_input(&input);
    // A serialized map-only vessel is detached, so both member pointers stay
    // visible until the normal AddClusters lifecycle activates the group.
    assert_eq!(arena.node_order, vec![first, second]);
    assert!(!arena.cluster_is_active(&arena.clusters[0]));
    assert_eq!(arena.clusters.len(), 1);
    assert_eq!(arena.nodes[first.0 as usize].cluster, Some(0));
    assert_eq!(arena.nodes[second.0 as usize].cluster, Some(0));
    assert_eq!(
        arena.pending_cluster_vessel_positions.get(&0),
        Some(&Point { x: 12.0, y: 24.0 })
    );
    assert_eq!(
        arena.cluster_vessel_size(0),
        Size {
            width: 180.0,
            height: 100.0
        }
    );
    assert_eq!(
        arena.serialized_vessel_sizes_by_tala.get(&99),
        Some(&Size {
            width: 180.0,
            height: 100.0
        })
    );
    assert_eq!(
        arena.serialized_cluster_containers_by_tala.get(&99),
        Some(&None)
    );
    assert_eq!(
        arena.serialized_cluster_edge_abductions[&99][0],
        ResolvedAggregateEdgeAbduction {
            edge_tala_id: -8,
            edge: Some(edge),
            originally_from: Some(AggregateNodeRef::GraphNode {
                node: first,
                tala_id: 1,
            }),
            originally_to: None,
            current_from: Some(AggregateNodeRef::Vessel { tala_id: 99 }),
            current_to: Some(AggregateNodeRef::GraphNode {
                node: second,
                tala_id: 2,
            }),
        }
    );
}

#[test]
fn graphjson_cluster_abductions_keep_their_vessel_identity() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 80.0, 40.0));
    let second = input.add_node(node("second", 80.0, 40.0));
    let external = input.add_node(node("external", 80.0, 40.0));
    let first_edge = input.add_edge(Edge {
        source: first,
        target: external,
    });
    let second_edge = input.add_edge(Edge {
        source: second,
        target: external,
    });
    input.tala_id_overrides = vec![Some(1), Some(2), Some(3)];
    input.tala_edge_ids[first_edge.0 as usize] = Some(-11);
    input.tala_edge_ids[second_edge.0 as usize] = Some(-12);
    let cluster = |vessel_tala_id, member_tala_id, edge_id| SerializedClusterState {
        vessel_tala_id,
        member_tala_ids: vec![1, 2],
        arrangement: "Row".to_owned(),
        desired_arrangement: "Row".to_owned(),
        edge_abductions: vec![SerializedAggregateEdgeAbduction {
            edge_id,
            originally_from_node: member_tala_id,
            originally_to_node: 0,
            current_from_node: vessel_tala_id,
            current_to_node: 3,
        }],
        container_tala_id: 0,
        fixed_size: false,
        padding: 10.0,
        vessel_position: None,
        vessel_size: Size::default(),
    };
    input
        .serialized_clusters
        .extend([cluster(99, 1, -11), cluster(100, 2, -12)]);

    let arena = ArenaGraph::from_input(&input);
    let first_abduction = arena.serialized_cluster_edge_abductions(99).unwrap()[0];
    let second_abduction = arena.serialized_cluster_edge_abductions(100).unwrap()[0];
    assert_eq!(first_abduction.edge, Some(first_edge));
    assert_eq!(
        first_abduction.current_from,
        Some(AggregateNodeRef::Vessel { tala_id: 99 })
    );
    assert_eq!(second_abduction.edge, Some(second_edge));
    assert_eq!(
        second_abduction.current_from,
        Some(AggregateNodeRef::Vessel { tala_id: 100 })
    );

    let cloned = arena.clone();
    assert_eq!(
        cloned.serialized_cluster_edge_abductions(99).unwrap()[0].edge,
        Some(first_edge)
    );
    assert_eq!(
        cloned.serialized_cluster_edge_abductions(100).unwrap()[0].edge,
        Some(second_edge)
    );
}

#[test]
fn graphjson_cluster_reset_updates_the_topology_used_by_later_scopes() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 80.0, 40.0));
    let _second = input.add_node(node("second", 80.0, 40.0));
    let restored_target = input.add_node(node("restored-target", 80.0, 40.0));
    let serialized_target = input.add_node(node("serialized-target", 80.0, 40.0));
    let first_edge = input.add_edge(Edge {
        source: first,
        target: serialized_target,
    });
    let second_edge = input.add_edge(Edge {
        source: _second,
        target: serialized_target,
    });
    input.tala_id_overrides = vec![Some(10), Some(20), Some(30), Some(40)];
    input.tala_edge_ids[first_edge.0 as usize] = Some(-7);
    input.tala_edge_ids[second_edge.0 as usize] = Some(-8);
    input.serialized_clusters.push(SerializedClusterState {
        vessel_tala_id: -99,
        member_tala_ids: vec![10, 20],
        arrangement: "Column".to_owned(),
        desired_arrangement: "Column".to_owned(),
        edge_abductions: vec![
            SerializedAggregateEdgeAbduction {
                edge_id: -7,
                originally_from_node: 10,
                originally_to_node: 30,
                current_from_node: -99,
                current_to_node: 40,
            },
            SerializedAggregateEdgeAbduction {
                edge_id: -8,
                originally_from_node: 20,
                originally_to_node: 30,
                current_from_node: -99,
                current_to_node: 40,
            },
        ],
        container_tala_id: 0,
        fixed_size: false,
        padding: 10.0,
        vessel_position: Some(Point { x: 12.0, y: 24.0 }),
        vessel_size: Size {
            width: 180.0,
            height: 100.0,
        },
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.reset_serialized_clusters_for_layout();

    assert_eq!(arena.edge_records[first_edge.0 as usize].from, first);
    assert_eq!(
        arena.edge_records[first_edge.0 as usize].to,
        restored_target
    );
    assert_eq!(arena.edge_records[second_edge.0 as usize].from, _second);
    assert_eq!(
        arena.edge_records[second_edge.0 as usize].to,
        restored_target
    );
    assert!(arena.nodes[serialized_target.0 as usize].edges.is_empty());
    assert_eq!(
        arena.nodes[restored_target.0 as usize].edges,
        vec![first_edge, second_edge]
    );
    assert_eq!(arena.source_edge_endpoints[first_edge.0 as usize], (10, 30));
    assert_eq!(
        arena.source_edge_endpoints[second_edge.0 as usize],
        (20, 30)
    );
    assert_eq!(arena.source_edge_endpoints_by_id[&-7], (10, 30));
    assert!(arena.clusters.is_empty());
    assert!(arena.serialized_cluster_edge_abductions.is_empty());
    assert!(!arena.serialized_vessel_positions_by_tala.contains_key(&-99));
    assert!(!arena.serialized_vessel_sizes_by_tala.contains_key(&-99));

    let pipeline = Pipeline::new(&input, 1, false, false);
    assert_eq!(pipeline.input.edges[first_edge.0 as usize].source, first);
    assert_eq!(
        pipeline.input.edges[first_edge.0 as usize].target,
        restored_target
    );
    assert_eq!(pipeline.input.edges[second_edge.0 as usize].source, _second);
    assert_eq!(
        pipeline.input.edges[second_edge.0 as usize].target,
        restored_target
    );
    assert_eq!(
        pipeline.input.source_edge_endpoints[first_edge.0 as usize],
        (10, 30)
    );
    assert!(
        !pipeline
            .graph
            .serialized_vessel_positions_by_tala
            .contains_key(&-99)
    );
    assert!(
        !pipeline
            .graph
            .serialized_vessel_sizes_by_tala
            .contains_key(&-99)
    );
    let snapshot = layout_snapshot(&input, 1, LayoutStage::Normalize);
    assert_eq!(snapshot.edges[first_edge.0 as usize].source_node, first);
    assert_eq!(
        snapshot.edges[first_edge.0 as usize].target_node,
        restored_target
    );
    assert_eq!(snapshot.edges[second_edge.0 as usize].source_node, _second);
    assert_eq!(
        snapshot.edges[second_edge.0 as usize].target_node,
        restored_target
    );
}

#[test]
fn graphjson_sequence_vessel_state_keeps_its_distinct_geometry_and_abductions() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 80.0, 40.0));
    let second = input.add_node(node("second", 80.0, 40.0));
    let edge = input.add_edge(Edge {
        source: first,
        target: second,
    });
    input.tala_id_overrides = vec![Some(-11), Some(22)];
    input.tala_edge_ids[edge.0 as usize] = Some(-7);
    input.serialized_sequences.push(SerializedSequenceState {
        vessel_tala_id: -99,
        member_tala_ids: vec![-11, 22],
        edge_abductions: vec![SerializedAggregateEdgeAbduction {
            edge_id: -7,
            originally_from_node: -11,
            originally_to_node: 22,
            current_from_node: -99,
            current_to_node: 22,
        }],
        container_tala_id: 0,
        vessel_position: Some(Point { x: 12.0, y: 24.0 }),
        vessel_size: Size {
            width: 180.0,
            height: 100.0,
        },
    });

    let arena = ArenaGraph::from_input(&input);
    assert_eq!(arena.sequences.len(), 1);
    assert_eq!(
        arena.sequence_vessel_size(0),
        Size {
            width: 180.0,
            height: 100.0
        }
    );
    assert_eq!(arena.active_node_position(first), None);
    assert_eq!(arena.active_node_position(second), None);
    assert!(!arena.sequence_backlinks_complete);
    assert_eq!(arena.sequences[0].members, vec![first, second]);
    assert_eq!(
        arena.serialized_vessel_positions_by_tala.get(&-99),
        Some(&Point { x: 12.0, y: 24.0 })
    );
    assert_eq!(
        arena.serialized_sequence_edge_abductions[&-99][0].current_from,
        Some(AggregateNodeRef::Vessel { tala_id: -99 })
    );
}

#[test]
fn rebuilt_sequence_uses_live_step_geometry_instead_of_the_decoded_vessel_box() {
    let mut input = Graph::default();
    let mut first_node = node("first", 80.0, 40.0);
    first_node.shape = ShapeKind::Step;
    let first = input.add_node(first_node);
    let mut second_node = node("second", 80.0, 40.0);
    second_node.shape = ShapeKind::Step;
    let second = input.add_node(second_node);
    let edge = input.add_edge(Edge {
        source: first,
        target: second,
    });
    input.tala_id_overrides = vec![Some(-11), Some(22)];
    input.tala_edge_ids[edge.0 as usize] = Some(-7);
    input.serialized_sequences.push(SerializedSequenceState {
        vessel_tala_id: -99,
        member_tala_ids: vec![-11, 22],
        edge_abductions: Vec::new(),
        container_tala_id: 0,
        vessel_position: Some(Point { x: 12.0, y: 24.0 }),
        vessel_size: Size {
            width: 180.0,
            height: 100.0,
        },
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 55.0, y: 66.0 });
    arena.set_position(second, Point { x: 100.0, y: 66.0 });
    let mut misc_rng = go_rng::GoRng::new(1);
    arena.assign_sequences(&mut misc_rng);

    assert_eq!(arena.sequences.len(), 1);
    assert_eq!(arena.sequences[0].vessel_tala_id, -99);
    assert_eq!(misc_rng.draw_count(), 1);
    assert_eq!(
        arena.sequence_vessel_size(0),
        Size {
            width: 125.0,
            height: 40.0
        }
    );
    assert_eq!(
        arena.active_node_position(first),
        Some(Point { x: 55.0, y: 66.0 })
    );
    assert!(!arena.serialized_vessel_positions_by_tala.contains_key(&-99));
    assert!(!arena.serialized_vessel_sizes_by_tala.contains_key(&-99));
}

#[test]
fn cluster_vessel_direction_applies_to_sizeless_scoring() {
    let mut input = Graph::default();
    let vessel = input.add_node(node("vessel", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);

    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Row);
    assert_eq!(
        arena.edge_length_direction(vessel, false, true),
        (Orientation::Bottom, 0.2)
    );

    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Column);
    assert_eq!(
        arena.edge_length_direction(vessel, false, true),
        (Orientation::Right, 0.2)
    );
}

#[test]
fn sized_compaction_rounds_off_grid_floor_for_fixed_graphs() {
    let mut input = Graph::default();
    let mut fixed_node = node("fixed", 534.0, 480.0);
    fixed_node.locked_position = Some(Point { x: 240.0, y: 180.0 });
    let fixed = input.add_node(fixed_node);
    let moving = input.add_node(node("moving", 142.0, 84.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 126.0;
    arena.set_position(fixed, Point { x: 240.0, y: 180.0 });
    arena.set_position(moving, Point { x: 630.0, y: 756.0 });

    assert!(arena.has_current_fixed_node());
    assert_eq!(
        arena
            .possible_sized_compaction_moves(
                moving,
                1.0,
                true,
                0,
                arena.has_current_fixed_node(),
                &[],
            )
            .unwrap(),
        vec![
            Point { x: 252.0, y: 756.0 },
            Point { x: 378.0, y: 756.0 },
            Point { x: 504.0, y: 756.0 },
            Point { x: 630.0, y: 756.0 },
        ]
    );
}

#[test]
fn sized_compaction_observes_the_live_global_anchor_coordinate() {
    let mut input = Graph::default();
    let global = input.add_node(node("global", 100.0, 100.0));
    let later_root = input.add_node(node("later", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(global, Point { x: 0.0, y: -990.0 });
    arena.set_position(
        later_root,
        Point {
            x: 200.0,
            y: -990.0,
        },
    );

    assert_eq!(
        arena.sized_compaction_floor_decrease(later_root, global, false),
        2
    );

    // Recovered shiftSubgraphs keeps a *Node, so an earlier accepted move of
    // that node changes the equality tested for every later root in the pass.
    arena.set_position(global, Point { x: 0.0, y: -1188.0 });
    assert_eq!(
        arena.sized_compaction_floor_decrease(later_root, global, false),
        0
    );
}

#[test]
fn container_refit_updates_live_projected_endpoint_offsets() {
    let mut input = Graph::default();
    let carrier = input.add_node(node("carrier", 200.0, 200.0));
    let adjacent = input.add_node(node("adjacent", 80.0, 60.0));
    let edge = input.add_edge(Edge {
        source: carrier,
        target: adjacent,
    });
    let mut arena = ArenaGraph::from_input(&input);
    let projected = ProjectedAdjacent {
        owner: carrier,
        tala_id: 99,
        container_tala_id: Some(arena.nodes[carrier.0 as usize].tala_id),
        offset: Point { x: 60.0, y: 243.0 },
        size: Size {
            width: 80.0,
            height: 60.0,
        },
        cluster_member: false,
    };
    arena
        .sized_adjacent_overrides
        .insert((adjacent, edge), projected);
    arena.sized_edge_abductions.push(SizedEdgeAbduction {
        source_edge: edge,
        edge_tala_id: arena.edge_records[edge.0 as usize].tala_id,
        edge: Some(edge),
        current_from: Some(AbductionCurrentNodeRef::local(carrier)),
        current_to: Some(AbductionCurrentNodeRef::local(adjacent)),
        originally_from: Some(projected.into()),
        originally_to: None,
        originally_from_table_neighbors: Vec::new(),
        originally_to_table_neighbors: Vec::new(),
        originally_from_container: Some(carrier),
        originally_to_container: None,
        sequence_abduction: false,
        obstructions_from_to: Vec::new(),
        obstructions_to_from: Vec::new(),
        obstruction_sets_from_to: OrderedObstructionInventory::default(),
        obstruction_sets_to_from: OrderedObstructionInventory::default(),
    });

    // TALA keeps an original *Node in both carriers. Moving only its wrapping
    // container down one pixel therefore changes the live relative offset
    // from 243 to 242 without moving the hidden endpoint itself.
    arena.adjust_projected_offsets_for_refit(carrier, Point { x: 0.0, y: 1.0 });

    assert_eq!(
        arena.sized_adjacent_overrides[&(adjacent, edge)].offset,
        Point { x: 60.0, y: 242.0 }
    );
    assert_eq!(
        arena.sized_edge_abductions[0]
            .originally_from
            .expect("projected original endpoint")
            .offset,
        Some(Point { x: 60.0, y: 242.0 })
    );
}

#[test]
fn cluster_sync_publishes_retained_member_positions_to_projected_endpoints() {
    let mut input = Graph::default();
    let carrier = input.add_node(node("carrier", 200.0, 200.0));
    let adjacent = input.add_node(node("adjacent", 80.0, 60.0));
    let edge = input.add_edge(Edge {
        source: carrier,
        target: adjacent,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[carrier.0 as usize].tala_id = 456;
    arena.nodes[carrier.0 as usize].is_container = true;
    arena.set_position(carrier, Point { x: 10.0, y: 20.0 });
    let projected = ProjectedAdjacent {
        owner: carrier,
        tala_id: 99,
        container_tala_id: None,
        offset: Point { x: 60.0, y: 65.0 },
        size: Size {
            width: 80.0,
            height: 60.0,
        },
        cluster_member: true,
    };
    arena
        .sized_adjacent_overrides
        .insert((adjacent, edge), projected);

    let mut retained_member = arena.nodes[carrier.0 as usize].clone();
    retained_member.tala_id = projected.tala_id;
    retained_member.position = Some(Point { x: 70.0, y: 85.0 });
    retained_member.rect.origin = retained_member.position.unwrap();
    arena
        .transaction_external_aggregate_children
        .insert(123, vec![retained_member]);
    let mut retained_vessel = arena.nodes[carrier.0 as usize].clone();
    retained_vessel.tala_id = 123;
    retained_vessel.position = Some(Point { x: 70.0, y: 84.0 });
    retained_vessel.rect.origin = retained_vessel.position.unwrap();
    arena
        .transaction_external_container_children
        .insert(456, vec![retained_vessel]);
    arena.transaction_external_cluster_layouts.insert(
        123,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Column,
            padding: 20.0,
            fixed_size: false,
        },
    );

    arena.sync_clusters();

    assert_eq!(
        arena.sized_adjacent_overrides[&(adjacent, edge)].offset,
        Point { x: 60.0, y: 64.0 }
    );
}

#[test]
fn projected_sync_nested_rebases_hidden_aggregate_vessel_only_once() {
    const PARENT_TALA_ID: i64 = 90_000;
    const VESSEL_TALA_ID: i64 = 90_001;

    let mut input = Graph::default();
    let parent = input.add_node(node("hidden parent", 556.0, 272.0));
    let vessel_node = node("aggregate vessel", 54.0, 152.0);
    let vessel = input.add_node(vessel_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[parent.0 as usize].tala_id = PARENT_TALA_ID;
    arena.nodes[parent.0 as usize].is_container = true;
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Column);
    arena.nodes[vessel.0 as usize].transaction_position_was_nil = true;
    arena.set_position(parent, Point::default());
    arena.set_position(vessel, Point::default());
    // The enclosing graph retains these pointers in Containers, but the
    // current induced graph's Nodes slice does not contain the parent.
    arena.node_order.clear();
    arena.transaction_external_containers = vec![arena.nodes[parent.0 as usize].clone()];
    arena
        .transaction_external_container_children
        .insert(PARENT_TALA_ID, vec![arena.nodes[vessel.0 as usize].clone()]);
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Column,
            padding: 20.0,
            fixed_size: false,
        },
    );

    arena.sync_nested_projected_container_children();
    let first_position = arena.transaction_external_container_children[&PARENT_TALA_ID][0].position;
    assert_eq!(first_position, Some(Point { x: 60.0, y: 60.0 }));
    assert!(
        !arena.transaction_external_container_children[&PARENT_TALA_ID][0]
            .transaction_position_was_nil,
        "rebasing consumes the logical-nil marker"
    );

    arena.sync_nested_projected_container_children();
    assert_eq!(
        arena.transaction_external_container_children[&PARENT_TALA_ID][0].position, first_position,
        "an already framed vessel must not receive the parent's padding again"
    );
}

#[test]
fn projected_sync_nested_recovers_non_nil_cluster_vessel_from_member_frame() {
    const PARENT_TALA_ID: i64 = 90_020;
    const VESSEL_TALA_ID: i64 = 90_021;
    const FIRST_TALA_ID: i64 = 90_022;
    const SECOND_TALA_ID: i64 = 90_023;

    let mut input = Graph::default();
    let parent = input.add_node(node("hidden parent", 970.0, 3000.0));
    let first = input.add_node(node("first member", 687.0, 673.0));
    let second = input.add_node(node("second member", 687.0, 673.0));
    let vessel = input.add_node(node("cluster vessel", 687.0, 2059.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[parent.0 as usize].tala_id = PARENT_TALA_ID;
    arena.nodes[parent.0 as usize].is_container = true;
    arena.nodes[first.0 as usize].tala_id = FIRST_TALA_ID;
    arena.nodes[second.0 as usize].tala_id = SECOND_TALA_ID;
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Column);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: VESSEL_TALA_ID,
        fixed_size: false,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);

    // The hidden parent's child map carries a non-nil local vessel origin,
    // while the retained member pointers have already moved to the parent's
    // absolute frame. Their exact cluster offsets recover the shared origin.
    arena.set_position(parent, Point { x: 0.0, y: 2179.0 });
    arena.set_position(vessel, Point::default());
    arena.set_position(first, Point { x: 60.0, y: 2239.0 });
    arena.set_position(second, Point { x: 60.0, y: 2932.0 });
    arena.node_order.clear();
    arena.transaction_external_containers = vec![arena.nodes[parent.0 as usize].clone()];
    arena
        .transaction_external_container_children
        .insert(PARENT_TALA_ID, vec![arena.nodes[vessel.0 as usize].clone()]);
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
        ],
    );
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Column,
            padding: 20.0,
            fixed_size: false,
        },
    );

    arena.sync_nested_projected_container_children();
    let first_position = arena.transaction_external_container_children[&PARENT_TALA_ID][0].position;
    assert_eq!(first_position, Some(Point { x: 60.0, y: 2239.0 }));

    arena.sync_nested_projected_container_children();
    assert_eq!(
        arena.transaction_external_container_children[&PARENT_TALA_ID][0].position, first_position,
        "a vessel recovered from its member pointers must not be rebased twice"
    );
}

#[test]
fn projected_sync_nested_preserves_positioned_overflowing_aggregate_vessel() {
    const PARENT_TALA_ID: i64 = 90_010;
    const VESSEL_TALA_ID: i64 = 90_011;

    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 970.0, 2179.0));
    let vessel = input.add_node(node("aggregate vessel", 687.0, 2153.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[parent.0 as usize].tala_id = PARENT_TALA_ID;
    arena.nodes[parent.0 as usize].is_container = true;
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Column);
    let parent_position = Point {
        x: 2419.0,
        y: 2700.0,
    };
    let vessel_position = Point {
        x: 2479.0,
        y: 2760.0,
    };
    arena.set_position(parent, parent_position);
    arena.set_position(vessel, vessel_position);
    // The parent is retained in the source hierarchy, but absent from this
    // induced graph's live node order. The vessel position came from an
    // already-laid-out graph and is absolute even though its resized box now
    // extends past the parent's original bottom edge.
    arena.node_order.clear();
    arena.transaction_external_containers = vec![arena.nodes[parent.0 as usize].clone()];
    arena
        .transaction_external_container_children
        .insert(PARENT_TALA_ID, vec![arena.nodes[vessel.0 as usize].clone()]);
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Column,
            padding: 20.0,
            fixed_size: false,
        },
    );

    arena.sync_nested_projected_container_children();

    assert_eq!(
        arena.transaction_external_container_children[&PARENT_TALA_ID][0].position,
        Some(vessel_position),
        "an existing absolute position must not be rebased based on the old parent bounds"
    );
}

#[test]
fn projected_sync_nested_pads_cluster_container_member_children() {
    const VESSEL_TALA_ID: i64 = 1_543_039_099_823_358_511;
    const MEMBER_TALA_ID: i64 = 1_788_937_150;
    const PEER_TALA_ID: i64 = 3_869_595_486;
    const CHILD_TALA_ID: i64 = 3_054_522_831;
    const OFFSET_CHILD_TALA_ID: i64 = 30_545_222_832;
    const GRANDCHILD_TALA_ID: i64 = 30_545_222_833;

    let mut input = Graph::default();
    let vessel = input.add_node(node("cluster vessel", 264.0, 392.0));
    let mut member_node = node("container member", 264.0, 186.0);
    member_node.shape = ShapeKind::Square;
    let member = input.add_node(member_node);
    let peer = input.add_node(node("peer member", 264.0, 186.0));
    let mut child_node = node("nested child", 144.0, 66.0);
    child_node.parent = Some(member);
    let child = input.add_node(child_node);
    let mut offset_child_node = node("offset nested container", 120.0, 80.0);
    offset_child_node.parent = Some(member);
    let offset_child = input.add_node(offset_child_node);
    let mut grandchild_node = node("nested grandchild", 60.0, 40.0);
    grandchild_node.parent = Some(offset_child);
    let grandchild = input.add_node(grandchild_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Column);
    arena.nodes[member.0 as usize].tala_id = MEMBER_TALA_ID;
    arena.nodes[peer.0 as usize].tala_id = PEER_TALA_ID;
    arena.nodes[child.0 as usize].tala_id = CHILD_TALA_ID;
    arena.nodes[offset_child.0 as usize].tala_id = OFFSET_CHILD_TALA_ID;
    arena.nodes[grandchild.0 as usize].tala_id = GRANDCHILD_TALA_ID;
    // The induced graph owns only the synthetic vessel. Its cluster members
    // and nested child remain shared pointers in the projected maps.
    arena.node_order = vec![vessel];
    let vessel_top_left = Point { x: 693.0, y: 99.0 };
    arena.set_position(vessel, vessel_top_left);
    arena.set_position(member, vessel_top_left);
    arena.set_position(peer, Point { x: 693.0, y: 305.0 });
    arena.set_position(child, vessel_top_left);
    arena.set_position(offset_child, Point { x: 710.0, y: 120.0 });
    arena.set_position(grandchild, Point { x: 720.0, y: 130.0 });

    let vessel_projection = arena.nodes[vessel.0 as usize].clone();
    let member_projection = arena.nodes[member.0 as usize].clone();
    let peer_projection = arena.nodes[peer.0 as usize].clone();
    let child_projection = arena.nodes[child.0 as usize].clone();
    let offset_child_projection = arena.nodes[offset_child.0 as usize].clone();
    let grandchild_projection = arena.nodes[grandchild.0 as usize].clone();
    arena
        .transaction_external_container_children
        .insert(824_686_997, vec![vessel_projection]);
    arena.transaction_external_container_children.insert(
        MEMBER_TALA_ID,
        vec![child_projection, offset_child_projection],
    );
    arena
        .transaction_external_container_children
        .insert(OFFSET_CHILD_TALA_ID, vec![grandchild_projection]);
    arena
        .transaction_external_aggregate_children
        .insert(VESSEL_TALA_ID, vec![member_projection, peer_projection]);
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Column,
            padding: 20.0,
            fixed_size: false,
        },
    );

    let mut without_live_vessel = arena.clone();
    without_live_vessel.node_order.clear();
    without_live_vessel.sync_nested_projected_container_children();
    assert_eq!(
        without_live_vessel.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(vessel_top_left),
        "an external layout alone must not synthesize a syncNested vessel pass"
    );
    assert_eq!(
        without_live_vessel.transaction_external_container_children[&MEMBER_TALA_ID][1].position,
        Some(Point { x: 710.0, y: 120.0 }),
        "the absent-vessel path must preserve an existing child offset"
    );

    arena.sync_nested_projected_container_children();

    let members = &arena.transaction_external_aggregate_children[&VESSEL_TALA_ID];
    assert_eq!(members[0].position, Some(vessel_top_left));
    assert_eq!(members[1].position, Some(Point { x: 693.0, y: 305.0 }));
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(Point { x: 753.0, y: 159.0 })
    );
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][1].position,
        Some(Point { x: 770.0, y: 180.0 }),
        "padding must add to, not replace, the child's existing offset"
    );
    assert_eq!(
        arena.transaction_external_container_children[&OFFSET_CHILD_TALA_ID][0].position,
        Some(Point { x: 780.0, y: 190.0 }),
        "moveNodeWithChildren must translate the complete descendant subtree"
    );
    assert_eq!(arena.position(vessel), Some(vessel_top_left));
}

#[test]
fn projected_cluster_arrange_preserves_nil_member_through_anchor_materialization() {
    const VESSEL_TALA_ID: i64 = 1_543_039_099_823_358_511;
    const MEMBER_TALA_ID: i64 = 1_788_937_150;
    const PEER_TALA_ID: i64 = 3_869_595_486;
    const CHILD_TALA_ID: i64 = 3_054_522_831;

    let mut input = Graph::default();
    let vessel = input.add_node(node("cluster vessel", 264.0, 392.0));
    let mut member_node = node("container member", 264.0, 186.0);
    member_node.shape = ShapeKind::Square;
    let member = input.add_node(member_node);
    let peer = input.add_node(node("peer member", 264.0, 186.0));
    let mut child_node = node("nested child", 144.0, 66.0);
    child_node.parent = Some(member);
    let child = input.add_node(child_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[member.0 as usize].tala_id = MEMBER_TALA_ID;
    arena.nodes[peer.0 as usize].tala_id = PEER_TALA_ID;
    arena.nodes[child.0 as usize].tala_id = CHILD_TALA_ID;

    let vessel_position = Point { x: 6.0, y: 7.0 };
    arena.set_position(vessel, vessel_position);
    arena.set_position(child, Point::default());

    let mut member_projection = arena.nodes[member.0 as usize].clone();
    member_projection.position = None;
    member_projection.transaction_anchor_tala_id = Some(VESSEL_TALA_ID);
    member_projection.transaction_anchor_offset = Some(Point::default());
    let mut peer_projection = arena.nodes[peer.0 as usize].clone();
    peer_projection.position = None;
    peer_projection.transaction_anchor_tala_id = Some(VESSEL_TALA_ID);
    peer_projection.transaction_anchor_offset = Some(Point { x: 0.0, y: 206.0 });
    arena
        .transaction_external_aggregate_children
        .insert(VESSEL_TALA_ID, vec![member_projection, peer_projection]);
    arena
        .transaction_external_container_children
        .insert(MEMBER_TALA_ID, vec![arena.nodes[child.0 as usize].clone()]);
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Column,
            padding: 20.0,
            fixed_size: false,
        },
    );

    arena.initialize_transaction_projected_positions();
    assert_eq!(
        arena.transaction_external_aggregate_children[&VESSEL_TALA_ID][0].position,
        Some(vessel_position)
    );
    assert!(
        arena.transaction_external_aggregate_children[&VESSEL_TALA_ID][0]
            .transaction_position_was_nil
    );

    arena.sync_external_cluster(VESSEL_TALA_ID);
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(vessel_position),
        "the first ArrangeClusterNodes sync must take the recovered nil-member arm"
    );
    assert!(
        !arena.transaction_external_aggregate_children[&VESSEL_TALA_ID][0]
            .transaction_position_was_nil
    );

    arena.sync_external_cluster(VESSEL_TALA_ID);
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(vessel_position),
        "logical nilness is consumed exactly once"
    );

    arena.sync_nested_projected_cluster_member_children(VESSEL_TALA_ID);
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(Point { x: 66.0, y: 67.0 }),
        "syncNested adds the member's full 60-pixel container padding after Arrange"
    );
    arena.translate_projected_subtrees(&[MEMBER_TALA_ID], Point { x: 687.0, y: 92.0 });
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(Point { x: 753.0, y: 159.0 }),
        "outer shared-pointer copyback must preserve both recovered deltas"
    );
}

#[test]
fn projected_cluster_sync_resizes_fixed_vessel_without_equalizing_members() {
    const VESSEL_TALA_ID: i64 = 90_001;
    const FIRST_TALA_ID: i64 = 90_002;
    const SECOND_TALA_ID: i64 = 90_003;

    let mut input = Graph::default();
    let vessel = input.add_node(node("fixed cluster vessel", 10.0, 10.0));
    let first = input.add_node(node("first fixed member", 80.0, 40.0));
    let second = input.add_node(node("second fixed member", 120.0, 60.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Row);
    arena.nodes[first.0 as usize].tala_id = FIRST_TALA_ID;
    arena.nodes[second.0 as usize].tala_id = SECOND_TALA_ID;
    arena.node_order = vec![vessel];
    let vessel_top_left = Point { x: 100.0, y: 200.0 };
    arena.set_position(vessel, vessel_top_left);
    arena.set_position(first, Point { x: 10.0, y: 20.0 });
    arena.set_position(second, Point { x: 30.0, y: 40.0 });

    arena
        .transaction_external_container_children
        .insert(90_000, vec![arena.nodes[vessel.0 as usize].clone()]);
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
        ],
    );
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Row,
            padding: 20.0,
            fixed_size: true,
        },
    );

    arena.sync_nested_projected_container_children();

    let expected_vessel_size = Size {
        width: 260.0,
        height: 60.0,
    };
    assert_eq!(
        arena.nodes[vessel.0 as usize].rect.size,
        expected_vessel_size
    );
    assert_eq!(
        arena.transaction_external_container_children[&90_000][0]
            .rect
            .size,
        expected_vessel_size,
        "Cluster.Resize must publish the fixed cluster's vessel dimensions through every alias"
    );
    let members = &arena.transaction_external_aggregate_children[&VESSEL_TALA_ID];
    assert_eq!(
        members[0].rect.size,
        Size {
            width: 80.0,
            height: 40.0
        }
    );
    assert_eq!(
        members[1].rect.size,
        Size {
            width: 120.0,
            height: 60.0
        }
    );
    assert_eq!(members[0].position, Some(Point { x: 100.0, y: 210.0 }));
    assert_eq!(members[1].position, Some(Point { x: 200.0, y: 200.0 }));
}

#[test]
fn projected_cluster_resize_updates_cached_endpoint_and_obstruction_sizes() {
    const VESSEL_TALA_ID: i64 = 90_101;
    const SMALL_TALA_ID: i64 = 90_102;
    const LARGE_TALA_ID: i64 = 90_103;

    let mut input = Graph::default();
    let vessel = input.add_node(node("resized cluster vessel", 10.0, 10.0));
    let small = input.add_node(node("small member", 80.0, 40.0));
    let large = input.add_node(node("large member", 120.0, 60.0));
    let adjacent = input.add_node(node("adjacent", 40.0, 40.0));
    let edge = input.add_edge(Edge {
        source: vessel,
        target: adjacent,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Row);
    arena.nodes[small.0 as usize].tala_id = SMALL_TALA_ID;
    arena.nodes[large.0 as usize].tala_id = LARGE_TALA_ID;
    arena.node_order = vec![vessel];
    arena.set_position(vessel, Point { x: 100.0, y: 200.0 });
    arena.set_position(small, Point { x: 10.0, y: 20.0 });
    arena.set_position(large, Point { x: 30.0, y: 40.0 });
    arena.set_position(adjacent, Point { x: 500.0, y: 500.0 });
    arena
        .transaction_external_container_children
        .insert(90_100, vec![arena.nodes[vessel.0 as usize].clone()]);
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[small.0 as usize].clone(),
            arena.nodes[large.0 as usize].clone(),
        ],
    );
    arena.transaction_external_cluster_layouts.insert(
        VESSEL_TALA_ID,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Row,
            padding: 20.0,
            fixed_size: false,
        },
    );

    let small_projection = ProjectedAdjacent {
        owner: vessel,
        tala_id: SMALL_TALA_ID,
        container_tala_id: None,
        offset: Point {
            x: -90.0,
            y: -180.0,
        },
        size: Size {
            width: 80.0,
            height: 40.0,
        },
        cluster_member: true,
    };
    let vessel_obstruction = ProjectedAdjacent {
        tala_id: VESSEL_TALA_ID,
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        ..small_projection
    };
    let control_obstruction = ProjectedAdjacent {
        tala_id: 90_199,
        size: Size {
            width: 33.0,
            height: 44.0,
        },
        ..small_projection
    };
    arena
        .sized_adjacent_overrides
        .insert((adjacent, edge), small_projection);
    arena.sized_edge_abductions.push(SizedEdgeAbduction {
        source_edge: edge,
        edge_tala_id: arena.edge_records[edge.0 as usize].tala_id,
        edge: Some(edge),
        current_from: Some(AbductionCurrentNodeRef::local(vessel)),
        current_to: Some(AbductionCurrentNodeRef::local(adjacent)),
        originally_from: Some(small_projection.into()),
        originally_to: Some(small_projection.into()),
        originally_from_table_neighbors: vec![ProjectedTableColumnNeighbor {
            other: small_projection,
            column_index: 0,
        }],
        originally_to_table_neighbors: vec![ProjectedTableColumnNeighbor {
            other: small_projection,
            column_index: 0,
        }],
        originally_from_container: None,
        originally_to_container: None,
        sequence_abduction: false,
        obstructions_from_to: vec![small_projection],
        obstructions_to_from: vec![small_projection],
        obstruction_sets_from_to: OrderedObstructionInventory::default(),
        obstruction_sets_to_from: OrderedObstructionInventory::default(),
    });
    arena.sized_projected_obstructions.insert(
        (vessel, edge),
        vec![small_projection, vessel_obstruction, control_obstruction].into(),
    );
    arena
        .sized_collapsed_symmetry_neighbors
        .insert(SMALL_TALA_ID, vec![small_projection]);
    arena.sized_cluster_distance_boxes.insert(
        (vessel, SMALL_TALA_ID),
        ProjectedClusterDistance {
            offset: Point::default(),
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            arrangement: ClusterArrangement::Row,
            vessel_tala_id: VESSEL_TALA_ID,
            external_connected: vec![small_projection],
        },
    );

    let mut without_vessel_position = arena.clone();
    without_vessel_position.nodes[vessel.0 as usize].position = None;
    without_vessel_position.nodes[vessel.0 as usize].rect.origin = Point::default();
    let projected_vessel = &mut without_vessel_position
        .transaction_external_container_children
        .get_mut(&90_100)
        .expect("projected cluster vessel")[0];
    projected_vessel.position = None;
    projected_vessel.rect.origin = Point::default();
    without_vessel_position.sync_nested_projected_container_children();
    assert_eq!(
        without_vessel_position.nodes[small.0 as usize].rect.size,
        Size {
            width: 120.0,
            height: 60.0
        },
        "Cluster.Resize must equalize members before ArrangeClusterNodes observes nil TopLeft"
    );
    assert_eq!(
        without_vessel_position.nodes[vessel.0 as usize].rect.size,
        Size {
            width: 260.0,
            height: 60.0
        },
        "Cluster.Resize must publish vessel dimensions even when arrangement is skipped"
    );
    assert_eq!(
        without_vessel_position.sized_adjacent_overrides[&(adjacent, edge)].size,
        Size {
            width: 120.0,
            height: 60.0
        }
    );

    arena.sync_nested_projected_container_children();

    let member_size = Size {
        width: 120.0,
        height: 60.0,
    };
    let vessel_size = Size {
        width: 260.0,
        height: 60.0,
    };
    let assert_member_size = |projected: ProjectedAdjacent| {
        assert_eq!(projected.size, member_size);
    };
    assert_member_size(arena.sized_adjacent_overrides[&(adjacent, edge)]);
    let abduction = &arena.sized_edge_abductions[0];
    assert_member_size(abduction.originally_from.unwrap().projection().unwrap());
    assert_member_size(abduction.originally_to.unwrap().projection().unwrap());
    assert_member_size(abduction.originally_from_table_neighbors[0].other);
    assert_member_size(abduction.originally_to_table_neighbors[0].other);
    assert_member_size(abduction.obstructions_from_to[0]);
    assert_member_size(abduction.obstructions_to_from[0]);
    assert_member_size(arena.sized_projected_obstructions[&(vessel, edge)][0]);
    assert_eq!(
        arena.sized_projected_obstructions[&(vessel, edge)][1].size,
        vessel_size
    );
    assert_eq!(
        arena.sized_projected_obstructions[&(vessel, edge)][2].size,
        Size {
            width: 33.0,
            height: 44.0
        },
        "a Box resize must not alter a different projected TALA pointer"
    );
    assert_member_size(arena.sized_collapsed_symmetry_neighbors[&SMALL_TALA_ID][0]);
    let distance = &arena.sized_cluster_distance_boxes[&(vessel, SMALL_TALA_ID)];
    assert_eq!(distance.size, vessel_size);
    assert_member_size(distance.external_connected[0]);
}

#[test]
fn projected_cluster_commit_walks_reachable_nested_vessels_in_postorder() {
    const OUTER_VESSEL: i64 = 100;
    const INNER_VESSEL: i64 = 900;
    const INNER_FIRST: i64 = 901;
    const INNER_SECOND: i64 = 902;
    const OUTER_PEER: i64 = 101;

    let mut input = Graph::default();
    let outer = input.add_node(node("outer cluster vessel", 10.0, 10.0));
    let inner = input.add_node(node("inner cluster vessel", 10.0, 10.0));
    let inner_first = input.add_node(node("inner first", 80.0, 40.0));
    let inner_second = input.add_node(node("inner second", 120.0, 60.0));
    let outer_peer = input.add_node(node("outer peer", 100.0, 50.0));
    let mut arena = ArenaGraph::from_input(&input);
    for (node, tala_id) in [
        (outer, OUTER_VESSEL),
        (inner, INNER_VESSEL),
        (inner_first, INNER_FIRST),
        (inner_second, INNER_SECOND),
        (outer_peer, OUTER_PEER),
    ] {
        arena.nodes[node.0 as usize].tala_id = tala_id;
    }
    for vessel in [outer, inner] {
        arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
        arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Row);
    }
    // Only the outer vessel belongs to current Graph.Nodes. The inner
    // vessel's distinct positioned pointer exists solely as an outer member.
    arena.node_order = vec![outer];
    arena.set_position(outer, Point { x: 10.0, y: 20.0 });
    arena.nodes[inner.0 as usize].position = None;
    arena.nodes[inner.0 as usize].rect.origin = Point::default();
    arena.set_position(
        inner_first,
        Point {
            x: -100.0,
            y: -100.0,
        },
    );
    arena.set_position(
        inner_second,
        Point {
            x: -200.0,
            y: -200.0,
        },
    );
    arena.set_position(outer_peer, Point { x: 500.0, y: 500.0 });

    let mut inner_vessel = arena.nodes[inner.0 as usize].clone();
    inner_vessel.position = Some(Point { x: 50.0, y: 60.0 });
    inner_vessel.rect.origin = inner_vessel.position.unwrap();
    arena.transaction_external_aggregate_children.insert(
        OUTER_VESSEL,
        vec![inner_vessel, arena.nodes[outer_peer.0 as usize].clone()],
    );
    arena.transaction_external_aggregate_children.insert(
        INNER_VESSEL,
        vec![
            arena.nodes[inner_first.0 as usize].clone(),
            arena.nodes[inner_second.0 as usize].clone(),
        ],
    );
    for vessel_tala_id in [OUTER_VESSEL, INNER_VESSEL] {
        arena.transaction_external_cluster_layouts.insert(
            vessel_tala_id,
            ExternalClusterLayout {
                arrangement: ClusterArrangement::Row,
                padding: 20.0,
                fixed_size: true,
            },
        );
    }

    let mut unreachable = arena.clone();
    unreachable.node_order.clear();
    unreachable.sync_clusters();
    assert_eq!(
        unreachable.nodes[outer.0 as usize].rect.size,
        Size {
            width: 10.0,
            height: 10.0
        },
        "stale projected layouts outside Graph.Nodes reachability must not synchronize"
    );

    arena.sync_clusters();

    assert_eq!(
        arena.nodes[inner.0 as usize].rect.size,
        Size {
            width: 260.0,
            height: 60.0
        }
    );
    assert_eq!(
        arena.nodes[outer.0 as usize].rect.size,
        Size {
            width: 540.0,
            height: 60.0
        },
        "inner Resize must publish before the outer cluster computes its vessel"
    );
    let inner_members = &arena.transaction_external_aggregate_children[&INNER_VESSEL];
    assert_eq!(inner_members[0].position, Some(Point { x: 10.0, y: 30.0 }));
    assert_eq!(inner_members[1].position, Some(Point { x: 110.0, y: 20.0 }));
}

#[test]
fn projected_reverse_dfs_preserves_repeated_pointer_visits() {
    let mut input = Graph::default();
    let root = input.add_node(node("root", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[root.0 as usize].tala_id = 93_001;
    arena.node_order = vec![root, root];

    assert_eq!(arena.projected_reverse_dfs_order(), vec![93_001, 93_001]);
}

#[test]
fn projected_ordinary_container_moves_full_subtree_through_every_alias() {
    const CONTAINER_TALA_ID: i64 = 91_001;
    const VESSEL_TALA_ID: i64 = 91_002;
    const MEMBER_TALA_ID: i64 = 91_003;
    const CHILD_TALA_ID: i64 = 91_004;

    let mut input = Graph::default();
    let container = input.add_node(node("ordinary container", 300.0, 300.0));
    let vessel = input.add_node(node("nested aggregate vessel", 100.0, 100.0));
    let member = input.add_node(node("nested aggregate member", 80.0, 80.0));
    let child = input.add_node(node("nested member child", 20.0, 20.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[container.0 as usize].tala_id = CONTAINER_TALA_ID;
    arena.nodes[container.0 as usize].is_container = true;
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[member.0 as usize].tala_id = MEMBER_TALA_ID;
    arena.nodes[member.0 as usize].is_container = true;
    arena.nodes[child.0 as usize].tala_id = CHILD_TALA_ID;
    arena.node_order = vec![container];
    arena.set_position(container, Point { x: 100.0, y: 200.0 });
    arena.set_position(vessel, Point { x: 20.0, y: 30.0 });
    arena.set_position(member, Point { x: 30.0, y: 40.0 });
    arena.set_position(child, Point { x: 35.0, y: 45.0 });

    arena.transaction_external_container_children.insert(
        CONTAINER_TALA_ID,
        vec![arena.nodes[vessel.0 as usize].clone()],
    );
    arena
        .transaction_external_aggregate_children
        .insert(VESSEL_TALA_ID, vec![arena.nodes[member.0 as usize].clone()]);
    arena
        .transaction_external_container_children
        .insert(MEMBER_TALA_ID, vec![arena.nodes[child.0 as usize].clone()]);
    arena
        .transaction_external_containers
        .push(arena.nodes[member.0 as usize].clone());

    arena.sync_nested_projected_container_children();

    let vessel_target = Point { x: 160.0, y: 260.0 };
    let member_target = Point { x: 170.0, y: 270.0 };
    let child_target = Point { x: 175.0, y: 275.0 };
    assert_eq!(arena.position(vessel), Some(vessel_target));
    assert_eq!(
        arena.transaction_external_container_children[&CONTAINER_TALA_ID][0].position,
        Some(vessel_target)
    );
    assert_eq!(arena.position(member), Some(member_target));
    assert_eq!(
        arena.transaction_external_aggregate_children[&VESSEL_TALA_ID][0].position,
        Some(member_target)
    );
    assert_eq!(
        arena.transaction_external_containers[0].position,
        Some(member_target)
    );
    assert_eq!(arena.position(child), Some(child_target));
    assert_eq!(
        arena.transaction_external_container_children[&MEMBER_TALA_ID][0].position,
        Some(child_target)
    );
}

#[test]
fn projected_sequence_sync_runs_after_ordinary_container_in_node_order() {
    const CONTAINER_TALA_ID: i64 = 92_001;
    const VESSEL_TALA_ID: i64 = 92_002;
    const FIRST_TALA_ID: i64 = 92_003;
    const SECOND_TALA_ID: i64 = 92_004;
    const THIRD_TALA_ID: i64 = 92_005;

    let mut input = Graph::default();
    let container = input.add_node(node("sequence container", 300.0, 300.0));
    let vessel = input.add_node(node("sequence vessel", 10.0, 10.0));
    let first = input.add_node(node("first step", 80.0, 40.0));
    let second = input.add_node(node("second step", 120.0, 40.0));
    let third = input.add_node(node("third step", 60.0, 40.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[container.0 as usize].tala_id = CONTAINER_TALA_ID;
    arena.nodes[container.0 as usize].is_container = true;
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[first.0 as usize].tala_id = FIRST_TALA_ID;
    arena.nodes[second.0 as usize].tala_id = SECOND_TALA_ID;
    arena.nodes[third.0 as usize].tala_id = THIRD_TALA_ID;
    arena.node_order = vec![container, vessel];
    arena.set_position(container, Point { x: 100.0, y: 200.0 });
    arena.set_position(vessel, Point { x: 20.0, y: 30.0 });
    arena.set_position(first, Point { x: -10.0, y: -20.0 });
    arena.set_position(second, Point { x: 5.0, y: 15.0 });
    arena.set_position(third, Point { x: 25.0, y: 35.0 });

    arena.transaction_external_container_children.insert(
        CONTAINER_TALA_ID,
        vec![arena.nodes[vessel.0 as usize].clone()],
    );
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
            arena.nodes[third.0 as usize].clone(),
        ],
    );

    let initial_member_positions = [
        arena.position(first),
        arena.position(second),
        arena.position(third),
    ];
    let mut without_vessel_position = arena.clone();
    without_vessel_position.nodes[vessel.0 as usize].position = None;
    without_vessel_position.nodes[vessel.0 as usize].rect.origin = Point::default();
    let projected_vessel = &mut without_vessel_position
        .transaction_external_container_children
        .get_mut(&CONTAINER_TALA_ID)
        .expect("projected sequence vessel")[0];
    projected_vessel.position = None;
    projected_vessel.rect.origin = Point::default();
    without_vessel_position.sync_nested_projected_container_children();
    assert_eq!(
        without_vessel_position.nodes[vessel.0 as usize].rect.size,
        Size {
            width: 190.0,
            height: 40.0
        },
        "Sequence.resize must run before arrangeSteps observes a nil vessel TopLeft"
    );
    assert_eq!(
        [
            without_vessel_position.position(first),
            without_vessel_position.position(second),
            without_vessel_position.position(third),
        ],
        initial_member_positions,
        "a nil vessel TopLeft must leave the ordered member positions unchanged"
    );

    arena.sync_nested_projected_container_children();

    let vessel_target = Point { x: 160.0, y: 260.0 };
    assert_eq!(arena.position(vessel), Some(vessel_target));
    assert_eq!(
        arena.nodes[vessel.0 as usize].rect.size,
        Size {
            width: 190.0,
            height: 40.0
        }
    );
    assert_eq!(
        arena.transaction_external_container_children[&CONTAINER_TALA_ID][0]
            .rect
            .size,
        Size {
            width: 190.0,
            height: 40.0
        }
    );
    let members = &arena.transaction_external_aggregate_children[&VESSEL_TALA_ID];
    assert_eq!(members[0].position, Some(vessel_target));
    assert_eq!(members[1].position, Some(Point { x: 205.0, y: 260.0 }));
    assert_eq!(members[2].position, Some(Point { x: 290.0, y: 260.0 }));
    assert_eq!(arena.position(first), members[0].position);
    assert_eq!(arena.position(second), members[1].position);
    assert_eq!(arena.position(third), members[2].position);
}

#[test]
fn projected_sequence_sync_runs_at_transaction_commit_and_reconciles_endpoints() {
    const ROOT_TALA_ID: i64 = 92_101;
    const VESSEL_TALA_ID: i64 = 92_102;
    const FIRST_TALA_ID: i64 = 92_103;
    const SECOND_TALA_ID: i64 = 92_104;

    let mut input = Graph::default();
    let root = input.add_node(node("transaction root", 300.0, 300.0));
    let vessel = input.add_node(node("nested sequence vessel", 10.0, 10.0));
    let first = input.add_node(node("first nested step", 80.0, 40.0));
    let second = input.add_node(node("second nested step", 120.0, 40.0));
    let adjacent = input.add_node(node("transaction adjacent", 40.0, 40.0));
    let edge = input.add_edge(Edge {
        source: vessel,
        target: adjacent,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[root.0 as usize].tala_id = ROOT_TALA_ID;
    arena.nodes[root.0 as usize].is_container = true;
    arena.nodes[vessel.0 as usize].tala_id = VESSEL_TALA_ID;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[first.0 as usize].tala_id = FIRST_TALA_ID;
    arena.nodes[second.0 as usize].tala_id = SECOND_TALA_ID;
    // The current graph owns only the ancestor. Graph.syncSequences reaches
    // the nested sequence vessel through rdfsWalk after every Commit.
    arena.node_order = vec![root];
    arena.set_position(root, Point::default());
    arena.set_position(vessel, Point { x: 300.0, y: 400.0 });
    arena.set_position(first, Point { x: -20.0, y: -30.0 });
    arena.set_position(second, Point { x: 50.0, y: 60.0 });
    arena.set_position(adjacent, Point { x: 700.0, y: 700.0 });
    arena
        .transaction_external_container_children
        .insert(ROOT_TALA_ID, vec![arena.nodes[vessel.0 as usize].clone()]);
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
        ],
    );
    arena.sized_adjacent_overrides.insert(
        (adjacent, edge),
        ProjectedAdjacent {
            owner: vessel,
            tala_id: FIRST_TALA_ID,
            container_tala_id: None,
            offset: Point {
                x: -320.0,
                y: -430.0,
            },
            size: Size {
                width: 80.0,
                height: 40.0,
            },
            cluster_member: false,
        },
    );

    let mut direct_sync = arena.clone();
    direct_sync.sync_sequences();
    assert_eq!(
        direct_sync.nodes[vessel.0 as usize].rect.size,
        Size {
            width: 165.0,
            height: 40.0
        },
        "Graph.syncSequences itself must own projected sequence publication"
    );
    let direct_members = &direct_sync.transaction_external_aggregate_children[&VESSEL_TALA_ID];
    assert_eq!(
        direct_members[0].position,
        Some(Point { x: 300.0, y: 400.0 })
    );
    assert_eq!(
        direct_members[1].position,
        Some(Point { x: 345.0, y: 400.0 })
    );

    arena.reposition_ordinary_containers();

    assert_eq!(
        arena.nodes[vessel.0 as usize].rect.size,
        Size {
            width: 165.0,
            height: 40.0
        }
    );
    let members = &arena.transaction_external_aggregate_children[&VESSEL_TALA_ID];
    assert_eq!(members[0].position, Some(Point { x: 300.0, y: 400.0 }));
    assert_eq!(members[1].position, Some(Point { x: 345.0, y: 400.0 }));
    assert_eq!(
        arena.sized_adjacent_overrides[&(adjacent, edge)].offset,
        Point::default(),
        "Transaction.Commit sequence sync must publish the shared member position before scoring"
    );
}

#[test]
fn projected_container_refit_reaches_nested_sequence_alias_before_commit_sync() {
    const OUTER_VESSEL: i64 = 94_001;
    const SEQUENCE_VESSEL: i64 = 94_002;
    const OUTER_PEER: i64 = 94_003;
    const STEP_ONE: i64 = 94_004;
    const STEP_TWO: i64 = 94_005;
    const CHILD: i64 = 94_006;

    let mut input = Graph::default();
    let outer = input.add_node(node("outer cluster vessel", 10.0, 10.0));
    let sequence = input.add_node(node("container sequence vessel", 10.0, 10.0));
    let peer = input.add_node(node("outer peer", 50.0, 50.0));
    let first = input.add_node(node("first step", 80.0, 40.0));
    let second = input.add_node(node("second step", 120.0, 40.0));
    let child = input.add_node(node("sequence vessel child", 20.0, 20.0));
    let mut arena = ArenaGraph::from_input(&input);
    for (node, tala_id) in [
        (outer, OUTER_VESSEL),
        (sequence, SEQUENCE_VESSEL),
        (peer, OUTER_PEER),
        (first, STEP_ONE),
        (second, STEP_TWO),
        (child, CHILD),
    ] {
        arena.nodes[node.0 as usize].tala_id = tala_id;
    }
    arena.nodes[outer.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[outer.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Row);
    arena.nodes[sequence.0 as usize].scoring_is_aggregate_vessel = true;
    arena.nodes[sequence.0 as usize].is_container = true;
    arena.node_order = vec![outer];
    arena.set_position(outer, Point { x: 100.0, y: 100.0 });
    arena.set_position(sequence, Point { x: 300.0, y: 400.0 });
    arena.set_position(peer, Point { x: 500.0, y: 500.0 });
    arena.set_position(
        first,
        Point {
            x: -100.0,
            y: -100.0,
        },
    );
    arena.set_position(
        second,
        Point {
            x: -200.0,
            y: -200.0,
        },
    );
    arena.set_position(child, Point { x: 360.0, y: 460.0 });

    let sequence_projection = arena.nodes[sequence.0 as usize].clone();
    arena
        .transaction_external_containers
        .push(sequence_projection.clone());
    arena
        .transaction_external_container_children
        .insert(SEQUENCE_VESSEL, vec![arena.nodes[child.0 as usize].clone()]);
    arena.transaction_external_aggregate_children.insert(
        OUTER_VESSEL,
        vec![sequence_projection, arena.nodes[peer.0 as usize].clone()],
    );
    arena.transaction_external_aggregate_children.insert(
        SEQUENCE_VESSEL,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
        ],
    );
    arena.transaction_external_cluster_layouts.insert(
        OUTER_VESSEL,
        ExternalClusterLayout {
            arrangement: ClusterArrangement::Row,
            padding: 20.0,
            fixed_size: true,
        },
    );

    arena.reposition_ordinary_containers();

    assert_eq!(
        arena.nodes[outer.0 as usize].rect.size,
        Size {
            width: 300.0,
            height: 140.0
        },
        "outer Cluster.Resize must observe the sequence container's refitted shared Box"
    );
    assert_eq!(
        arena.nodes[sequence.0 as usize].rect.size,
        Size {
            width: 165.0,
            height: 40.0
        },
        "the later Sequence.resize must still publish after cluster sync"
    );
    let steps = &arena.transaction_external_aggregate_children[&SEQUENCE_VESSEL];
    assert_eq!(steps[0].position, Some(Point { x: 100.0, y: 100.0 }));
    assert_eq!(steps[1].position, Some(Point { x: 145.0, y: 100.0 }));
}

#[test]
fn mirrored_external_children_are_reflected_and_refitted_in_the_container_frame() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 400.0, 400.0));
    let first = input.add_node(node("first", 100.0, 50.0));
    let second = input.add_node(node("second", 200.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[container.0 as usize].is_container = true;
    arena.set_position(
        container,
        Point {
            x: 1000.0,
            y: 2000.0,
        },
    );

    let mut first_external = arena.nodes[first.0 as usize].clone();
    first_external.position = Some(Point {
        x: -500.0,
        y: -400.0,
    });
    first_external.rect.origin = first_external.position.unwrap();
    let mut second_external = arena.nodes[second.0 as usize].clone();
    second_external.position = Some(Point {
        x: -500.0,
        y: -300.0,
    });
    second_external.rect.origin = second_external.position.unwrap();

    let mirrored = Pipeline::mirrored_external_container_children(
        &arena,
        container,
        &[first_external, second_external],
        (true, true),
    );

    assert_eq!(
        mirrored[0].position,
        Some(Point {
            x: 1160.0,
            y: 2210.0
        })
    );
    assert_eq!(
        mirrored[1].position,
        Some(Point {
            x: 1060.0,
            y: 2060.0
        })
    );
}

#[test]
fn mirrored_nested_children_keep_directed_absolute_half_rounding() {
    let mut input = Graph::default();
    let mut container_node = node("container", 411.0, 372.0);
    container_node.label_size = Some(Size {
        width: 177.0,
        height: 55.0,
    });
    container_node.label_position = LabelPosition::InsideTopCenter;
    let container = input.add_node(container_node);
    let inlet = input.add_node(node("inlet", 222.0, 69.0));
    let vessel = input.add_node(node("vessel", 291.0, 158.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[container.0 as usize].is_container = true;
    let container_position = Point {
        x: 1_470.0,
        y: 3_093.0,
    };
    arena.set_position(container, container_position);

    let mut inlet_external = arena.nodes[inlet.0 as usize].clone();
    inlet_external.position = Some(Point {
        x: -1_821.0,
        y: -3_223.0,
    });
    inlet_external.rect.origin = inlet_external.position.unwrap();
    let mut vessel_external = arena.nodes[vessel.0 as usize].clone();
    vessel_external.position = Some(Point {
        x: -1_821.0,
        y: -3_401.0,
    });
    vessel_external.rect.origin = vessel_external.position.unwrap();

    let directed_tala_ids = BTreeSet::from([arena.nodes[container.0 as usize].tala_id]);
    let projected_vessel_tala_ids = BTreeSet::from([vessel_external.tala_id]);
    assert!(Pipeline::external_children_have_hidden_aggregate(
        &[inlet_external.clone(), vessel_external.clone()],
        &directed_tala_ids,
        &projected_vessel_tala_ids,
    ));
    assert!(
        !Pipeline::external_children_have_hidden_aggregate(
            &[inlet_external.clone()],
            &directed_tala_ids,
            &projected_vessel_tala_ids,
        ),
        "an aggregate in another container must not authorize this container's copyback"
    );

    let mirrored = Pipeline::mirrored_external_container_children(
        &arena,
        container,
        &[inlet_external, vessel_external],
        (true, true),
    );
    let offsets = mirrored
        .iter()
        .map(|child| {
            let position = child.position.unwrap();
            Point {
                x: position.x - container_position.x,
                y: position.y - container_position.y,
            }
        })
        .collect::<Vec<_>>();

    assert_eq!(
        offsets,
        vec![Point { x: 129.0, y: 65.0 }, Point { x: 60.0, y: 154.0 }],
        "nested copyback must publish the offsets computed in the temporary graph's absolute frame"
    );
}

#[test]
fn undirected_cluster_projection_without_container_direction_has_no_direction_cost() {
    let mut input = Graph::default();
    let vessel = input.add_node(node("vessel", 100.0, 100.0));
    let adjacent = input.add_node(node("adjacent", 100.0, 100.0));
    let edge = input.add_edge(Edge {
        source: vessel,
        target: adjacent,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(vessel, Point { x: 0.0, y: 0.0 });
    arena.set_position(adjacent, Point { x: 200.0, y: 200.0 });
    arena.cell_size = 100.0;
    arena.set_turn_cost(50.0);
    arena.nodes[vessel.0 as usize].scoring_cluster_arrangement = Some(ClusterArrangement::Column);
    arena.sized_adjacent_overrides.insert(
        (adjacent, edge),
        ProjectedAdjacent {
            owner: vessel,
            tala_id: arena.nodes[vessel.0 as usize].tala_id,
            container_tala_id: None,
            offset: Point::default(),
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            cluster_member: true,
        },
    );

    // Recovered Node.edgeLength gates the direction term on
    // e.isDirected() || outerNode.GetContainerDirection() != geo.NONE.
    // Merely replacing an endpoint with a cluster member does not bypass
    // that gate.
    assert_eq!(
        arena.sized_edge_length(vessel, true),
        arena.sized_edge_length(vessel, false)
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn descendant_cluster_projection_scores_from_retained_vessel_boxes() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 138.0, 120.0));
    let carrier = input.add_node(node("carrier", 584.0, 272.0));
    let mut edges = Vec::new();
    for _ in 0..4 {
        let edge = input.add_edge(Edge {
            source,
            target: carrier,
        });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        edges.push(edge);
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(
        source,
        Point {
            x: 360.0,
            y: -180.0,
        },
    );
    arena.set_position(carrier, Point::default());
    arena.cell_size = 180.0;
    arena.set_turn_cost(304.0);
    for (edge, tala_id, leaf_offset, leaf_size, vessel_id, vessel_offset, vessel_size) in [
        (
            edges[0],
            101,
            Point { x: 370.0, y: 60.0 },
            Size {
                width: 154.0,
                height: 66.0,
            },
            201,
            Point { x: 370.0, y: 60.0 },
            Size {
                width: 154.0,
                height: 152.0,
            },
        ),
        (
            edges[1],
            102,
            Point { x: 370.0, y: 146.0 },
            Size {
                width: 154.0,
                height: 66.0,
            },
            201,
            Point { x: 370.0, y: 60.0 },
            Size {
                width: 154.0,
                height: 152.0,
            },
        ),
        (
            edges[2],
            103,
            Point { x: 60.0, y: 60.0 },
            Size {
                width: 155.0,
                height: 66.0,
            },
            202,
            Point { x: 60.0, y: 60.0 },
            Size {
                width: 155.0,
                height: 152.0,
            },
        ),
        (
            edges[3],
            104,
            Point { x: 60.0, y: 146.0 },
            Size {
                width: 155.0,
                height: 66.0,
            },
            202,
            Point { x: 60.0, y: 60.0 },
            Size {
                width: 155.0,
                height: 152.0,
            },
        ),
    ] {
        arena.sized_adjacent_overrides.insert(
            (source, edge),
            ProjectedAdjacent {
                owner: carrier,
                tala_id,
                container_tala_id: None,
                offset: leaf_offset,
                size: leaf_size,
                cluster_member: true,
            },
        );
        arena.sized_cluster_distance_boxes.insert(
            (carrier, tala_id),
            ProjectedClusterDistance {
                offset: vessel_offset,
                size: vessel_size,
                arrangement: ClusterArrangement::Column,
                vessel_tala_id: vessel_id,
                external_connected: Vec::new(),
            },
        );
    }

    let score = arena.sized_edge_length(source, true);
    assert!((score - 3_129.530_886_524_994_5).abs() < 1e-9);
}

#[test]
fn nearest_visibility_predecessor_retains_first_equal_trailing_edge() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 160.0, 160.0));
    let second = input.add_node(node("second", 160.0, 160.0));
    let target = input.add_node(node("target", 160.0, 160.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 128.0 });
    arena.set_position(second, Point { x: 256.0, y: 128.0 });
    arena.set_position(target, Point { x: 128.0, y: 512.0 });

    assert_eq!(
        arena.nearest_visibility_predecessor(
            target,
            false,
            true,
            &[(first, target), (second, target)],
        ),
        Some(first)
    );
}

#[test]
fn global_crossings_use_recovered_container_levels_and_endpoint_contact() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 0.0, 0.0));
    let add_positioned = |input: &mut Graph, name: &str, point: Point, parent: Option<NodeId>| {
        let mut positioned = node(name, 0.0, 0.0);
        positioned.locked_position = Some(point);
        positioned.parent = parent;
        input.add_node(positioned)
    };

    // These two child-level segments meet at geometrically identical
    // endpoints owned by distinct nodes. Recovered `doesCross` counts the
    // contact because its s/t bounds are inclusive.
    let a = add_positioned(&mut input, "a", Point { x: 0.0, y: 0.0 }, Some(container));
    let b = add_positioned(&mut input, "b", Point { x: 10.0, y: 10.0 }, Some(container));
    let c = add_positioned(&mut input, "c", Point { x: 10.0, y: 10.0 }, Some(container));
    let d = add_positioned(&mut input, "d", Point { x: 20.0, y: 0.0 }, Some(container));
    input.add_edge(Edge {
        source: a,
        target: b,
    });
    input.add_edge(Edge {
        source: c,
        target: d,
    });

    // This root-level segment geometrically crosses both child edges.
    // Graph.getGlobalEdgeCrossings groups by getContainerLevel, so neither
    // cross-level pair contributes.
    let root_a = add_positioned(&mut input, "root-a", Point { x: 10.0, y: -5.0 }, None);
    let root_b = add_positioned(&mut input, "root-b", Point { x: 10.0, y: 15.0 }, None);
    input.add_edge(Edge {
        source: root_a,
        target: root_b,
    });

    let arena = ArenaGraph::from_input(&input);
    assert_eq!(arena.global_edge_crossings(), 1);
}

#[test]
fn hierarchy_scope_abduction_and_network_simplex_match_pristine_nested_cycle() {
    // Pristine ARM64 capture:
    // a,d → b,e,f → c,g, with g's nested descendants inheriting level 2.
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 200.0));
    let add_child = |input: &mut Graph, name: &str| {
        let mut child = node(name, 40.0, 30.0);
        child.parent = Some(container);
        input.add_node(child)
    };
    let a = add_child(&mut input, "a");
    let b = add_child(&mut input, "b");
    let c = add_child(&mut input, "c");
    let d = add_child(&mut input, "d");
    let e = add_child(&mut input, "e");
    let f = add_child(&mut input, "f");
    let g = add_child(&mut input, "g");
    let mut h_node = node("h", 40.0, 30.0);
    h_node.parent = Some(g);
    let h = input.add_node(h_node);
    let mut i_node = node("i", 40.0, 30.0);
    i_node.parent = Some(h);
    let i = input.add_node(i_node);
    for (source, target) in [
        (a, b),
        (b, c),
        (c, a),
        (d, e),
        (e, i),
        (d, f),
        (f, h),
        (b, g),
    ] {
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
    }

    let mut arena = ArenaGraph::from_input(&input);
    let edges = arena.hierarchy_scope_edges(Some(container));
    assert!(edges.contains(&(e, g, true)));
    assert!(edges.contains(&(f, g, true)));
    let component = arena
        .hierarchy_components(&[a, b, c, d, e, f, g], &edges)
        .into_iter()
        .next()
        .expect("one connected hierarchy component");
    assert_eq!(component, vec![a, b, c, g, e, f, d]);
    let rank = arena
        .rank_hierarchy_component(&component, &edges, false)
        .expect("same general hierarchy TALA accepted");
    assert_eq!(rank.level_count, 3);
    assert_eq!(rank.level[&a], 0);
    assert_eq!(rank.level[&b], 1);
    assert_eq!(rank.level[&c], 2);
    assert_eq!(rank.level[&d], 0);
    assert_eq!(rank.level[&e], 1);
    assert_eq!(rank.level[&f], 1);
    assert_eq!(rank.level[&g], 2);
    let mut placement = hierarchy_placement::HierarchyPlacement::build(
        &arena,
        &component,
        &rank,
        &mut go_rng::GoRng::new(1),
    );
    let placement_points = placement.place_ranked_nodes(&mut arena);
    // `placeDescendants` establishes a concrete nested leaf coordinate
    // before `syncContainers` reconstructs h and g around it.
    assert!(placement_points.contains_key(&i));

    // The same component is the one automatic hierarchy TALA accepted
    // in the pristine ARM64 corpus capture.  This exercises eligibility,
    // nested-container recursion, component splitting, and validation as
    // one general rule rather than a fixture-shaped special case.
    let automatic = arena.automatic_hierarchy_candidates();
    assert_eq!(automatic.len(), 1);
    assert_eq!(
        automatic[0].0.iter().copied().collect::<BTreeSet<_>>(),
        component.iter().copied().collect()
    );
    assert_eq!(automatic[0].1.level_count, 3);
    assert_eq!(automatic[0].1.level[&g], 2);

    arena.assign_automatic_hierarchies();
    let membership = arena.nodes[a.0 as usize]
        .hierarchy
        .clone()
        .expect("accepted hierarchy is materialized");
    assert_eq!(membership.scope, Some(container));
    assert_eq!(membership.level(), 0);
    let g_membership = arena.nodes[g.0 as usize].hierarchy.as_ref().unwrap();
    assert!(g_membership.record.same_allocation(&membership.record));
    assert_eq!(g_membership.scope, Some(container));
    assert_eq!(g_membership.level(), 2);
    assert_eq!(g_membership.level_count(), 3);
    // Descendants share the carrier but do not become root placement
    // members of the outer hierarchy.
    assert_eq!(
        arena.nodes[i.0 as usize].hierarchy.clone(),
        arena.nodes[g.0 as usize].hierarchy.clone()
    );
}

#[test]
fn hierarchy_recognition_preserves_distinct_abducted_endpoints() {
    // Pinned hierarchy/discovery.go:is1N1 compares OriginallyFrom/To before
    // rejecting three-rank components. D43's D2 witness is
    // n; id->x->y.z; id->x.d->y. The two sink endpoints must remain distinct.
    for nested in [false, true] {
        for reverse in [false, true] {
            for distinct_sink in [false, true] {
                let mut input = Graph::default();
                let parent = nested.then(|| input.add_node(node("scope", 100.0, 100.0)));
                let mut add = |name: &str, parent| {
                    let mut value = node(name, 54.0, 66.0);
                    value.parent = parent;
                    input.add_node(value)
                };
                let isolated = add("n", parent);
                let source = add("id", parent);
                let middle = add("x", parent);
                let middle_child = add("x.d", Some(middle));
                let sink = add("y", parent);
                let sink_child = add("y.z", Some(sink));
                for (from, to) in [
                    (source, middle),
                    (middle, sink_child),
                    (source, middle_child),
                    (middle_child, if distinct_sink { sink } else { sink_child }),
                ] {
                    let (from, to) = if reverse { (to, from) } else { (from, to) };
                    let edge = input.add_edge(Edge {
                        source: from,
                        target: to,
                    });
                    input.set_edge_arrows(
                        edge,
                        EdgeArrows {
                            source: false,
                            target: true,
                        },
                    );
                }
                let mut arena = ArenaGraph::from_input(&input);
                arena.assign_automatic_hierarchies();
                assert!(arena.nodes[isolated.0 as usize].hierarchy.is_none());
                for (node, level) in [(source, 0), (middle, 1), (sink, 2)] {
                    let membership = arena.nodes[node.0 as usize].hierarchy.as_ref();
                    assert_eq!(membership.is_some(), distinct_sink, "nested={nested}");
                    if let Some(membership) = membership {
                        assert_eq!(membership.scope, parent);
                        assert_eq!(membership.level(), if reverse { 2 - level } else { level });
                        assert_eq!(membership.level_count(), 3);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_descendants_do_not_trigger_branched_workflow_exception() {
    let mut input = Graph::default();
    let scope = input.add_node(node("scope", 300.0, 300.0));
    let mut members = Vec::new();
    for name in ["first", "second", "nested", "fourth", "last"] {
        let mut member = node(name, 50.0, 30.0);
        member.parent = Some(scope);
        members.push(input.add_node(member));
    }
    let mut descendant = node("nested.leaf", 40.0, 30.0);
    descendant.parent = Some(members[2]);
    input.add_node(descendant);

    for pair in members.windows(2) {
        let edge = input.add_edge(Edge {
            source: pair[0],
            target: pair[1],
        });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
    }

    let arena = ArenaGraph::from_input(&input);
    let scope_edges = arena.hierarchy_scope_edges(Some(scope));
    let component = arena
        .hierarchy_components(&members, &scope_edges)
        .into_iter()
        .next()
        .expect("the directed chain is connected");
    assert_eq!(component.len(), 5);
    assert_eq!(arena.descendants(members[2]).len(), 1);

    assert!(
        arena
            .rank_hierarchy_component(&component, &scope_edges, false)
            .is_none()
    );
}

#[test]
fn hierarchical_container_rejects_an_edge_between_its_descendants() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 200.0));
    let mut first_node = node("first", 40.0, 30.0);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 40.0, 30.0);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);
    input.add_edge(Edge {
        source: first,
        target: second,
    });

    let arena = ArenaGraph::from_input(&input);
    assert!(!arena.is_hierarchical_container(container));
}

#[test]
fn cross_scope_edges_contribute_to_tree_fringe_degree() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 300.0));
    let outside = input.add_node(node("outside", 60.0, 40.0));
    let add_child = |input: &mut Graph, name: &str| {
        let mut child = node(name, 40.0, 30.0);
        child.parent = Some(container);
        input.add_node(child)
    };
    let first = add_child(&mut input, "first");
    let second = add_child(&mut input, "second");
    let third = add_child(&mut input, "third");
    let fourth = add_child(&mut input, "fourth");

    for (source, target) in [
        (first, second),
        (second, third),
        (third, fourth),
        (first, outside),
        (third, outside),
        (fourth, outside),
    ] {
        input.add_edge(Edge { source, target });
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();

    assert!(
        [first, second, third, fourth]
            .into_iter()
            .all(|child| !arena.tree_routing_nodes.contains_key(&child))
    );
}

#[test]
fn preprocess_trees_counts_self_loops_in_fringe_degree_and_preserves_order() {
    let mut input = Graph::default();
    let a = input.add_node(node("a", 40.0, 30.0));
    let b = input.add_node(node("b", 40.0, 30.0));
    let c = input.add_node(node("c", 40.0, 30.0));
    let d = input.add_node(node("d", 40.0, 30.0));
    let a_b = input.add_edge(Edge {
        source: a,
        target: b,
    });
    let b_c = input.add_edge(Edge {
        source: b,
        target: c,
    });
    let b_loop = input.add_edge(Edge {
        source: b,
        target: b,
    });
    let c_d = input.add_edge(Edge {
        source: c,
        target: d,
    });
    let d_loop = input.add_edge(Edge {
        source: d,
        target: d,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();

    // Go's Node.Edges contains a self-loop once, so b and d are not peeled
    // as leaves. Only a is restored after the b-c-d core is processed.
    assert_eq!(arena.graph_node_order(), vec![b, c, d, a]);
    assert_eq!(
        arena.edge_order,
        vec![b_c, b_loop, c_d, d_loop, a_b],
        "recovered graph edge order follows surviving core edges before restored tree edges"
    );
}

#[test]
fn tree_extraction_uses_serialized_container_order_for_branch_roots() {
    let mut input = Graph::default();
    let root = input.add_node(node("root", 40.0, 30.0));
    let mut peer_node = node("peer", 40.0, 30.0);
    peer_node.locked_position = Some(Point { x: 200.0, y: 0.0 });
    let peer = input.add_node(peer_node);
    // Dense IDs intentionally put the right branch before the left branch;
    // the serialized child slice below puts the left branch first.
    let right = input.add_node(node("right", 40.0, 30.0));
    let left = input.add_node(node("left", 40.0, 30.0));
    let left_first = input.add_node(node("left.first", 40.0, 30.0));
    let right_first = input.add_node(node("right.first", 40.0, 30.0));
    let left_second = input.add_node(node("left.second", 40.0, 30.0));
    let right_second = input.add_node(node("right.second", 40.0, 30.0));

    for (source, target) in [
        (root, peer),
        (root, left),
        (root, right),
        (left, left_first),
        (left, left_second),
        (right, right_first),
        (right, right_second),
    ] {
        input.add_edge(Edge { source, target });
    }
    input.set_hierarchy_children_order(BTreeMap::from([(
        None,
        vec![
            root,
            peer,
            left,
            left_first,
            left_second,
            right,
            right_first,
            right_second,
        ],
    )]));

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();

    assert!(
        arena.tree_routing_nodes[&left].extraction_order
            < arena.tree_routing_nodes[&right].extraction_order
    );
}

#[test]
fn preprocess_trees_preserves_recovered_graph_nodes_append_lifecycle() {
    let mut input = Graph::default();
    let pair_root = input.add_node(node("pair.root", 40.0, 30.0));
    let pair_leaf = input.add_node(node("pair.leaf", 40.0, 30.0));
    let chain_left = input.add_node(node("chain.left", 40.0, 30.0));
    let chain_middle = input.add_node(node("chain.middle", 40.0, 30.0));
    let chain_right = input.add_node(node("chain.right", 40.0, 30.0));
    for (source, target) in [
        (pair_root, pair_leaf),
        (chain_left, chain_middle),
        (chain_middle, chain_right),
    ] {
        input.add_edge(Edge { source, target });
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();

    // TALA removes peeled nodes from Graph.Nodes, appends the endpoint
    // promoted while rerooting the three-node chain, and only then appends
    // all restored non-branching nodes. The isolated pair is restored
    // before the rerooted chain.
    assert_eq!(
        arena.graph_node_order(),
        vec![pair_root, chain_left, pair_leaf, chain_middle, chain_right]
    );
    assert_eq!(
        arena.containers.nodes(&None),
        vec![pair_root, chain_left, pair_leaf, chain_middle, chain_right]
    );
}

#[test]
fn place_trees_appends_nodes_in_reconnect_preorder_not_arena_order() {
    let mut input = Graph::default();
    // Deliberately create nodes in an order different from the recovered
    // tree walk. Go appends nodes as reconnectTree visits each sorted branch.
    let alpha = input.add_node(node("alpha", 40.0, 30.0));
    let branch_a = input.add_node(node("branch_a", 40.0, 30.0));
    let beta = input.add_node(node("beta", 40.0, 30.0));
    let mut middle_node = node("middle", 40.0, 30.0);
    middle_node.locked_position = Some(Point::default());
    let middle = input.add_node(middle_node);
    let branch_b = input.add_node(node("branch_b", 40.0, 30.0));
    let gamma = input.add_node(node("gamma", 40.0, 30.0));
    let delta = input.add_node(node("delta", 40.0, 30.0));
    for (source, target) in [
        (alpha, branch_a),
        (beta, branch_a),
        (branch_a, middle),
        (middle, branch_b),
        (gamma, branch_b),
        (delta, branch_b),
    ] {
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();
    assert_eq!(arena.graph_node_order(), vec![middle]);

    arena.place_trees(&[middle]);

    assert_eq!(
        arena.graph_node_order(),
        vec![middle, branch_b, gamma, delta, branch_a, alpha, beta]
    );
}

#[test]
fn placement_scope_replays_restored_tree_edges_after_retained_endpoint_edges() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 200.0));
    let outside_left = input.add_node(node("outside left", 40.0, 30.0));
    let outside_right = input.add_node(node("outside right", 40.0, 30.0));
    let add_child = |input: &mut Graph, name: &str| {
        let mut child = node(name, 40.0, 30.0);
        child.parent = Some(container);
        input.add_node(child)
    };
    let first = add_child(&mut input, "first");
    let hub = add_child(&mut input, "hub");
    let third = add_child(&mut input, "third");
    let fourth = add_child(&mut input, "fourth");

    let first_edge = input.add_edge(Edge {
        source: first,
        target: hub,
    });
    let third_edge = input.add_edge(Edge {
        source: hub,
        target: third,
    });
    input.add_edge(Edge {
        source: hub,
        target: fourth,
    });
    // These cross-scope edges prevent `fourth` and `hub` from becoming
    // fringe nodes during the original whole-graph ExtractTrees pass.
    input.add_edge(Edge {
        source: outside_left,
        target: fourth,
    });
    input.add_edge(Edge {
        source: hub,
        target: outside_right,
    });

    let mut pipeline = Pipeline::new(&input, 1, false, false);
    pipeline.run_preprocess_trees();

    assert_eq!(
        pipeline.graph.preprocessed_tree_children[&Some(container)],
        vec![hub, fourth, first, third]
    );
    assert_eq!(
        pipeline.graph.restored_tree_edges,
        BTreeSet::from([first_edge, third_edge])
    );

    let placement = pipeline
        .placement_scope_graph(Some(container))
        .expect("nonempty child scope");
    let old_by_new = placement
        .old_to_new
        .iter()
        .map(|(&old, &new)| (new, old))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        placement
            .graph
            .nodes()
            .map(|(node, _)| old_by_new[&node])
            .collect::<Vec<_>>(),
        vec![hub, fourth, first, third]
    );
}

#[test]
fn direct_parent_endpoint_does_not_create_a_recursive_herd() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 300.0, 200.0));
    let add_child = |input: &mut Graph, name: &str, container| {
        let mut child = node(name, 40.0, 30.0);
        child.parent = Some(container);
        input.add_node(child)
    };
    let local = add_child(&mut input, "local", parent);
    let direct_uncle = add_child(&mut input, "direct uncle", parent);
    let other_uncle = add_child(&mut input, "other uncle", parent);
    let first = add_child(&mut input, "first", local);
    let second = add_child(&mut input, "second", local);
    let cousin_parent = add_child(&mut input, "cousin parent", other_uncle);
    let cousin = add_child(&mut input, "cousin", cousin_parent);

    // At the parent recursion, only the local endpoints are abducted on these
    // two edges; the direct uncle remains the edge's concrete endpoint. TALA
    // consequently leaves OriginallyFrom nil and groupSheep cannot herd the
    // local pair. The final edge proves that a genuine two-sided parent
    // abduction is still considered, but it forms only a one-node group.
    input.add_edge(Edge {
        source: direct_uncle,
        target: first,
    });
    input.add_edge(Edge {
        source: direct_uncle,
        target: second,
    });
    input.add_edge(Edge {
        source: first,
        target: cousin,
    });

    let mut pipeline = Pipeline::new(&input, 1, false, false);
    assign_test_parent_herds(&mut pipeline, local);

    assert!(!pipeline.graph.has_herd_assignment(first));
    assert!(!pipeline.graph.has_herd_assignment(second));
}

#[test]
fn production_herds_retain_distinct_records_and_canonical_pair_maps() {
    let mut input = Graph::default();
    let local = input.add_node(node("local", 200.0, 120.0));
    let uncle_a = input.add_node(node("uncle a", 200.0, 120.0));
    let uncle_b = input.add_node(node("uncle b", 200.0, 120.0));
    let mut add_child = |name: &str, parent| {
        let mut child = node(name, 40.0, 30.0);
        child.parent = Some(parent);
        input.add_node(child)
    };
    let first = add_child("first", local);
    let second = add_child("second", local);
    let cousin_a = add_child("cousin a", uncle_a);
    let cousin_b = add_child("cousin b", uncle_b);
    let copied = add_child("copied", local);
    for cousin in [cousin_a, cousin_b] {
        for child in [first, second] {
            input.add_edge(Edge {
                source: child,
                target: cousin,
            });
        }
    }
    let mut pipeline = Pipeline::new(&input, 1, false, false);
    // Source pairing keys are node pointers, even when entity IDs coincide.
    pipeline.graph.nodes[uncle_b.0 as usize].tala_id =
        pipeline.graph.nodes[uncle_a.0 as usize].tala_id;
    for cousin in [cousin_a, cousin_b] {
        pipeline.graph.set_position(cousin, Point::default());
        pipeline.graph.create_node_herd(cousin, Orientation::Top);
    }
    assign_test_parent_herds(&mut pipeline, local);
    let first_record = pipeline.graph.owner_node_herd(first).unwrap().unwrap();
    let second_record = pipeline.graph.owner_node_herd(second).unwrap().unwrap();
    assert!(!first_record.same_allocation(&second_record));
    let fields = pipeline.graph.owner_herd_fields(&first_record).unwrap();
    assert_eq!(fields.orientation, 6); // Go geo.Bottom.
    assert_eq!(
        fields
            .opposite_side
            .keys()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([Some(uncle_a), Some(uncle_b)])
    );
    assert!(!fields.same_side.is_nil());
    assert!(!fields.opposite_side.is_nil());
    let mut projection = pipeline.graph.herd_assignment_snapshot(first).unwrap();
    assert_eq!(projection.opposite_side_paired.len(), 2);
    projection.value = 123.5;
    pipeline
        .graph
        .publish_herd_projection(first, Some(projection));
    let published = pipeline.graph.owner_node_herd(first).unwrap().unwrap();
    assert!(first_record.same_allocation(&published));
    let published_fields = pipeline.graph.owner_herd_fields(&published).unwrap();
    assert_eq!(published_fields.value, 123.5);
    assert!(
        fields
            .same_side
            .same_map_allocation(&published_fields.same_side)
    );
    assert!(
        fields
            .opposite_side
            .same_map_allocation(&published_fields.opposite_side)
    );
    assert!(
        pipeline.graph.nodes[first.0 as usize]
            .herd_assignment
            .is_none()
    );

    pipeline.graph.copy_node_herd(first, copied);
    let copied_record = pipeline.graph.owner_node_herd(copied).unwrap().unwrap();
    assert!(!first_record.same_allocation(&copied_record));
    let copied_fields = pipeline.graph.owner_herd_fields(&copied_record).unwrap();
    assert_eq!(copied_fields.value, 123.5);
    assert_eq!(
        copied_fields
            .opposite_side
            .keys()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([Some(uncle_a), Some(uncle_b)])
    );
    assert!(
        !published_fields
            .same_side
            .same_map_allocation(&copied_fields.same_side)
    );
    assert!(
        !published_fields
            .opposite_side
            .same_map_allocation(&copied_fields.opposite_side)
    );
    pipeline.graph.record_node_herd_pair(copied, local, true);
    assert_eq!(
        pipeline
            .graph
            .owner_herd_fields(&first_record)
            .unwrap()
            .opposite_side
            .len(),
        2
    );
    assert_eq!(
        pipeline
            .graph
            .owner_herd_fields(&copied_record)
            .unwrap()
            .opposite_side
            .len(),
        3
    );
}

#[test]
fn recursive_herd_inherits_from_the_climbed_cousin_carrier() {
    let mut input = Graph::default();
    let local = input.add_node(node("local", 200.0, 120.0));
    let uncle = input.add_node(node("uncle", 200.0, 120.0));
    let add_child = |input: &mut Graph, name: &str, container| {
        let mut child = node(name, 40.0, 30.0);
        child.parent = Some(container);
        input.add_node(child)
    };
    let first = add_child(&mut input, "first", local);
    let second = add_child(&mut input, "second", local);
    let first_carrier = add_child(&mut input, "first carrier", uncle);
    let second_carrier = add_child(&mut input, "second carrier", uncle);
    let first_cousin = add_child(&mut input, "first cousin", first_carrier);
    let second_cousin = add_child(&mut input, "second cousin", second_carrier);
    input.add_edge(Edge {
        source: first_cousin,
        target: first,
    });
    input.add_edge(Edge {
        source: second_cousin,
        target: second,
    });

    let mut pipeline = Pipeline::new(&input, 1, false, false);
    // Recovered assignHerds processes an uncle only after its first child has
    // been placed by the earlier hierarchy scope.
    pipeline
        .graph
        .set_position(first_carrier, Point { x: 0.0, y: 0.0 });
    pipeline
        .graph
        .set_position(second_carrier, Point { x: 60.0, y: 0.0 });
    for carrier in [first_carrier, second_carrier] {
        pipeline.graph.nodes[carrier.0 as usize].herd_assignment = Some(HerdAssignment {
            orientation: Orientation::Top,
            value: 0.0,
            same_side_paired: BTreeSet::new(),
            opposite_side_paired: BTreeSet::new(),
        });
    }
    assign_test_parent_herds(&mut pipeline, local);

    for child in [first, second] {
        assert!(matches!(
            pipeline.graph.herd_assignment_snapshot(child),
            Some(HerdAssignment {
                orientation: Orientation::Bottom,
                ..
            })
        ));
    }

    // A wide uncle can use both horizontal faces. Once each cousin has more
    // opposite-side history than same-side history, TALA balances the next
    // scope onto the cousin's same side.
    pipeline.graph.nodes[uncle.0 as usize].rect.size.width = 300.0;
    for carrier in [first_carrier, second_carrier] {
        let assignment = pipeline.graph.nodes[carrier.0 as usize]
            .herd_assignment
            .as_mut()
            .expect("carrier assignment");
        assignment.opposite_side_paired.insert(42);
    }
    for child in [first, second] {
        pipeline.graph.nodes[child.0 as usize].herd_assignment = None;
    }
    assign_test_parent_herds(&mut pipeline, local);
    for child in [first, second] {
        assert!(matches!(
            pipeline.graph.herd_assignment_snapshot(child),
            Some(HerdAssignment {
                orientation: Orientation::Top,
                ..
            })
        ));
    }
}

#[test]
fn directed_tree_extraction_stops_at_conflicting_subtree_directions() {
    let mut input = Graph::default();
    let upstream_leaf = input.add_node(node("upstream leaf", 40.0, 30.0));
    let upstream = input.add_node(node("upstream", 40.0, 30.0));
    let junction = input.add_node(node("junction", 40.0, 30.0));
    let sentinel = input.add_node(node("sentinel", 40.0, 30.0));
    let trunk = input.add_node(node("trunk", 40.0, 30.0));
    let branch = input.add_node(node("branch", 40.0, 30.0));
    let junction_leaf = input.add_node(node("junction leaf", 40.0, 30.0));
    let trunk_leaf = input.add_node(node("trunk leaf", 40.0, 30.0));
    let branch_left = input.add_node(node("branch left", 40.0, 30.0));
    let branch_right = input.add_node(node("branch right", 40.0, 30.0));

    let edges = [
        (upstream_leaf, upstream),
        (upstream, junction),
        (junction, sentinel),
        (sentinel, trunk),
        (trunk, branch),
        (junction, junction_leaf),
        (trunk, trunk_leaf),
        (branch, branch_left),
        (branch, branch_right),
    ]
    .map(|(source, target)| {
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        edge
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();

    // The two roots peeled through `junction` have opposite dominant
    // directions, so recovered ExtractTrees promotes it to a sentinel.
    // The ordinary upstream path is restored; only the branching
    // downstream half remains in NodeToTree for PlaceTrees.
    assert!(
        [upstream_leaf, upstream, junction, sentinel]
            .into_iter()
            .all(|node| !arena.tree_routing_nodes.contains_key(&node))
    );
    assert_eq!(
        arena
            .tree_routing_nodes
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([trunk, trunk_leaf, branch, branch_left, branch_right])
    );
    assert_eq!(arena.tree_routing_nodes[&trunk].parent, sentinel);
    assert_eq!(arena.tree_routing_nodes[&trunk_leaf].parent, trunk);
    assert_eq!(arena.tree_routing_nodes[&branch].parent, trunk);
    assert_eq!(arena.tree_routing_nodes[&branch_left].parent, branch);
    assert_eq!(arena.tree_routing_nodes[&branch_right].parent, branch);
    assert_eq!(
        arena.graph_node_order(),
        vec![junction, sentinel, junction_leaf, upstream, upstream_leaf]
    );
    assert_eq!(
        arena.edge_order,
        vec![edges[2], edges[5], edges[1], edges[0]]
    );
    assert!(arena.tree_sentinels.contains(&sentinel));
    let members = arena.graph_node_order();
    let (temporary, old_by_new) = arena.induced_placement_subgraph(&members);
    let temporary_sentinel = NodeId(
        old_by_new
            .iter()
            .position(|node| *node == sentinel)
            .expect("sentinel remains in the ordinary component") as u32,
    );
    assert!(temporary.is_tree_sentinel(temporary_sentinel));
}

#[test]
fn sibling_tree_roots_remain_eligible_for_clustering() {
    let mut input = Graph::default();
    let main = input.add_node(node("main", 300.0, 300.0));
    let mut engine_node = node("engine", 200.0, 200.0);
    engine_node.parent = Some(main);
    let engine = input.add_node(engine_node);
    let mut peer_node = node("peer", 40.0, 40.0);
    peer_node.parent = Some(main);
    let peer = input.add_node(peer_node);
    let add_root = |input: &mut Graph, name: &str| {
        let mut root = node(name, 80.0, 40.0);
        root.parent = Some(engine);
        input.add_node(root)
    };
    let first = add_root(&mut input, "first");
    let second = add_root(&mut input, "second");
    let sentinel_edges = [first, second].map(|root| {
        input.add_edge(Edge {
            source: root,
            target: peer,
        })
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.tree_routing_nodes.clear();
    for (root, sentinel_edge) in [first, second].into_iter().zip(sentinel_edges) {
        arena.tree_routing_nodes.insert(
            root,
            TreeRoutingNode {
                parent: peer,
                sentinel_edge,
                orientation: Orientation::None,
                extraction_order: 0,
            },
        );
    }

    arena.assign_clusters(1, false);

    assert!(arena.nodes[first.0 as usize].cluster.is_some());
    assert_eq!(
        arena.nodes[first.0 as usize].cluster,
        arena.nodes[second.0 as usize].cluster
    );
}

#[test]
fn root_hierarchy_assigns_one_explicit_component_carrier() {
    let mut input = Graph {
        root_hierarchy: true,
        ..Graph::default()
    };
    let first = input.add_node(node("first", 40.0, 40.0));
    let second = input.add_node(node("second", 40.0, 40.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });

    let mut arena = ArenaGraph::from_input(&input);
    let first_membership = arena.nodes[first.0 as usize].hierarchy.clone().unwrap();
    let second_membership = arena.nodes[second.0 as usize].hierarchy.clone().unwrap();
    // TALA shares the hierarchy object but stores each node's level in
    // the hierarchy's level map. Rust carries that map entry alongside
    // the stable hierarchy ID, so only the carrier ID is shared.
    assert_eq!(first_membership.id, second_membership.id);
    assert_eq!(first_membership.scope, None);
    assert_eq!(second_membership.scope, None);
    assert_eq!(first_membership.level_count(), 2);
    assert_eq!(second_membership.level_count(), 2);
    assert_eq!(first_membership.level(), 0);
    assert_eq!(second_membership.level(), 1);
    assert!(arena.first_node_owns_hierarchy());
    let mut hierarchy_rng = go_rng::GoRng::new(1);
    let preview = arena
        .hierarchy_placement_preview(first_membership.id, &mut hierarchy_rng)
        .expect("ranked forced hierarchy has a placement graph");
    assert!(preview.contains_key(&first));
    assert!(preview.contains_key(&second));
    let mut apply_rng = go_rng::GoRng::new(1);
    assert!(arena.apply_hierarchy_placement(first_membership.id, &mut apply_rng));
    assert!(arena.position(first).is_some());
    assert!(arena.position(second).is_some());
}

#[test]
fn hierarchy_container_sync_wraps_nested_children_from_pristine_capture() {
    // Pristine ARM64 `syncContainers` capture from the accepted nested
    // grid hierarchy: i stays at (124,482), h wraps it at (64,422), and
    // g then wraps h at (4,362), with sixty-unit rectangle padding.
    let mut input = Graph::default();
    let g = input.add_node(node("g", 55.0, 71.0));
    let mut h_node = node("h", 52.0, 66.0);
    h_node.parent = Some(g);
    let h = input.add_node(h_node);
    let mut i_node = node("i", 49.0, 66.0);
    i_node.parent = Some(h);
    let i = input.add_node(i_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(i, Point { x: 124.0, y: 482.0 });

    arena.sync_hierarchy_containers(&[g]);

    assert_eq!(arena.position(i), Some(Point { x: 124.0, y: 482.0 }));
    assert_eq!(arena.position(h), Some(Point { x: 64.0, y: 422.0 }));
    assert_eq!(
        arena.nodes[h.0 as usize].rect.size,
        Size {
            width: 169.0,
            height: 186.0
        }
    );
    assert_eq!(arena.position(g), Some(Point { x: 4.0, y: 362.0 }));
    assert_eq!(
        arena.nodes[g.0 as usize].rect.size,
        Size {
            width: 289.0,
            height: 306.0
        }
    );
}

#[test]
fn hierarchy_container_sync_reserves_boundary_child_label_overhang() {
    // Pinned hierarchy/brandes_kopf.go syncContainers calls WrapChildren,
    // which expands a boundary child label before fitting the container.  A
    // horizontal hierarchy transpose can leave the right child one pixel
    // narrower than its centered label, as in the F13 nested-ID witness.
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 55.0, 71.0));
    let mut left_node = node("left", 66.0, 52.0);
    left_node.parent = Some(parent);
    let left = input.add_node(left_node);
    let mut right_node = node("right", 66.0, 52.0);
    right_node.parent = Some(parent);
    let right = input.add_node(right_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(left, Point::default());
    arena.set_position(right, Point { x: 186.0, y: 0.0 });
    arena.nodes[right.0 as usize].label_size = Some(Size {
        width: 67.0,
        height: 21.0,
    });

    arena.sync_hierarchy_containers(&[parent]);

    assert_eq!(
        arena.nodes[parent.0 as usize].rect.size,
        Size {
            width: 373.0,
            height: 172.0,
        }
    );
    assert_eq!(arena.position(left), Some(Point::default()));
    assert_eq!(arena.position(right), Some(Point { x: 186.0, y: 0.0 }));
}

#[test]
fn connected_nodes_follow_edges_and_container_hierarchy_without_crossing_exclusions() {
    let mut input = Graph::with_direction(Direction::Down);
    let network = input.add_node(node("network", 100.0, 100.0));
    let mut tower_node = node("tower", 40.0, 40.0);
    tower_node.parent = Some(network);
    let tower = input.add_node(tower_node);
    let user = input.add_node(node("user", 40.0, 40.0));
    let portal = input.add_node(node("portal", 100.0, 100.0));
    let mut ui_node = node("ui", 40.0, 40.0);
    ui_node.parent = Some(portal);
    let ui = input.add_node(ui_node);
    let processor = input.add_node(node("processor", 100.0, 100.0));
    let mut storage_node = node("storage", 40.0, 40.0);
    storage_node.parent = Some(processor);
    let storage = input.add_node(storage_node);
    let server = input.add_node(node("server", 40.0, 40.0));
    let logs = input.add_node(node("logs", 40.0, 40.0));

    for (source, target) in [
        (user, tower),
        (user, ui),
        (ui, server),
        // An excluded node's ancestor may be reached by an unrelated
        // edge, but must not bridge the two sides of the transaction.
        (ui, network),
        (server, processor),
        (server, logs),
    ] {
        input.add_edge(Edge { source, target });
    }
    let arena = ArenaGraph::from_input(&input);
    let connected = arena.connected_nodes_excluding(user, &BTreeSet::from([tower]));
    assert_eq!(
        connected,
        vec![user, ui, server, portal, processor, logs, storage]
    );
}

#[test]
fn tree_reconnection_orders_deep_fringe_edges_after_cyclic_cores() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 200.0));
    let root_b = input.add_node(node("root b", 40.0, 40.0));
    let root_c = input.add_node(node("root c", 40.0, 40.0));
    let root_fringe = input.add_node(node("root fringe", 40.0, 40.0));
    let add_child = |input: &mut Graph, name: &str| {
        let mut child = node(name, 30.0, 30.0);
        child.parent = Some(container);
        input.add_node(child)
    };
    let inner_fringe = add_child(&mut input, "inner fringe");
    let inner_a = add_child(&mut input, "inner a");
    let inner_b = add_child(&mut input, "inner b");

    let root_fringe_edge = input.add_edge(Edge {
        source: root_fringe,
        target: container,
    });
    let inner_fringe_edge = input.add_edge(Edge {
        source: inner_fringe,
        target: inner_a,
    });
    let inner_forward = input.add_edge(Edge {
        source: inner_a,
        target: inner_b,
    });
    let inner_reverse = input.add_edge(Edge {
        source: inner_b,
        target: inner_a,
    });
    let root_first = input.add_edge(Edge {
        source: container,
        target: root_b,
    });
    let root_second = input.add_edge(Edge {
        source: root_b,
        target: root_c,
    });
    let root_third = input.add_edge(Edge {
        source: root_c,
        target: container,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();
    arena.sync_edge_adjacency_order();

    assert_eq!(
        arena.edge_order,
        vec![
            inner_forward,
            inner_reverse,
            root_first,
            root_second,
            root_third,
            inner_fringe_edge,
            root_fringe_edge,
        ]
    );
    assert_eq!(
        arena.nodes[inner_a.0 as usize].edges,
        vec![inner_forward, inner_reverse, inner_fringe_edge]
    );
}

#[test]
fn branching_tree_copyback_positions_labels_and_publishes_owner_order() {
    let mut input = Graph::with_direction(Direction::Down);
    let mut sentinel_node = node("sentinel", 100.0, 60.0);
    sentinel_node.locked_position = Some(Point { x: 0.0, y: 0.0 });
    let sentinel = input.add_node(sentinel_node);
    let root = input.add_node(node("root", 100.0, 60.0));
    let left = input.add_node(node("left", 100.0, 60.0));
    let right = input.add_node(node("right", 100.0, 60.0));
    let root_edge = input.add_edge(Edge {
        source: sentinel,
        target: root,
    });
    let left_edge = input.add_edge(Edge {
        source: root,
        target: left,
    });
    let right_edge = input.add_edge(Edge {
        source: root,
        target: right,
    });
    for (edge, text) in [(left_edge, "left"), (right_edge, "right")] {
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: text.to_owned(),
                size: Size {
                    width: 30.0,
                    height: 13.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
    }

    let mut owner = ArenaGraph::from_input(&input);
    owner.preprocess_trees();
    assert!(owner.edge_order.is_empty());
    assert!(owner.is_tree_edge(root_edge));
    assert!(owner.is_tree_edge(left_edge));
    assert!(owner.is_tree_edge(right_edge));

    // Source appends tree children to the retained prefix header, including
    // an offset and spare capacity. A captured alias observes the appends,
    // while its own length and the preceding backing slot remain unchanged.
    let mut appending = owner.clone();
    let storage = SharedNodeSlice::with_len_cap(2, 8).unwrap();
    storage.write_capacity_slot(0, Some(right)).unwrap();
    storage.write_capacity_slot(1, Some(sentinel)).unwrap();
    storage.write_capacity_slot(7, Some(left)).unwrap();
    let prefix = storage.reslice(1, 2).unwrap();
    appending.containers.replace_header(None, prefix.clone());
    appending.publish_placed_tree_edges(&[sentinel], false);
    let appended = appending.containers.get_header(&None).unwrap();
    assert_eq!(
        appending.graph_node_order(),
        vec![sentinel, left, right, root]
    );
    assert_eq!(
        appending.containers.nodes(&None),
        vec![sentinel, root, left, right]
    );
    assert!(appended.same_backing_allocation(&prefix));
    assert_eq!((appended.offset(), appended.capacity()), (1, 7));
    assert_eq!(prefix.visible_slots(), vec![Some(sentinel)]);
    assert_eq!(
        prefix.capacity_slots(),
        vec![
            Some(sentinel),
            Some(root),
            Some(left),
            Some(right),
            None,
            None,
            Some(left)
        ]
    );
    assert_eq!(storage.read_capacity_slot(0), Some(Some(right)));

    let mut placed = owner.clone();
    placed.place_trees(&[sentinel]);
    for (edge_id, child) in [(left_edge, left), (right_edge, right)] {
        let label = placed.edge_records[edge_id.0 as usize]
            .label
            .as_ref()
            .unwrap();
        let child_position = placed.position(child).unwrap();
        let root_position = placed.position(root).unwrap();
        let dx = child_position.x + 50.0 - (root_position.x + 50.0);
        let dy = child_position.y - (root_position.y + 60.0);
        let total_length = dx.abs() + dy;
        let expected_percentage = (total_length - (dy - 50.0) * 0.5) / total_length;
        assert_eq!(label.percentage, expected_percentage);
        assert_eq!(
            label.position,
            if dx < 0.0 {
                LabelPosition::UnlockedBottom
            } else {
                LabelPosition::UnlockedTop
            }
        );
    }

    // The temporary graph reconnects edges root-first, while Graph.placeNodes
    // publishes the owner graph breadth-first and appends each root last.
    owner.containers.clone_from(&placed.containers);
    let mut directly_placed = owner.clone();
    directly_placed.containers.clone_from(&placed.containers);
    directly_placed.publish_placed_tree_edges(&[sentinel], true);
    assert_eq!(
        directly_placed.edge_order,
        vec![root_edge, left_edge, right_edge]
    );
    assert_eq!(
        directly_placed.graph_node_order(),
        vec![sentinel, root, left, right]
    );

    assert_eq!(
        owner.containers.nodes(&None),
        vec![sentinel, root, left, right]
    );
    let prior_header = owner.containers.get_header(&None).unwrap();
    owner.publish_placed_tree_edges(&[sentinel], false);
    assert_eq!(
        owner
            .owner_node_incident_header(sentinel)
            .unwrap()
            .iter_slots()
            .collect::<Vec<_>>(),
        vec![Some(root_edge)]
    );
    assert_eq!(
        owner
            .owner_node_incident_header(root)
            .unwrap()
            .iter_slots()
            .collect::<Vec<_>>(),
        vec![Some(root_edge), Some(left_edge), Some(right_edge)]
    );
    assert_eq!(
        owner
            .owner_node_incident_header(left)
            .unwrap()
            .iter_slots()
            .collect::<Vec<_>>(),
        vec![Some(left_edge)]
    );
    assert_eq!(
        owner
            .owner_node_incident_header(right)
            .unwrap()
            .iter_slots()
            .collect::<Vec<_>>(),
        vec![Some(right_edge)]
    );
    assert_eq!(owner.edge_order, vec![left_edge, right_edge, root_edge]);
    assert_eq!(owner.graph_node_order(), vec![sentinel, left, right, root]);
    assert_eq!(
        owner.containers.nodes(&None),
        vec![sentinel, root, left, right]
    );
    let published_header = owner.containers.get_header(&None).unwrap();
    assert!(published_header.same_backing_allocation(&prior_header));
    assert_eq!(
        published_header.capacity_slots(),
        prior_header.capacity_slots()
    );
    assert_eq!(published_header.capacity(), prior_header.capacity());
    owner.sync_edge_adjacency_order();
    assert_eq!(
        owner.nodes[root.0 as usize].edges,
        vec![root_edge, left_edge, right_edge]
    );
}

#[test]
fn tree_preprocessing_disconnects_live_incidents_before_optimizer_selection() {
    struct QuietContext;
    impl limits::Context for QuietContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            None
        }
    }
    for nested in [false, true] {
        let mut input = Graph::default();
        let container = nested.then(|| input.add_node(node("container", 100.0, 100.0)));
        let nodes = ["sentinel", "branch", "left", "right"].map(|name| {
            let mut value = node(name, 40.0, 30.0);
            value.parent = container;
            input.add_node(value)
        });
        let edges = [(0, 1), (1, 2), (1, 3)].map(|(from, to)| {
            input.add_edge(Edge {
                source: nodes[from],
                target: nodes[to],
            })
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner
            .materialize_owner_node_incident_header(nodes[0])
            .unwrap();
        let retained = owner.owner_node_incident_header(nodes[0]).unwrap();
        owner.preprocess_trees();
        assert!(owner.edge_order.is_empty());
        for node in nodes {
            assert_eq!(owner.owner_node_incident_header(node).unwrap().len(), 0);
        }
        assert_eq!(
            retained.get(0),
            Some(Some(edges[0])),
            "old one-slot header remains readable after source reslice"
        );
        let view = owner
            .create_owner_graph_view(vec![nodes[0]], vec![])
            .unwrap();
        let mut guard =
            limits::OptimizationWorkGuard::new(Some(&QuietContext), "sizelessOptimizer.setup", 100)
                .unwrap();
        assert!(
            !owner_near_eligibility::can_optimize_owner_node_guarded(
                &mut owner,
                Some(nodes[0]),
                Some(view),
                &mut guard
            )
            .unwrap()
        );
        owner.publish_placed_tree_edges(&[nodes[0]], true);
        assert_eq!(
            owner
                .owner_node_incident_header(nodes[1])
                .unwrap()
                .iter_slots()
                .collect::<Vec<_>>(),
            edges.map(Some)
        );
    }
}

#[test]
fn equal_sized_tree_placement_uses_dense_node_order_for_orientation() {
    let mut input = Graph::with_direction(Direction::Down);
    let mut sentinel_node = node("sentinel", 100.0, 60.0);
    sentinel_node.locked_position = Some(Point { x: 0.0, y: 0.0 });
    let sentinel = input.add_node(sentinel_node);
    // The first branch deliberately has a larger FNV-derived TALA ID than the
    // second. Go's placement sort uses dense Node.ID, so this branch must still
    // receive the first (Bottom) orientation.
    let first_root = input.add_node(node("z", 100.0, 60.0));
    let first_leaf = input.add_node(node("za", 100.0, 60.0));
    let second_root = input.add_node(node("a", 100.0, 60.0));
    let second_leaf = input.add_node(node("aa", 100.0, 60.0));
    let first_edge = input.add_edge(Edge {
        source: first_root,
        target: sentinel,
    });
    let first_leaf_edge = input.add_edge(Edge {
        source: first_leaf,
        target: first_root,
    });
    let second_edge = input.add_edge(Edge {
        source: sentinel,
        target: second_root,
    });
    let second_leaf_edge = input.add_edge(Edge {
        source: second_leaf,
        target: second_root,
    });
    for edge in [first_edge, first_leaf_edge, second_edge, second_leaf_edge] {
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
    }
    // GraphJSON supplies the durable layoutgraph EntityID explicitly.  Use
    // the same dense IDs here so this unit fixture exercises the upstream
    // comparator instead of the FNV fallback used for ordinary named nodes.
    input.tala_id_overrides = vec![Some(0), Some(1), Some(2), Some(3), Some(4)];

    let mut arena = ArenaGraph::from_input(&input);
    arena.preprocess_trees();
    arena.place_trees(&[sentinel]);

    assert_eq!(
        arena.tree_routing_nodes[&first_root].orientation,
        Orientation::Bottom
    );
    assert_eq!(
        arena.tree_routing_nodes[&second_root].orientation,
        Orientation::Top
    );
}

#[test]
fn branching_tree_without_explicit_direction_has_no_downward_score_preference() {
    let mut input = Graph::default();
    let root = input.add_node(node("root", 100.0, 60.0));
    let arena = ArenaGraph::from_input(&input);
    assert_eq!(arena.tree_placement_direction(root), Orientation::None);

    let mut directed = Graph::with_direction(Direction::Down);
    let directed_root = directed.add_node(node("root", 100.0, 60.0));
    let directed_arena = ArenaGraph::from_input(&directed);
    assert_eq!(
        directed_arena.tree_placement_direction(directed_root),
        Orientation::Bottom
    );
}

#[test]
fn temporary_graph_retains_direction_for_an_unmaterialized_container_key() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 60.0));
    let second = input.add_node(node("second", 100.0, 60.0));
    let mut arena = ArenaGraph::from_input(&input);
    let absent_container_tala_id = 42;
    arena.nodes[first.0 as usize].scoring_container_parent = Some(absent_container_tala_id);
    arena.nodes[second.0 as usize].scoring_container_parent = Some(absent_container_tala_id);
    arena
        .scoring_directions_by_tala
        .insert(Some(absent_container_tala_id), Direction::Right);

    assert!(arena.projected_outer_direction_is_set(None, None, first, second));
}

#[test]
fn projected_left_parent_preserves_sized_direction_cost() {
    let mut input = Graph::with_direction(Direction::Left);
    let source = input.add_node(node("source", 100.0, 60.0));
    let target = input.add_node(node("target", 100.0, 60.0));
    input.add_edge(Edge { source, target });
    let mut owner = ArenaGraph::from_input(&input);
    owner.set_position(source, Point { x: 0.0, y: 0.0 });
    owner.set_position(target, Point { x: 300.0, y: 100.0 });
    let expected = owner.sized_edge_length(source, true);
    let mut projected = owner.clone();
    projected.directions.insert(None, Direction::Right);
    projected.directions_by_tala.insert(None, Direction::Right);
    projected.scoring_directions_by_tala.clear();
    projected
        .scoring_directions_by_tala
        .insert(Some(42), Direction::Left);
    for node in &mut projected.nodes {
        node.scoring_container_parent = Some(42);
    }
    assert_eq!(
        projected.sized_edge_length(source, true).to_bits(),
        expected.to_bits()
    );
}

#[test]
fn placement_scope_shares_all_owner_direction_keys() {
    let mut input = Graph::default();
    let mut container_node = node("container", 100.0, 60.0);
    container_node.direction = Some(Direction::Right);
    let container = input.add_node(container_node);
    for name in ["first", "second"] {
        let mut child = node(name, 100.0, 60.0);
        child.parent = Some(container);
        input.add_node(child);
    }

    let pipeline = Pipeline::new(&input, 1, false, true);
    let container_tala_id = pipeline.graph.nodes[container.0 as usize].tala_id;
    let placement = pipeline
        .placement_scope_graph(Some(container))
        .expect("nonempty child scope");
    assert_eq!(
        placement
            .scoring_directions_by_tala
            .get(&Some(container_tala_id)),
        Some(&Direction::Right)
    );

    let placed = Pipeline::place_flat_scope(&placement, 1, true).unwrap();
    assert_eq!(
        placed
            .graph
            .scoring_directions_by_tala
            .get(&Some(container_tala_id)),
        Some(&Direction::Right)
    );
}

#[test]
fn external_nil_container_stays_nil_with_large_projected_carrier() {
    for count in [1, 65] {
        let mut input = Graph::default();
        for index in 0..count {
            input.add_node(node(&format!("n{index}"), 50.0, 50.0));
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_projected_transaction_nodes_from_current_graph();
        let mut container = graph.nodes[0].clone();
        container.tala_id = 9_000_000_000;
        container.input_id = NodeId(u32::MAX);
        container.is_container = true;
        container.position = None;
        let mut child = graph.nodes[0].clone();
        child.position = Some(Point { x: 60.0, y: 60.0 });
        child.rect.origin = child.position.unwrap();
        graph
            .transaction_external_container_children
            .insert(container.tala_id, vec![child]);
        graph.transaction_external_containers.push(container);

        graph.reposition_ordinary_containers_without_sync();
        assert_eq!(
            graph.transaction_external_containers[0].position, None,
            "carrier count {count}"
        );
    }
}

#[test]
fn placement_scope_preserves_live_directed_container_child_order() {
    let mut input = Graph::default();
    let outer = input.add_node(node("outer", 100.0, 60.0));
    let mut container_node = node("container", 100.0, 60.0);
    container_node.parent = Some(outer);
    container_node.direction = Some(Direction::Down);
    let container = input.add_node(container_node);
    let children = ["first", "second", "third", "fourth"].map(|name| {
        let mut child = node(name, 50.0, 50.0);
        child.parent = Some(container);
        input.add_node(child)
    });
    let pipeline = Pipeline::new(&input, 1, false, true);
    // Tree restoration can replace the declaration-order slice before the
    // parent creates its placement scope. Go shares this live slice verbatim.
    let live_order = vec![children[1], children[3], children[0], children[2]];
    pipeline
        .graph
        .containers
        .insert(Some(container), live_order.clone());
    let container_id = pipeline.graph.nodes[container.0 as usize].tala_id;
    let placement = pipeline.placement_scope_graph(Some(outer)).unwrap();
    assert_eq!(
        placement.transaction_external_container_children[&container_id]
            .iter()
            .map(|child| child.tala_id)
            .collect::<Vec<_>>(),
        live_order
            .iter()
            .map(|child| pipeline.graph.nodes[child.0 as usize].tala_id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn placement_scope_retains_children_of_an_unpositioned_container() {
    let mut input = Graph::default();
    let outer = input.add_node(node("outer", 100.0, 60.0));
    let mut container_node = node("container", 471.0, 186.0);
    container_node.parent = Some(outer);
    container_node.label_size = Some(Size {
        width: 180.0,
        height: 31.0,
    });
    container_node.label_position = LabelPosition::InsideMiddleRight;
    let container = input.add_node(container_node);
    let mut child_node = node("child", 173.0, 66.0);
    child_node.parent = Some(container);
    let child = input.add_node(child_node);

    let mut pipeline = Pipeline::new(&input, 1, false, true);
    pipeline
        .graph
        .set_position(child, Point { x: 108.0, y: 60.0 });
    assert_eq!(pipeline.graph.position(container), None);

    let placement = pipeline
        .placement_scope_graph(Some(outer))
        .expect("nonempty outer scope");
    let container_tala_id = pipeline.graph.nodes[container.0 as usize].tala_id;
    let child_tala_id = pipeline.graph.nodes[child.0 as usize].tala_id;
    let projected_children = placement
        .transaction_external_container_children
        .get(&container_tala_id)
        .expect("unpositioned container retains its Graph.Containers entry");
    assert_eq!(projected_children.len(), 1);
    assert_eq!(projected_children[0].tala_id, child_tala_id);
}

#[test]
fn nonisolated_tree_root_edge_remains_available_to_ordinary_stages() {
    let mut input = Graph::default();
    let mut sentinel_node = node("sentinel", 100.0, 60.0);
    sentinel_node.locked_position = Some(Point { x: 0.0, y: 0.0 });
    let sentinel = input.add_node(sentinel_node);
    let root = input.add_node(node("root", 100.0, 60.0));
    let leaf = input.add_node(node("leaf", 100.0, 60.0));
    let mut peer_node = node("peer", 100.0, 60.0);
    peer_node.locked_position = Some(Point { x: 300.0, y: 0.0 });
    let peer = input.add_node(peer_node);
    let root_edge = input.add_edge(Edge {
        source: sentinel,
        target: root,
    });
    let internal_edge = input.add_edge(Edge {
        source: root,
        target: leaf,
    });
    let ordinary_edge = input.add_edge(Edge {
        source: sentinel,
        target: peer,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.tree_routing_nodes.insert(
        root,
        TreeRoutingNode {
            parent: sentinel,
            sentinel_edge: root_edge,
            orientation: Orientation::None,
            extraction_order: 0,
        },
    );
    arena.tree_routing_nodes.insert(
        leaf,
        TreeRoutingNode {
            parent: root,
            sentinel_edge: internal_edge,
            orientation: Orientation::None,
            extraction_order: 1,
        },
    );

    // Publish real Tree allocations and NodeToTree, as preprocessing does.
    arena.publish_preprocessed_owner_trees();

    assert!(!arena.is_tree_edge(root_edge));
    assert!(arena.edge_touches_tree_node(root_edge));
    assert!(arena.is_tree_edge(internal_edge));
    assert!(arena.edge_touches_tree_node(internal_edge));
    assert!(!arena.is_tree_edge(ordinary_edge));
    assert!(!arena.edge_touches_tree_node(ordinary_edge));

    arena.nodes[sentinel.0 as usize].position = Some(Point { x: 0.0, y: 0.0 });
    arena.nodes[root.0 as usize].position = Some(Point { x: 300.0, y: 100.0 });
    arena.nodes[leaf.0 as usize].position = Some(Point { x: 500.0, y: 100.0 });
    arena.nodes[peer.0 as usize].position = Some(Point { x: 300.0, y: 0.0 });
    let root_route = vec![
        Point { x: 100.0, y: 30.0 },
        Point { x: 150.0, y: 30.0 },
        Point { x: 150.0, y: 130.0 },
        Point { x: 300.0, y: 130.0 },
    ];
    arena.edge_records[root_edge.0 as usize]
        .route
        .replace(RoutePoints::literal(&(root_route.clone())));
    arena.edge_records[internal_edge.0 as usize]
        .route
        .replace(RoutePoints::literal(
            &[Point { x: 400.0, y: 130.0 }, Point { x: 500.0, y: 130.0 }],
        ));
    arena.edge_records[ordinary_edge.0 as usize]
        .route
        .replace(RoutePoints::literal(
            &[Point { x: 100.0, y: 30.0 }, Point { x: 300.0, y: 30.0 }],
        ));

    routing::balance_edge_segments(&mut arena);

    assert_eq!(
        arena.edge_records[root_edge.0 as usize].route.read(),
        root_route,
        "BalanceEdgeSegments classifies every NodeToTree endpoint as special, even when its root sentinel edge is routed ordinarily"
    );
}

#[test]
fn nonisolated_tree_root_edge_uses_the_ordinary_router() {
    let mut input = Graph::default();
    let sentinel = input.add_node(node("sentinel", 100.0, 60.0));
    let root = input.add_node(node("root", 100.0, 60.0));
    let peer = input.add_node(node("peer", 100.0, 60.0));
    let root_edge = input.add_edge(Edge {
        source: sentinel,
        target: root,
    });
    input.add_edge(Edge {
        source: sentinel,
        target: peer,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[sentinel.0 as usize].position = Some(Point { x: 200.0, y: 200.0 });
    arena.nodes[root.0 as usize].position = Some(Point { x: 0.0, y: 0.0 });
    arena.nodes[peer.0 as usize].position = Some(Point { x: 400.0, y: 200.0 });
    arena.tree_routing_nodes.insert(
        root,
        TreeRoutingNode {
            parent: sentinel,
            sentinel_edge: root_edge,
            orientation: Orientation::Top,
            extraction_order: 0,
        },
    );
    assert!(!arena.is_tree_edge(root_edge));

    let routed_with_tree_metadata = routing::route_edges(&arena);
    arena.tree_routing_nodes.clear();
    let routed_as_ordinary_graph = routing::route_edges(&arena);
    assert_eq!(
        routed_with_tree_metadata[root_edge.0 as usize],
        routed_as_ordinary_graph[root_edge.0 as usize],
        "a non-isolated root sentinel edge must not enter the tree pre-router"
    );
}

#[test]
fn root_bin_pack_keeps_one_connected_root_group_atomic() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 200.0));
    let start = input.add_node(node("start", 20.0, 20.0));
    let search = input.add_node(node("search", 80.0, 40.0));
    let mut first_child = node("first child", 40.0, 40.0);
    first_child.parent = Some(container);
    let first_child = input.add_node(first_child);
    let mut second_child = node("second child", 40.0, 40.0);
    second_child.parent = Some(container);
    let second_child = input.add_node(second_child);
    input.add_edge(Edge {
        source: start,
        target: container,
    });
    input.add_edge(Edge {
        source: container,
        target: search,
    });
    input.add_edge(Edge {
        source: first_child,
        target: second_child,
    });

    let mut arena = ArenaGraph::from_input(&input);
    for (node, position) in [
        (start, Point { x: 0.0, y: 0.0 }),
        (container, Point { x: 100.0, y: 0.0 }),
        (first_child, Point { x: 140.0, y: 60.0 }),
        (second_child, Point { x: 220.0, y: 60.0 }),
        (search, Point { x: 340.0, y: 0.0 }),
    ] {
        arena.set_position(node, position);
    }
    let before = arena
        .nodes
        .iter()
        .map(|node| node.position)
        .collect::<Vec<_>>();

    arena.bin_pack_root().unwrap();

    assert_eq!(
        arena
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>(),
        before
    );
}

fn recovered_ent2d2_right_input() -> Graph {
    let mut input = Graph::with_direction(Direction::Right);
    let dimensions = [
        ("User", 201.0, 144.0),
        ("Pet", 194.0, 108.0),
        ("Card", 194.0, 108.0),
        ("Post", 222.0, 144.0),
        ("Metadata", 146.0, 108.0),
        ("Info", 308.0, 108.0),
    ];
    let ids: Vec<_> = dimensions
        .into_iter()
        .map(|(name, width, height)| {
            let mut value = node(name, width, height);
            value.shape = ShapeKind::SqlTable;
            input.add_node(value)
        })
        .collect();
    // e2etests/testdata/files/ent2d2_{right,basic}.d2 declares these
    // columns. The adapter carries their counts explicitly; box height is
    // not a substitute for the shape's semantic table state.
    for (&id, columns) in ids.iter().zip([3, 2, 2, 3, 2, 2]) {
        input.set_table_column_count(id, Some(columns));
    }
    let targets = [ids[0], ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]];
    let target_arrowheads = [
        "cf-one", "cf-many", "cf-many", "cf-one", "cf-many", "cf-many", "cf-many",
    ];
    for (target, target_arrowhead) in targets.into_iter().zip(target_arrowheads) {
        let edge = input.add_edge(super::super::Edge {
            source: ids[0],
            target,
        });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: true,
                target: true,
            },
        );
        input.set_edge_arrowheads(
            edge,
            crate::EdgeArrowheads {
                source: Some("cf-one-required".into()),
                target: Some(target_arrowhead.into()),
            },
        );
    }
    input
}

fn recovered_ent2d2_labeled_unset_input() -> Graph {
    let mut input = recovered_ent2d2_right_input();
    input.explicit_direction = None;
    let labels = [
        ("spouse", 48.0),
        ("children/parent/ancestor", 167.0),
        ("pets/owner", 77.0),
        ("card/owner", 78.0),
        ("posts/author", 88.0),
        ("metadata/user", 101.0),
        ("info/user", 60.0),
    ];
    for (index, (text, width)) in labels.into_iter().enumerate() {
        input.set_edge_label(
            EdgeId(index as u32),
            Some(EdgeLabel {
                text: text.into(),
                size: Size {
                    width,
                    height: 21.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
    }
    input
}

#[test]
fn stage_order_matches_recovered_new_pipeline() {
    let names: Vec<_> = STAGES.iter().map(|stage| stage.name()).collect();
    assert_eq!(names.len(), 36);
    assert_eq!(names[0], "Prescale");
    assert_eq!(names[8], "NodePlacement");
    assert_eq!(names[22], "EdgeRouting");
    assert_eq!(names[25], "EdgeRouting");
    assert_eq!(names[35], "Normalize");
    assert_eq!(names.iter().filter(|name| **name == "AlignAxes").count(), 4);
    assert_eq!(names.iter().filter(|name| **name == "BinPack").count(), 2);
}

#[test]
fn arena_preserves_pointer_identity_as_stable_ids() {
    let mut input = Graph::with_direction(Direction::Right);
    let parent = input.add_node(node("parent", 100.0, 80.0));
    let mut child = node("child", 50.0, 40.0);
    child.parent = Some(parent);
    let child = input.add_node(child);
    let edge = input.add_edge(super::super::Edge {
        source: parent,
        target: child,
    });

    let arena = ArenaGraph::from_input(&input);

    assert_eq!(arena.nodes[parent.0 as usize].input_id, parent);
    assert_eq!(arena.nodes[parent.0 as usize].tala_id, fnv1a32(b"parent"));
    assert_eq!(arena.nodes[child.0 as usize].container, Some(parent));
    assert!(arena.nodes[parent.0 as usize].is_container);
    assert_eq!(arena.nodes[parent.0 as usize].edges, vec![edge]);
    assert_eq!(arena.nodes[child.0 as usize].edges, vec![edge]);
    assert_eq!(arena.containers.nodes(&Some(parent)), vec![child]);
    assert_eq!(arena.directions[&None], Direction::Right);
}

#[test]
fn placement_components_exclude_isolates_and_preserve_component_order() {
    let mut input = Graph::with_direction(Direction::Right);
    let a = input.add_node(node("a", 100.0, 60.0));
    let b = input.add_node(node("b", 100.0, 60.0));
    let c = input.add_node(node("c", 100.0, 60.0));
    let d = input.add_node(node("d", 100.0, 60.0));
    let _isolated = input.add_node(node("isolated", 100.0, 60.0));
    input.add_edge(Edge {
        source: a,
        target: b,
    });
    input.add_edge(Edge {
        source: c,
        target: d,
    });

    let arena = ArenaGraph::from_input(&input);
    assert_eq!(arena.optimizable_components(), vec![vec![a, b], vec![c, d]]);
    assert_eq!(arena.largest_optimizable_component_size(), 2);
}

#[test]
fn descendant_cross_scope_edge_marks_its_container_leaky() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 160.0));
    let mut child_node = node("child", 80.0, 60.0);
    child_node.parent = Some(container);
    let child = input.add_node(child_node);
    let external = input.add_node(node("external", 80.0, 60.0));
    input.add_edge(Edge {
        source: child,
        target: external,
    });

    let arena = ArenaGraph::from_input(&input);

    assert!(arena.has_leaky_edge(container));
    assert!(!arena.has_leaky_edge(child));
    assert!(!arena.has_leaky_edge(external));
}

#[test]
fn descendant_edge_to_its_own_container_is_not_leaky() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 160.0));
    let mut child_node = node("child", 80.0, 60.0);
    child_node.parent = Some(container);
    let child = input.add_node(child_node);
    input.add_edge(Edge {
        source: child,
        target: container,
    });

    let arena = ArenaGraph::from_input(&input);

    assert!(!arena.has_leaky_edge(container));
}

#[test]
fn temporary_container_identity_adds_recovered_alignment_cost_only_to_scoring() {
    let mut input = Graph::with_direction(Direction::Right);
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 200.0, y: 0.0 });
    arena.cell_size = 100.0;
    arena.set_turn_cost(50.0);

    let without_container_cost = arena.sized_edge_length(first, true);
    arena.nodes[first.0 as usize].scoring_is_container = true;
    arena.nodes[second.0 as usize].scoring_is_container = true;
    let with_container_cost = arena.sized_edge_length(first, true);

    assert!(!arena.nodes[first.0 as usize].is_container);
    assert!(!arena.nodes[second.0 as usize].is_container);
    assert_eq!(with_container_cost - without_container_cost, 50.0);

    arena.nodes[first.0 as usize].scoring_is_container = false;
    arena.nodes[second.0 as usize].scoring_is_container = false;
    arena.nodes[first.0 as usize].is_container = true;
    arena.nodes[second.0 as usize].is_container = true;
    assert_eq!(
        arena.sized_edge_length(first, true) - without_container_cost,
        50.0
    );
}

#[test]
fn three_parallel_labels_add_the_recovered_nonhorizontal_turn_cost() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let mut edges = Vec::new();
    for index in 0..3 {
        let edge = input.add_edge(Edge {
            source: first,
            target: second,
        });
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: format!("label-{index}"),
                size: Size {
                    width: 50.0,
                    height: 20.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        edges.push(edge);
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 200.0, y: 200.0 });
    arena.cell_size = 100.0;
    arena.set_turn_cost(50.0);
    let with_three_labels = arena.sized_edge_length(first, true);

    arena.edge_records[edges[2].0 as usize].label = None;
    let with_two_labels = arena.sized_edge_length(first, true);

    // Recovered node.go adds count * turnCost to every one of the three
    // nonhorizontal labeled edges once their parallel label count exceeds
    // two: 3 edges * 3 labels * 50.
    assert_eq!(with_three_labels - with_two_labels, 450.0);
}

#[test]
fn large_arrowhead_label_adds_the_recovered_nonhorizontal_turn_cost() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let edge = input.add_edge(Edge {
        source: first,
        target: second,
    });
    input.set_edge_arrowhead_labels(
        edge,
        crate::EdgeArrowheadLabels {
            source: Some(crate::ArrowheadLabel {
                text: "long".into(),
                size: Size {
                    width: 40.0,
                    height: 20.0,
                },
                color: None,
            }),
            target: None,
        },
    );

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 200.0, y: 200.0 });
    arena.cell_size = 100.0;
    arena.set_turn_cost(50.0);
    let with_large_label = arena.sized_edge_length(first, true);

    arena.edge_records[edge.0 as usize]
        .source_arrowhead_label
        .as_mut()
        .unwrap()
        .text = "abc".into();
    let with_short_label = arena.sized_edge_length(first, true);

    assert_eq!(with_large_label - with_short_label, 500.0);
}

#[test]
fn projected_edge_alignment_uses_carrier_boxes_not_restored_endpoints() {
    let mut input = Graph::with_direction(Direction::Down);
    let first = input.add_node(node("first carrier", 100.0, 100.0));
    let second = input.add_node(node("second carrier", 100.0, 200.0));
    let edge = input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 0.0, y: 250.0 });
    arena.cell_size = 100.0;
    arena.set_turn_cost(50.0);
    arena.sized_adjacent_overrides.insert(
        (second, edge),
        ProjectedAdjacent {
            owner: first,
            tala_id: 10,
            container_tala_id: None,
            offset: Point::default(),
            size: Size {
                width: 50.0,
                height: 50.0,
            },
            cluster_member: false,
        },
    );
    arena.sized_adjacent_overrides.insert(
        (first, edge),
        ProjectedAdjacent {
            owner: second,
            tala_id: 11,
            container_tala_id: None,
            offset: Point { x: 25.0, y: 0.0 },
            size: Size {
                width: 50.0,
                height: 50.0,
            },
            cluster_member: false,
        },
    );

    let without_container_cost = arena.sized_edge_length(first, true);
    arena.nodes[first.0 as usize].scoring_is_container = true;
    arena.nodes[second.0 as usize].scoring_is_container = true;

    assert_eq!(arena.sized_edge_length(first, true), without_container_cost);
}

#[test]
fn node_label_placement_uses_recovered_node_and_container_base_orders() {
    let mut input = Graph::default();
    let mut container_node = node("container", 200.0, 200.0);
    container_node.label_size = Some(Size {
        width: 20.0,
        height: 20.0,
    });
    let container = input.add_node(container_node);
    let mut child_node = node("child", 100.0, 100.0);
    child_node.parent = Some(container);
    child_node.label_size = Some(Size {
        width: 20.0,
        height: 20.0,
    });
    let child = input.add_node(child_node);
    let mut code_node = node("code", 100.0, 100.0);
    code_node.shape = ShapeKind::Code;
    code_node.label_size = Some(Size {
        width: 20.0,
        height: 20.0,
    });
    let code = input.add_node(code_node);

    let mut arena = ArenaGraph::from_input(&input);
    // This exercises private layoutgraph wrapper preferences with an
    // explicitly installed Label. D2 adapter.Object.HasLabel omits that label
    // for content shapes, so install the private Source fixture after conversion.
    arena.nodes[code.0 as usize].label_size = input.node(code).unwrap().label_size;
    arena.nodes[code.0 as usize].label_text_present = true;
    arena.set_position(container, Point { x: 0.0, y: 0.0 });
    arena.set_position(child, Point { x: 50.0, y: 50.0 });
    arena.set_position(code, Point { x: 300.0, y: 0.0 });
    labels::place_node_labels(&mut arena);

    assert_eq!(
        arena.nodes[container.0 as usize].label_position,
        LabelPosition::InsideTopCenter
    );
    assert_eq!(
        arena.nodes[child.0 as usize].label_position,
        LabelPosition::InsideMiddleCenter
    );
    assert_eq!(
        arena.nodes[code.0 as usize].label_position,
        LabelPosition::InsideMiddleCenter,
        "TALA's code/class/table/text wrappers inherit square preferences"
    );
}

#[test]
fn node_label_placement_uses_recovered_source_order() {
    let mut input = Graph::default();
    let root = input.add_node(node("root", 20.0, 20.0));
    let mut icon_node = node("icon", 200.0, 200.0);
    icon_node.has_icon = true;
    let icon = input.add_node(icon_node);
    let mut decorated_node = node("decorated", 40.0, 40.0);
    decorated_node.parent = Some(root);
    decorated_node.label_size = Some(Size {
        width: 20.0,
        height: 20.0,
    });
    let decorated = input.add_node(decorated_node);

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(root, Point::default());
    arena.set_position(decorated, Point { x: 280.0, y: 80.0 });
    arena.set_position(icon, Point { x: 200.0, y: 0.0 });
    labels::place_node_labels(&mut arena);

    assert_eq!(
        arena.nodes[icon.0 as usize].icon_position,
        Some(LabelPosition::InsideMiddleCenter),
        "source-order label placement processes the root icon before the later descendant"
    );
}

#[test]
fn persistent_label_obstacles_keep_icons_and_discard_fixed_labels() {
    let mut input = Graph::default();
    let mut fixed = node("fixed", 100.0, 100.0);
    fixed.label_size = Some(Size {
        width: 30.0,
        height: 20.0,
    });
    fixed.label_position = LabelPosition::InsideTopLeft;
    fixed.has_icon = true;
    fixed.icon_position = Some(LabelPosition::InsideBottomRight);
    let fixed = input.add_node(fixed);

    let mut automatic = node("automatic", 100.0, 100.0);
    automatic.label_size = Some(Size {
        width: 30.0,
        height: 20.0,
    });
    let automatic = input.add_node(automatic);

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(fixed, Point { x: 0.0, y: 0.0 });
    arena.set_position(automatic, Point { x: 200.0, y: 0.0 });
    arena.nodes[automatic.0 as usize].label_position = LabelPosition::InsideMiddleCenter;

    let fixed_obstacles = labels::positioned_node_label_obstacles(&arena, fixed);
    let automatic_obstacles = labels::positioned_node_label_obstacles(&arena, automatic);
    assert_eq!(fixed_obstacles.len(), 1, "only the icon persists");
    assert_eq!(
        fixed_obstacles[0].size,
        Size {
            width: 50.0,
            height: 50.0,
        }
    );
    assert_eq!(automatic_obstacles.len(), 1, "the placed label persists");
    assert_eq!(
        automatic_obstacles[0].size,
        Size {
            width: 30.0,
            height: 20.0,
        }
    );
}

#[test]
fn rejoined_containers_restore_their_scoring_identity() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 200.0, 160.0));
    let mut child_node = node("child", 80.0, 60.0);
    child_node.parent = Some(container);
    input.add_node(child_node);

    let mut arena = ArenaGraph::from_input(&input);

    assert!(arena.nodes[container.0 as usize].is_container);
    assert!(!arena.nodes[container.0 as usize].scoring_is_container);

    arena.restore_rejoined_scoring_container_identity();

    assert!(arena.nodes[container.0 as usize].scoring_is_container);
}

#[test]
fn combined_turn_cost_uses_the_recovered_whole_maximum_edge_length() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 300.0, y: 0.0 });

    arena.initialize_combined_scoring_costs();

    // Recovered getMaxLength is the 200-unit border distance; getTurnCost
    // multiplies it directly by 0.125 and the single-edge count.
    assert_eq!(arena.turn_cost_value(), 25.0);
}

#[test]
fn gap_normalization_refreshes_only_the_lazy_turn_cost() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 400.0, y: 0.0 });
    arena.set_turn_cost(1.0);
    arena.set_crossing_cost(7.0);
    arena.set_non_center_port_cost(9.0);

    arena.reset_turn_cost();
    assert_eq!(arena.raw_turn_cost().to_bits(), 0);

    assert_eq!(arena.turn_cost_value(), 37.5);
    assert_eq!(arena.raw_crossing_cost(), 7.0);
    assert_eq!(arena.raw_non_center_port_cost(), 9.0);
}

#[test]
fn combined_non_center_port_cost_uses_the_recovered_whole_graph_formula() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    for _ in 0..3 {
        input.add_edge(Edge {
            source: first,
            target: second,
        });
    }
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 1100.0, y: 0.0 });

    arena.initialize_combined_scoring_costs();

    // Recovered getNonCenterPortCost uses the full 1000-unit border
    // distance on the rejoined graph, not the optimizer's half-length
    // carrier and not a hierarchy-specific compatibility predicate.
    assert_eq!(
        arena.non_center_port_cost_value(),
        0.04287499999999999_f64 * 3.0 * 1000.0
    );
}

#[test]
fn positioned_max_length_ignores_nodes_retained_outside_the_active_graph() {
    let mut input = Graph::default();
    let active_first = input.add_node(node("active-first", 100.0, 100.0));
    let active_second = input.add_node(node("active-second", 100.0, 100.0));
    let retained_first = input.add_node(node("retained-first", 100.0, 100.0));
    let retained_second = input.add_node(node("retained-second", 100.0, 100.0));
    input.add_edge(Edge {
        source: retained_first,
        target: retained_second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(active_first, Point { x: 0.0, y: 0.0 });
    arena.set_position(active_second, Point { x: 100.0, y: 0.0 });
    arena.set_position(retained_first, Point { x: 0.0, y: 0.0 });
    arena.set_position(retained_second, Point { x: 1100.0, y: 0.0 });
    arena.refresh_node_order_membership();
    arena.active_graph_node_order = vec![active_first, active_second];

    // Go's maxEdgeLength walks Graph.Nodes and does not visit retained
    // arena nodes that are absent from the current temporary graph.
    assert_eq!(arena.positioned_max_length(), 0.0);
}

#[test]
fn positioned_max_length_uses_node_incidence_outside_graph_edge_slice() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 1000.0, y: 0.0 });
    arena.edge_order.clear();
    arena.refresh_node_order_membership();

    // Graph.maxEdgeLength follows Node.Edges even when the optimizer's
    // graph-wide Edge slice no longer contains this relationship.
    assert_eq!(arena.positioned_max_length(), 900.0);
}

#[test]
fn container_alignment_cost_counts_unaligned_equal_sized_siblings() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[first.0 as usize].is_container = true;
    arena.nodes[second.0 as usize].is_container = true;
    arena.set_non_center_port_cost(37.0);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 200.0, y: 200.0 });

    assert_eq!(arena.container_alignment_cost(), 37.0);
    arena.nodes[second.0 as usize].declared_size = Some(Size {
        width: 120.0,
        height: 100.0,
    });
    assert_eq!(arena.container_alignment_cost(), 0.0);
    arena.nodes[second.0 as usize].declared_size = None;
    arena.set_position(second, Point { x: 0.0, y: 200.0 });
    assert_eq!(arena.container_alignment_cost(), 0.0);
}

#[test]
fn container_alignment_cost_is_first_scored_from_candidate_geometry() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[first.0 as usize].is_container = true;
    arena.nodes[second.0 as usize].is_container = true;
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 1000.0, y: 0.0 });
    arena.reset_alignment_container_cost();

    // The siblings share one aligned axis, so Go has not called
    // NonCenterPortCost yet. A speculative move creates the first scored
    // pair, and the lazy value must use that candidate's longer edge.
    assert_eq!(arena.container_alignment_cost(), 0.0);
    assert_eq!(arena.raw_non_center_port_cost(), 0.0);
    arena.set_position(
        second,
        Point {
            x: 1000.0,
            y: 1000.0,
        },
    );
    let candidate_unit = arena.non_center_port_cost_for_max_length(arena.positioned_max_length());

    assert_eq!(arena.container_alignment_cost(), candidate_unit);
    assert_eq!(arena.raw_non_center_port_cost(), candidate_unit);
}

#[test]
fn container_alignment_ignores_retained_inactive_siblings() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let retained = input.add_node(node("retained", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    for node_id in [first, second, retained] {
        arena.nodes[node_id.0 as usize].is_container = true;
        arena.set_position(
            node_id,
            match node_id {
                id if id == first => Point { x: 0.0, y: 0.0 },
                id if id == second => Point { x: 0.0, y: 200.0 },
                _ => Point { x: 200.0, y: 200.0 },
            },
        );
    }
    arena.set_non_center_port_cost(37.0);
    arena.refresh_node_order_membership();
    arena.active_graph_node_order = vec![first, second];

    // The two live siblings share an x coordinate. The retained node would
    // create two false penalties if the scorer scanned the stable arena.
    assert_eq!(arena.container_alignment_cost(), 0.0);
}

#[test]
fn container_alignment_does_not_inherit_cluster_members_container_flags() {
    // grouping.CreateVessel constructs a fresh non-container Node. The two
    // ungrouped peers remain containers and pay one NonCenterPortCost in
    // placementcost.ContainerAlignmentCost, with or without the cluster.
    for clustered in [false, true] {
        let mut input = Graph::default();
        let first = input.add_node(node("first", 100.0, 100.0));
        let second = input.add_node(node("second", 100.0, 100.0));
        let left = input.add_node(node("left", 100.0, 220.0));
        let right = input.add_node(node("right", 100.0, 220.0));
        for parent in [first, second, left, right] {
            let mut child = node(&format!("child{}", parent.0), 10.0, 10.0);
            child.parent = Some(parent);
            input.add_node(child);
        }
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(first, Point::default());
        arena.set_position(second, Point { x: 0.0, y: 120.0 });
        arena.set_position(left, Point { x: 300.0, y: 300.0 });
        arena.set_position(right, Point { x: 600.0, y: 600.0 });
        if clustered {
            arena.clusters.push(ClusterState {
                members: vec![first, second],
                arrangement: ClusterArrangement::Column,
                desired_arrangement: ClusterArrangement::Column,
                padding: 20.0,
                vessel_tala_id: 10_001,
                fixed_size: false,
            });
            arena.nodes[first.0 as usize].cluster = Some(0);
            arena.nodes[second.0 as usize].cluster = Some(0);
        }
        arena.rebuild_active_aggregate_node_order();
        arena.set_non_center_port_cost(37.0);

        assert_eq!(arena.container_alignment_cost(), 37.0, "{clustered}");
    }
}

#[test]
fn alignment_transaction_clones_share_the_graph_owned_lazy_cost() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    for node_id in [first, second] {
        arena.nodes[node_id.0 as usize].is_container = true;
    }
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 200.0, y: 200.0 });
    arena.set_non_center_port_cost(37.0);

    // Go's transaction rollback restores geometry but retains this Graph's
    // memoized placement cost. Rust snapshots of that graph share the cache.
    let mut trial = arena.clone();
    assert_eq!(trial.container_alignment_cost(), 37.0);
    assert_eq!(arena.raw_non_center_port_cost(), 37.0);
}

#[test]
fn alignment_stages_reuse_the_graph_owned_first_alignment_cost() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    for node_id in [first, second] {
        arena.nodes[node_id.0 as usize].is_container = true;
    }
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 200.0, y: 200.0 });
    arena.set_non_center_port_cost(37.0);
    assert_eq!(arena.container_alignment_cost(), 37.0);

    let mut repeated_view = arena.clone();
    repeated_view.set_position(second, Point { x: 300.0, y: 300.0 });

    // Go keeps this cache on the Graph through later alignment stages. Rust
    // transaction and stage clones of that Graph share the same lazy value.
    assert_eq!(repeated_view.container_alignment_cost(), 37.0);
}

#[test]
fn flat_optimizer_nodes_preserve_recovered_fifo_connectivity_order_at_any_size() {
    let mut input = Graph::with_direction(Direction::Right);
    let source = input.add_node(node("source", 100.0, 60.0));
    let auth = input.add_node(node("auth", 100.0, 60.0));
    let cache = input.add_node(node("cache", 100.0, 60.0));
    let router = input.add_node(node("router", 100.0, 60.0));
    let sink = input.add_node(node("sink", 100.0, 60.0));
    let fallback = input.add_node(node("fallback", 100.0, 60.0));
    let isolates = (0..5)
        .map(|index| input.add_node(node(&format!("isolate-{index}"), 100.0, 60.0)))
        .collect::<Vec<_>>();
    for (source, target) in [
        (source, auth),
        (auth, source),
        (source, router),
        (router, cache),
        (cache, router),
        (router, sink),
        (sink, source),
        (router, fallback),
        (fallback, sink),
    ] {
        input.add_edge(Edge { source, target });
    }
    for (source, target) in std::iter::once((fallback, isolates[0]))
        .chain(isolates.windows(2).map(|pair| (pair[0], pair[1])))
    {
        input.add_edge(Edge { source, target });
    }

    let arena = ArenaGraph::from_input(&input);
    let mut expected = vec![source, auth, router, sink, cache, fallback];
    expected.extend(isolates);
    assert_eq!(arena.optimizer_nodes(), expected);
    assert_eq!(arena.sized_optimizer_nodes(), expected);

    let mut fixed_input = input.clone();
    fixed_input.node_mut(cache).unwrap().locked_position = Some(Point { x: 0.0, y: 0.0 });
    let fixed_arena = ArenaGraph::from_input(&fixed_input);
    assert!(!fixed_arena.optimizer_nodes().contains(&cache));
    assert_eq!(fixed_arena.sized_optimizer_nodes(), expected);
}

#[test]
fn child_scope_nears_remain_owned_by_main_graph_nodes() {
    let mut input = Graph::default();
    let root = input.add_node(node("root", 100.0, 100.0));
    let mut start_node = node("root.start", 100.0, 100.0);
    start_node.parent = Some(root);
    let start = input.add_node(start_node);
    let mut end_node = node("root.end", 100.0, 100.0);
    end_node.parent = Some(root);
    let end = input.add_node(end_node);

    let mut start_one_node = node("root.start.1", 40.0, 40.0);
    start_one_node.parent = Some(start);
    let start_one = input.add_node(start_one_node);
    let mut end_one_node = node("root.end.1", 40.0, 40.0);
    end_one_node.parent = Some(end);
    let end_one = input.add_node(end_one_node);
    let mut start_two_node = node("root.start.2", 40.0, 40.0);
    start_two_node.parent = Some(start);
    let start_two = input.add_node(start_two_node);
    let mut end_two_node = node("root.end.2", 40.0, 40.0);
    end_two_node.parent = Some(end);
    let end_two = input.add_node(end_two_node);
    input.add_edge(Edge {
        source: start_one,
        target: end_one,
    });
    input.add_edge(Edge {
        source: start_two,
        target: end_two,
    });

    let mut pipeline = Pipeline::new(&input, 1, false, false);
    let PlacementScope {
        old_to_new,
        assigned_nears,
        ..
    } = pipeline.placement_scope_graph(Some(start)).unwrap();
    let assigned_nears = Pipeline::map_scope_nears(&old_to_new, &assigned_nears);
    pipeline.retain_scope_nears(&assigned_nears);

    assert_eq!(assigned_nears.len(), 1);
    assert_eq!(
        pipeline.graph.nodes[start_one.0 as usize].nears.keys(),
        vec![start_two]
    );
    assert_eq!(
        pipeline.graph.nodes[start_two.0 as usize].nears.keys(),
        vec![start_one]
    );
}

#[test]
fn placement_scope_mirror_reach_uses_the_temporary_nodes_container() {
    let mut input = Graph::default();
    let carrier = input.add_node(node("carrier", 300.0, 300.0));
    let mut nested_node = node("nested", 200.0, 200.0);
    nested_node.parent = Some(carrier);
    let nested = input.add_node(nested_node);
    let mut short_node = node("short", 80.0, 40.0);
    short_node.parent = Some(nested);
    let short = input.add_node(short_node);
    let mut tall_node = node("tall", 80.0, 100.0);
    tall_node.parent = Some(carrier);
    let tall = input.add_node(tall_node);
    let mut tall_child_node = node("tall child", 20.0, 20.0);
    tall_child_node.parent = Some(tall);
    input.add_node(tall_child_node);
    input.add_edge(Edge {
        source: short,
        target: tall,
    });

    let pipeline = Pipeline::new(&input, 1, false, false);
    let root_placement = pipeline.placement_scope_graph(None).unwrap();
    let local_carrier = root_placement.old_to_new[&carrier];
    let root_metadata = root_placement
        .scoring_node_metadata
        .iter()
        .find(|metadata| metadata.node == local_carrier)
        .unwrap();
    assert_eq!(root_metadata.parent_container, None);

    let nested_placement = pipeline.placement_scope_graph(Some(carrier)).unwrap();
    assert!(
        nested_placement
            .scoring_node_metadata
            .iter()
            .all(|metadata| metadata.parent_container == Some(carrier))
    );
    let temporary = ArenaGraph::from_input(&nested_placement.graph);
    let reachable = crate::internal::hierarchy_scope::restored_mirror_reachable_containers(
        &nested_placement,
        &temporary,
        &temporary.graph_node_order(),
    );
    assert_eq!(
        reachable,
        BTreeSet::from([Some(pipeline.graph.nodes[carrier.0 as usize].tala_id)]),
        "the original parent identity must not alias compact node index zero"
    );

    // A node with an explicit nil parent remains a top-level pointer even
    // when the placement scope itself has a non-nil parent. Go's reachability
    // map records both keys independently.
    let mut mixed_scope = nested_placement.clone();
    let local_nested = nested_placement.old_to_new[&nested];
    let local_tall = nested_placement.old_to_new[&tall];
    let nested_metadata = mixed_scope
        .scoring_node_metadata
        .iter_mut()
        .find(|metadata| metadata.node == local_nested)
        .unwrap();
    nested_metadata.parent_container = None;
    nested_metadata.parent_tala_id = None;
    mixed_scope
        .scoring_node_metadata
        .sort_by_key(|metadata| (metadata.node != local_tall, metadata.node));
    let mixed_graph = ArenaGraph::from_input(&mixed_scope.graph);
    let mixed_reachable = crate::internal::hierarchy_scope::restored_mirror_reachable_containers(
        &mixed_scope,
        &mixed_graph,
        &[local_nested],
    );
    assert_eq!(mixed_reachable, BTreeSet::from([None]));
}

#[test]
fn shared_child_offsets_precede_mirror_replay_without_remirroring_the_carrier() {
    let mut input = Graph::default();
    let carrier = input.add_node(node("carrier", 300.0, 300.0));
    let mut short_node = node("short", 80.0, 40.0);
    short_node.parent = Some(carrier);
    let short = input.add_node(short_node);
    let mut tall_node = node("tall", 80.0, 100.0);
    tall_node.parent = Some(carrier);
    let tall = input.add_node(tall_node);
    let mut pipeline = Pipeline::new(&input, 1, false, false);
    let carrier_target = Point { x: 100.0, y: 100.0 };
    pipeline.graph.set_position(carrier, carrier_target);
    pipeline
        .graph
        .set_position(short, Point { x: 900.0, y: 900.0 });
    pipeline
        .graph
        .set_position(tall, Point { x: 160.0, y: 300.0 });

    let carrier_tala_id = pipeline.graph.nodes[carrier.0 as usize].tala_id;
    let short_tala_id = pipeline.graph.nodes[short.0 as usize].tala_id;
    pipeline.publish_external_shared_child_offsets(&[(
        carrier_tala_id,
        short_tala_id,
        Point { x: 60.0, y: 60.0 },
    )]);
    assert_eq!(
        pipeline.graph.position(short),
        Some(Point { x: 160.0, y: 160.0 })
    );

    pipeline.graph.mirror_subtree_axes(
        carrier,
        false,
        true,
        &BTreeSet::from([None, Some(carrier)]),
    );
    pipeline
        .graph
        .move_node_abs_with_children(carrier, carrier_target);

    assert_eq!(pipeline.graph.position(carrier), Some(carrier_target));
    let mirrored_short = pipeline.graph.position(short).unwrap();
    let mirrored_tall = pipeline.graph.position(tall).unwrap();
    assert_eq!(mirrored_short.x, 160.0);
    assert_eq!(mirrored_tall.x, 160.0);
    assert!(
        mirrored_short.y > mirrored_tall.y,
        "the published upper child must participate in the later vertical mirror"
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mirror_positions_hidden_projected_container_children_without_mirroring_carrier() {
    const SEQUENCE_VESSEL_TALA_ID: i64 = 95_100;

    let mut input = Graph::default();
    let parent = input.add_node(node("directed parent", 300.0, 300.0));
    let mut hidden_container_node = node("hidden square", 442.0, 358.0);
    hidden_container_node.parent = Some(parent);
    hidden_container_node.shape = ShapeKind::Square;
    let hidden_container = input.add_node(hidden_container_node);
    let mut first_leaf_node = node("first leaf", 53.0, 66.0);
    first_leaf_node.parent = Some(hidden_container);
    let first_leaf = input.add_node(first_leaf_node);
    let mut second_leaf_node = node("second leaf", 53.0, 66.0);
    second_leaf_node.parent = Some(hidden_container);
    let second_leaf = input.add_node(second_leaf_node);
    let target = input.add_node(node("direction target", 40.0, 40.0));
    let edge = input.add_edge(Edge {
        source: parent,
        target,
    });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(parent, Point { x: 100.0, y: 100.0 });
    arena.set_position(target, Point::default());
    let carrier_position = Point {
        x: -676.0,
        y: 32_900.0,
    };
    arena.set_position(hidden_container, carrier_position);
    arena.set_position(
        first_leaf,
        Point {
            x: -561.0,
            y: 32_960.0,
        },
    );
    arena.set_position(
        second_leaf,
        Point {
            x: -487.0,
            y: 32_960.0,
        },
    );
    arena.nodes[hidden_container.0 as usize].node_padding = Insets {
        top: 60.0,
        right: 60.0,
        bottom: 60.0,
        left: 115.0,
    };

    let parent_tala_id = arena.nodes[parent.0 as usize].tala_id;
    let container_tala_id = arena.nodes[hidden_container.0 as usize].tala_id;
    let mut sequence_vessel = arena.nodes[hidden_container.0 as usize].clone();
    sequence_vessel.tala_id = SEQUENCE_VESSEL_TALA_ID;
    sequence_vessel.is_container = false;
    arena
        .transaction_external_container_children
        .insert(parent_tala_id, vec![sequence_vessel]);
    arena.transaction_external_aggregate_children.insert(
        SEQUENCE_VESSEL_TALA_ID,
        vec![arena.nodes[hidden_container.0 as usize].clone()],
    );
    arena.transaction_external_container_children.insert(
        container_tala_id,
        vec![
            arena.nodes[first_leaf.0 as usize].clone(),
            arena.nodes[second_leaf.0 as usize].clone(),
        ],
    );
    arena
        .node_order
        .retain(|node| *node != hidden_container && *node != first_leaf && *node != second_leaf);
    arena.invalidate_node_order_membership();

    assert_eq!(arena.direction_transforms(None), (true, true));
    let (mirrors, projected_refits) = arena.direct_with_projected_mirror_refits(false);
    assert_eq!(mirrors, (true, true));
    assert_eq!(projected_refits, BTreeSet::from([container_tala_id]));

    let hidden_carrier =
        &arena.transaction_external_aggregate_children[&SEQUENCE_VESSEL_TALA_ID][0];
    assert_eq!(hidden_carrier.position, Some(carrier_position));
    let hidden_children = &arena.transaction_external_container_children[&container_tala_id];
    assert_eq!(
        hidden_children
            .iter()
            .map(|child| child.position.unwrap().x)
            .collect::<Vec<_>>(),
        vec![-562.0, -488.0]
    );

    let offsets = Pipeline::projected_mirror_refit_offsets(&arena, &projected_refits);
    assert_eq!(
        offsets
            .iter()
            .map(|(_, child, offset)| (*child, offset.x))
            .collect::<Vec<_>>(),
        vec![
            (arena.nodes[first_leaf.0 as usize].tala_id, 114.0),
            (arena.nodes[second_leaf.0 as usize].tala_id, 188.0),
        ]
    );

    // The owning graph replays the mirror after translating the carrier.
    // Publishing the directed clone's offsets afterward must preserve its
    // absolute-rounding result in that final frame.
    let mut owner = Pipeline::new(&input, 1, false, false);
    owner.graph.set_position(
        hidden_container,
        Point {
            x: 1_769.0,
            y: 2_050.0,
        },
    );
    owner.graph.set_position(
        first_leaf,
        Point {
            x: 1_884.0,
            y: 2_110.0,
        },
    );
    owner.graph.set_position(
        second_leaf,
        Point {
            x: 1_958.0,
            y: 2_110.0,
        },
    );
    owner.publish_mirrored_external_child_offsets(&offsets);
    assert_eq!(
        owner.graph.position(first_leaf),
        Some(Point {
            x: 1_883.0,
            y: 2_110.0
        })
    );
    assert_eq!(
        owner.graph.position(second_leaf),
        Some(Point {
            x: 1_957.0,
            y: 2_110.0
        })
    );
}

#[test]
fn shared_sequence_vessel_offset_moves_all_steps_and_descendants() {
    const SEQUENCE_VESSEL: i64 = 95_001;

    let mut input = Graph::default();
    let container = input.add_node(node("container", 300.0, 300.0));
    let mut first_node = node("first step", 80.0, 40.0);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second step", 120.0, 40.0);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);
    let mut descendant_node = node("first step descendant", 20.0, 20.0);
    descendant_node.parent = Some(first);
    let descendant = input.add_node(descendant_node);
    let mut pipeline = Pipeline::new(&input, 1, false, false);
    pipeline.graph.sequences.push(SequenceState {
        members: vec![first, second],
        vessel_tala_id: SEQUENCE_VESSEL,
        container: Some(container),
        has_edge_abductions: false,
    });
    pipeline.graph.nodes[first.0 as usize].sequence = Some(0);
    pipeline.graph.nodes[second.0 as usize].sequence = Some(0);
    // The first stable member carries the current sequence vessel; the second
    // step is absent from Graph.Nodes while the sequence is active.
    pipeline.graph.node_order = vec![container, first];
    pipeline
        .graph
        .set_position(container, Point { x: 100.0, y: 100.0 });
    pipeline
        .graph
        .set_position(first, Point { x: 10.0, y: 20.0 });
    pipeline
        .graph
        .set_position(second, Point { x: 100.0, y: 20.0 });
    pipeline
        .graph
        .set_position(descendant, Point { x: 20.0, y: 30.0 });

    pipeline.publish_external_shared_child_offsets(&[(
        pipeline.graph.nodes[container.0 as usize].tala_id,
        SEQUENCE_VESSEL,
        Point { x: 60.0, y: 70.0 },
    )]);

    assert_eq!(
        pipeline.graph.position(first),
        Some(Point { x: 160.0, y: 170.0 })
    );
    assert_eq!(
        pipeline.graph.position(second),
        Some(Point { x: 250.0, y: 170.0 })
    );
    assert_eq!(
        pipeline.graph.position(descendant),
        Some(Point { x: 170.0, y: 180.0 }),
        "moving the synthetic sequence vessel must traverse step descendants"
    );
}

#[test]
fn cluster_mirror_visits_hidden_members_and_distinct_vessel_box() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 500.0, 300.0));
    let mut first_node = node("first", 100.0, 60.0);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 100.0, 60.0);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);

    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    arena.set_position(first, Point { x: 100.0, y: 40.0 });
    arena.set_position(second, Point { x: 300.0, y: 40.0 });
    arena
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 50.0, y: 20.0 });
    arena.rebuild_active_aggregate_node_order();
    let vessel_size = arena.cluster_vessel_size(0);

    arena.mirror_subtree_axes(first, true, false, &BTreeSet::from([Some(container)]));

    assert_eq!(arena.position(first), Some(Point { x: -200.0, y: 40.0 }));
    assert_eq!(arena.position(second), Some(Point { x: -400.0, y: 40.0 }));
    assert_eq!(
        arena.pending_cluster_vessel_positions.get(&0),
        Some(&Point {
            x: -50.0 - vessel_size.width,
            y: 20.0,
        })
    );

    arena.translate_active_node_with_children(first, Point { x: 10.0, y: 20.0 });
    assert_eq!(arena.position(first), Some(Point { x: -190.0, y: 60.0 }));
    assert_eq!(arena.position(second), Some(Point { x: -390.0, y: 60.0 }));
    assert_eq!(
        arena.pending_cluster_vessel_positions.get(&0),
        Some(&Point {
            x: -40.0 - vessel_size.width,
            y: 40.0,
        })
    );
}

#[test]
fn arranging_cluster_members_does_not_move_the_distinct_vessel() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 80.0, 60.0));
    let second = input.add_node(node("second", 80.0, 60.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    arena.set_position(first, Point { x: 300.0, y: 200.0 });
    arena.set_position(second, Point { x: 500.0, y: 200.0 });
    let vessel_top_left = Point { x: 40.0, y: 70.0 };
    arena
        .pending_cluster_vessel_positions
        .insert(0, vessel_top_left);

    arena.arrange_cluster_members(0, vessel_top_left);

    assert_eq!(arena.position(first), Some(vessel_top_left));
    assert_eq!(arena.position(second), Some(Point { x: 140.0, y: 70.0 }));
    assert_eq!(
        arena.pending_cluster_vessel_positions.get(&0),
        Some(&vessel_top_left),
        "moving a real first member must not translate the separate vessel"
    );
}

#[test]
fn placement_scope_preserves_nil_cluster_member_before_first_arrangement() {
    let mut input = Graph::default();
    let peer = input.add_node(node("peer", 264.0, 186.0));
    let container = input.add_node(node("container", 264.0, 186.0));
    let mut child_node = node("child", 144.0, 66.0);
    child_node.parent = Some(container);
    let child = input.add_node(child_node);
    let mut pipeline = Pipeline::new(&input, 1, false, true);
    pipeline.graph.clusters.push(ClusterState {
        members: vec![peer, container],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    pipeline.graph.nodes[peer.0 as usize].cluster = Some(0);
    pipeline.graph.nodes[container.0 as usize].cluster = Some(0);
    pipeline.graph.set_position(child, Point::default());
    let container_tala_id = pipeline.graph.nodes[container.0 as usize].tala_id;
    let placement = pipeline.placement_scope_graph(None).expect("root scope");
    let member = placement.transaction_external_aggregate_children[&10_001]
        .iter()
        .find(|member| member.tala_id == container_tala_id)
        .unwrap();
    assert!(
        member.position.is_some(),
        "scope supplies a synthetic arrangement position"
    );
    assert!(
        member.transaction_position_was_nil,
        "synthetic position must not bypass ArrangeClusterNodes' initial child placement"
    );
}

#[test]
fn arranging_unpositioned_cluster_container_positions_its_children() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 264.0, 186.0));
    let peer = input.add_node(node("peer", 264.0, 186.0));
    let mut child_node = node("child", 144.0, 66.0);
    child_node.parent = Some(container);
    let child = input.add_node(child_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![container, peer],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    arena.nodes[container.0 as usize].cluster = Some(0);
    arena.nodes[peer.0 as usize].cluster = Some(0);
    arena.set_position(child, Point::default());
    let vessel_top_left = Point { x: 693.0, y: 99.0 };

    arena.arrange_cluster_members(0, vessel_top_left);

    assert_eq!(arena.position(container), Some(vessel_top_left));
    assert_eq!(arena.position(child), Some(vessel_top_left));
    assert_eq!(arena.position(peer), Some(Point { x: 693.0, y: 305.0 }));
}

#[test]
fn moving_cluster_ancestor_translates_the_traversed_vessel_once() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 300.0, 200.0));
    let mut first_node = node("first", 80.0, 60.0);
    first_node.parent = Some(parent);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 80.0, 60.0);
    second_node.parent = Some(parent);
    let second = input.add_node(second_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    arena.set_position(parent, Point { x: 0.0, y: 0.0 });
    arena.set_position(first, Point { x: 40.0, y: 70.0 });
    arena.set_position(second, Point { x: 140.0, y: 70.0 });
    arena
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 40.0, y: 70.0 });
    arena.rebuild_active_aggregate_node_order();

    arena.move_node_abs_with_children(parent, Point { x: 10.0, y: 15.0 });

    assert_eq!(arena.position(first), Some(Point { x: 50.0, y: 85.0 }));
    assert_eq!(arena.position(second), Some(Point { x: 150.0, y: 85.0 }));
    assert_eq!(
        arena.pending_cluster_vessel_positions.get(&0),
        Some(&Point { x: 50.0, y: 85.0 }),
        "walking the aggregate vessel below its container must translate it once"
    );
}

#[test]
fn cluster_cleanup_uses_fixed_size_vessel_origin_not_centered_first_member() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 80.0, 80.0));
    let second = input.add_node(node("second", 120.0, 120.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: true,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    let vessel_top_left = Point { x: 50.0, y: 20.0 };
    arena
        .pending_cluster_vessel_positions
        .insert(0, vessel_top_left);
    // ArrangeClusterNodes centers the shorter first member inside the
    // independent 120-high vessel. Its y coordinate therefore cannot stand
    // in for the vessel's y coordinate during CleanupStuff.
    arena.set_position(first, Point { x: 50.0, y: 40.0 });
    arena.set_position(second, Point { x: 150.0, y: 20.0 });
    arena.rebuild_active_aggregate_node_order();

    arena.restore_aggregate_members_to_node_order();

    assert_eq!(arena.position(first), Some(Point { x: 50.0, y: 40.0 }));
    assert_eq!(arena.position(second), Some(Point { x: 150.0, y: 20.0 }));
}

#[test]
fn mirrored_cluster_offset_preserves_reflected_row_member_order() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 300.0, 200.0));
    let mut first_node = node("first", 40.0, 30.0);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 40.0, 30.0);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);

    let mut pipeline = Pipeline::new(&input, 1, false, false);
    pipeline.graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 10.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    pipeline.graph.nodes[first.0 as usize].cluster = Some(0);
    pipeline.graph.nodes[second.0 as usize].cluster = Some(0);
    pipeline.graph.set_position(container, Point::default());
    pipeline
        .graph
        .set_position(first, Point { x: 60.0, y: 60.0 });
    pipeline
        .graph
        .set_position(second, Point { x: 110.0, y: 60.0 });
    pipeline
        .graph
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 60.0, y: 60.0 });
    pipeline.graph.rebuild_active_aggregate_node_order();

    pipeline
        .graph
        .mirror_subtree_axes(first, true, false, &BTreeSet::from([Some(container)]));
    let container_tala_id = pipeline.graph.nodes[container.0 as usize].tala_id;
    pipeline.publish_mirrored_external_child_offsets(&[(
        container_tala_id,
        10_001,
        Point { x: 60.0, y: 60.0 },
    )]);

    assert_eq!(
        pipeline.graph.pending_cluster_vessel_positions.get(&0),
        Some(&Point { x: 60.0, y: 60.0 })
    );
    assert_eq!(
        pipeline.graph.position(second),
        Some(Point { x: 60.0, y: 60.0 })
    );
    assert_eq!(
        pipeline.graph.position(first),
        Some(Point { x: 110.0, y: 60.0 })
    );
}

#[test]
fn cluster_mirror_walks_steps_below_absorbed_sequence_vessel() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 500.0, 300.0));
    let mut first_step_node = node("first-step", 80.0, 50.0);
    first_step_node.parent = Some(container);
    let first_step = input.add_node(first_step_node);
    let mut second_step_node = node("second-step", 80.0, 50.0);
    second_step_node.parent = Some(container);
    let second_step = input.add_node(second_step_node);
    let mut peer_node = node("peer", 80.0, 50.0);
    peer_node.parent = Some(container);
    let peer = input.add_node(peer_node);

    let mut arena = ArenaGraph::from_input(&input);
    arena.sequences.push(SequenceState {
        members: vec![first_step, second_step],
        vessel_tala_id: 10_001,
        container: Some(container),
        has_edge_abductions: false,
    });
    arena.clusters.push(ClusterState {
        members: vec![first_step, peer],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_002,
        fixed_size: false,
    });
    for step in [first_step, second_step] {
        arena.nodes[step.0 as usize].sequence = Some(0);
    }
    arena.nodes[first_step.0 as usize].cluster = Some(0);
    arena.nodes[peer.0 as usize].cluster = Some(0);
    arena.set_position(first_step, Point { x: 100.0, y: 40.0 });
    arena.set_position(second_step, Point { x: 250.0, y: 40.0 });
    arena.set_position(peer, Point { x: 400.0, y: 40.0 });
    arena
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 80.0, y: 20.0 });
    arena.rebuild_active_aggregate_node_order();

    arena.mirror_subtree_axes(first_step, true, false, &BTreeSet::from([Some(container)]));

    // The first stable slot now carries the reflected sequence vessel. The
    // other hidden step still proves that rdfsWalk descended through the
    // sequence before visiting that vessel.
    assert_eq!(
        arena.position(first_step),
        Some(Point { x: -225.0, y: 40.0 })
    );
    assert_eq!(
        arena.position(second_step),
        Some(Point { x: -330.0, y: 40.0 })
    );
    assert_eq!(arena.position(peer), Some(Point { x: -480.0, y: 40.0 }));
}

#[test]
fn vertical_subtree_mirror_preserves_unreachable_nested_container_order() {
    let mut input = Graph::default();
    let room = input.add_node(node("room", 418.0, 690.0));
    let mut allowlist_node = node("allowlist", 298.0, 272.0);
    allowlist_node.parent = Some(room);
    let allowlist = input.add_node(allowlist_node);
    let mut gate_node = node("gate", 158.0, 66.0);
    gate_node.parent = Some(allowlist);
    let gate = input.add_node(gate_node);
    let mut token_node = node("token", 178.0, 66.0);
    token_node.parent = Some(allowlist);
    let token = input.add_node(token_node);
    let mut fleet_node = node("fleet", 263.0, 186.0);
    fleet_node.parent = Some(room);
    let fleet = input.add_node(fleet_node);
    let mut artifact_node = node("artifact", 81.0, 66.0);
    artifact_node.parent = Some(fleet);
    let artifact = input.add_node(artifact_node);
    let mut arena = ArenaGraph::from_input(&input);
    for (node, position) in [
        (room, Point { x: 0.0, y: 0.0 }),
        (allowlist, Point { x: 60.0, y: 358.0 }),
        (gate, Point { x: 120.0, y: 504.0 }),
        (token, Point { x: 120.0, y: 418.0 }),
        (fleet, Point { x: 60.0, y: 60.0 }),
        (artifact, Point { x: 120.0, y: 120.0 }),
    ] {
        arena.set_position(node, position);
    }

    arena.mirror_subtree_axes(room, false, true, &BTreeSet::from([None]));

    let room_y = arena.position(room).unwrap().y;
    assert_eq!(arena.position(fleet).unwrap().y - room_y, 60.0);
    assert_eq!(arena.position(artifact).unwrap().y - room_y, 120.0);
    assert_eq!(arena.position(allowlist).unwrap().y - room_y, 358.0);
    assert_eq!(arena.position(gate).unwrap().y - room_y, 504.0);
    assert_eq!(arena.position(token).unwrap().y - room_y, 418.0);
}

#[test]
fn compaction_axis_ties_use_recovered_numeric_node_ids() {
    let mut input = Graph::default();
    let enter = input.add_node(node("Check PIN.Enter PIN", 120.0, 120.0));
    let start = input.add_node(node("Check PIN.start", 10.0, 10.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(enter, Point { x: 63.0, y: 39.0 });
    arena.set_position(start, Point { x: 62.0, y: 39.0 });

    assert!(arena.nodes[start.0 as usize].tala_id < arena.nodes[enter.0 as usize].tala_id);
    assert_eq!(
        arena.ordered_subset_along_axis(&[enter, start], false),
        vec![start, enter]
    );
}

#[test]
fn sizeless_subgraph_floor_tracks_the_mutated_global_root() {
    let mut input = Graph::with_direction(Direction::Right);
    let users = input.add_node(node("users", 100.0, 100.0));
    let via = input.add_node(node("via", 100.0, 100.0));
    let teleport = input.add_node(node("teleport", 100.0, 100.0));
    let destination = input.add_node(node("jita", 100.0, 100.0));
    let identity = input.add_node(node("identity provider", 100.0, 100.0));
    input.add_edge(Edge {
        source: users,
        target: via,
    });
    input.add_edge(Edge {
        source: via,
        target: teleport,
    });
    let directed = input.add_edge(Edge {
        source: teleport,
        target: destination,
    });
    input.set_edge_arrows(
        directed,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let to_identity = input.add_edge(Edge {
        source: teleport,
        target: identity,
    });
    input.set_edge_arrows(
        to_identity,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let from_identity = input.add_edge(Edge {
        source: teleport,
        target: identity,
    });
    input.set_edge_arrows(
        from_identity,
        EdgeArrows {
            source: true,
            target: false,
        },
    );

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(users, Point { x: 22.0, y: 23.0 });
    arena.set_position(via, Point { x: 22.0, y: 25.0 });
    arena.set_position(teleport, Point { x: 25.0, y: 25.0 });
    arena.set_position(destination, Point { x: 23.0, y: 24.0 });
    arena.set_position(identity, Point { x: 23.0, y: 26.0 });
    let active = vec![users, via, teleport, destination, identity];
    let visibility = arena.sizeless_visibility_edges(true, &active);

    assert!(
        arena
            .shift_sizeless_subgraphs(true, 3.0, &visibility, &active)
            .unwrap()
    );
    assert_eq!(arena.position(users), Some(Point { x: 21.0, y: 23.0 }));
    // TALA keeps a pointer to `users` as globallyFurthestBehind. Once
    // users moves to x=21, via is no longer tied with that live anchor and
    // must not receive the extra two-cell floor decrease.
    assert_eq!(arena.position(via), Some(Point { x: 21.0, y: 25.0 }));
    assert_eq!(arena.position(teleport), Some(Point { x: 24.0, y: 25.0 }));
}

#[test]
fn transaction_validates_external_cluster_members_through_their_active_vessel() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 100.0));
    let second = input.add_node(node("second", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 500.0, y: 0.0 });

    let mut external = arena.nodes[first.0 as usize].clone();
    external.input_id = NodeId(u32::MAX);
    external.tala_id = -1_i64;
    external.position = Some(Point { x: 50.0, y: 0.0 });
    external.rect.origin = Point { x: 50.0, y: 0.0 };
    external.edges.clear();
    external.nears.clear();
    external.is_container = true;
    external.scoring_is_container = true;
    arena.transaction_external_containers.push(external);

    // A plain external container is checked at its own geometry.
    assert!(!arena.transaction_external_containers_are_valid());

    // TALA synchronizes an active cluster member from the vessel before
    // IsBadState. The flattened scope represents that moving boundary
    // with its current vessel node rather than this stale member snapshot.
    let vessel_tala_id = arena.nodes[second.0 as usize].tala_id;
    arena.transaction_external_containers[0].scoring_cluster_vessel = Some(vessel_tala_id);
    assert!(arena.transaction_external_containers_are_valid());
}

#[test]
fn transaction_rejects_a_local_node_that_escapes_an_external_container() {
    let mut input = Graph::default();
    let container_template = input.add_node(node("container", 100.0, 100.0));
    let child = input.add_node(node("child", 10.0, 10.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(child, Point { x: 10.0, y: -1.0 });

    let mut external = arena.nodes[container_template.0 as usize].clone();
    external.input_id = NodeId(u32::MAX);
    external.tala_id = i64::MAX;
    external.position = Some(Point { x: 0.0, y: 0.0 });
    external.rect.origin = Point { x: 0.0, y: 0.0 };
    external.edges.clear();
    external.nears.clear();
    external.is_container = true;
    external.scoring_is_container = true;
    let container_tala_id = external.tala_id;
    arena.nodes[child.0 as usize].scoring_container_ancestors = vec![container_tala_id];
    arena.transaction_external_containers.push(external);

    // Ancestor relationships exempt spacing overlap, but the child must still
    // remain strictly inside its positioned parent as Go's structural check
    // requires for a temporary placement scope.
    assert!(!arena.transaction_external_containers_are_valid());

    arena.set_position(child, Point { x: 10.0, y: 10.0 });
    assert!(arena.transaction_external_containers_are_valid());
}

#[test]
fn transaction_uses_refitted_external_container_when_validation_alias_is_stale() {
    let mut input = Graph::default();
    let local = input.add_node(node("local", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(local, Point { x: 0.0, y: 0.0 });

    let mut refitted = arena.nodes[local.0 as usize].clone();
    refitted.input_id = NodeId(u32::MAX);
    refitted.tala_id = i64::MAX - 1;
    refitted.position = Some(Point { x: 500.0, y: 0.0 });
    refitted.rect.origin = Point { x: 500.0, y: 0.0 };
    refitted.is_container = true;
    refitted.scoring_is_container = true;
    let stale_validation_alias = refitted.clone();
    arena.transaction_external_containers.push(refitted);
    arena
        .transaction_validation_external_containers
        .push(stale_validation_alias);

    // Go's transaction validates the shared container pointer after refitting
    // it. The validation-only copy must not reintroduce its stale geometry.
    assert!(arena.transaction_external_containers_are_valid());
}

#[test]
fn transaction_refits_projected_hexagon_only_once_without_active_containers() {
    const CONTAINER_TALA_ID: i64 = 92_100;

    let mut input = Graph::default();
    let mut parent_node = node("external hexagon", 393.0, 402.0);
    parent_node.shape = ShapeKind::Hexagon;
    let parent = input.add_node(parent_node);
    let child_specs = [
        ("upper-left", 63.0, 56.0, Point { x: 0.0, y: 82.0 }),
        ("top", 62.0, 62.0, Point { x: 0.0, y: 0.0 }),
        ("right", 60.0, 50.0, Point { x: 82.0, y: 0.0 }),
        ("lower-right", 50.0, 66.0, Point { x: 83.0, y: 82.0 }),
    ];
    let children = child_specs
        .iter()
        .map(|(name, width, height, _)| {
            let mut child = node(name, *width, *height);
            child.parent = Some(parent);
            input.add_node(child)
        })
        .collect::<Vec<_>>();
    let mut arena = ArenaGraph::from_input(&input);
    let mut projected_parent = arena.nodes[parent.0 as usize].clone();
    projected_parent.input_id = NodeId(u32::MAX);
    projected_parent.tala_id = CONTAINER_TALA_ID;
    projected_parent.position = Some(Point {
        x: 5680.0,
        y: 8839.0,
    });
    projected_parent.rect.origin = Point {
        x: 5680.0,
        y: 8839.0,
    };
    projected_parent.is_container = true;
    projected_parent.scoring_is_container = true;
    arena.transaction_external_containers.push(projected_parent);
    arena.transaction_external_container_children.insert(
        CONTAINER_TALA_ID,
        children
            .iter()
            .zip(child_specs.iter())
            .map(|(child, (_, _, _, position))| {
                let mut projected = arena.nodes[child.0 as usize].clone();
                projected.position = Some(*position);
                projected.rect.origin = *position;
                projected
            })
            .collect(),
    );

    // Go's unified container pass wraps the projected owner once when this
    // scope has no active containers. Repeating this hexagon refit shifts it
    // by one unit despite the child bounds staying fixed.
    arena.reposition_ordinary_containers();

    assert_eq!(
        arena.transaction_external_containers[0].position,
        Some(Point {
            x: -126.0,
            y: -127.0
        })
    );
}

#[test]
fn transaction_does_not_refit_unchanged_projected_diamond_twice() {
    const CONTAINER_TALA_ID: i64 = 92_101;

    let mut input = Graph::default();
    let mut parent_node = node("external diamond", 504.0, 666.0);
    parent_node.shape = ShapeKind::Diamond;
    let parent = input.add_node(parent_node);
    let child_specs = [
        ("18", 50.0, 59.0, Point { x: 81.0, y: 79.0 }),
        ("19", 53.0, 54.0, Point { x: 0.0, y: 153.0 }),
        ("20", 51.0, 59.0, Point { x: 81.0, y: 0.0 }),
        ("21", 50.0, 55.0, Point { x: 81.0, y: 158.0 }),
        ("22", 58.0, 50.0, Point { x: 0.0, y: 83.0 }),
        ("23", 61.0, 63.0, Point { x: 0.0, y: 0.0 }),
    ];
    let children = child_specs
        .iter()
        .map(|(name, width, height, _)| {
            let mut child = node(name, *width, *height);
            child.parent = Some(parent);
            input.add_node(child)
        })
        .collect::<Vec<_>>();

    // This positioned but unplaced container makes the arena take the second
    // refit path while leaving the external Diamond's children untouched.
    let other_parent = input.add_node(node("unplaced active container", 40.0, 40.0));
    let mut other_child = node("unplaced active child", 20.0, 20.0);
    other_child.parent = Some(other_parent);
    input.add_node(other_child);

    let mut arena = ArenaGraph::from_input(&input);
    let mut projected_parent = arena.nodes[parent.0 as usize].clone();
    projected_parent.input_id = NodeId(u32::MAX);
    projected_parent.tala_id = CONTAINER_TALA_ID;
    projected_parent.position = Some(Point {
        x: -186.0,
        y: -226.0,
    });
    projected_parent.rect.origin = Point {
        x: -186.0,
        y: -226.0,
    };
    projected_parent.rect.size = Size {
        width: 504.0,
        height: 666.0,
    };
    projected_parent.is_container = true;
    projected_parent.scoring_is_container = true;
    arena.transaction_external_containers.push(projected_parent);
    arena.transaction_external_container_children.insert(
        CONTAINER_TALA_ID,
        children
            .iter()
            .zip(child_specs.iter())
            .enumerate()
            .map(|(index, (child, (_, _, _, position)))| {
                let mut projected = arena.nodes[child.0 as usize].clone();
                projected.tala_id = 18 + index as i64;
                projected.position = Some(*position);
                projected.rect.origin = *position;
                projected
            })
            .collect(),
    );
    arena.set_position(
        other_parent,
        Point {
            x: 1000.0,
            y: 1000.0,
        },
    );

    arena.reposition_ordinary_containers();

    assert_eq!(
        arena.transaction_external_containers[0].position,
        Some(Point {
            x: -186.0,
            y: -227.0
        }),
        "the external Diamond is wrapped once when unrelated active geometry is unchanged"
    );
}

#[test]
fn transaction_keeps_the_refitted_origin_for_a_live_external_container() {
    let mut input = Graph::default();
    let local = input.add_node(node("local", 10.0, 10.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(
        local,
        Point {
            x: 600.0,
            y: -120.0,
        },
    );

    let parent_tala_id = i64::MAX - 2;
    let mut parent = arena.nodes[local.0 as usize].clone();
    parent.input_id = NodeId(u32::MAX);
    parent.tala_id = parent_tala_id;
    parent.position = Some(Point { x: 98.0, y: -326.0 });
    parent.rect.origin = Point { x: 98.0, y: -326.0 };
    parent.rect.size = Size {
        width: 1302.0,
        height: 1302.0,
    };
    parent.edges.clear();
    parent.nears.clear();
    parent.is_container = true;
    parent.scoring_is_container = true;

    arena.nodes[local.0 as usize].scoring_container_ancestors = vec![parent_tala_id];
    let mut child_projection = arena.nodes[local.0 as usize].clone();
    child_projection.input_id = NodeId(u32::MAX - 1);
    arena
        .transaction_external_container_children
        .insert(parent_tala_id, vec![child_projection]);
    arena.transaction_external_containers.push(parent);

    // This live carrier has already been refitted around child coordinates.
    // Its 502px x-offset happens to cross the split-scope frame heuristic's
    // threshold, but resetting its origin to (0, 0) would make the negative-y
    // child appear to escape. Go validates the shared, refitted pointer at
    // its actual origin.
    assert!(arena.transaction_external_containers_are_valid());
}

#[test]
fn transaction_does_not_pairwise_check_external_containers() {
    let mut input = Graph::default();
    let local = input.add_node(node("local", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(local, Point { x: 0.0, y: 0.0 });

    let mut first = arena.nodes[local.0 as usize].clone();
    first.input_id = NodeId(u32::MAX - 1);
    first.tala_id = -1_i64 - 1;
    first.position = Some(Point { x: 500.0, y: 0.0 });
    first.rect.origin = Point { x: 500.0, y: 0.0 };
    first.is_container = true;
    first.scoring_is_container = true;

    let mut second = first.clone();
    second.input_id = NodeId(u32::MAX);
    second.tala_id = -1_i64;
    second.position = Some(Point { x: 550.0, y: 0.0 });
    second.rect.origin = Point { x: 550.0, y: 0.0 };

    arena.transaction_external_containers = vec![first, second];
    // Transaction.Commit invokes IsBadState for each positioned external
    // container, and IsBadState compares that container only with Graph.Nodes.
    // Separate keys in Graph.Containers are not compared with one another.
    assert!(arena.transaction_external_containers_are_valid());
}

#[test]
fn external_transaction_container_tracks_its_represented_ancestor() {
    let mut input = Graph::default();
    let anchor = input.add_node(node("anchor", 100.0, 100.0));
    let obstacle = input.add_node(node("obstacle", 100.0, 100.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(anchor, Point { x: 0.0, y: 0.0 });
    arena.set_position(obstacle, Point { x: 500.0, y: 0.0 });

    let anchor_tala_id = arena.nodes[anchor.0 as usize].tala_id;
    let mut external = arena.nodes[anchor.0 as usize].clone();
    external.input_id = NodeId(u32::MAX);
    external.tala_id = -1_i64;
    external.position = Some(Point { x: 50.0, y: 0.0 });
    external.rect.origin = Point { x: 50.0, y: 0.0 };
    external.scoring_container_ancestors = vec![anchor_tala_id];
    external.transaction_anchor_tala_id = Some(anchor_tala_id);
    external.transaction_anchor_offset = Some(Point { x: 50.0, y: 0.0 });
    arena.transaction_external_containers.push(external);

    assert!(arena.transaction_external_containers_are_valid());
    arena.set_position(anchor, Point { x: 450.0, y: 0.0 });
    assert!(!arena.transaction_external_containers_are_valid());
}

#[test]
fn external_transaction_container_refits_anchored_children() {
    let mut input = Graph::default();
    let anchor = input.add_node(node("anchor", 100.0, 100.0));
    let container_template = input.add_node(node("container", 473.0, 358.0));
    let child_template = input.add_node(node("child", 267.0, 238.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(
        anchor,
        Point {
            x: 1000.0,
            y: 2000.0,
        },
    );
    let anchor_tala_id = arena.nodes[anchor.0 as usize].tala_id;

    let mut container = arena.nodes[container_template.0 as usize].clone();
    container.tala_id = -1_i64 - 1;
    container.is_container = true;
    container.scoring_is_container = true;
    container.node_padding = Insets {
        top: 60.0,
        right: 129.0,
        bottom: 60.0,
        left: 60.0,
    };
    container.position = Some(Point {
        x: 1065.0,
        y: 2050.0,
    });
    container.rect.origin = container.position.unwrap();
    // A nested scope has already materialized its child coordinates in the
    // owning frame. The enclosing transaction must still refit this shared
    // container, matching Go's pointer-visible wrapChildren lifecycle.
    container.scope_translation_materialized = true;
    container.transaction_anchor_tala_id = Some(anchor_tala_id);
    container.transaction_anchor_offset = Some(Point { x: 65.0, y: 50.0 });

    let mut child = arena.nodes[child_template.0 as usize].clone();
    child.position = Some(Point {
        x: 1142.0,
        y: 2110.0,
    });
    child.rect.origin = child.position.unwrap();
    child.scope_translation_materialized = true;
    child.transaction_anchor_tala_id = Some(anchor_tala_id);
    child.transaction_anchor_offset = Some(Point { x: 142.0, y: 110.0 });

    arena.transaction_external_containers.push(container);
    arena
        .transaction_external_container_children
        .insert(-1_i64 - 1, vec![child]);
    arena.reposition_ordinary_containers();

    assert_eq!(
        arena.transaction_external_containers[0].rect.size,
        Size {
            width: 456.0,
            height: 358.0,
        }
    );
    assert_eq!(
        arena.transaction_external_containers[0].position,
        Some(Point {
            x: 1082.0,
            y: 2050.0,
        })
    );

    arena.set_position(
        anchor,
        Point {
            x: 1200.0,
            y: 2300.0,
        },
    );
    arena.reposition_ordinary_containers();
    assert_eq!(
        arena.transaction_external_containers[0].position,
        Some(Point {
            x: 1282.0,
            y: 2350.0,
        })
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn sized_transpose_rollback_retains_an_external_ordinary_container_refit() {
    let mut input = Graph::with_direction(Direction::Right);
    let moving = input.add_node(node("moving", 100.0, 100.0));
    let center = input.add_node(node("center", 100.0, 100.0));
    input.add_edge(Edge {
        source: moving,
        target: center,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(moving, Point::default());
    arena.set_position(center, Point { x: 300.0, y: 0.0 });

    let mut external = arena.nodes[center.0 as usize].clone();
    external.input_id = NodeId(u32::MAX);
    external.tala_id = -1_i64;
    external.position = Some(Point {
        x: 1000.0,
        y: 1000.0,
    });
    external.rect.origin = external.position.unwrap();
    external.rect.size = Size {
        width: 500.0,
        height: 300.0,
    };
    external.is_container = true;
    external.scoring_is_container = true;
    external.grid_columns = Some(1);
    external.content_insets = Insets::uniform(60.0);

    let mut external_child = arena.nodes[moving.0 as usize].clone();
    external_child.input_id = NodeId(u32::MAX - 1);
    external_child.tala_id = -1_i64 - 1;
    external_child.position = Some(Point {
        x: 1060.0,
        y: 1060.0,
    });
    external_child.rect.origin = external_child.position.unwrap();
    external_child.rect.size = Size {
        width: 100.0,
        height: 80.0,
    };

    arena.transaction_external_containers.push(external);
    arena
        .transaction_external_container_children
        .insert(-1_i64, vec![external_child]);

    // Recovered GraphState snapshots the current Graph.Nodes plus
    // cluster/sequence members. This ordinary nested container is outside
    // all three sets, so a rejected AffectContainers transaction rolls
    // back the active nodes while leaving wrapChildren's refit observable.
    assert!(!arena.transpose_node(moving, true));
    assert_eq!(
        arena.transaction_external_containers[0].rect.size,
        Size {
            width: 220.0,
            height: 200.0,
        }
    );
    assert_eq!(
        arena.transaction_external_containers[0].position,
        Some(Point {
            x: 1000.0,
            y: 1000.0,
        })
    );
}

#[test]
fn induced_copyback_preserves_hidden_external_refit_over_stale_child_alias() {
    const PARENT_TALA_ID: i64 = 233_611_931;
    const CONTAINER_TALA_ID: i64 = 1_149_337_423;
    const LEAF_TALA_ID: i64 = 1_402_186_134;

    let mut input = Graph::default();
    let template = input.add_node(node("active", 100.0, 100.0));
    let mut owner = ArenaGraph::from_input(&input);

    let mut carrier = owner.nodes[template.0 as usize].clone();
    carrier.input_id = NodeId(u32::MAX);
    carrier.tala_id = CONTAINER_TALA_ID;
    carrier.is_container = true;
    carrier.scoring_is_container = true;
    carrier.position = Some(Point {
        x: -676.0,
        y: 32_900.0,
    });
    carrier.rect.origin = carrier.position.unwrap();
    carrier.rect.size = Size {
        width: 452.0,
        height: 358.0,
    };

    let mut stale_parent_child = carrier.clone();
    stale_parent_child.input_id = NodeId(u32::MAX - 1);
    stale_parent_child.scoring_container_parent = Some(PARENT_TALA_ID);

    let mut leaf = owner.nodes[template.0 as usize].clone();
    leaf.input_id = NodeId(u32::MAX - 2);
    leaf.tala_id = LEAF_TALA_ID;
    leaf.position = Some(Point {
        x: -561.0,
        y: 33_046.0,
    });
    leaf.rect.origin = leaf.position.unwrap();

    owner.transaction_external_containers.push(carrier);
    owner
        .transaction_external_container_children
        .insert(PARENT_TALA_ID, vec![stale_parent_child]);
    owner
        .transaction_external_container_children
        .insert(CONTAINER_TALA_ID, vec![leaf]);

    assert!(
        owner
            .nodes
            .iter()
            .all(|node| !matches!(node.tala_id, CONTAINER_TALA_ID | LEAF_TALA_ID))
    );
    assert!(owner.transaction_external_aggregate_children.is_empty());

    let mut placed = owner.clone();
    let refitted = placed
        .transaction_external_containers
        .iter_mut()
        .find(|node| node.tala_id == CONTAINER_TALA_ID)
        .unwrap();
    refitted.position = Some(Point {
        x: -675.0,
        y: 32_900.0,
    });
    refitted.rect.origin = refitted.position.unwrap();
    refitted.rect.size = Size {
        width: 442.0,
        height: 358.0,
    };
    placed
        .transaction_external_container_children
        .get_mut(&PARENT_TALA_ID)
        .unwrap()[0]
        .rect
        .size = refitted.rect.size;

    Pipeline::publish_induced_external_geometry(&mut owner, &placed);

    let published = owner
        .transaction_external_containers
        .iter()
        .find(|node| node.tala_id == CONTAINER_TALA_ID)
        .unwrap();
    let stale_parent_child = &owner.transaction_external_container_children[&PARENT_TALA_ID][0];
    let leaf = &owner.transaction_external_container_children[&CONTAINER_TALA_ID][0];

    assert_eq!(published.position.unwrap().x, -675.0);
    assert_eq!(published.rect.origin.x, -675.0);
    assert_eq!(published.rect.size.width, 442.0);
    assert_eq!(stale_parent_child.position.unwrap().x, -676.0);
    assert_eq!(stale_parent_child.rect.size.width, 442.0);
    assert_eq!(leaf.position.unwrap().x, -561.0);
    assert_eq!(
        leaf.position.unwrap().x - published.position.unwrap().x,
        114.0
    );
    assert!(owner.transaction_external_aggregate_children.is_empty());
}

#[test]
fn obstructed_mirrored_pair_does_not_earn_symmetry_credit() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 20.0, 20.0));
    let above = input.add_node(node("above", 20.0, 20.0));
    let below = input.add_node(node("below", 20.0, 20.0));
    let obstruction = input.add_node(node("obstruction", 20.0, 20.0));
    for index in 0..7 {
        input.add_node(node(&format!("filler {index}"), 20.0, 20.0));
    }
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 100.0, y: 100.0 });
    arena.set_position(above, Point { x: 100.0, y: 0.0 });
    arena.set_position(below, Point { x: 100.0, y: 200.0 });
    arena.set_position(obstruction, Point { x: 100.0, y: 50.0 });

    assert!(arena.symmetry_pair_obstructed(center, above, below));
    assert_eq!(
        arena.symmetry_pair_score(center, &[above, below]),
        (0.0, BTreeSet::new())
    );

    arena.set_position(obstruction, Point { x: 300.0, y: 50.0 });
    assert!(!arena.symmetry_pair_obstructed(center, above, below));
    assert_eq!(arena.symmetry_pair_score(center, &[above, below]).0, 2.0);
}

#[test]
fn mirrored_neighbors_in_different_containers_do_not_earn_symmetry_credit() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 20.0, 20.0));
    let first_container = input.add_node(node("first container", 300.0, 300.0));
    let second_container = input.add_node(node("second container", 300.0, 300.0));
    let mut first_node = node("first", 20.0, 20.0);
    first_node.parent = Some(first_container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 20.0, 20.0);
    second_node.parent = Some(second_container);
    let second = input.add_node(second_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 100.0, y: 100.0 });
    arena.set_position(first, Point { x: 100.0, y: 0.0 });
    arena.set_position(second, Point { x: 100.0, y: 200.0 });

    assert_eq!(
        arena.symmetry_pair_score(center, &[first, second]),
        (0.0, BTreeSet::new())
    );
}

#[test]
fn sized_symmetry_recurses_through_an_unmatched_self_edge() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 20.0, 20.0));
    let above = input.add_node(node("above", 20.0, 20.0));
    let below = input.add_node(node("below", 20.0, 20.0));
    input.add_edge(Edge {
        source: center,
        target: center,
    });
    input.add_edge(Edge {
        source: center,
        target: above,
    });
    input.add_edge(Edge {
        source: center,
        target: below,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 100.0, y: 100.0 });
    arena.set_position(above, Point { x: 100.0, y: 0.0 });
    arena.set_position(below, Point { x: 100.0, y: 200.0 });

    // Go's unmatched-neighbor recursion includes the center itself when
    // a self-edge is present. It calls getSymmetry(checkNeighbors=false),
    // so this is finite and contributes the center's direct score again.
    assert_eq!(arena.sized_symmetry(center, false), 2.0 / 3.0);
    assert_eq!(arena.sized_symmetry(center, true), 8.0 / 9.0);
}

#[test]
fn sized_symmetry_compares_the_restored_abducted_endpoint_container() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 100.0, 100.0));
    let left = input.add_node(node("left", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 100.0, 100.0));
    input.add_edge(Edge {
        source: center,
        target: left,
    });
    let carrier_edge = input.add_edge(Edge {
        source: center,
        target: carrier,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 200.0, y: 100.0 });
    arena.set_position(left, Point { x: 0.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 400.0, y: 100.0 });
    assert_eq!(arena.sized_symmetry(center, false), 1.0);

    arena.sized_adjacent_overrides.insert(
        (center, carrier_edge),
        ProjectedAdjacent {
            owner: carrier,
            tala_id: 99,
            container_tala_id: Some(arena.nodes[carrier.0 as usize].tala_id),
            offset: Point::default(),
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            cluster_member: false,
        },
    );
    assert_eq!(arena.sized_symmetry(center, false), 0.0);
}

#[test]
fn sized_symmetry_recurses_through_an_unmatched_abducted_endpoint() {
    let mut input = Graph::default();
    let left = input.add_node(node("left", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 100.0, 100.0));
    let right = input.add_node(node("right", 100.0, 100.0));
    let left_edge = input.add_edge(Edge {
        source: left,
        target: carrier,
    });
    let right_edge = input.add_edge(Edge {
        source: carrier,
        target: right,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(left, Point { x: 0.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 200.0, y: 100.0 });
    arena.set_position(right, Point { x: 400.0, y: 100.0 });
    let original_child = arena.nodes[carrier.0 as usize].tala_id.wrapping_add(1);
    for (owner, edge) in [(left, left_edge), (right, right_edge)] {
        let child = ProjectedAdjacent {
            owner: carrier,
            tala_id: original_child,
            container_tala_id: Some(arena.nodes[carrier.0 as usize].tala_id),
            offset: Point::default(),
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            cluster_member: false,
        };
        arena.sized_adjacent_overrides.insert((owner, edge), child);
        // Supply the real EdgeAbduction inputs described by the endpoint
        // hints. Source symmetry.go:290 recovers the child's adjacency from
        // this ordered slice, not from the direct-neighbor lookup hints.
        arena
            .sized_edge_abductions
            .push(crate::internal::model::SizedEdgeAbduction {
                source_edge: edge,
                edge_tala_id: arena.edge_records[edge.0 as usize].tala_id,
                edge: Some(edge),
                current_from: Some(AbductionCurrentNodeRef::local(
                    arena.edge_records[edge.0 as usize].from,
                )),
                current_to: Some(AbductionCurrentNodeRef::local(
                    arena.edge_records[edge.0 as usize].to,
                )),
                originally_from: (owner == right).then_some(child.into()),
                originally_to: (owner == left).then_some(child.into()),
                originally_from_table_neighbors: Vec::new(),
                originally_to_table_neighbors: Vec::new(),
                originally_from_container: None,
                originally_to_container: None,
                sequence_abduction: false,
                obstructions_from_to: Vec::new(),
                obstructions_to_from: Vec::new(),
                obstruction_sets_from_to: OrderedObstructionInventory::default(),
                obstruction_sets_to_from: OrderedObstructionInventory::default(),
            });
    }

    // The right node has one unmatched neighbor: the original child
    // endpoint now represented by `carrier`. Recovered getSymmetry
    // recursively evaluates that original child, where left and right
    // form a mirrored pair, and propagates its unit score.
    assert_eq!(arena.sized_symmetry(right, true), 1.0);
    assert_eq!(arena.sized_symmetry(right, false), 0.0);
}

#[test]
fn sized_symmetry_keeps_distinct_abducted_endpoints_on_one_carrier() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 100.0, 100.0));
    let left = input.add_node(node("left", 100.0, 100.0));
    let right = input.add_node(node("right", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 100.0, 100.0));
    input.add_edge(Edge {
        source: center,
        target: left,
    });
    input.add_edge(Edge {
        source: center,
        target: right,
    });
    let first_projected = input.add_edge(Edge {
        source: center,
        target: carrier,
    });
    let second_projected = input.add_edge(Edge {
        source: center,
        target: carrier,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 200.0, y: 100.0 });
    arena.set_position(left, Point { x: 0.0, y: 100.0 });
    arena.set_position(right, Point { x: 400.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 600.0, y: 100.0 });
    for (edge, tala_id, container_tala_id, offset) in [
        (first_projected, 99, Some(1001), Point::default()),
        (
            second_projected,
            100,
            Some(1002),
            Point { x: 100.0, y: 0.0 },
        ),
    ] {
        arena.sized_adjacent_overrides.insert(
            (center, edge),
            ProjectedAdjacent {
                owner: carrier,
                tala_id,
                container_tala_id,
                offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
    }

    // TALA restores both original endpoints before deduplication. They
    // therefore remain two denominator entries even though both current
    // edge endpoints are the same abducting carrier.
    assert_eq!(arena.sized_symmetry(center, false), 0.5);
}

#[test]
fn sized_symmetry_deduplicates_active_cluster_member_edges_to_the_vessel() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 100.0, 100.0));
    let first_member = input.add_node(node("first member", 100.0, 100.0));
    let second_member = input.add_node(node("second member", 100.0, 100.0));
    let top = input.add_node(node("top", 100.0, 100.0));
    let bottom = input.add_node(node("bottom", 100.0, 100.0));
    let first_edge = input.add_edge(Edge {
        source: center,
        target: first_member,
    });
    let second_edge = input.add_edge(Edge {
        source: center,
        target: second_member,
    });
    input.add_edge(Edge {
        source: center,
        target: top,
    });
    input.add_edge(Edge {
        source: center,
        target: bottom,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 200.0, y: 200.0 });
    arena.set_position(first_member, Point { x: 0.0, y: 200.0 });
    arena.set_position(top, Point { x: 200.0, y: 0.0 });
    arena.set_position(bottom, Point { x: 200.0, y: 400.0 });
    arena.cell_size = 100.0;
    arena.clusters.push(ClusterState {
        members: vec![first_member, second_member],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 10.0,
        vessel_tala_id: 10_000,
        fixed_size: true,
    });
    arena.node_order = vec![center, first_member, top, bottom];
    for (edge, tala_id, offset) in [
        (first_edge, 10_001, Point::default()),
        (second_edge, 10_002, Point { x: 110.0, y: 0.0 }),
    ] {
        arena.sized_adjacent_overrides.insert(
            (center, edge),
            ProjectedAdjacent {
                owner: first_member,
                tala_id,
                container_tala_id: None,
                offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: true,
            },
        );
    }

    // The top/bottom pair earns 2 points. TALA exposes three deduplicated
    // neighbors to getSymmetry: the cluster vessel, top, and bottom.
    assert_eq!(arena.sized_symmetry(center, false), 2.0 / 3.0);
}

#[test]
fn sized_symmetry_deduplicates_flattened_aggregate_edges_to_the_vessel() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 100.0, 100.0));
    let vessel = input.add_node(node("vessel", 300.0, 100.0));
    let top = input.add_node(node("top", 100.0, 100.0));
    let bottom = input.add_node(node("bottom", 100.0, 100.0));
    let first_edge = input.add_edge(Edge {
        source: center,
        target: vessel,
    });
    let second_edge = input.add_edge(Edge {
        source: center,
        target: vessel,
    });
    input.add_edge(Edge {
        source: center,
        target: top,
    });
    input.add_edge(Edge {
        source: center,
        target: bottom,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 200.0, y: 200.0 });
    arena.set_position(vessel, Point { x: 400.0, y: 200.0 });
    arena.set_position(top, Point { x: 200.0, y: 0.0 });
    arena.set_position(bottom, Point { x: 200.0, y: 400.0 });
    arena.cell_size = 100.0;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;
    for (edge, tala_id, offset) in [
        (first_edge, 10_001, Point::default()),
        (second_edge, 10_002, Point { x: 110.0, y: 0.0 }),
    ] {
        arena.sized_adjacent_overrides.insert(
            (center, edge),
            ProjectedAdjacent {
                owner: vessel,
                tala_id,
                container_tala_id: None,
                offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: true,
            },
        );
    }

    // TALA skips endpoint restoration when the current endpoint is either
    // kind of active aggregate vessel. Both edges therefore contribute
    // one deduplicated vessel neighbor beside the top/bottom pair.
    assert_eq!(arena.sized_symmetry(center, false), 2.0 / 3.0);
}

#[test]
fn sized_symmetry_does_not_restore_edges_from_an_aggregate_center() {
    let mut input = Graph::default();
    let center = input.add_node(node("center vessel", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 100.0, 100.0));
    let first_edge = input.add_edge(Edge {
        source: center,
        target: carrier,
    });
    let second_edge = input.add_edge(Edge {
        source: center,
        target: carrier,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(center, Point { x: 200.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 600.0, y: 100.0 });
    arena.nodes[center.0 as usize].scoring_is_aggregate_vessel = true;
    for (edge, tala_id, offset) in [
        (first_edge, 10_001, Point { x: -600.0, y: 0.0 }),
        (second_edge, 10_002, Point { x: -200.0, y: 0.0 }),
    ] {
        arena.sized_adjacent_overrides.insert(
            (center, edge),
            ProjectedAdjacent {
                owner: carrier,
                tala_id,
                container_tala_id: None,
                offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
    }

    // getSymmetry skips every EdgeAbduction whose current From or To is
    // a cluster/sequence vessel. Both edges therefore remain attached to
    // one deduplicated carrier instead of becoming a mirrored member pair.
    assert_eq!(arena.sized_symmetry(center, false), 0.0);
}

#[test]
fn sized_symmetry_container_consumes_reverse_abductions_before_restoration() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 300.0, 100.0));
    let first_edge = input.add_edge(Edge {
        source: carrier,
        target: container,
    });
    let second_edge = input.add_edge(Edge {
        source: carrier,
        target: container,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(container, Point { x: 200.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 0.0, y: 100.0 });
    arena.nodes[container.0 as usize].scoring_is_container = true;
    for (edge, tala_id, offset) in [
        (first_edge, 10_001, Point::default()),
        (second_edge, 10_002, Point { x: 400.0, y: 0.0 }),
    ] {
        arena.sized_adjacent_overrides.insert(
            (container, edge),
            ProjectedAdjacent {
                owner: carrier,
                tala_id,
                container_tala_id: None,
                offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
    }

    // The two restored endpoints would mirror around the container.
    // TALA's container pre-scan consumes both reverse abductions first,
    // leaving one deduplicated current carrier and therefore no pair.
    assert_eq!(arena.sized_symmetry(container, false), 0.0);
}

#[test]
fn sized_symmetry_scores_forward_abducted_container_children() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 100.0, 100.0));
    let first_edge = input.add_edge(Edge {
        source: container,
        target: carrier,
    });
    let second_edge = input.add_edge(Edge {
        source: container,
        target: carrier,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(container, Point { x: 200.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 600.0, y: 100.0 });
    arena.nodes[container.0 as usize].scoring_is_container = true;

    for (edge, child_id, adjacent_id, adjacent_offset) in [
        (first_edge, 10_001, 20_001, Point { x: -600.0, y: 0.0 }),
        (second_edge, 10_002, 20_002, Point { x: -200.0, y: 0.0 }),
    ] {
        // OriginallyFrom: a child inside the current From container.
        arena.sized_adjacent_overrides.insert(
            (carrier, edge),
            ProjectedAdjacent {
                owner: container,
                tala_id: child_id,
                container_tala_id: Some(arena.nodes[container.0 as usize].tala_id),
                offset: Point::default(),
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
        // OriginallyTo: an external endpoint. These two boxes mirror
        // around the container and would incorrectly earn full credit if
        // the current carrier edges were scored directly.
        arena.sized_adjacent_overrides.insert(
            (container, edge),
            ProjectedAdjacent {
                owner: carrier,
                tala_id: adjacent_id,
                container_tala_id: None,
                offset: adjacent_offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
    }

    // Recovered getSymmetry scores the two OriginallyFrom children,
    // consumes both abductions, and skips the two current carrier edges.
    // Each original child has only one neighbor and therefore contributes
    // zero symmetry with a denominator of one.
    assert_eq!(arena.sized_symmetry(container, false), 0.0);
}

#[test]
fn projected_child_symmetry_ignores_disconnected_arena_edges() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 100.0, 100.0));
    let carrier = input.add_node(node("carrier", 100.0, 100.0));
    let active_edge = input.add_edge(Edge {
        source: container,
        target: carrier,
    });
    let disconnected_edge = input.add_edge(Edge {
        source: container,
        target: carrier,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(container, Point { x: 200.0, y: 100.0 });
    arena.set_position(carrier, Point { x: 0.0, y: 100.0 });
    arena.cell_size = 100.0;
    arena.nodes[container.0 as usize].scoring_is_container = true;
    arena.edge_order.retain(|edge| *edge != disconnected_edge);

    let child = ProjectedAdjacent {
        owner: container,
        tala_id: 10_001,
        container_tala_id: Some(arena.nodes[container.0 as usize].tala_id),
        offset: Point::default(),
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        cluster_member: false,
    };
    let left = ProjectedAdjacent {
        owner: carrier,
        tala_id: 20_001,
        container_tala_id: None,
        offset: Point::default(),
        size: child.size,
        cluster_member: false,
    };
    let right = ProjectedAdjacent {
        owner: carrier,
        tala_id: 20_002,
        container_tala_id: None,
        offset: Point { x: 400.0, y: 0.0 },
        size: child.size,
        cluster_member: false,
    };
    arena
        .sized_adjacent_overrides
        .insert((carrier, active_edge), child);
    arena
        .sized_adjacent_overrides
        .insert((container, active_edge), left);
    arena
        .sized_adjacent_overrides
        .insert((carrier, disconnected_edge), child);
    arena
        .sized_adjacent_overrides
        .insert((container, disconnected_edge), right);
    arena.sized_edge_abductions.push(SizedEdgeAbduction {
        source_edge: active_edge,
        edge_tala_id: arena.edge_records[active_edge.0 as usize].tala_id,
        edge: Some(active_edge),
        current_from: Some(AbductionCurrentNodeRef::local(container)),
        current_to: Some(AbductionCurrentNodeRef::local(carrier)),
        originally_from: Some(child.into()),
        originally_to: Some(left.into()),
        originally_from_table_neighbors: Vec::new(),
        originally_to_table_neighbors: Vec::new(),
        originally_from_container: None,
        originally_to_container: None,
        sequence_abduction: false,
        obstructions_from_to: Vec::new(),
        obstructions_to_from: Vec::new(),
        obstruction_sets_from_to: OrderedObstructionInventory::default(),
        obstruction_sets_to_from: OrderedObstructionInventory::default(),
    });

    // Stable arena storage retains the disconnected second edge, but TALA's
    // original child Node.Edges slice does not. Resurrecting it would turn
    // left/right into a mirrored pair and incorrectly return one.
    assert_eq!(arena.sized_symmetry(container, false), 0.0);
}

#[test]
fn abducted_child_symmetry_keeps_an_active_sequence_vessel() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 100.0, 100.0));
    let vessel = input.add_node(node("sequence vessel", 300.0, 100.0));
    let ordinary = input.add_node(node("ordinary", 100.0, 100.0));
    let vessel_edge = input.add_edge(Edge {
        source: container,
        target: vessel,
    });
    let ordinary_edge = input.add_edge(Edge {
        source: container,
        target: ordinary,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(container, Point { x: 200.0, y: 200.0 });
    arena.set_position(vessel, Point { x: 600.0, y: 200.0 });
    arena.set_position(ordinary, Point { x: 400.0, y: 200.0 });
    arena.nodes[container.0 as usize].scoring_is_container = true;
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;

    for (edge, carrier, opposite_id, opposite_offset) in [
        (vessel_edge, vessel, 20_001, Point { x: -600.0, y: 0.0 }),
        (ordinary_edge, ordinary, 20_002, Point::default()),
    ] {
        arena.sized_adjacent_overrides.insert(
            (carrier, edge),
            ProjectedAdjacent {
                owner: container,
                tala_id: 10_001,
                container_tala_id: Some(arena.nodes[container.0 as usize].tala_id),
                offset: Point::default(),
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
        arena.sized_adjacent_overrides.insert(
            (container, edge),
            ProjectedAdjacent {
                owner: carrier,
                tala_id: opposite_id,
                container_tala_id: None,
                offset: opposite_offset,
                size: Size {
                    width: 100.0,
                    height: 100.0,
                },
                cluster_member: false,
            },
        );
    }

    // Restoring both original opposite endpoints would manufacture a
    // mirrored pair around child 10_001. TALA keeps the current sequence
    // vessel for the first edge, so the pair does not exist.
    assert_eq!(arena.sized_symmetry(container, false), 0.0);
}

#[test]
fn semi_diagonal_edge_uses_clear_alternate_before_double_turn_penalty() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 171.0, 69.0));
    let target = input.add_node(node("target", 171.0, 66.0));
    let lower_obstruction = input.add_node(node("lower", 54.0, 66.0));
    let upper_obstruction = input.add_node(node("upper", 54.0, 20.0));
    let edge = input.add_edge(Edge { source, target });
    input.add_edge(Edge {
        source,
        target: lower_obstruction,
    });
    input.add_edge(Edge {
        source: lower_obstruction,
        target: upper_obstruction,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(source, Point { x: 0.0, y: 64.0 });
    arena.set_position(target, Point { x: 352.0, y: 0.0 });
    arena.set_position(lower_obstruction, Point { x: 224.0, y: 0.0 });
    arena.set_turn_cost(100.0);

    assert_eq!(arena.sized_orientation(source, target), Orientation::Left);
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, None, None, false),
        100.0
    );

    arena.set_position(upper_obstruction, Point { x: 224.0, y: 90.0 });
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, None, None, false),
        200.0
    );
}

#[test]
fn global_score_reads_restored_tree_obstructions_only_in_materialized_mode() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 600.0, 300.0));
    let mut source_node = node("source", 171.0, 69.0);
    source_node.parent = Some(container);
    let source = input.add_node(source_node);
    let mut target_node = node("target", 171.0, 66.0);
    target_node.parent = Some(container);
    let target = input.add_node(target_node);
    let mut lower_node = node("lower restored obstruction", 54.0, 66.0);
    lower_node.parent = Some(container);
    let lower = input.add_node(lower_node);
    let mut upper_node = node("upper restored obstruction", 54.0, 20.0);
    upper_node.parent = Some(container);
    let upper = input.add_node(upper_node);
    input.add_edge(Edge { source, target });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(container, Point::default());
    arena.set_position(source, Point { x: 0.0, y: 64.0 });
    arena.set_position(target, Point { x: 352.0, y: 0.0 });
    arena.set_position(lower, Point { x: 224.0, y: 0.0 });
    arena.set_position(upper, Point { x: 224.0, y: 90.0 });
    arena.set_turn_cost(100.0);
    arena
        .preprocessed_tree_children
        .insert(Some(container), vec![source, target]);

    let stale = arena.global_sized_edge_length();
    let materialized = arena.global_sized_edge_length_after_tree_restoration(true);
    assert!(((materialized - stale) - 400.0).abs() < 1e-9);
}

#[test]
fn parallel_global_scoring_preserves_serial_terms_and_repeatability() {
    let mut input = Graph::default();
    let nodes = (0..12)
        .map(|index| input.add_node(node(&format!("node-{index}"), 54.0, 66.0)))
        .collect::<Vec<_>>();
    for pair in nodes.windows(2) {
        input.add_edge(Edge {
            source: pair[0],
            target: pair[1],
        });
    }

    let mut arena = ArenaGraph::from_input(&input);
    for (index, node_id) in nodes.iter().copied().enumerate() {
        arena.set_position(
            node_id,
            Point {
                x: index as f64 * 120.0,
                y: (index % 3) as f64 * 90.0,
            },
        );
    }

    let serial = arena.global_sized_edge_length_with_direction_serial_for_test(true);
    let parallel = arena.global_sized_edge_length_with_direction(true);
    assert_eq!(parallel.to_bits(), serial.to_bits());

    let serial_alignment = arena.global_sized_edge_length_for_alignment_serial_for_test();
    let parallel_alignment = arena.global_sized_edge_length_for_alignment();
    assert_eq!(parallel_alignment.to_bits(), serial_alignment.to_bits());

    for _ in 0..8 {
        assert_eq!(
            arena
                .global_sized_edge_length_with_direction(true)
                .to_bits(),
            parallel.to_bits()
        );
        assert_eq!(
            arena.global_sized_edge_length_for_alignment().to_bits(),
            parallel_alignment.to_bits()
        );
    }
}

#[test]
fn active_cluster_edges_route_from_their_retained_member_replacements() {
    let mut input = Graph::default();
    let first = input.add_node(node("first member", 145.0, 92.0));
    let second = input.add_node(node("second member", 145.0, 92.0));
    let third = input.add_node(node("third member", 145.0, 92.0));
    let fourth = input.add_node(node("fourth member", 145.0, 92.0));
    let endpoint = input.add_node(node("endpoint", 136.0, 120.0));
    let obstruction = input.add_node(node("obstruction", 390.0, 358.0));
    let first_edge = input.add_edge(Edge {
        source: endpoint,
        target: first,
    });
    let second_edge = input.add_edge(Edge {
        source: endpoint,
        target: second,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(
        first,
        Point {
            x: 594.0,
            y: 1782.0,
        },
    );
    arena.set_position(
        endpoint,
        Point {
            x: 990.0,
            y: 1584.0,
        },
    );
    arena.set_position(
        obstruction,
        Point {
            x: 495.0,
            y: 1386.0,
        },
    );
    arena.clusters.push(ClusterState {
        members: vec![first, second, third, fourth],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 29.0 / 3.0,
        vessel_tala_id: 10_000,
        fixed_size: true,
    });
    arena.node_order = vec![first, endpoint, obstruction];
    arena.set_turn_cost(3_904.0);

    let first_projection = arena
        .active_aggregate_endpoint_projection(first)
        .expect("active first-member projection");
    assert_eq!(first_projection.owner, first);
    assert_eq!(
        first_projection.tala_id,
        arena.nodes[first.0 as usize].tala_id
    );
    assert_eq!(first_projection.offset, Point::default());
    assert_eq!(
        first_projection.size,
        Size {
            width: 145.0,
            height: 92.0,
        }
    );

    let (represented, external) = arena.active_edge_original_endpoints(first, second_edge);
    assert_eq!((represented, external), (second, endpoint));
    let second_projection = arena
        .active_aggregate_endpoint_projection(represented)
        .expect("active second-member projection");
    assert_eq!(
        second_projection.tala_id,
        arena.nodes[second.0 as usize].tala_id
    );
    assert_eq!(
        second_projection.offset,
        Point {
            x: 145.0 + 29.0 / 3.0,
            y: 0.0,
        }
    );

    let first_box = arena
        .sized_projected_box(first_projection)
        .expect("positioned retained member");
    let endpoint_position = arena.position(endpoint).expect("positioned endpoint");
    let endpoint_size = arena.nodes[endpoint.0 as usize].rect.size;
    assert_eq!(
        arena.edge_route_penalty(
            endpoint,
            first,
            first_edge,
            None,
            Some((
                (
                    endpoint_position,
                    endpoint_size,
                    arena.nodes[endpoint.0 as usize].tala_id,
                ),
                (first_box.0, first_box.1, first_projection.tala_id),
            )),
            true,
        ),
        7_808.0,
        "the retained member route crosses the active container"
    );

    assert_eq!(
        arena.edge_route_penalty(
            endpoint,
            first,
            first_edge,
            None,
            Some((
                (
                    endpoint_position,
                    endpoint_size,
                    arena.nodes[endpoint.0 as usize].tala_id,
                ),
                (
                    arena.position(first).expect("positioned vessel"),
                    arena.active_node_size(first),
                    arena.clusters[0].vessel_tala_id,
                ),
            )),
            true,
        ),
        0.0,
        "routing from the whole vessel would incorrectly miss it"
    );
}

#[test]
fn active_cluster_member_box_prefers_live_pointer_to_vessel_projection() {
    let mut input = Graph::default();
    let first = input.add_node(node("first member", 145.0, 92.0));
    let second = input.add_node(node("second member", 145.0, 92.0));

    let mut arena = ArenaGraph::from_input(&input);
    let live_member_position = Point {
        x: 4_369.0,
        y: 3_732.0,
    };
    let vessel_position = Point {
        x: 4_234.0,
        y: 3_732.0,
    };
    arena.set_position(first, live_member_position);
    arena.set_position(second, vessel_position);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 10.0,
        vessel_tala_id: 90_002,
        fixed_size: true,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    arena.node_order = vec![first];
    arena
        .pending_cluster_vessel_positions
        .insert(0, vessel_position);
    arena.refresh_node_order_membership();

    assert_eq!(arena.active_node_position(first), Some(vessel_position));
    assert_eq!(arena.position(first), Some(live_member_position));
    let projection = arena
        .active_aggregate_endpoint_projection(first)
        .expect("active cluster member projection");
    assert!(projection.cluster_member);
    assert_eq!(
        arena.sized_projected_box(projection),
        Some((
            live_member_position,
            arena.nodes[first.0 as usize].rect.size
        )),
        "edge abduction retains the live member pointer while the active vessel is elsewhere"
    );
}

#[test]
fn retained_cluster_member_uses_aggregate_pointer_after_cluster_state_is_gone() {
    const AGGREGATE_TALA_ID: i64 = 90_001;

    let mut input = Graph::default();
    let owner = input.add_node(node("owner", 1_000.0, 800.0));
    let mut member_node = node("member", 314.0, 131.0);
    member_node.parent = Some(owner);
    let member = input.add_node(member_node);
    let mut arena = ArenaGraph::from_input(&input);
    let owner_position = Point {
        x: -3_743.0,
        y: -2_167.0,
    };
    let stale_member_position = Point {
        x: -3_563.0,
        y: -1_685.0,
    };
    let retained_pointer_position = Point {
        x: -3_623.0,
        y: -1_745.0,
    };
    arena.set_position(owner, owner_position);
    arena.set_position(member, stale_member_position);

    let mut retained_pointer = arena.nodes[member.0 as usize].clone();
    retained_pointer.position = Some(retained_pointer_position);
    retained_pointer.rect.origin = retained_pointer_position;
    arena
        .transaction_external_aggregate_children
        .insert(AGGREGATE_TALA_ID, vec![retained_pointer]);

    let projection = ProjectedAdjacent {
        owner,
        tala_id: arena.nodes[member.0 as usize].tala_id,
        container_tala_id: Some(arena.nodes[owner.0 as usize].tala_id),
        offset: Point { x: 180.0, y: 482.0 },
        size: Size {
            width: 314.0,
            height: 131.0,
        },
        cluster_member: true,
    };

    assert_eq!(
        arena.sized_projected_box(projection),
        Some((retained_pointer_position, projection.size)),
        "the induced aggregate retains the live member pointer after its ClusterState is removed"
    );
}

#[test]
fn alignment_sequence_abduction_requires_the_full_current_endpoint_pair() {
    let mut input = Graph::default();
    let first_step = input.add_node(node("first step", 100.0, 60.0));
    let second_step = input.add_node(node("second step", 120.0, 60.0));
    let cluster_first = input.add_node(node("cluster first", 80.0, 70.0));
    let cluster_second = input.add_node(node("cluster second", 80.0, 70.0));
    let ordinary = input.add_node(node("ordinary", 90.0, 50.0));

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(first_step, Point { x: 10.0, y: 20.0 });
    arena.set_position(cluster_first, Point { x: 400.0, y: 20.0 });
    arena.sequences.push(SequenceState {
        members: vec![first_step, second_step],
        vessel_tala_id: 10_001,
        container: None,
        has_edge_abductions: true,
    });
    arena.clusters.push(ClusterState {
        members: vec![cluster_first, cluster_second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 10.0,
        vessel_tala_id: 10_002,
        fixed_size: true,
    });
    arena.node_order = vec![first_step, cluster_first, ordinary];

    let retained_step = arena
        .active_aggregate_edge_endpoint_projection(second_step, ordinary, true)
        .expect("sequence abduction still matches an ordinary endpoint");
    assert_eq!(
        retained_step.tala_id,
        arena.nodes[second_step.0 as usize].tala_id
    );
    assert_eq!(
        retained_step.offset,
        Point {
            x: 100.0 - 35.0,
            y: 0.0,
        }
    );
    assert_eq!(
        retained_step.size,
        Size {
            width: 120.0,
            height: 60.0,
        }
    );

    let current_sequence = arena
        .active_aggregate_edge_endpoint_projection(second_step, cluster_first, true)
        .expect("cluster reconnection invalidates the sequence abduction pair");
    assert_eq!(current_sequence.tala_id, 10_001);
    assert_eq!(current_sequence.offset, Point::default());
    assert_eq!(
        current_sequence.size,
        Size {
            width: 185.0,
            height: 60.0,
        }
    );

    let current_cluster = arena
        .active_aggregate_edge_endpoint_projection(cluster_first, second_step, true)
        .expect("alignment receives no cluster abductions");
    assert_eq!(current_cluster.tala_id, 10_002);
    assert_eq!(current_cluster.offset, Point::default());
    assert_eq!(current_cluster.size, arena.cluster_vessel_size(0));
    assert!(!current_cluster.cluster_member);
}

#[test]
fn alignment_delta_restores_sequence_endpoint_after_abduction() {
    let mut input = Graph::default();
    let cluster_owner = input.add_node(node("cluster owner", 100.0, 100.0));
    let cluster_peer = input.add_node(node("cluster peer", 100.0, 100.0));
    let sequence_owner = input.add_node(node("sequence owner", 80.0, 60.0));
    let sequence_step = input.add_node(node("sequence step", 120.0, 60.0));
    let edge = input.add_edge(Edge {
        source: cluster_peer,
        target: sequence_step,
    });

    let mut graph = ArenaGraph::from_input(&input);
    graph.set_position(cluster_owner, Point { x: 0.0, y: 0.0 });
    graph.set_position(cluster_peer, Point { x: 120.0, y: 0.0 });
    graph.set_position(sequence_owner, Point { x: 300.0, y: 200.0 });
    graph.set_position(sequence_step, Point { x: 345.0, y: 200.0 });
    graph.nodes[cluster_owner.0 as usize].cluster = Some(0);
    graph.nodes[cluster_peer.0 as usize].cluster = Some(0);
    graph.nodes[sequence_owner.0 as usize].sequence = Some(0);
    graph.nodes[sequence_step.0 as usize].sequence = Some(0);
    graph.clusters.push(ClusterState {
        members: vec![cluster_owner, cluster_peer],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_001,
        fixed_size: true,
    });
    graph.sequences.push(SequenceState {
        members: vec![sequence_owner, sequence_step],
        vessel_tala_id: 10_002,
        container: None,
        has_edge_abductions: true,
    });
    graph.node_order = vec![cluster_owner, sequence_owner];

    assert_eq!(graph.alignment_deltas(edge), Point { x: 295.0, y: 180.0 });
}

#[test]
fn alignment_uses_cluster_vessel_box_after_cluster_abduction() {
    let mut input = Graph::default();
    let external = input.add_node(node("external", 100.0, 100.0));
    let first_member = input.add_node(node("cluster first", 100.0, 100.0));
    let second_member = input.add_node(node("cluster second", 100.0, 100.0));
    let edge = input.add_edge(Edge {
        source: external,
        target: second_member,
    });

    let mut graph = ArenaGraph::from_input(&input);
    graph.set_position(external, Point { x: 160.0, y: 150.0 });
    graph.set_position(first_member, Point { x: 100.0, y: 100.0 });
    graph.set_position(second_member, Point { x: 220.0, y: 100.0 });
    for member in [first_member, second_member] {
        graph.nodes[member.0 as usize].cluster = Some(0);
    }
    graph.clusters.push(ClusterState {
        members: vec![first_member, second_member],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: 10_003,
        fixed_size: true,
    });
    graph.cluster_vessel_sizes = vec![Size {
        width: 220.0,
        height: 100.0,
    }];
    graph
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 100.0, y: 100.0 });
    graph.node_order = vec![external, first_member];
    graph.refresh_node_order_membership();

    // The temporary vessel center is x=210, which is aligned with the
    // external node center. The retained first member center is x=150; using
    // that child box would incorrectly report a diagonal edge.
    assert!(graph.edge_axis_aligned(edge));
}

#[test]
fn nested_aggregate_translation_moves_each_retained_box_once() {
    let mut input = Graph::default();
    let sequence_owner = input.add_node(node("sequence owner", 100.0, 60.0));
    let mut sequence_child_node = node("sequence child", 80.0, 50.0);
    sequence_child_node.parent = Some(sequence_owner);
    let sequence_child = input.add_node(sequence_child_node);
    let cluster_peer = input.add_node(node("cluster peer", 90.0, 70.0));

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(sequence_owner, Point { x: 10.0, y: 20.0 });
    arena.set_position(sequence_child, Point { x: 30.0, y: 40.0 });
    arena.set_position(cluster_peer, Point { x: 200.0, y: 60.0 });
    arena.sequences.push(SequenceState {
        members: vec![sequence_owner, sequence_child],
        vessel_tala_id: 10_001,
        container: None,
        has_edge_abductions: true,
    });
    arena.clusters.push(ClusterState {
        members: vec![sequence_owner, cluster_peer],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 10.0,
        vessel_tala_id: 10_002,
        fixed_size: true,
    });
    arena.node_order = vec![sequence_owner];

    arena.translate_active_node_boxes(&[sequence_owner, cluster_peer], Point { x: -7.0, y: 11.0 });

    assert_eq!(
        arena.position(sequence_owner),
        Some(Point { x: 3.0, y: 31.0 })
    );
    assert_eq!(
        arena.position(sequence_child),
        Some(Point { x: 23.0, y: 51.0 }),
        "the child belongs to both the owner descendant set and the retained sequence"
    );
    assert_eq!(
        arena.position(cluster_peer),
        Some(Point { x: 193.0, y: 71.0 })
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn projected_obstruction_inventory_preserves_recovered_endpoint_chain_order() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 171.0, 69.0));
    let target = input.add_node(node("target", 171.0, 66.0));
    let edge = input.add_edge(Edge { source, target });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(source, Point { x: 0.0, y: 64.0 });
    arena.set_position(target, Point { x: 352.0, y: 0.0 });
    arena.set_turn_cost(100.0);
    let alternate_obstruction = ProjectedAdjacent {
        owner: source,
        tala_id: 10,
        container_tala_id: None,
        offset: Point { x: 224.0, y: 26.0 },
        size: Size {
            width: 54.0,
            height: 20.0,
        },
        cluster_member: false,
    };
    let direct_obstruction = ProjectedAdjacent {
        owner: source,
        tala_id: 11,
        container_tala_id: None,
        offset: Point { x: 224.0, y: -64.0 },
        size: Size {
            width: 54.0,
            height: 66.0,
        },
        cluster_member: false,
    };

    arena.sized_projected_obstructions.insert(
        (source, edge),
        vec![alternate_obstruction, direct_obstruction].into(),
    );
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, None, None, false),
        100.0
    );

    arena.sized_projected_obstructions.insert(
        (target, edge),
        vec![direct_obstruction, alternate_obstruction].into(),
    );
    assert_eq!(
        arena.edge_route_penalty(target, source, edge, None, None, false),
        200.0
    );
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, None, None, false),
        100.0,
        "the opposite receiver retains its own endpoint-chain order"
    );
    arena.sized_projected_obstructions.insert(
        (source, edge),
        vec![direct_obstruction, alternate_obstruction].into(),
    );
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, None, None, false),
        200.0
    );
}

#[test]
fn ordered_abduction_and_physical_edges_preserve_parent_route_obstructions() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 171.0, 69.0));
    let target = input.add_node(node("target", 171.0, 66.0));
    let first = input.add_edge(Edge { source, target });
    let second = input.add_edge(Edge { source, target });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(source, Point { x: 0.0, y: 64.0 });
    arena.set_position(target, Point { x: 352.0, y: 0.0 });
    arena.set_turn_cost(100.0);
    let alternate_obstruction = ProjectedAdjacent {
        owner: source,
        tala_id: 10,
        container_tala_id: None,
        offset: Point { x: 224.0, y: 26.0 },
        size: Size {
            width: 54.0,
            height: 20.0,
        },
        cluster_member: false,
    };
    let direct_obstruction = ProjectedAdjacent {
        owner: source,
        tala_id: 11,
        container_tala_id: None,
        offset: Point { x: 224.0, y: -64.0 },
        size: Size {
            width: 54.0,
            height: 66.0,
        },
        cluster_member: false,
    };
    arena.sized_edge_abductions.push(SizedEdgeAbduction {
        source_edge: first,
        edge_tala_id: arena.edge_records[first.0 as usize].tala_id,
        edge: Some(first),
        current_from: Some(AbductionCurrentNodeRef::local(source)),
        current_to: Some(AbductionCurrentNodeRef::local(target)),
        originally_from: None,
        originally_to: None,
        originally_from_table_neighbors: Vec::new(),
        originally_to_table_neighbors: Vec::new(),
        originally_from_container: None,
        originally_to_container: None,
        sequence_abduction: false,
        obstructions_from_to: vec![alternate_obstruction, direct_obstruction],
        obstructions_to_from: vec![direct_obstruction, alternate_obstruction],
        obstruction_sets_from_to: OrderedObstructionInventory::default(),
        obstruction_sets_to_from: OrderedObstructionInventory::default(),
    });

    let one_turn = arena.sized_edge_length(source, false);
    arena.sized_edge_abductions[0].obstructions_from_to =
        vec![direct_obstruction, alternate_obstruction];
    let two_turns = arena.sized_edge_length(source, false);
    assert_eq!(two_turns - one_turn, 100.0);

    // The sole ordered abduction record is consumed by `first`. `second`
    // remains on its current carrier endpoints, but TALA's temporary
    // SplitSubgraphs graph still reads the parent Graph.Containers slices.
    // Its per-edge projection therefore remains the authoritative obstruction
    // inventory rather than falling back to temporary-graph node order.
    arena.sized_edge_abductions[0].obstructions_from_to.clear();
    arena.sized_projected_obstructions.insert(
        (source, second),
        vec![direct_obstruction, alternate_obstruction].into(),
    );
    let with_parent_projection = arena.sized_edge_length(source, false);
    arena.sized_projected_obstructions.remove(&(source, second));
    let without_parent_projection = arena.sized_edge_length(source, false);
    assert!((with_parent_projection - without_parent_projection - 200.0).abs() < 1e-9);
    assert!(arena.edge_order.contains(&first));
}

#[test]
fn segment_intersection_matches_tala_orientation_geometry() {
    assert!(!ArenaGraph::segment_intersects_box(
        Point {
            x: 858.5,
            y: 1241.0,
        },
        Point {
            x: 1352.5,
            y: 1584.0,
        },
        Point {
            x: 1089.0,
            y: 1089.0,
        },
        Size {
            width: 404.0,
            height: 311.0,
        },
    ));
}

#[test]
fn sorted_obstruction_scan_includes_a_box_touching_the_route_boundary() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 171.0, 69.0));
    let target = input.add_node(node("target", 54.0, 66.0));
    let obstruction = input.add_node(node("obstruction", 171.0, 66.0));
    let edge = input.add_edge(Edge { source, target });
    input.add_edge(Edge {
        source,
        target: obstruction,
    });

    let mut arena = ArenaGraph::from_input(&input);
    if arena.common_uncle_siblings.is_nil() {
        arena
            .common_uncle_siblings
            .replace_header(SharedSliceMap::new());
    }
    arena
        .common_uncle_siblings
        .insert(source, vec![source, target]);
    arena.set_position(source, Point { x: 64.0, y: -128.0 });
    arena.set_position(target, Point { x: 32.0, y: 128.0 });
    arena.set_position(obstruction, Point { x: -96.0, y: 32.0 });
    arena.set_turn_cost(100.0);

    let sorted = [obstruction, target, source];
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, None, None, false),
        100.0
    );
    assert_eq!(
        arena.edge_route_penalty(source, target, edge, Some((&sorted, 171.0)), None, false,),
        100.0
    );
}

#[test]
fn transaction_rejects_a_spacing_overlap_that_becomes_exact() {
    let mut input = Graph::default();
    let left = input.add_node(node("left", 100.0, 60.0));
    let right = input.add_node(node("right", 100.0, 60.0));
    input.add_edge(Edge {
        source: left,
        target: right,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(left, Point { x: 0.0, y: 0.0 });
    arena.set_position(right, Point { x: 110.0, y: 0.0 });
    let existing_overlaps = arena.existing_overlap_pairs();
    let existing_exact_overlaps = arena.exact_overlap_pairs();
    let moved = BTreeSet::from([right]);

    assert!(existing_overlaps.contains(&(left, right)));
    assert!(!existing_exact_overlaps.contains(&(left, right)));
    assert!(
        !arena.existing_spacing_overlap_became_exact(&existing_overlaps, &existing_exact_overlaps,)
    );
    assert!(
        !arena.existing_spacing_overlap_became_exact_for_moved_nodes(
            &existing_overlaps,
            &existing_exact_overlaps,
            &moved,
        )
    );

    arena.set_position(right, Point { x: 90.0, y: 0.0 });
    assert!(
        arena.existing_spacing_overlap_became_exact(&existing_overlaps, &existing_exact_overlaps,)
    );
    assert!(arena.existing_spacing_overlap_became_exact_for_moved_nodes(
        &existing_overlaps,
        &existing_exact_overlaps,
        &moved,
    ));
}

#[test]
fn transaction_spacing_uses_facing_recovered_label_margins() {
    let mut input = Graph::default();
    let mut upper_node = node("upper", 170.0, 217.0);
    upper_node.layout_margins.bottom = 113.0;
    let upper = input.add_node(upper_node);
    let lower = input.add_node(node("lower", 195.0, 66.0));

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(upper, Point { x: 0.0, y: 0.0 });
    arena.set_position(lower, Point { x: 0.0, y: 330.0 });
    assert_eq!(
        arena.spacing_delta(upper, lower, Point { x: 0.0, y: 0.0 }),
        113.0
    );
    let existing_overlaps = arena.existing_overlap_pairs();
    assert!(!existing_overlaps.contains(&(upper, lower)));

    // Moving the lower node 23 units upward leaves a 90-unit ordinary
    // box gap, but violates the upper node's 113-unit facing margin.
    arena.set_position(lower, Point { x: 0.0, y: 307.0 });
    assert!(arena.transaction_has_new_overlap(&existing_overlaps));
    assert!(
        arena.transaction_has_new_overlap_for_nodes(&existing_overlaps, &BTreeSet::from([lower]),)
    );
}

#[test]
fn moved_pair_transaction_checks_match_complete_graph_scan() {
    let mut input = Graph::default();
    let left = input.add_node(node("left", 100.0, 60.0));
    let near = input.add_node(node("near", 100.0, 60.0));
    let middle = input.add_node(node("middle", 100.0, 60.0));
    let far = input.add_node(node("far", 100.0, 60.0));
    input.add_edge(Edge {
        source: left,
        target: near,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(left, Point { x: 0.0, y: 0.0 });
    arena.set_position(near, Point { x: 110.0, y: 0.0 });
    arena.set_position(middle, Point { x: 400.0, y: 0.0 });
    arena.set_position(far, Point { x: 700.0, y: 0.0 });
    // Deliberately make graph order disagree with stable NodeId order. The
    // restricted scan must preserve the complete scan's operand direction.
    arena.node_order = vec![far, middle, near, left];
    let existing_overlaps = arena.existing_overlap_pairs();
    let existing_exact_overlaps = arena.exact_overlap_pairs();
    let moved = BTreeSet::from([near, far]);

    for delta_x in [0.0, -20.0, 250.0, -250.0] {
        let mut trial = arena.clone();
        for node in moved.iter().copied() {
            let current = trial.position(node).unwrap();
            trial.set_position(
                node,
                Point {
                    x: current.x + delta_x,
                    y: current.y,
                },
            );
        }
        assert_eq!(
            trial.transaction_has_new_overlap(&existing_overlaps),
            trial.transaction_has_new_overlap_for_nodes(&existing_overlaps, &moved),
            "new-overlap predicate diverged for delta {delta_x}",
        );
        assert_eq!(
            trial.existing_spacing_overlap_became_exact(
                &existing_overlaps,
                &existing_exact_overlaps,
            ),
            trial.existing_spacing_overlap_became_exact_for_moved_nodes(
                &existing_overlaps,
                &existing_exact_overlaps,
                &moved,
            ),
            "spacing-to-exact predicate diverged for delta {delta_x}",
        );
    }
}

#[test]
fn sized_transpose_uses_local_edge_score_and_cell_rounded_rotation() {
    let mut input = Graph::default();
    let users = input.add_node(node("users", 54.0, 66.0));
    let api = input.add_node(node("api", 171.0, 69.0));
    let jobs = input.add_node(node("jobs", 171.0, 66.0));
    let note = input.add_node(node("note", 114.0, 21.0));
    let database = input.add_node(node("database", 125.0, 118.0));
    let table = input.add_node(node("table", 123.0, 36.0));
    for (source, target) in [
        (users, api),
        (api, jobs),
        (api, note),
        (jobs, database),
        (database, table),
    ] {
        input.add_edge(Edge { source, target });
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[api.0 as usize].nears.push(database);
    arena.nodes[database.0 as usize].nears.push(api);
    // AddNears precedes PreprocessTrees in TALA. This test mutates the
    // materialized near set directly, so rebuild NodeToTree at the same
    // source boundary before exercising transpose.
    arena.preprocess_trees();
    for (node, position) in [
        (
            users,
            Point {
                x: -288.0,
                y: 160.0,
            },
        ),
        (
            api,
            Point {
                x: -320.0,
                y: -288.0,
            },
        ),
        (
            jobs,
            Point {
                x: -160.0,
                y: -128.0,
            },
        ),
        (
            note,
            Point {
                x: -64.0,
                y: -224.0,
            },
        ),
        (database, Point { x: 96.0, y: -224.0 }),
        (table, Point { x: 96.0, y: -32.0 }),
    ] {
        arena.set_position(node, position);
    }
    arena.initialize_turn_cost();

    assert!(arena.transpose_node(database, true));
    assert_eq!(
        arena.position(database),
        Some(Point {
            x: -352.0,
            y: -96.0
        })
    );
    assert_eq!(
        arena.position(table),
        Some(Point {
            x: -384.0,
            y: -192.0
        })
    );
}

#[test]
fn join_distanced_cluster_aims_at_fixed_node_bounds_center() {
    let mut input = Graph::default();
    let mut source_node = node("source", 534.0, 480.0);
    source_node.locked_position = Some(Point { x: 240.0, y: 180.0 });
    let source = input.add_node(source_node);
    let moving = input.add_node(node("moving", 142.0, 84.0));
    let companion = input.add_node(node("companion", 142.0, 84.0));
    input.add_edge(Edge {
        source,
        target: moving,
    });
    input.add_edge(Edge {
        source,
        target: companion,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 126.0;
    arena.set_position(source, Point { x: 240.0, y: 180.0 });
    arena.set_position(
        moving,
        Point {
            x: 1260.0,
            y: 378.0,
        },
    );
    arena.set_position(companion, Point { x: 378.0, y: 756.0 });

    // Go's JoinDistancedClusters uses Nodes.CenterWithWorkGuard, which centers
    // the node's actual 240,180..774,660 bounds. GetFixedOrigin's 0,0 anchor
    // is not part of that center calculation.
    assert_eq!(
        arena.node_set_center(&[source]),
        Point { x: 507.0, y: 420.0 }
    );

    arena.join_distanced_clusters();

    // The corrected center makes the source-to-moving direction horizontal;
    // the next cell would overlap the fixed source's right edge.
    assert_eq!(arena.position(moving), Some(Point { x: 882.0, y: 378.0 }));
}

#[test]
fn join_distanced_clusters_matches_recovered_mixed_first_join() {
    let mut input = Graph::default();
    let users = input.add_node(node("users", 54.0, 66.0));
    let api = input.add_node(node("api", 171.0, 69.0));
    let jobs = input.add_node(node("jobs", 171.0, 66.0));
    let note = input.add_node(node("note", 114.0, 21.0));
    let database = input.add_node(node("database", 125.0, 118.0));
    let mut table_node = node("table", 123.0, 36.0);
    table_node.shape = ShapeKind::SqlTable;
    let table = input.add_node(table_node);
    for (source, target) in [
        (users, api),
        (api, jobs),
        (jobs, database),
        (database, table),
        (note, api),
    ] {
        input.add_edge(Edge { source, target });
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 32.0;
    for (node, tala_id, position) in [
        (
            users,
            3_895_026_828,
            Point {
                x: 160.0,
                y: -192.0,
            },
        ),
        (api, 3_507_694_700, Point { x: 96.0, y: -32.0 }),
        (jobs, 2_438_492_876, Point { x: 384.0, y: -32.0 }),
        (note, 2_683_332_832, Point { x: 128.0, y: 160.0 }),
        (database, 271_300_150, Point { x: 544.0, y: 128.0 }),
        (table, 1_861_364_904, Point { x: 544.0, y: 416.0 }),
    ] {
        arena.nodes[node.0 as usize].tala_id = tala_id;
        arena.set_position(node, position);
    }

    assert_eq!(
        arena.distanced_clusters(96.0),
        Some(vec![
            vec![api, users],
            vec![database, jobs],
            vec![note],
            vec![table],
        ])
    );
    arena.join_distanced_clusters();
    assert_eq!(
        arena.position(users),
        Some(Point {
            x: 192.0,
            y: -160.0
        })
    );
    assert_eq!(arena.position(api), Some(Point { x: 128.0, y: 0.0 }));
    assert_eq!(arena.position(jobs), Some(Point { x: 384.0, y: -32.0 }));
    assert_eq!(arena.position(note), Some(Point { x: 128.0, y: 160.0 }));
    assert_eq!(arena.position(database), Some(Point { x: 544.0, y: 128.0 }));
    assert_eq!(arena.position(table), Some(Point { x: 448.0, y: 320.0 }));
}

#[test]
fn join_distanced_clusters_preserves_aligned_axis_like_go_orientation() {
    let mut input = Graph::default();
    let check = input.add_node(node("check", 340.0, 320.0));
    let search = input.add_node(node("search", 159.0, 66.0));
    let off = input.add_node(node("off", 68.0, 66.0));
    let start = input.add_node(node("start", 10.0, 10.0));
    let ready = input.add_node(node("ready", 89.0, 66.0));
    for (source, target) in [
        (check, search),
        (search, ready),
        (check, off),
        (search, off),
        (ready, off),
        (start, check),
    ] {
        input.add_edge(Edge { source, target });
    }

    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 15.0;
    for (node, tala_id, position) in [
        (
            check,
            441_746_996,
            Point {
                x: 240.0,
                y: -1140.0,
            },
        ),
        (
            search,
            3_960_817_907,
            Point {
                x: 570.0,
                y: -690.0,
            },
        ),
        (
            off,
            231_126_186,
            Point {
                x: 1020.0,
                y: -825.0,
            },
        ),
        (
            start,
            1_697_318_111,
            Point {
                x: 285.0,
                y: -750.0,
            },
        ),
        (
            ready,
            197_800_596,
            Point {
                x: 1020.0,
                y: -960.0,
            },
        ),
    ] {
        arena.nodes[node.0 as usize].tala_id = tala_id;
        arena.set_position(node, position);
    }

    arena.join_distanced_clusters();
    assert_eq!(
        arena.position(check),
        Some(Point {
            x: 240.0,
            y: -1140.0,
        })
    );
    assert_eq!(
        arena.position(search),
        Some(Point {
            x: 570.0,
            y: -750.0,
        })
    );
    assert_eq!(
        arena.position(off),
        Some(Point {
            x: 795.0,
            y: -825.0,
        })
    );
    assert_eq!(
        arena.position(start),
        Some(Point {
            x: 285.0,
            y: -750.0,
        })
    );
    assert_eq!(
        arena.position(ready),
        Some(Point {
            x: 930.0,
            y: -870.0,
        })
    );
}

#[test]
fn join_cluster_center_uses_recovered_node_render_bounds() {
    let mut input = Graph::default();
    let mut decorated = node("decorated", 100.0, 80.0);
    decorated.external_label = Some(ExternalLabel {
        size: Size {
            width: 40.0,
            height: 20.0,
        },
        side: ExternalSide::Bottom,
        alignment: ExternalAlignment::Center,
        automatic: false,
        reserve_space: true,
    });
    let decorated = input.add_node(decorated);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(decorated, Point { x: 100.0, y: 100.0 });

    // Recovered Nodes.getCenter consumes Node.getBoundingBox, so the
    // outside label extends the bottom from 180 to 210 before centering.
    assert_eq!(
        arena.node_set_center(&[decorated]),
        Point { x: 150.0, y: 155.0 }
    );
}

#[test]
fn join_cluster_center_uses_neutral_temporary_vessel_bounds() {
    let mut input = Graph::default();
    let owner = input.add_node(node("owner", 100.0, 80.0));
    let member = input.add_node(node("member", 100.0, 80.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(owner, Point { x: 100.0, y: 100.0 });
    arena.set_position(member, Point { x: 230.0, y: 100.0 });
    arena.nodes[owner.0 as usize].loop_offsets = Some(LoopOffsetsMap::from([(4, 31.0)]));
    arena.nodes[owner.0 as usize].cluster = Some(0);
    arena.nodes[member.0 as usize].cluster = Some(0);
    arena.clusters.push(ClusterState {
        members: vec![owner, member],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 30.0,
        vessel_tala_id: 10_001,
        fixed_size: false,
    });
    arena.node_order.retain(|node| *node != member);
    arena
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 100.0, y: 100.0 });

    let vessel_size = arena.active_node_size(owner);
    assert_eq!(
        arena.node_set_center(&[owner]),
        Point {
            x: 100.0 + vessel_size.width * 0.5,
            y: 100.0 + vessel_size.height * 0.5,
        }
    );
}

#[test]
fn join_cluster_center_uses_neutral_materialized_vessel_bounds() {
    let mut input = Graph::default();
    let vessel = input.add_node(node("vessel", 128.0, 446.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(vessel, Point { x: 100.0, y: 198.0 });
    arena.nodes[vessel.0 as usize].loop_offsets = Some(LoopOffsetsMap::from([(4, 31.0)]));
    arena.nodes[vessel.0 as usize].scoring_is_aggregate_vessel = true;

    assert_eq!(
        arena.node_set_center(&[vessel]),
        Point { x: 164.0, y: 421.0 }
    );
}

#[test]
fn join_distanced_cluster_retries_axis_allowed_by_fixed_origin() {
    let mut input = Graph::default();
    let mut fixed_node = node("fixed", 10.0, 10.0);
    fixed_node.locked_position = Some(Point { x: 0.0, y: 0.0 });
    let fixed = input.add_node(fixed_node);
    let moving = input.add_node(node("moving", 10.0, 10.0));
    let companion = input.add_node(node("companion", 10.0, 10.0));
    input.add_edge(Edge {
        source: fixed,
        target: moving,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 10.0;
    arena.set_position(fixed, Point { x: 0.0, y: 0.0 });
    arena.set_position(moving, Point { x: 0.0, y: 100.0 });
    arena.set_position(companion, Point { x: 20.0, y: 100.0 });

    arena.join_distanced_clusters();

    // The diagonal step violates only the fixed X origin. TALA rolls it
    // back and retries Y, preserving X while moving toward the target.
    let moved = arena.position(moving).unwrap();
    let companion_moved = arena.position(companion).unwrap();
    assert_eq!(moved.x, 0.0);
    assert_eq!(companion_moved.x, 20.0);
    assert!(moved.y < 100.0);
    assert!(companion_moved.y < 100.0);
}

fn recovered_mixed_hierarchy_child_scope() -> (Pipeline, [NodeId; 6]) {
    let mut input = Graph::default();
    let users = input.add_node(node("users", 54.0, 66.0));
    let api = input.add_node(node("api", 171.0, 69.0));
    let jobs = input.add_node(node("jobs", 171.0, 66.0));
    let mut note_node = node("note", 114.0, 21.0);
    note_node.shape = ShapeKind::Text;
    let note = input.add_node(note_node);
    let mut database_node = node("database", 125.0, 118.0);
    database_node.shape = ShapeKind::Cylinder;
    let database = input.add_node(database_node);
    let mut table_node = node("table", 123.0, 36.0);
    table_node.shape = ShapeKind::SqlTable;
    let table = input.add_node(table_node);
    for (source, target) in [
        (users, api),
        (api, jobs),
        (jobs, database),
        (database, table),
        (note, api),
    ] {
        let edge = input.add_edge(Edge { source, target });
        if source == note && target == api {
            input.set_edge_label(
                edge,
                Some(EdgeLabel {
                    text: "documents".to_owned(),
                    size: Size {
                        width: 74.0,
                        height: 21.0,
                    },
                    position: LabelPosition::UnlockedTop,
                    percentage: 0.0,
                }),
            );
        }
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
    }

    let mut scope = Pipeline::new(&input, 1, false, true);
    for (node, tala_id) in [
        (users, 3_895_026_828),
        (api, 3_507_694_700),
        (jobs, 2_438_492_876),
        (note, 2_683_332_832),
        (database, 271_300_150),
        (table, 1_861_364_904),
    ] {
        scope.graph.nodes[node.0 as usize].tala_id = tala_id;
    }
    scope.graph.nodes[api.0 as usize].nears.push(database);
    scope.graph.nodes[database.0 as usize].nears.push(api);
    for sibling in [api, database] {
        if scope.graph.common_uncle_siblings.is_nil() {
            scope
                .graph
                .common_uncle_siblings
                .replace_header(SharedSliceMap::new());
        }
        scope
            .graph
            .common_uncle_siblings
            .insert(sibling, vec![api, database]);
    }
    // This helper starts at the post-PreprocessHubs optimizer boundary.
    scope.graph.compute_hubs();
    (scope, [users, api, jobs, note, database, table])
}

fn recovered_mixed_hierarchy_root_scope() -> (Pipeline, [NodeId; 2]) {
    let mut input = Graph::default();
    let platform = input.add_node(node("platform", 458.0, 510.0));
    let mut external_node = node("external", 193.0, 84.0);
    external_node.shape = ShapeKind::Cloud;
    let external = input.add_node(external_node);
    let mut projected_edges = Vec::new();
    for (source, target, text, width) in [
        (external, platform, "webhook", 60.0),
        (platform, external, "export", 43.0),
    ] {
        let edge = input.add_edge(Edge { source, target });
        projected_edges.push(edge);
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: text.to_owned(),
                size: Size {
                    width,
                    height: 21.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
    }

    let mut scope = Pipeline::new(&input, 1, false, true);
    scope.graph.nodes[platform.0 as usize].tala_id = 709_505_714;
    scope.graph.nodes[external.0 as usize].tala_id = 857_568_394;
    for (edge, tala_id, offset, size) in [
        (
            projected_edges[0],
            3_507_694_700,
            Point { x: 220.0, y: 188.0 },
            Size {
                width: 171.0,
                height: 69.0,
            },
        ),
        (
            projected_edges[1],
            271_300_150,
            Point { x: 60.0, y: 188.0 },
            Size {
                width: 125.0,
                height: 118.0,
            },
        ),
    ] {
        scope.graph.sized_adjacent_overrides.insert(
            (external, edge),
            ProjectedAdjacent {
                owner: platform,
                tala_id,
                container_tala_id: None,
                offset,
                size,
                cluster_member: false,
            },
        );
    }
    for (tala_id, offset, size) in [
        (
            3_895_026_828,
            Point { x: 316.0, y: 348.0 },
            Size {
                width: 54.0,
                height: 66.0,
            },
        ),
        (
            3_507_694_700,
            Point { x: 220.0, y: 188.0 },
            Size {
                width: 171.0,
                height: 69.0,
            },
        ),
        (
            2_438_492_876,
            Point { x: 60.0, y: 60.0 },
            Size {
                width: 171.0,
                height: 66.0,
            },
        ),
        (
            2_683_332_832,
            Point { x: 284.0, y: 92.0 },
            Size {
                width: 114.0,
                height: 21.0,
            },
        ),
        (
            271_300_150,
            Point { x: 60.0, y: 188.0 },
            Size {
                width: 125.0,
                height: 118.0,
            },
        ),
        (
            1_861_364_904,
            Point { x: 60.0, y: 380.0 },
            Size {
                width: 123.0,
                height: 36.0,
            },
        ),
    ] {
        for edge in projected_edges.iter().copied() {
            for receiver in [platform, external] {
                scope
                    .graph
                    .sized_projected_obstructions
                    .entry((receiver, edge))
                    .or_default()
                    .push(ProjectedAdjacent {
                        owner: platform,
                        tala_id,
                        container_tala_id: None,
                        offset,
                        size,
                        cluster_member: false,
                    });
            }
        }
    }
    (scope, [platform, external])
}

fn recovered_mixed_hierarchy_input() -> (Graph, [NodeId; 8]) {
    let mut input = Graph::default();
    let platform = input.add_node(node("platform", 100.0, 36.0));

    let mut child = |name: &str, width: f64, height: f64, shape: ShapeKind| {
        let mut value = node(name, width, height);
        value.parent = Some(platform);
        value.shape = shape;
        input.add_node(value)
    };
    let users = child("users", 54.0, 66.0, ShapeKind::Person);
    let api = child("api", 171.0, 69.0, ShapeKind::Hexagon);
    let jobs = child("jobs", 171.0, 66.0, ShapeKind::Queue);
    let database = child("db", 125.0, 118.0, ShapeKind::Cylinder);
    let table = child("table", 123.0, 36.0, ShapeKind::SqlTable);
    let note = child("note", 114.0, 21.0, ShapeKind::Text);
    drop(child);

    let mut external_node = node("external", 193.0, 84.0);
    external_node.shape = ShapeKind::Cloud;
    let external = input.add_node(external_node);

    for (source, target, label) in [
        (users, api, None),
        (api, jobs, None),
        (jobs, database, None),
        (database, table, None),
        (note, api, Some(("documents", 74.0))),
        (external, api, Some(("webhook", 60.0))),
        (database, external, Some(("export", 43.0))),
    ] {
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        if let Some((text, width)) = label {
            input.set_edge_label(
                edge,
                Some(EdgeLabel {
                    text: text.to_owned(),
                    size: Size {
                        width,
                        height: 21.0,
                    },
                    position: LabelPosition::Unset,
                    percentage: 0.0,
                }),
            );
        }
    }
    (
        input,
        [platform, users, api, jobs, database, table, note, external],
    )
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_rejoins_at_recovered_node_placement_boundary() {
    let (input, nodes) = recovered_mixed_hierarchy_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::NodePlacement);
    assert_eq!(
        nodes.map(|node| snapshot.nodes[node.0 as usize].position.unwrap()),
        [
            Point { x: 0.0, y: 0.0 },
            Point { x: 316.0, y: 348.0 },
            Point { x: 220.0, y: 188.0 },
            Point { x: 60.0, y: 60.0 },
            Point { x: 60.0, y: 188.0 },
            Point { x: 60.0, y: 380.0 },
            Point { x: 284.0, y: 92.0 },
            Point { x: 630.0, y: 252.0 },
        ]
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_routing_recovers_scope_tunnels_and_lane_interactions() {
    let (input, _) = recovered_mixed_hierarchy_input();
    let routed = layout_snapshot(&input, 1, LayoutStage::EdgeRouting);
    assert_eq!(
        routed.edges[1].points,
        vec![
            Point {
                x: 1291.0,
                y: 1188.0
            },
            Point {
                x: 1291.0,
                y: 1157.0
            },
            Point {
                x: 1167.0,
                y: 1157.0
            },
            Point {
                x: 1167.0,
                y: 1126.0
            },
        ]
    );
    assert_eq!(
        routed.edges[2].points,
        vec![
            Point {
                x: 1144.0,
                y: 1126.0
            },
            Point {
                x: 1144.0,
                y: 1194.0
            },
        ]
    );
    assert_eq!(
        routed.edges[5].points,
        vec![
            Point {
                x: 1702.0,
                y: 1265.0
            },
            Point {
                x: 1702.0,
                y: 1223.0
            },
            Point {
                x: 1419.0,
                y: 1223.0
            },
        ]
    );

    let balanced = layout_snapshot(&input, 1, LayoutStage::BalanceEdgeSegments);
    assert_eq!(
        balanced.edges[1].points,
        vec![
            Point {
                x: 1292.0,
                y: 1188.0
            },
            Point {
                x: 1292.0,
                y: 1157.0
            },
            Point {
                x: 1163.0,
                y: 1157.0
            },
            Point {
                x: 1163.0,
                y: 1126.0
            },
        ]
    );
    assert_eq!(
        balanced.edges[2].points,
        vec![
            Point {
                x: 1122.0,
                y: 1126.0
            },
            Point {
                x: 1122.0,
                y: 1194.0
            },
        ]
    );
    assert_eq!(
        balanced.edges[5].points,
        vec![
            Point {
                x: 1725.0,
                y: 1265.0
            },
            Point {
                x: 1725.0,
                y: 1220.0
            },
            Point {
                x: 1419.0,
                y: 1220.0
            },
        ]
    );

    let traced = layout_snapshot(&input, 1, LayoutStage::TraceEdgesToShapeBorder);
    assert_eq!(
        traced.edges[5].points,
        vec![
            Point {
                x: 1725.0,
                y: 1254.0
            },
            Point {
                x: 1725.0,
                y: 1220.0
            },
            Point {
                x: 1416.0,
                y: 1220.0
            },
        ]
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_matches_recovered_first_alignment_and_gap_boundaries() {
    let (input, nodes) = recovered_mixed_hierarchy_input();
    let aligned = layout_snapshot(&input, 1, LayoutStage::AlignAxes);
    assert_eq!(
        nodes.map(|node| aligned.nodes[node.0 as usize].position.unwrap()),
        [
            Point { x: -1.0, y: 0.0 },
            Point { x: 315.0, y: 348.0 },
            Point { x: 219.0, y: 188.0 },
            Point { x: 59.0, y: 60.0 },
            Point { x: 59.0, y: 188.0 },
            Point { x: 60.0, y: 380.0 },
            Point { x: 283.0, y: 92.0 },
            Point { x: 629.0, y: 252.0 },
        ]
    );

    let normalized = layout_snapshot(&input, 1, LayoutStage::GapNormalization);
    assert_eq!(
        nodes.map(|node| normalized.nodes[node.0 as usize].position.unwrap()),
        [
            Point { x: 21.0, y: 0.0 },
            Point { x: 337.0, y: 348.0 },
            Point { x: 248.0, y: 188.0 },
            Point { x: 81.0, y: 60.0 },
            Point { x: 81.0, y: 188.0 },
            Point { x: 82.0, y: 380.0 },
            Point { x: 305.0, y: 92.0 },
            Point { x: 629.0, y: 252.0 },
        ]
    );

    let equidistant = layout_snapshot(&input, 1, LayoutStage::Equidistance);
    assert_eq!(
        nodes.map(|node| equidistant.nodes[node.0 as usize].position.unwrap()),
        [
            Point { x: 21.0, y: 0.0 },
            Point { x: 337.0, y: 348.0 },
            Point { x: 248.0, y: 188.0 },
            Point { x: 81.0, y: 60.0 },
            Point { x: 81.0, y: 194.0 },
            Point { x: 82.0, y: 380.0 },
            Point { x: 276.0, y: 92.0 },
            Point { x: 629.0, y: 252.0 },
        ]
    );
}

#[test]
fn mixed_hierarchy_root_scope_matches_recovered_early_boundaries() {
    let (mut scope, [platform, external]) = recovered_mixed_hierarchy_root_scope();
    scope.run_initialize_nodes().unwrap();
    assert_eq!(
        [platform, external].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 2.0, y: 2.0 }),
            Some(Point { x: 1.0, y: 2.0 })
        ]
    );

    scope.run_sizeless_anneal();
    assert_eq!(
        [platform, external].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 12.0, y: 10.0 }),
            Some(Point { x: 11.0, y: 10.0 })
        ]
    );

    scope.graph.transition_compact();
    scope.graph.snap_nonfixed_to_cells();
    assert_eq!(
        [platform, external].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 630.0, y: 0.0 }),
            Some(Point { x: 0.0, y: 0.0 })
        ]
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_root_scope_matches_recovered_sized_result() {
    let (mut scope, [platform, external]) = recovered_mixed_hierarchy_root_scope();
    scope.run_initialize_nodes().unwrap();
    let node_count = scope.graph.largest_optimizable_component_size();
    let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
    let pass_count = iterations.saturating_sub(iterations / 2 + 1);
    scope.run_sized_pass(
        pass_count,
        false,
        true,
        true,
        Some(BTreeSet::from([platform])),
    );
    let platform_position = scope.graph.position(platform).unwrap();
    let external_position = scope.graph.position(external).unwrap();
    // The optimizer may translate an entire connected component during
    // its random walk. Graph.normalize removes that origin; the recovered
    // observable is the exact relative placement.
    assert_eq!(
        Point {
            x: external_position.x - platform_position.x,
            y: external_position.y - platform_position.y,
        },
        Point { x: 630.0, y: 252.0 }
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_root_scope_matches_recovered_first_sized_pass() {
    let (mut scope, [platform, external]) = recovered_mixed_hierarchy_root_scope();
    scope.run_initialize_nodes().unwrap();
    let rng = scope.run_sizeless_anneal_with_rng();
    scope.graph.transition_compact();
    scope.graph.snap_nonfixed_to_cells();
    scope.graph.initialize_turn_cost();

    let node_count = scope.graph.largest_optimizable_component_size();
    let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
    let initial_temperature = 2.0 * (node_count as f64).sqrt();
    let cooling_factor = (0.2 / initial_temperature).powf(1.0 / iterations as f64);
    let temperature = initial_temperature * cooling_factor.powi((iterations / 2) as i32);
    let mut optimizer =
        sized::SizedOptimizer::new(&mut scope.graph, rng, Some(BTreeSet::from([platform])));
    optimizer.optimize(temperature);

    assert_eq!(
        [platform, external].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 0.0, y: -630.0 }),
            Some(Point {
                x: 630.0,
                y: -378.0
            })
        ]
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_root_scope_matches_recovered_external_candidate_scores() {
    let (mut scope, [platform, external]) = recovered_mixed_hierarchy_root_scope();
    scope.run_initialize_nodes().unwrap();
    let _rng = scope.run_sizeless_anneal_with_rng();
    scope.graph.transition_compact();
    scope.graph.snap_nonfixed_to_cells();
    // getTurnCost is cached at the sized-phase boundary, before the first
    // candidate moves either endpoint.
    scope.graph.initialize_turn_cost();
    scope
        .graph
        .set_position(platform, Point { x: 0.0, y: -630.0 });
    scope.graph.set_position(external, Point { x: 0.0, y: 0.0 });
    let (obstructions, max_obstruction_width) = scope.graph.sorted_overlap_candidates(external);

    for (candidate, expected_edge_length) in [
        (
            Point {
                x: 630.0,
                y: -378.0,
            },
            721.834_999_676_438_2,
        ),
        (
            Point {
                x: -378.0,
                y: -504.0,
            },
            742.462_364_912_961_9,
        ),
    ] {
        scope.graph.set_position(external, candidate);
        let edge_length = scope.graph.sized_edge_length_with_obstructions(
            external,
            true,
            Some((&obstructions, max_obstruction_width)),
        );
        assert!(
            (edge_length - expected_edge_length).abs() < 1e-9,
            "candidate {candidate:?}: {edge_length}"
        );
    }
}

#[test]
fn mixed_hierarchy_child_scope_matches_recovered_early_boundaries() {
    let (mut scope, [users, api, jobs, note, database, table]) =
        recovered_mixed_hierarchy_child_scope();
    scope.run_initialize_nodes().unwrap();
    assert_eq!(
        [users, api, jobs, note, database, table].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 6.0, y: 6.0 }),
            Some(Point { x: 7.0, y: 6.0 }),
            Some(Point { x: 7.0, y: 7.0 }),
            Some(Point { x: 7.0, y: 5.0 }),
            Some(Point { x: 8.0, y: 7.0 }),
            Some(Point { x: 9.0, y: 7.0 }),
        ]
    );

    scope.run_sizeless_anneal();
    assert_eq!(
        [users, api, jobs, note, database, table].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 62.0, y: 31.0 }),
            Some(Point { x: 63.0, y: 31.0 }),
            Some(Point { x: 63.0, y: 30.0 }),
            Some(Point { x: 64.0, y: 31.0 }),
            Some(Point { x: 62.0, y: 30.0 }),
            Some(Point { x: 63.0, y: 29.0 }),
        ]
    );

    scope.graph.transition_compact();
    scope.graph.snap_nonfixed_to_cells();
    assert_eq!(
        [users, api, jobs, note, database, table].map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 32.0, y: 384.0 }),
            Some(Point { x: 448.0, y: 384.0 }),
            Some(Point { x: 448.0, y: 160.0 }),
            Some(Point { x: 992.0, y: 0.0 }),
            Some(Point { x: 32.0, y: 0.0 }),
            Some(Point { x: 448.0, y: 0.0 }),
        ]
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_child_scope_matches_recovered_first_sized_pass() {
    let (mut scope, nodes) = recovered_mixed_hierarchy_child_scope();
    scope.run_initialize_nodes().unwrap();
    let rng = scope.run_sizeless_anneal_with_rng();
    scope.graph.transition_compact();
    scope.graph.snap_nonfixed_to_cells();
    scope.graph.initialize_turn_cost();

    let node_count = scope.graph.largest_optimizable_component_size();
    let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
    let initial_temperature = 2.0 * (node_count as f64).sqrt();
    let cooling_factor = (0.2 / initial_temperature).powf(1.0 / iterations as f64);
    let temperature = initial_temperature * cooling_factor.powi((iterations / 2) as i32);
    let mut optimizer = sized::SizedOptimizer::new(&mut scope.graph, rng, Some(BTreeSet::new()));
    optimizer.optimize(temperature);
    assert_eq!(
        nodes.map(|node| scope.graph.position(node)),
        [
            Some(Point { x: 512.0, y: 256.0 }),
            Some(Point { x: 416.0, y: -64.0 }),
            Some(Point { x: 256.0, y: 96.0 }),
            Some(Point {
                x: 448.0,
                y: -160.0,
            }),
            Some(Point { x: 64.0, y: 0.0 }),
            Some(Point { x: 0.0, y: -96.0 }),
        ]
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn mixed_hierarchy_child_scope_matches_recovered_sized_pass_entries() {
    let (mut scope, nodes) = recovered_mixed_hierarchy_child_scope();
    scope.run_initialize_nodes().unwrap();
    let mut rng = scope.run_sizeless_anneal_with_rng();
    scope.graph.transition_compact();
    scope.graph.snap_nonfixed_to_cells();
    scope.graph.initialize_turn_cost();

    let node_count = scope.graph.largest_optimizable_component_size();
    let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
    let initial_temperature = 2.0 * (node_count as f64).sqrt();
    let cooling_factor = (0.2 / initial_temperature).powf(1.0 / iterations as f64);
    let mut temperature = initial_temperature * cooling_factor.powi((iterations / 2) as i32);
    let mut horizontal_compaction = false;
    for pass in 1..=109 {
        let expected = match pass {
            4 => Some([
                Point { x: 256.0, y: 128.0 },
                Point { x: 384.0, y: 96.0 },
                Point { x: 448.0, y: -96.0 },
                Point { x: 416.0, y: 0.0 },
                Point {
                    x: 480.0,
                    y: -288.0,
                },
                Point {
                    x: 480.0,
                    y: -384.0,
                },
            ]),
            8 => Some([
                Point {
                    x: 192.0,
                    y: -160.0,
                },
                Point { x: 128.0, y: 0.0 },
                Point { x: 384.0, y: -32.0 },
                Point { x: 128.0, y: 160.0 },
                Point { x: 544.0, y: 128.0 },
                Point { x: 448.0, y: 320.0 },
            ]),
            12 => Some([
                Point { x: 32.0, y: 128.0 },
                Point { x: -32.0, y: 256.0 },
                Point { x: -96.0, y: 32.0 },
                Point { x: 384.0, y: 32.0 },
                Point { x: 192.0, y: 64.0 },
                Point { x: 192.0, y: -32.0 },
            ]),
            _ => None,
        };
        if let Some(expected) = expected {
            assert_eq!(
                nodes.map(|node| scope.graph.position(node).unwrap()),
                expected,
                "sized pass {pass} entry"
            );
        }

        let mut optimizer =
            sized::SizedOptimizer::new(&mut scope.graph, rng, Some(BTreeSet::new()));
        optimizer.optimize(temperature);
        if pass == 4 {
            rng = optimizer.into_rng();
            assert_eq!(
                nodes.map(|node| scope.graph.position(node).unwrap()),
                [
                    Point { x: 448.0, y: 256.0 },
                    Point { x: 192.0, y: 320.0 },
                    Point { x: 544.0, y: 256.0 },
                    Point { x: 384.0, y: 0.0 },
                    Point { x: 576.0, y: 32.0 },
                    Point {
                        x: 480.0,
                        y: -384.0,
                    },
                ],
                "sized pass 4 result"
            );
            temperature *= cooling_factor;
            continue;
        }
        if pass == 12 {
            rng = optimizer.into_rng();
            assert_eq!(
                nodes.map(|node| scope.graph.position(node).unwrap()),
                [
                    Point {
                        x: -288.0,
                        y: 160.0,
                    },
                    Point {
                        x: -160.0,
                        y: 160.0,
                    },
                    Point { x: -64.0, y: 32.0 },
                    Point { x: -256.0, y: 64.0 },
                    Point { x: 32.0, y: 160.0 },
                    Point { x: 224.0, y: -32.0 },
                ],
                "sized pass 12 result"
            );
            temperature *= cooling_factor;
            continue;
        }
        if pass == 13 {
            rng = optimizer.into_rng();
            assert_eq!(
                nodes.map(|node| scope.graph.position(node).unwrap()),
                [
                    Point {
                        x: -288.0,
                        y: 160.0,
                    },
                    Point {
                        x: -96.0,
                        y: -128.0,
                    },
                    Point { x: 64.0, y: -352.0 },
                    Point {
                        x: -64.0,
                        y: -224.0,
                    },
                    Point { x: 96.0, y: -224.0 },
                    Point { x: 96.0, y: -32.0 },
                ],
                "sized pass 13 result"
            );
            temperature *= cooling_factor;
            continue;
        }
        let iteration = iterations / 2 + pass;
        if iteration.is_multiple_of(9) {
            let factor = (1.0
                + (2.0 * (iterations as f64 - iteration as f64 - 30.0))
                    / (0.5 * iterations as f64))
                .max(1.0);
            optimizer.compact(horizontal_compaction, factor).unwrap();
            optimizer.join_distanced_clusters();
            horizontal_compaction = !horizontal_compaction;
        }
        rng = optimizer.into_rng();
        let expected_result = match pass {
            22 => Some([
                Point {
                    x: 1056.0,
                    y: -256.0,
                },
                Point {
                    x: 640.0,
                    y: -352.0,
                },
                Point {
                    x: 640.0,
                    y: -480.0,
                },
                Point {
                    x: 832.0,
                    y: -384.0,
                },
                Point {
                    x: 704.0,
                    y: -672.0,
                },
                Point {
                    x: 704.0,
                    y: -768.0,
                },
            ]),
            78 => Some([
                Point {
                    x: 576.0,
                    y: -1248.0,
                },
                Point {
                    x: 480.0,
                    y: -1120.0,
                },
                Point {
                    x: 320.0,
                    y: -1248.0,
                },
                Point {
                    x: 576.0,
                    y: -960.0,
                },
                Point {
                    x: 320.0,
                    y: -1120.0,
                },
                Point {
                    x: 320.0,
                    y: -928.0,
                },
            ]),
            109 => Some([
                Point {
                    x: 1312.0,
                    y: -960.0,
                },
                Point {
                    x: 1216.0,
                    y: -1120.0,
                },
                Point {
                    x: 1120.0,
                    y: -1248.0,
                },
                Point {
                    x: 1312.0,
                    y: -1216.0,
                },
                Point {
                    x: 1056.0,
                    y: -1120.0,
                },
                Point {
                    x: 1056.0,
                    y: -928.0,
                },
            ]),
            _ => None,
        };
        if let Some(expected_result) = expected_result {
            assert_eq!(
                nodes.map(|node| scope.graph.position(node).unwrap()),
                expected_result,
                "sized pass {pass} result"
            );
        }
        temperature *= cooling_factor;
    }

    let mut optimizer = sized::SizedOptimizer::new(&mut scope.graph, rng, Some(BTreeSet::new()));
    optimizer.join_distanced_clusters();
    for _ in 0..10 {
        if !optimizer.optimize(0.0) {
            break;
        }
    }
    assert_eq!(
        nodes.map(|node| scope.graph.position(node).unwrap()),
        [
            Point {
                x: 1312.0,
                y: -960.0,
            },
            Point {
                x: 1216.0,
                y: -1120.0,
            },
            Point {
                x: 1056.0,
                y: -1248.0,
            },
            Point {
                x: 1280.0,
                y: -1216.0,
            },
            Point {
                x: 1056.0,
                y: -1120.0,
            },
            Point {
                x: 1056.0,
                y: -928.0,
            },
        ],
        "sized zero-temperature result"
    );
}

#[test]
fn direct_mirrors_dominant_edges_toward_unset_default_axes() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 20.0, 30.0));
    let target = input.add_node(node("target", 40.0, 50.0));
    let edge = input.add_edge(Edge { source, target });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(source, Point { x: 100.0, y: 100.0 });
    arena.set_position(target, Point { x: 0.0, y: 0.0 });

    assert_eq!(arena.direction_transforms(None), (true, true));
    arena.direct(false);

    assert_eq!(
        arena.position(source),
        Some(Point {
            x: -120.0,
            y: -130.0
        })
    );
    assert_eq!(arena.position(target), Some(Point { x: -40.0, y: -50.0 }));
}

#[test]
fn direct_score_comparison_uses_the_recovered_open_precision_boundary() {
    let precision = 0.0001_f64;
    assert!(!ArenaGraph::direct_score_is_worse(
        precision.next_down(),
        0.0
    ));
    assert!(ArenaGraph::direct_score_is_worse(precision, 0.0));
    assert!(!ArenaGraph::direct_score_is_worse(-precision, 0.0));
    assert!(ArenaGraph::direct_score_is_worse(f64::NAN, 0.0));
}

#[test]
fn direct_skips_a_fixed_descendant_absent_from_placement_nodes() {
    let mut input = Graph::with_direction(Direction::Right);
    let group = input.add_node(node("group", 100.0, 100.0));
    let outside = input.add_node(node("outside", 80.0, 80.0));
    let edge = input.add_edge(Edge {
        source: group,
        target: outside,
    });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(group, Point { x: 300.0, y: 0.0 });
    arena.set_position(outside, Point::default());
    arena.nodes[group.0 as usize].scoring_is_container = true;
    let group_tala_id = arena.nodes[group.0 as usize].tala_id;
    let child_position = Point { x: 320.0, y: 20.0 };
    let mut fixed_child = arena.nodes[outside.0 as usize].clone();
    fixed_child.tala_id = 30_001;
    fixed_child.position = Some(child_position);
    fixed_child.rect.origin = child_position;
    fixed_child.fixed_top_left = Some(child_position);
    fixed_child.scoring_container_parent = Some(group_tala_id);
    fixed_child.scoring_container_ancestors = vec![group_tala_id];

    // An unrelated fixed snapshot is not a descendant of either current node.
    let mut control = arena.clone();
    control
        .transaction_external_container_children
        .insert(42, vec![fixed_child.clone()]);
    assert_eq!(control.direction_transforms(None), (true, false));
    assert_eq!(control.direct(false), (true, false));

    // Original child pointers survive scope extraction, even though the
    // temporary graph has only the group and outside nodes.
    arena
        .transaction_external_container_children
        .insert(group_tala_id, vec![fixed_child]);
    assert_eq!(arena.direction_transforms(None), (true, false));
    assert_eq!(arena.direct(false), (false, false));
    assert_eq!(arena.position(group), Some(Point { x: 300.0, y: 0.0 }));
    assert_eq!(arena.position(outside), Some(Point::default()));
    assert_eq!(
        arena.transaction_external_container_children[&group_tala_id][0].position,
        Some(child_position)
    );
}

#[test]
fn transpose_rejects_a_container_trial_that_moves_a_fixed_child() {
    let mut input = Graph::default();
    let group = input.add_node(node("group", 420.0, 340.0));
    let mut child = node("group.a", 80.0, 80.0);
    child.parent = Some(group);
    child.locked_position = Some(Point { x: 220.0, y: 140.0 });
    let child = input.add_node(child);
    let outside = input.add_node(node("outside", 80.0, 80.0));
    let edge = input.add_edge(Edge {
        source: outside,
        target: group,
    });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 120.0;
    arena.set_position(group, Point::default());
    arena.set_position(child, Point { x: 280.0, y: 200.0 });
    arena.set_position(outside, Point { x: 480.0, y: 120.0 });
    let before = arena
        .nodes
        .iter()
        .map(|node| (node.position, node.rect.size))
        .collect::<Vec<_>>();

    // The container itself is movable. Its FixedOrigin is nil, so Go opens
    // the trial, then IsBadState rejects changes to the fixed child's current
    // position. That snapshot position includes existing container padding.
    assert!(!arena.transpose_node(group, false));
    assert_eq!(
        arena
            .nodes
            .iter()
            .map(|node| (node.position, node.rect.size))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn transpose_rejects_crossing_a_fixed_origin_without_moving_the_fixed_node() {
    let mut input = Graph::default();
    let group = input.add_node(node("group", 200.0, 200.0));
    let mut child = node("group.a", 80.0, 80.0);
    child.parent = Some(group);
    let child = input.add_node(child);
    let mut fixed = node("fixed", 80.0, 80.0);
    fixed.locked_position = Some(Point::default());
    let fixed = input.add_node(fixed);
    let edge = input.add_edge(Edge {
        source: group,
        target: fixed,
    });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 120.0;
    arena.set_position(group, Point { x: 200.0, y: 0.0 });
    arena.set_position(child, Point { x: 260.0, y: 60.0 });
    arena.set_position(fixed, Point::default());
    let before = arena
        .nodes
        .iter()
        .map(|node| (node.position, node.rect.size))
        .collect::<Vec<_>>();

    // All three rotations leave the fixed peer untouched, but put the
    // movable container past the root origin in at least one coordinate.
    assert!(!arena.transpose_node(group, false));
    assert_eq!(
        arena
            .nodes
            .iter()
            .map(|node| (node.position, node.rect.size))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn root_mirror_commits_tree_orientation_with_node_geometry_once() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 20.0, 30.0));
    let target = input.add_node(node("target", 40.0, 50.0));
    let edge = input.add_edge(Edge { source, target });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(source, Point { x: 100.0, y: 100.0 });
    arena.set_position(target, Point { x: 0.0, y: 0.0 });
    arena.tree_routing_nodes.insert(
        source,
        TreeRoutingNode {
            parent: target,
            sentinel_edge: edge,
            orientation: Orientation::Right,
            extraction_order: 0,
        },
    );
    assert_eq!(arena.direct(false), (true, true));
    assert_eq!(
        arena.position(source),
        Some(Point {
            x: -120.0,
            y: -130.0
        })
    );
    assert_eq!(arena.position(target), Some(Point { x: -40.0, y: -50.0 }));
    assert_eq!(
        arena.tree_routing_nodes[&source].orientation,
        Orientation::Left
    );
    assert_eq!(arena.tree_routing_nodes[&source].parent, target);
    assert_eq!(arena.tree_routing_nodes[&source].sentinel_edge, edge);
    assert_eq!(arena.direct(false), (false, false));
    assert_eq!(
        arena.tree_routing_nodes[&source].orientation,
        Orientation::Left
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn root_mirror_publishes_cluster_vessel_and_member_pointer_aliases() {
    const VESSEL_TALA_ID: i64 = 10_001;

    let mut input = Graph::default();
    let first = input.add_node(node("first", 100.0, 60.0));
    let second = input.add_node(node("second", 100.0, 60.0));
    let edge = input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 20.0,
        vessel_tala_id: VESSEL_TALA_ID,
        fixed_size: false,
    });
    arena.nodes[first.0 as usize].cluster = Some(0);
    arena.nodes[second.0 as usize].cluster = Some(0);
    arena.set_position(first, Point { x: 100.0, y: 40.0 });
    arena.set_position(second, Point { x: 300.0, y: 40.0 });
    arena
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 50.0, y: 20.0 });
    arena.rebuild_active_aggregate_node_order();

    let vessel_size = arena.cluster_vessel_size(0);
    let mut vessel_alias = arena.nodes[first.0 as usize].clone();
    vessel_alias.tala_id = VESSEL_TALA_ID;
    vessel_alias.position = Some(Point { x: 50.0, y: 20.0 });
    vessel_alias.rect.origin = Point { x: 50.0, y: 20.0 };
    vessel_alias.rect.size = vessel_size;
    arena
        .transaction_external_container_children
        .insert(99_001, vec![vessel_alias]);
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
        ],
    );
    let second_tala_id = arena.nodes[second.0 as usize].tala_id;
    arena.sized_adjacent_overrides.insert(
        (first, edge),
        ProjectedAdjacent {
            owner: first,
            tala_id: second_tala_id,
            container_tala_id: None,
            offset: Point { x: 200.0, y: 0.0 },
            size: arena.nodes[second.0 as usize].rect.size,
            cluster_member: true,
        },
    );

    arena.mirror_axes(true, false);

    assert_eq!(arena.position(first), Some(Point { x: -200.0, y: 40.0 }));
    assert_eq!(arena.position(second), Some(Point { x: -400.0, y: 40.0 }));
    let mirrored_vessel = Point {
        x: -50.0 - vessel_size.width,
        y: 20.0,
    };
    assert_eq!(
        arena.pending_cluster_vessel_positions.get(&0),
        Some(&mirrored_vessel)
    );
    assert_eq!(
        arena
            .transaction_external_container_children
            .get(&99_001)
            .unwrap()[0]
            .position,
        Some(mirrored_vessel)
    );
    assert_eq!(
        arena
            .transaction_external_aggregate_children
            .get(&VESSEL_TALA_ID)
            .unwrap()
            .iter()
            .map(|member| member.position.unwrap())
            .collect::<Vec<_>>(),
        vec![Point { x: -200.0, y: 40.0 }, Point { x: -400.0, y: 40.0 },]
    );
    assert_eq!(
        arena.sized_adjacent_overrides[&(first, edge)].offset,
        Point { x: -200.0, y: 0.0 }
    );
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn root_mirror_keeps_first_sequence_step_distinct_from_wider_vessel_alias() {
    const VESSEL_TALA_ID: i64 = 10_001;

    let mut input = Graph::default();
    let container = input.add_node(node("container", 500.0, 300.0));
    let mut first_node = node("first", 80.0, 50.0);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 100.0, 50.0);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);
    let source = input.add_node(node("source", 20.0, 30.0));
    let target = input.add_node(node("target", 40.0, 50.0));
    let direction_edge = input.add_edge(Edge { source, target });
    input.set_edge_arrows(
        direction_edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );

    let mut arena = ArenaGraph::from_input(&input);
    arena.sequences.push(SequenceState {
        members: vec![first, second],
        vessel_tala_id: VESSEL_TALA_ID,
        container: Some(container),
        has_edge_abductions: false,
    });
    arena.nodes[first.0 as usize].sequence = Some(0);
    arena.nodes[second.0 as usize].sequence = Some(0);
    arena.set_position(container, Point::default());
    arena.set_position(first, Point { x: 100.0, y: 40.0 });
    arena.set_position(second, Point { x: 145.0, y: 40.0 });
    arena.set_position(source, Point { x: 100.0, y: 100.0 });
    arena.set_position(target, Point::default());
    arena.rebuild_active_aggregate_node_order();

    let first_tala_id = arena.nodes[first.0 as usize].tala_id;
    let second_tala_id = arena.nodes[second.0 as usize].tala_id;
    let vessel_size = arena.sequence_vessel_size(0);
    assert_ne!(
        vessel_size.width,
        arena.nodes[first.0 as usize].rect.size.width
    );

    let mut vessel_alias = arena.nodes[first.0 as usize].clone();
    vessel_alias.tala_id = VESSEL_TALA_ID;
    vessel_alias.rect.size = vessel_size;
    arena.transaction_external_container_children.insert(
        arena.nodes[container.0 as usize].tala_id,
        vec![vessel_alias],
    );
    arena.transaction_external_aggregate_children.insert(
        VESSEL_TALA_ID,
        vec![
            arena.nodes[first.0 as usize].clone(),
            arena.nodes[second.0 as usize].clone(),
        ],
    );

    let mut rejected = arena.clone();
    rejected.set_crossing_cost(f64::NAN);
    assert_eq!(rejected.direction_transforms(None), (true, true));
    assert_eq!(rejected.direct(true), (false, false));
    assert_eq!(rejected.position(first), Some(Point { x: 100.0, y: 40.0 }));
    assert_eq!(
        rejected.transaction_external_aggregate_children[&VESSEL_TALA_ID][0].position,
        Some(Point { x: 100.0, y: 40.0 })
    );
    assert_eq!(
        rejected.transaction_external_container_children
            [&rejected.nodes[container.0 as usize].tala_id][0]
            .position,
        Some(Point { x: 100.0, y: 40.0 })
    );

    let reflected_first = Point { x: -180.0, y: 40.0 };
    let reflected_second = Point { x: -245.0, y: 40.0 };
    let reflected_vessel = Point {
        x: -100.0 - vessel_size.width,
        y: 40.0,
    };
    arena.mirror_axes(true, false);

    let final_vessel = arena.position(first).unwrap();
    let refit_delta = Point {
        x: final_vessel.x - reflected_vessel.x,
        y: final_vessel.y - reflected_vessel.y,
    };
    assert_ne!(refit_delta, Point::default());
    let projected_steps = &arena.transaction_external_aggregate_children[&VESSEL_TALA_ID];
    assert_eq!(projected_steps[0].tala_id, first_tala_id);
    assert_eq!(
        projected_steps[0].position,
        Some(Point {
            x: reflected_first.x + refit_delta.x,
            y: reflected_first.y + refit_delta.y,
        })
    );
    assert_eq!(projected_steps[1].tala_id, second_tala_id);
    assert_eq!(
        projected_steps[1].position,
        Some(Point {
            x: reflected_second.x + refit_delta.x,
            y: reflected_second.y + refit_delta.y,
        })
    );
    assert_eq!(
        arena.transaction_external_container_children[&arena.nodes[container.0 as usize].tala_id]
            [0]
        .position,
        Some(final_vessel)
    );
    assert_ne!(projected_steps[0].position, Some(final_vessel));
}

#[test]
fn direct_ignores_edges_restored_below_both_projected_carriers() {
    let mut input = Graph::default();
    let source = input.add_node(node("source carrier", 171.0, 186.0));
    let target = input.add_node(node("target carrier", 349.0, 308.0));
    let edge = input.add_edge(Edge { source, target });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    let source_position = Point {
        x: 1_396.0,
        y: 349.0,
    };
    let target_position = Point { x: 698.0, y: 349.0 };
    arena.set_position(source, source_position);
    arena.set_position(target, target_position);
    arena.sized_adjacent_overrides.insert(
        (target, edge),
        ProjectedAdjacent {
            owner: source,
            tala_id: 1,
            container_tala_id: None,
            offset: Point { x: 60.0, y: 60.0 },
            size: Size {
                width: 51.0,
                height: 66.0,
            },
            cluster_member: false,
        },
    );
    arena.sized_adjacent_overrides.insert(
        (source, edge),
        ProjectedAdjacent {
            owner: target,
            tala_id: 2,
            container_tala_id: None,
            offset: Point { x: 235.0, y: 112.0 },
            size: Size {
                width: 54.0,
                height: 82.0,
            },
            cluster_member: false,
        },
    );

    assert_eq!(arena.direction_transforms(None), (false, false));
    assert_eq!(arena.direct(false), (false, false));
    assert_eq!(arena.position(source), Some(source_position));
    assert_eq!(arena.position(target), Some(target_position));
}

#[test]
fn direct_skips_when_the_first_component_node_owns_a_hierarchy() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 40.0, 50.0));
    let target = input.add_node(node("target", 20.0, 30.0));
    let mut child = node("child", 10.0, 10.0);
    child.parent = Some(container);
    input.add_node(child);
    let edge = input.add_edge(Edge {
        source: container,
        target,
    });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    arena.assign_hierarchy_member(container, 0, None, 0, 2);
    arena.set_position(container, Point { x: 100.0, y: 100.0 });
    arena.set_position(target, Point { x: 0.0, y: 0.0 });

    assert_eq!(arena.direction_transforms(None), (true, true));
    arena.direct(false);

    assert_eq!(
        arena.position(container),
        Some(Point { x: 100.0, y: 100.0 })
    );
    assert_eq!(arena.position(target), Some(Point { x: 0.0, y: 0.0 }));
}

#[test]
fn projected_sequence_member_uses_its_live_aggregate_position() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 500.0, 400.0));
    let carrier = input.add_node(node("sequence carrier", 278.0, 126.0));
    let mut member_node = node("sequence step", 120.0, 126.0);
    member_node.parent = Some(container);
    let member = input.add_node(member_node);

    let mut arena = ArenaGraph::from_input(&input);
    let container_position = Point::default();
    let carrier_position = Point { x: 278.0, y: 278.0 };
    let member_position = Point { x: 357.0, y: 278.0 };
    arena.set_position(container, container_position);
    arena.set_position(carrier, carrier_position);
    arena.set_position(member, member_position);
    let container_tala_id = arena.nodes[container.0 as usize].tala_id;
    let carrier_tala_id = arena.nodes[carrier.0 as usize].tala_id;
    let member_tala_id = arena.nodes[member.0 as usize].tala_id;
    arena.transaction_external_aggregate_children.insert(
        carrier_tala_id,
        vec![arena.nodes[member.0 as usize].clone()],
    );

    let projected = ProjectedAdjacent {
        owner: carrier,
        tala_id: member_tala_id,
        container_tala_id: Some(container_tala_id),
        offset: Point { x: 79.0, y: 0.0 },
        size: arena.nodes[member.0 as usize].rect.size,
        cluster_member: false,
    };

    assert_eq!(
        arena.projected_absolute_position(projected),
        Some(member_position),
        "a sequence member follows its active vessel instead of the stale offset in the outer-container frame"
    );
}

#[test]
fn direct_skips_a_projected_sequence_with_edge_abductions() {
    let mut input = Graph::default();
    let vessel = input.add_node(node("sequence vessel", 40.0, 50.0));
    let target = input.add_node(node("target", 20.0, 30.0));
    let edge = input.add_edge(Edge {
        source: vessel,
        target,
    });
    input.set_edge_arrows(
        edge,
        EdgeArrows {
            source: false,
            target: true,
        },
    );
    let mut arena = ArenaGraph::from_input(&input);
    let vessel_position = Point { x: 100.0, y: 100.0 };
    let target_position = Point { x: 0.0, y: 0.0 };
    arena.set_position(vessel, vessel_position);
    arena.set_position(target, target_position);
    arena.sized_edge_abductions.push(SizedEdgeAbduction {
        source_edge: edge,
        edge_tala_id: arena.edge_records[edge.0 as usize].tala_id,
        edge: Some(edge),
        current_from: Some(AbductionCurrentNodeRef::local(vessel)),
        current_to: Some(AbductionCurrentNodeRef::local(target)),
        originally_from: None,
        originally_to: None,
        originally_from_table_neighbors: Vec::new(),
        originally_to_table_neighbors: Vec::new(),
        originally_from_container: None,
        originally_to_container: None,
        sequence_abduction: true,
        obstructions_from_to: Vec::new(),
        obstructions_to_from: Vec::new(),
        obstruction_sets_from_to: OrderedObstructionInventory::default(),
        obstruction_sets_to_from: OrderedObstructionInventory::default(),
    });

    assert_eq!(arena.direction_transforms(None), (true, true));
    assert_eq!(arena.direct(false), (false, false));
    assert_eq!(arena.position(vessel), Some(vessel_position));
    assert_eq!(arena.position(target), Some(target_position));
}

#[test]
fn mirrored_component_reflects_only_reachable_container_descendants() {
    let mut input = Graph::default();
    let outer = input.add_node(node("outer", 300.0, 300.0));
    let mut first_node = node("first", 100.0, 100.0);
    first_node.parent = Some(outer);
    let first = input.add_node(first_node);
    let mut second_node = node("second", 100.0, 100.0);
    second_node.parent = Some(outer);
    let second = input.add_node(second_node);
    let mut first_grandchild_node = node("first grandchild", 20.0, 20.0);
    first_grandchild_node.parent = Some(first);
    let first_grandchild = input.add_node(first_grandchild_node);
    let mut second_grandchild_node = node("second grandchild", 20.0, 20.0);
    second_grandchild_node.parent = Some(first);
    let second_grandchild = input.add_node(second_grandchild_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(outer, Point { x: 100.0, y: 100.0 });
    arena.set_position(first, Point { x: 160.0, y: 160.0 });
    arena.set_position(second, Point { x: 220.0, y: 200.0 });
    arena.set_position(first_grandchild, Point { x: 170.0, y: 170.0 });
    arena.set_position(second_grandchild, Point { x: 190.0, y: 180.0 });
    let sibling_before = Point {
        x: arena.position(second).unwrap().x - arena.position(first).unwrap().x,
        y: arena.position(second).unwrap().y - arena.position(first).unwrap().y,
    };
    let grandchildren_before = Point {
        x: arena.position(second_grandchild).unwrap().x
            - arena.position(first_grandchild).unwrap().x,
        y: arena.position(second_grandchild).unwrap().y
            - arena.position(first_grandchild).unwrap().y,
    };

    arena.mirror_subtree_axes(outer, false, true, &BTreeSet::from([None, Some(outer)]));

    let sibling_after = Point {
        x: arena.position(second).unwrap().x - arena.position(first).unwrap().x,
        y: arena.position(second).unwrap().y - arena.position(first).unwrap().y,
    };
    let grandchildren_after = Point {
        x: arena.position(second_grandchild).unwrap().x
            - arena.position(first_grandchild).unwrap().x,
        y: arena.position(second_grandchild).unwrap().y
            - arena.position(first_grandchild).unwrap().y,
    };
    assert_eq!(sibling_after.x, sibling_before.x);
    assert_eq!(sibling_after.y, -sibling_before.y);
    assert_eq!(grandchildren_after, grandchildren_before);
    assert_eq!(
        arena.position(outer),
        Some(Point {
            x: 100.0,
            y: -400.0
        })
    );
}

#[test]
fn vertical_mirror_bottom_aligns_unequal_height_nested_children() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 300.0, 300.0));
    let mut short_node = node("short", 80.0, 40.0);
    short_node.parent = Some(container);
    let short = input.add_node(short_node);
    let mut tall_node = node("tall", 80.0, 100.0);
    tall_node.parent = Some(container);
    let tall = input.add_node(tall_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(container, Point { x: 100.0, y: 100.0 });
    arena.set_position(short, Point { x: 160.0, y: 160.0 });
    arena.set_position(tall, Point { x: 300.0, y: 160.0 });

    arena.mirror_subtree_axes(
        container,
        false,
        true,
        &BTreeSet::from([None, Some(container)]),
    );

    let short_bottom =
        arena.position(short).unwrap().y + arena.nodes[short.0 as usize].rect.size.height;
    let tall_bottom =
        arena.position(tall).unwrap().y + arena.nodes[tall.0 as usize].rect.size.height;
    assert_eq!(short_bottom, tall_bottom);
}

#[test]
fn compute_cell_size_matches_recovered_formula() {
    let mut input = Graph::default();
    input.add_node(node("small", 171.0, 186.0));
    input.add_node(node("large", 174.0, 186.0));
    assert_eq!(ArenaGraph::from_input(&input).cell_size, 186.0);

    let mut skewed = Graph::default();
    skewed.add_node(node("small", 66.0, 66.0));
    skewed.add_node(node("large", 494.0, 618.0));
    assert_eq!(ArenaGraph::from_input(&skewed).cell_size, 99.0);
}

#[test]
fn prescale_ports_recovered_edge_density_rule() {
    let mut input = Graph::default();
    let hub = input.add_node(node("hub", 100.0, 60.0));
    for index in 0..5 {
        let leaf = input.add_node(node(&format!("leaf {index}"), 50.0, 40.0));
        input.add_edge(super::super::Edge {
            source: hub,
            target: leaf,
        });
    }
    let mut pipeline = Pipeline::new(&input, 1, false, false);

    pipeline.run_prescale();

    assert_eq!(pipeline.graph.nodes[hub.0 as usize].rect.size.width, 120.0);
    assert_eq!(pipeline.graph.nodes[hub.0 as usize].rect.size.height, 120.0);
    assert_eq!(pipeline.completed_stages, vec![Stage::Prescale]);
}

#[test]
fn preprocess_hubs_materializes_recovered_connected_spoke_map() {
    let mut input = Graph::default();
    let hub = input.add_node(node("hub", 80.0, 60.0));
    let spoke = input.add_node(node("spoke", 40.0, 40.0));
    let connected = input.add_node(node("connected", 50.0, 40.0));
    let tail = input.add_node(node("tail", 50.0, 40.0));
    input.add_edge(Edge {
        source: hub,
        target: spoke,
    });
    input.add_edge(Edge {
        source: hub,
        target: connected,
    });
    input.add_edge(Edge {
        source: connected,
        target: tail,
    });
    let mut pipeline = Pipeline::new(&input, 1, false, false);

    assert!(pipeline.graph.hubs.is_empty());
    pipeline.run_preprocess_hubs();

    assert_eq!(
        pipeline.graph.hubs,
        BTreeMap::from([(hub, vec![spoke]), (connected, vec![tail])])
    );
    assert_eq!(pipeline.completed_stages, vec![Stage::PreprocessHubs]);
}

#[test]
fn sized_checked_offsets_are_shared_across_node_retries() {
    let mut input = Graph::default();
    let moving = input.add_node(node("moving", 80.0, 60.0));
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(moving, Point { x: 500.0, y: 500.0 });
    let optimizer = sized::SizedOptimizer::new(&mut arena, go_rng::GoRng::new(1), None);

    assert_eq!(
        optimizer.test_checked_offset_reuse(moving, Point::default()),
        (true, false)
    );
}

#[test]
fn sized_long_distance_neighbors_add_boundary_placements() {
    let mut input = Graph::default();
    let moving = input.add_node(node("moving", 20.0, 20.0));
    let adjacent = input.add_node(node("adjacent", 20.0, 20.0));
    for _ in 0..3 {
        input.add_edge(Edge {
            source: moving,
            target: adjacent,
        });
    }
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 100.0;
    arena.set_position(moving, Point { x: 0.0, y: 0.0 });
    arena.set_position(adjacent, Point { x: 500.0, y: 0.0 });
    for edge in &mut arena.edge_records {
        edge.min_width = 200.0;
    }
    let placements = {
        let optimizer = sized::SizedOptimizer::new(&mut arena, go_rng::GoRng::new(1), None);
        optimizer.test_placements(moving, Point::default(), 0.0, false)
    };

    assert!(placements.contains(&Point { x: 200.0, y: 0.0 }));
    assert!(placements.contains(&Point { x: 800.0, y: 0.0 }));

    // NewSizedOptimizer assigns Node.LongDistanceNeighborData only when the
    // newly computed requirements qualify; it does not clear the node field
    // when a later optimizer no longer qualifies.
    for edge in &mut arena.edge_records {
        edge.min_width = 0.0;
    }
    let persisted = {
        let optimizer = sized::SizedOptimizer::new(&mut arena, go_rng::GoRng::new(2), None);
        optimizer.test_placements(moving, Point::default(), 0.0, false)
    };
    assert!(persisted.contains(&Point { x: 200.0, y: 0.0 }));
    assert!(persisted.contains(&Point { x: 800.0, y: 0.0 }));
}

#[test]
fn prescale_keeps_unit_aspect_shapes_square() {
    let mut input = Graph::default();
    let mut square = node("square", 80.0, 120.0);
    square.shape = ShapeKind::RealSquare;
    let square = input.add_node(square);
    let mut pipeline = Pipeline::new(&input, 1, false, true);

    pipeline.run_prescale();

    assert_eq!(
        pipeline.graph.nodes[square.0 as usize].rect.size.width,
        120.0
    );
    assert_eq!(
        pipeline.graph.nodes[square.0 as usize].rect.size.height,
        120.0
    );
}

#[test]
fn sizeless_median_preserves_recovered_half_cell_bias() {
    let mut input = Graph::default();
    let center = input.add_node(node("center", 40.0, 40.0));
    let left = input.add_node(node("left", 40.0, 40.0));
    let right = input.add_node(node("right", 40.0, 40.0));
    input.add_edge(super::super::Edge {
        source: center,
        target: left,
    });
    input.add_edge(super::super::Edge {
        source: center,
        target: right,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(left, Point { x: 0.0, y: 4.0 });
    arena.set_position(right, Point { x: 2.0, y: 8.0 });

    assert_eq!(
        arena.median_to_neighbors(center),
        Point { x: 1.25, y: 6.25 }
    );
}

#[test]
fn moving_a_container_translates_all_descendants() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 100.0, 100.0));
    let mut child_node = node("child", 40.0, 40.0);
    child_node.parent = Some(parent);
    let child = input.add_node(child_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(parent, Point { x: 1.0, y: 2.0 });
    arena.set_position(child, Point { x: 3.0, y: 4.0 });

    arena.move_node_abs_with_children(parent, Point { x: 6.0, y: 8.0 });

    assert_eq!(arena.position(parent), Some(Point { x: 6.0, y: 8.0 }));
    assert_eq!(arena.position(child), Some(Point { x: 8.0, y: 10.0 }));
}

#[test]
fn induced_subgraph_preserves_an_explicitly_empty_obstruction_inventory() {
    let mut input = Graph::default();
    let source = input.add_node(node("source", 100.0, 100.0));
    let target = input.add_node(node("target", 100.0, 100.0));
    let obstruction = input.add_node(node("obstruction", 100.0, 100.0));
    let edge = input.add_edge(Edge { source, target });
    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(source, Point { x: 0.0, y: 0.0 });
    arena.set_position(target, Point { x: 400.0, y: 0.0 });
    arena.set_position(obstruction, Point { x: 200.0, y: 0.0 });
    arena.set_turn_cost(10.0);
    arena
        .sized_projected_obstructions
        .insert((source, edge), SharedProjections::default());

    let (mut subgraph, _) = arena.induced_placement_subgraph(&[source, target, obstruction]);
    subgraph.set_position(NodeId(0), Point { x: 0.0, y: 0.0 });
    subgraph.set_position(NodeId(1), Point { x: 400.0, y: 0.0 });
    subgraph.set_position(NodeId(2), Point { x: 200.0, y: 0.0 });

    assert!(
        subgraph
            .sized_projected_obstructions
            .get(&(NodeId(0), EdgeId(0)))
            .is_some_and(|projections| projections.is_empty())
    );
    assert_eq!(
        subgraph.edge_route_penalty(NodeId(0), NodeId(1), EdgeId(0), None, None, false),
        0.0
    );
}

#[test]
fn initialize_nodes_restores_positions_when_fixed_origin_search_exhausts() {
    // initialize.go snapshots all positions before clearing the fixed-node
    // component; candidate search failure must restore both present and nil.
    for fixed_position in [
        Point {
            x: 1.0,
            y: -1_000_000_000.0,
        },
        Point {
            x: -1_000_000_000.0,
            y: 1.0,
        },
    ] {
        for original_moving in [None, Some(Point { x: 250.0, y: 20.0 })] {
            let mut input = Graph::default();
            let mut fixed_node = node("fixed", 100.0, 100.0);
            fixed_node.locked_position = Some(fixed_position);
            let fixed = input.add_node(fixed_node);
            let moving = input.add_node(node("moving", 100.0, 100.0));
            input.add_edge(Edge {
                source: fixed,
                target: moving,
            });
            let mut arena = ArenaGraph::from_input(&input);
            arena.cell_size = 100.0;
            let original_fixed = Point { x: 10.0, y: 20.0 };
            arena.set_position(fixed, original_fixed);
            if let Some(position) = original_moving {
                arena.set_position(moving, position);
            }
            let error = arena.initialize_nodes(1).unwrap_err();
            assert!(matches!(error, LayoutError::NoUnoccupiedPoint { point }
            if point == Point {
                x: (fixed_position.x / 300.0).ceil(),
                y: (fixed_position.y / 300.0).ceil(),
            }));
            assert_eq!(arena.position(fixed), Some(original_fixed));
            assert_eq!(arena.position(moving), original_moving);
        }
    }
}

#[test]
fn initialize_nodes_places_every_component_deterministically() {
    let mut input = Graph::default();
    let a = input.add_node(node("a", 40.0, 40.0));
    let b = input.add_node(node("b", 40.0, 40.0));
    let c = input.add_node(node("c", 40.0, 40.0));
    input.add_edge(super::super::Edge {
        source: a,
        target: b,
    });
    let mut first = ArenaGraph::from_input(&input);
    let mut second = ArenaGraph::from_input(&input);

    first.initialize_nodes(1).unwrap();
    second.initialize_nodes(1).unwrap();

    let positions: Vec<_> = first.nodes.iter().map(|node| node.position).collect();
    assert_eq!(
        positions,
        second
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>()
    );
    assert!(positions.iter().all(Option::is_some));
    assert_eq!(positions[0], Some(Point { x: 3.0, y: 3.0 }));
    assert_ne!(positions[a.0 as usize], positions[b.0 as usize]);
    assert_ne!(positions[b.0 as usize], positions[c.0 as usize]);
}

#[test]
fn initialize_nodes_matches_recovered_ent2d2_right_boundary() {
    let input = recovered_ent2d2_right_input();

    let snapshot = layout_snapshot(&input, 1, LayoutStage::InitializeNodes);

    assert_eq!(snapshot.cell_size, 308.0);
    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 6.0, y: 6.0 },
            Point { x: 7.0, y: 6.0 },
            Point { x: 5.0, y: 6.0 },
            Point { x: 6.0, y: 5.0 },
            Point { x: 6.0, y: 7.0 },
            Point { x: 7.0, y: 5.0 },
        ]
    );
}

#[test]
fn initialize_nodes_matches_recovered_ent2d2_unset_direction_boundary() {
    let mut input = recovered_ent2d2_right_input();
    input.direction = Direction::Down;
    input.explicit_direction = None;

    let snapshot = layout_snapshot(&input, 1, LayoutStage::InitializeNodes);

    assert_eq!(snapshot.cell_size, 308.0);
    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 6.0, y: 6.0 },
            Point { x: 5.0, y: 6.0 },
            Point { x: 6.0, y: 5.0 },
            Point { x: 6.0, y: 7.0 },
            Point { x: 7.0, y: 6.0 },
            Point { x: 5.0, y: 5.0 },
        ]
    );
}

#[test]
fn sizeless_anneal_matches_recovered_ent2d2_right_boundary() {
    let input = recovered_ent2d2_right_input();

    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizelessAnneal);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 58.0, y: 42.0 },
            Point { x: 59.0, y: 43.0 },
            Point { x: 59.0, y: 42.0 },
            Point { x: 57.0, y: 42.0 },
            Point { x: 58.0, y: 43.0 },
            Point { x: 58.0, y: 41.0 },
        ]
    );
}

#[test]
fn transition_compaction_matches_recovered_ent2d2_right_boundary() {
    let input = recovered_ent2d2_right_input();

    let snapshot = layout_snapshot(&input, 1, LayoutStage::TransitionCompaction);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 1848.0,
                y: 616.0
            },
            Point { x: 1848.0, y: 42.0 },
            Point { x: 57.0, y: 42.0 },
            Point {
                x: 924.0,
                y: 1232.0,
            },
            Point { x: 924.0, y: 41.0 },
        ]
    );
}

#[test]
fn sized_start_matches_recovered_ent2d2_right_boundary() {
    let input = recovered_ent2d2_right_input();

    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedStart);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 1848.0,
                y: 616.0
            },
            Point { x: 1848.0, y: 0.0 },
            Point { x: 0.0, y: 0.0 },
            Point {
                x: 924.0,
                y: 1232.0,
            },
            Point { x: 924.0, y: 0.0 },
        ]
    );
}

#[test]
fn sized_first_node_matches_recovered_ent2d2_right_boundary() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedFirstNode);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 1848.0,
                y: 616.0
            },
            Point { x: 308.0, y: 616.0 },
            Point { x: 0.0, y: 0.0 },
            Point {
                x: 924.0,
                y: 1232.0
            },
            Point { x: 924.0, y: 0.0 },
        ]
    );
}

#[test]
fn sized_first_pass_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedFirstPass);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point { x: 924.0, y: 924.0 },
            Point { x: 308.0, y: 616.0 },
            Point {
                x: 1232.0,
                y: 1232.0,
            },
            Point {
                x: 1540.0,
                y: 616.0,
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.820_330_897_664_820_5));
}

#[test]
fn sized_second_pass_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedSecondPass);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point { x: 924.0, y: 924.0 },
            Point { x: 308.0, y: 616.0 },
            Point {
                x: 1848.0,
                y: 616.0
            },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.357_339_636_603_090_8));
}

#[test]
fn sized_pre_compaction_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedPreCompaction);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point { x: 924.0, y: 924.0 },
            Point { x: 308.0, y: 616.0 },
            Point { x: 616.0, y: 0.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.205_494_891_771_757_64));
}

#[test]
fn sized_first_compaction_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedFirstCompaction);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 924.0,
                y: 1232.0
            },
            Point { x: 308.0, y: 616.0 },
            Point {
                x: 1232.0,
                y: 924.0
            },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.554_349_242_028_687_3));
}

#[test]
fn sized_first_compaction_honors_recovered_self_loop_clearance() {
    let input = recovered_ent2d2_labeled_unset_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedFirstCompaction);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 924.0,
                y: 1232.0
            },
            Point {
                x: 1232.0,
                y: 1232.0
            },
            Point { x: 308.0, y: 616.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
}

#[test]
fn sized_second_compaction_matches_recovered_labeled_unset_boundary() {
    let input = recovered_ent2d2_labeled_unset_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedSecondCompaction);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 616.0,
                y: 1232.0
            },
            Point { x: 924.0, y: 924.0 },
            Point { x: 308.0, y: 616.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
}

#[test]
fn post_placement_swap_rejects_recovered_loop_clearance_overlaps() {
    let input = recovered_ent2d2_labeled_unset_input();
    let placed = layout_snapshot(&input, 1, LayoutStage::NodePlacement);
    let swapped = layout_snapshot(&input, 1, LayoutStage::SwapFirstPass);

    assert_eq!(
        swapped
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>(),
        placed
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>()
    );
}

#[test]
fn sized_swap_requires_spatial_adjacency_even_with_explicit_direction() {
    let mut input = Graph::with_direction(Direction::Right);
    let first = input.add_node(node("first", 20.0, 20.0));
    let second = input.add_node(node("second", 20.0, 20.0));
    input.add_edge(Edge {
        source: first,
        target: second,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 100.0;
    arena.set_position(first, Point { x: 0.0, y: 0.0 });
    arena.set_position(second, Point { x: 121.0, y: 0.0 });

    assert!(!arena.sized_swap_adjacent(first, second));

    arena.set_position(second, Point { x: 120.0, y: 0.0 });
    assert!(arena.sized_swap_adjacent(first, second));
}

#[test]
fn swap_optimize_includes_ordinary_container_nodes() {
    let mut input = Graph::default();
    let left = input.add_node(node("left", 20.0, 20.0));
    let middle = input.add_node(node("middle", 20.0, 20.0));
    let right = input.add_node(node("right", 20.0, 20.0));
    input.add_edge(Edge {
        source: left,
        target: middle,
    });
    input.add_edge(Edge {
        source: middle,
        target: right,
    });
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[middle.0 as usize].is_container = true;
    arena.set_position(left, Point { x: 0.0, y: 0.0 });
    arena.set_position(middle, Point { x: 400.0, y: 0.0 });
    arena.set_position(right, Point { x: 200.0, y: 0.0 });

    assert!(arena.swap_optimize());
    assert_eq!(arena.position(middle), Some(Point { x: 200.0, y: 0.0 }));
    assert_eq!(arena.position(right), Some(Point { x: 400.0, y: 0.0 }));
}

#[test]
fn equidistance_fixed_origin_rejects_connected_branch_move() {
    let mut input = Graph::default();
    let mut left_node = node("left", 100.0, 80.0);
    left_node.locked_position = Some(Point::default());
    let left = input.add_node(left_node);
    let center = input.add_node(node("center", 100.0, 80.0));
    let right = input.add_node(node("right", 100.0, 80.0));
    let perpendicular = input.add_node(node("perpendicular", 100.0, 80.0));
    let branch = input.add_node(node("branch", 100.0, 80.0));
    for (source, target) in [
        (left, center),
        (center, right),
        (center, perpendicular),
        (branch, perpendicular),
        // Parallel edges prevent fringe peeling without altering either
        // trial's relative edge cost. The pinned Go witness runs production
        // trees.Preprocess and confirms all five nodes remain ordinary.
        (left, center),
        (center, right),
        (branch, perpendicular),
    ] {
        input.add_edge(Edge { source, target });
    }
    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 20.0;
    assert!(arena.tree_routing_nodes.is_empty());
    assert!(!arena.is_tree_sentinel(center));
    for (id, position) in [
        (left, Point::default()),
        (center, Point { x: 600.0, y: 0.0 }),
        (right, Point { x: 1_000.0, y: 0.0 }),
        (perpendicular, Point { x: 600.0, y: 200.0 }),
        (branch, Point { x: 0.0, y: 200.0 }),
    ] {
        arena.set_position(id, position);
    }
    assert!(arena.transaction_fixed_origin_is_valid());

    // The horizontal midpoint asks for a -100 translation. Both trials have
    // equal edge cost: the solo move keeps vertical box alignment, while the
    // connected move preserves it and sends the branch past the fixed origin.
    // Go prefers the connected trial on a tie unless its transaction rejects.
    let mut unfixed = arena.clone();
    unfixed.nodes[left.0 as usize].fixed_top_left = None;
    assert!(unfixed.equidistance_node(center, true));
    assert_eq!(unfixed.position(center), Some(Point { x: 500.0, y: 0.0 }));
    assert_eq!(
        unfixed.position(perpendicular),
        Some(Point { x: 500.0, y: 200.0 })
    );
    assert_eq!(
        unfixed.position(branch),
        Some(Point {
            x: -100.0,
            y: 200.0
        })
    );

    assert!(arena.equidistance_node(center, true));
    assert_eq!(arena.position(left), Some(Point::default()));
    assert_eq!(arena.position(center), Some(Point { x: 500.0, y: 0.0 }));
    assert_eq!(
        arena.position(perpendicular),
        Some(Point { x: 600.0, y: 200.0 })
    );
    assert_eq!(arena.position(branch), Some(Point { x: 0.0, y: 200.0 }));
}

#[test]
fn swap_fixed_origin_rejects_label_refit_before_container_crosses_origin() {
    let mut input = Graph::default();
    let container = input.add_node(node("container", 1_160.0, 200.0));
    let mut fixed_peer_node = node("fixed-peer", 100.0, 80.0);
    fixed_peer_node.locked_position = Some(Point { x: 0.0, y: 1_000.0 });
    let fixed_peer = input.add_node(fixed_peer_node);
    let mut wide_label_node = node("wide-label", 100.0, 80.0);
    wide_label_node.parent = Some(container);
    wide_label_node.label_size = Some(Size {
        width: 800.0,
        height: 20.0,
    });
    wide_label_node.label_position = LabelPosition::InsideMiddleCenter;
    let wide_label = input.add_node(wide_label_node);
    let mut left_node = node("left", 100.0, 80.0);
    left_node.parent = Some(container);
    let left = input.add_node(left_node);
    let mut right_node = node("right", 100.0, 80.0);
    right_node.parent = Some(container);
    let right = input.add_node(right_node);
    input.add_edge(Edge {
        source: wide_label,
        target: fixed_peer,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.cell_size = 20.0;
    for (id, position) in [
        (container, Point::default()),
        (fixed_peer, Point { x: 0.0, y: 1_000.0 }),
        (wide_label, Point { x: 400.0, y: 60.0 }),
        (left, Point { x: 60.0, y: 60.0 }),
        (
            right,
            Point {
                x: 1_000.0,
                y: 60.0,
            },
        ),
    ] {
        arena.set_position(id, position);
    }
    // The pinned Go no-op AffectContainers transaction accepts this exact
    // baseline and leaves its fitted container geometry unchanged.
    arena.reposition_ordinary_containers();
    assert_eq!(arena.position(container), Some(Point::default()));
    assert_eq!(
        arena.nodes[container.0 as usize].rect.size,
        Size {
            width: 1_160.0,
            height: 200.0
        }
    );
    assert!(arena.transaction_containment_is_valid());
    assert!(arena.transaction_fixed_origin_is_valid());
    let before = arena.clone();
    let existing_overlaps = arena.existing_overlap_pairs();
    let existing_exact_overlaps = arena.exact_overlap_pairs();

    // Moving the wide-label child from interior to left boundary changes
    // expandForLabels, so refitting moves its container behind the root
    // origin. The child boxes and fixed peer themselves remain nonnegative.
    let mut unfixed = arena.clone();
    unfixed.nodes[fixed_peer.0 as usize].fixed_top_left = None;
    for smart in [false, true] {
        assert!(
            unfixed
                .evaluate_swap_trial(
                    wide_label,
                    left,
                    smart,
                    &existing_overlaps,
                    &existing_exact_overlaps
                )
                .is_some()
        );
    }
    let mut crossed = unfixed.clone();
    crossed.swap_positions(wide_label, left);
    crossed.reposition_ordinary_containers();
    assert_eq!(
        crossed.position(container),
        Some(Point { x: -350.0, y: 0.0 })
    );
    assert_eq!(
        crossed.nodes[container.0 as usize].rect.size,
        Size {
            width: 1_510.0,
            height: 200.0
        }
    );
    assert_eq!(
        crossed.position(wide_label),
        Some(Point { x: 60.0, y: 60.0 })
    );
    assert_eq!(crossed.position(left), Some(Point { x: 400.0, y: 60.0 }));
    assert_eq!(crossed.position(fixed_peer), before.position(fixed_peer));
    crossed.nodes[fixed_peer.0 as usize].fixed_top_left =
        before.nodes[fixed_peer.0 as usize].fixed_top_left;
    assert!(!crossed.transaction_fixed_origin_is_valid());

    for smart in [false, true] {
        assert!(
            arena
                .evaluate_swap_trial(
                    wide_label,
                    left,
                    smart,
                    &existing_overlaps,
                    &existing_exact_overlaps
                )
                .is_none()
        );
        for id in [container, fixed_peer, wide_label, left, right] {
            assert_eq!(arena.position(id), before.position(id));
            assert_eq!(
                arena.nodes[id.0 as usize].rect.size,
                before.nodes[id.0 as usize].rect.size
            );
        }
    }
}

#[test]
fn swap_trials_reuse_transaction_overlap_baseline_after_accepted_swap() {
    let mut input = Graph::default();
    let left = input.add_node(node("left", 100.0, 60.0));
    let middle = input.add_node(node("middle", 100.0, 60.0));
    let right = input.add_node(node("right", 100.0, 60.0));

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(left, Point { x: 0.0, y: 0.0 });
    arena.set_position(middle, Point { x: 110.0, y: 0.0 });
    arena.set_position(right, Point { x: 400.0, y: 0.0 });
    let transaction_overlaps = arena.existing_overlap_pairs();
    let transaction_exact_overlaps = arena.exact_overlap_pairs();
    assert!(transaction_overlaps.contains(&(left, middle)));
    assert!(!transaction_exact_overlaps.contains(&(left, middle)));

    // Model a swap accepted earlier in the same SwapOptimize invocation.
    // Transaction.UpdateState makes this the new rollback geometry, while
    // the transaction's original overlap maps remain unchanged.
    arena.set_position(left, Point { x: 400.0, y: 0.0 });
    arena.set_position(right, Point { x: 0.0, y: 0.0 });
    let per_trial_overlaps = arena.existing_overlap_pairs();
    let per_trial_exact_overlaps = arena.exact_overlap_pairs();
    assert!(per_trial_overlaps.contains(&(middle, right)));
    assert!(!per_trial_overlaps.contains(&(left, middle)));

    // Recomputing the maps for this trial rejects swapping left and right
    // because it treats the restored left/middle spacing overlap as new.
    assert!(
        arena
            .evaluate_swap_trial(
                left,
                right,
                false,
                &per_trial_overlaps,
                &per_trial_exact_overlaps,
            )
            .is_none()
    );

    // Reusing NewTransactionWithOptions' original maps accepts the same
    // trial, matching recovered Transaction.UpdateState semantics.
    assert!(
        arena
            .evaluate_swap_trial(
                left,
                right,
                false,
                &transaction_overlaps,
                &transaction_exact_overlaps,
            )
            .is_some()
    );
}

#[test]
fn gap_normalization_uses_recovered_pair_clearance_for_between_nodes() {
    let input = recovered_ent2d2_labeled_unset_input();
    let aligned = layout_snapshot(&input, 1, LayoutStage::AlignAxes);
    let normalized = layout_snapshot(&input, 1, LayoutStage::GapNormalization);
    let aligned_positions: Vec<_> = aligned
        .nodes
        .iter()
        .map(|node| node.position.unwrap())
        .collect();
    let normalized_positions: Vec<_> = normalized
        .nodes
        .iter()
        .map(|node| node.position.unwrap())
        .collect();

    // Pristine TALA's vertical forward pass moves Card upward and Info
    // downward around User. Post's later trial must roll back: Info is 54
    // units left of Post, outside their recovered 20-unit corridor, and
    // therefore is not a replacement gap boundary.
    assert_eq!(normalized_positions[2].y, aligned_positions[2].y - 14.0);
    assert_eq!(normalized_positions[5].y, aligned_positions[5].y + 50.0);
    assert_eq!(normalized_positions[3].y, aligned_positions[3].y);
    assert_eq!(normalized_positions[0].y, aligned_positions[0].y);
    assert_eq!(normalized_positions[1].y, aligned_positions[1].y);
    assert_eq!(normalized_positions[4].y, aligned_positions[4].y);
}

#[test]
fn routing_flavors_reserve_shared_ports_for_harder_edges() {
    let input = recovered_ent2d2_labeled_unset_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::Normalize);

    assert_eq!(
        snapshot.edges[2].points,
        vec![Point { x: 497.0, y: 730.0 }, Point { x: 912.0, y: 730.0 },]
    );
    assert_eq!(
        snapshot.edges[4].points,
        vec![
            Point { x: 497.0, y: 700.0 },
            Point { x: 715.0, y: 700.0 },
            Point { x: 715.0, y: 144.0 },
        ]
    );
    assert_eq!(
        snapshot.edges[5].points,
        vec![Point { x: 296.0, y: 715.0 }, Point { x: 146.0, y: 715.0 },]
    );
    assert_eq!(
        snapshot.edges[2].label.as_ref().unwrap().position,
        LabelPosition::OutsideBottomCenter
    );
    assert_eq!(
        snapshot.edges[5].label.as_ref().unwrap().position,
        LabelPosition::OutsideTopCenter
    );
}

#[test]
fn dejitter_allows_proposed_segments_inside_endpoint_ancestors() {
    let mut input = Graph::default();
    let outer = input.add_node(node("outer", 1_000.0, 1_000.0));
    let mut inner_node = node("inner", 800.0, 800.0);
    inner_node.parent = Some(outer);
    let inner = input.add_node(inner_node);
    let mut endpoint_node = node("endpoint", 40.0, 40.0);
    endpoint_node.parent = Some(inner);
    let endpoint = input.add_node(endpoint_node);
    let mut adjacent_node = node("adjacent", 40.0, 40.0);
    adjacent_node.parent = Some(inner);
    let adjacent = input.add_node(adjacent_node);
    let edge = input.add_edge(Edge {
        source: endpoint,
        target: adjacent,
    });

    let mut arena = ArenaGraph::from_input(&input);
    arena.set_position(outer, Point { x: 0.0, y: 0.0 });
    arena.set_position(inner, Point { x: 100.0, y: 100.0 });
    arena.set_position(endpoint, Point { x: 280.0, y: 330.0 });
    arena.set_position(adjacent, Point { x: 242.0, y: 110.0 });
    arena.edge_records[edge.0 as usize]
        .route
        .replace(RoutePoints::literal(
            &[Point { x: 300.0, y: 350.0 },
                Point { x: 300.0, y: 250.0 },
                Point { x: 262.0, y: 250.0 },
                Point { x: 262.0, y: 150.0 }],
        ));

    assert!(routing::dejitter(&mut arena));
    assert_eq!(arena.position(endpoint), Some(Point { x: 242.0, y: 330.0 }));
    assert_eq!(
        arena.edge_records[edge.0 as usize].route.read(),
        vec![Point { x: 262.0, y: 350.0 }, Point { x: 262.0, y: 150.0 },]
    );
}

#[test]
fn sized_second_compaction_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedSecondCompaction);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 2156.0,
                y: 616.0
            },
            Point { x: 308.0, y: 616.0 },
            Point { x: 924.0, y: 924.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.286_381_400_232_981_8));
}

#[test]
fn sized_anneal_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedAnneal);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 1848.0,
                y: 616.0
            },
            Point { x: 308.0, y: 616.0 },
            Point { x: 924.0, y: 924.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.754_048_363_010_997_4));
}

#[test]
fn sized_anneal_preserves_non_nil_empty_abductions_for_seed_two() {
    let input = recovered_ent2d2_labeled_unset_input();
    let snapshot = layout_snapshot(&input, 2, LayoutStage::SizedAnneal);

    // Pristine ARM64 trace:
    // analysis/gdb_ent2d2_seed2_placement_trace.log. The seventh sized
    // pass attempts transpose for every node, but no cell-rounded local
    // edge score improves on the 2663.250199701 baseline.
    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point { x: 924.0, y: 308.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 924.0 },
            Point { x: 616.0, y: 616.0 },
            Point {
                x: 924.0,
                y: 1232.0
            },
        ]
    );
}

#[test]
fn sized_zero_optimize_matches_recovered_ent2d2_right_boundary_and_rng() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::SizedZeroOptimize);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 924.0, y: 616.0 },
            Point {
                x: 1848.0,
                y: 616.0
            },
            Point { x: 308.0, y: 616.0 },
            Point { x: 924.0, y: 924.0 },
            Point {
                x: 1540.0,
                y: 616.0
            },
            Point { x: 924.0, y: 308.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.570_258_585_120_084_9));
}

#[test]
fn node_placement_matches_recovered_ent2d2_right_boundary() {
    let input = recovered_ent2d2_right_input();
    let snapshot = layout_snapshot(&input, 1, LayoutStage::NodePlacement);

    assert_eq!(
        snapshot
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 616.0, y: 308.0 },
            Point {
                x: 1540.0,
                y: 308.0
            },
            Point { x: 0.0, y: 308.0 },
            Point { x: 616.0, y: 616.0 },
            Point {
                x: 1232.0,
                y: 308.0
            },
            Point { x: 616.0, y: 0.0 },
        ]
    );
    assert_eq!(snapshot.next_rng_float, Some(0.570_258_585_120_084_9));
}

#[test]
#[ignore = "historical recovered expectation; OSS TALA parity corpus supersedes it"]
fn swap_passes_match_recovered_ent2d2_seed_three_boundaries() {
    let input = recovered_ent2d2_labeled_unset_input();
    let positions = |stage| {
        layout_snapshot(&input, 3, stage)
            .nodes
            .into_iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        positions(LayoutStage::SwapFirstPass),
        vec![
            Point { x: 616.0, y: 308.0 },
            Point {
                x: 1232.0,
                y: 308.0,
            },
            Point { x: 616.0, y: 0.0 },
            Point { x: 0.0, y: 308.0 },
            Point { x: 616.0, y: 616.0 },
            Point { x: 308.0, y: 924.0 },
        ]
    );
    assert_eq!(
        positions(LayoutStage::SwapSecondPass),
        vec![
            Point { x: 616.0, y: 308.0 },
            Point {
                x: 1232.0,
                y: 308.0,
            },
            Point { x: 0.0, y: 308.0 },
            Point { x: 616.0, y: 0.0 },
            Point { x: 616.0, y: 616.0 },
            Point { x: 308.0, y: 924.0 },
        ]
    );
    assert_eq!(
        positions(LayoutStage::SwapThirdPass),
        vec![
            Point { x: 616.0, y: 308.0 },
            Point { x: 616.0, y: 616.0 },
            Point { x: 0.0, y: 308.0 },
            Point { x: 616.0, y: 0.0 },
            Point {
                x: 1232.0,
                y: 308.0,
            },
            Point { x: 308.0, y: 924.0 },
        ]
    );
}

#[test]
fn swap_passes_match_recovered_ent2d2_right_boundaries() {
    let input = recovered_ent2d2_right_input();
    let positions = |stage| {
        layout_snapshot(&input, 1, stage)
            .nodes
            .into_iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        positions(LayoutStage::SwapFirstPass),
        vec![
            Point { x: 616.0, y: 308.0 },
            Point { x: 616.0, y: 0.0 },
            Point {
                x: 1540.0,
                y: 308.0
            },
            Point {
                x: 1156.0,
                y: 308.0
            },
            Point { x: 616.0, y: 652.0 },
            Point { x: 0.0, y: 308.0 },
        ]
    );
    assert_eq!(
        positions(LayoutStage::SwapSecondPass),
        vec![
            Point { x: 616.0, y: 308.0 },
            Point { x: 616.0, y: 0.0 },
            Point { x: 616.0, y: 652.0 },
            Point {
                x: 1156.0,
                y: 308.0
            },
            Point {
                x: 1540.0,
                y: 308.0
            },
            Point { x: 0.0, y: 308.0 },
        ]
    );
    let final_positions = vec![
        Point { x: 616.0, y: 308.0 },
        Point {
            x: 1156.0,
            y: 308.0,
        },
        Point { x: 616.0, y: 652.0 },
        Point { x: 616.0, y: 0.0 },
        Point {
            x: 1540.0,
            y: 308.0,
        },
        Point { x: 0.0, y: 308.0 },
    ];
    assert_eq!(positions(LayoutStage::SwapThirdPass), final_positions);
    assert_eq!(positions(LayoutStage::SwapStuff), final_positions);
    assert_eq!(positions(LayoutStage::Transpose), final_positions);
    assert_eq!(
        positions(LayoutStage::AlignAxes),
        vec![
            Point { x: 623.0, y: 272.0 },
            Point {
                x: 1163.0,
                y: 290.0
            },
            Point { x: 627.0, y: 616.0 },
            Point { x: 612.0, y: -36.0 },
            Point {
                x: 1547.0,
                y: 272.0
            },
            Point { x: 7.0, y: 290.0 },
        ]
    );
    let normalized_positions = vec![
        Point { x: 852.0, y: 272.0 },
        Point {
            x: 1203.0,
            y: 290.0,
        },
        Point { x: 856.0, y: 566.0 },
        Point { x: 841.0, y: -22.0 },
        Point {
            x: 1547.0,
            y: 272.0,
        },
        Point { x: 394.0, y: 290.0 },
    ];
    assert_eq!(
        positions(LayoutStage::GapNormalization),
        normalized_positions
    );
    assert_eq!(
        positions(LayoutStage::BalanceSymmetry),
        normalized_positions.clone()
    );
    assert_eq!(
        positions(LayoutStage::Equidistance),
        normalized_positions.clone()
    );
    assert_eq!(
        positions(LayoutStage::FirstBinPack),
        normalized_positions.clone()
    );
    assert_eq!(
        positions(LayoutStage::Rescale),
        normalized_positions
            .into_iter()
            .map(|position| Point {
                x: position.x + 1000.0,
                y: position.y + 1000.0,
            })
            .collect::<Vec<_>>()
    );
    let edge_snapshot = layout_snapshot(&input, 1, LayoutStage::EdgeRouting);
    // Pristine Linux ARM64 EdgeRoutingStage capture enters with an empty
    // lazy turn-cost cache. Graph.getMaxLength then observes the final
    // 494-unit User-to-Metadata box gap: 0.125 * 7 * 494 = 432.25.
    assert_eq!(edge_snapshot.turn_cost, 432.25);
    assert_eq!(
        edge_snapshot
            .edges
            .into_iter()
            .map(|edge| edge.points)
            .collect::<Vec<_>>(),
        vec![
            vec![
                Point {
                    x: 1852.0,
                    y: 1326.0
                },
                Point {
                    x: 1822.0,
                    y: 1326.0
                },
                Point {
                    x: 1822.0,
                    y: 1242.0
                },
                Point {
                    x: 1902.0,
                    y: 1242.0
                },
                Point {
                    x: 1902.0,
                    y: 1272.0
                },
            ],
            vec![
                Point {
                    x: 2003.0,
                    y: 1272.0
                },
                Point {
                    x: 2003.0,
                    y: 1242.0
                },
                Point {
                    x: 2083.0,
                    y: 1242.0
                },
                Point {
                    x: 2083.0,
                    y: 1326.0
                },
                Point {
                    x: 2053.0,
                    y: 1326.0
                },
            ],
            vec![
                Point {
                    x: 2053.0,
                    y: 1362.0
                },
                Point {
                    x: 2128.0,
                    y: 1362.0
                },
                Point {
                    x: 2128.0,
                    y: 1344.0
                },
                Point {
                    x: 2203.0,
                    y: 1344.0
                },
            ],
            vec![
                Point {
                    x: 1953.0,
                    y: 1416.0
                },
                Point {
                    x: 1953.0,
                    y: 1566.0
                }
            ],
            vec![
                Point {
                    x: 1953.0,
                    y: 1272.0
                },
                Point {
                    x: 1953.0,
                    y: 1197.0
                },
                Point {
                    x: 1952.0,
                    y: 1197.0
                },
                Point {
                    x: 1952.0,
                    y: 1122.0
                },
            ],
            vec![
                Point {
                    x: 2003.0,
                    y: 1416.0
                },
                Point {
                    x: 2003.0,
                    y: 1516.0
                },
                Point {
                    x: 2584.0,
                    y: 1516.0
                },
                Point {
                    x: 2584.0,
                    y: 1380.0
                },
            ],
            vec![
                Point {
                    x: 1852.0,
                    y: 1362.0
                },
                Point {
                    x: 1777.0,
                    y: 1362.0
                },
                Point {
                    x: 1777.0,
                    y: 1344.0
                },
                Point {
                    x: 1702.0,
                    y: 1344.0
                },
            ],
        ]
    );

    let routed = layout_snapshot(&input, 1, LayoutStage::EdgeRouting);
    let crosshatched = layout_snapshot(&input, 1, LayoutStage::Crosshatch);
    assert_eq!(crosshatched.nodes, routed.nodes);
    assert_eq!(crosshatched.edges, routed.edges);

    let dejittered = layout_snapshot(&input, 1, LayoutStage::Dejitter);
    assert!(dejittered.run_edge_route);
    assert_eq!(
        dejittered
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point {
                x: 1852.0,
                y: 1272.0
            },
            Point {
                x: 2203.0,
                y: 1308.0
            },
            Point {
                x: 1856.0,
                y: 1566.0
            },
            Point {
                x: 1842.0,
                y: 978.0
            },
            Point {
                x: 2547.0,
                y: 1272.0
            },
            Point {
                x: 1394.0,
                y: 1308.0
            },
        ]
    );
    assert_eq!(dejittered.edges[2].points.len(), 2);
    assert_eq!(dejittered.edges[4].points.len(), 2);
    assert_eq!(dejittered.edges[6].points.len(), 2);

    let rerouted = layout_snapshot(&input, 1, LayoutStage::SecondEdgeRouting);
    assert_eq!(rerouted.nodes, dejittered.nodes);
    assert_eq!(rerouted.edges, dejittered.edges);
    let simplified = layout_snapshot(&input, 1, LayoutStage::SimplifyEdgeRoutes);
    assert_eq!(simplified.nodes, rerouted.nodes);
    assert_eq!(simplified.edges, rerouted.edges);
    let swapped_ports = layout_snapshot(&input, 1, LayoutStage::SwapEdgePorts);
    assert_eq!(swapped_ports.nodes, simplified.nodes);
    assert_eq!(swapped_ports.edges, simplified.edges);
    let straight_fallback = layout_snapshot(&input, 1, LayoutStage::StraightEdgesFallback);
    assert_eq!(straight_fallback.nodes, swapped_ports.nodes);
    assert_eq!(straight_fallback.edges, swapped_ports.edges);
    let balanced = layout_snapshot(&input, 1, LayoutStage::BalanceEdgeSegments);
    assert_eq!(balanced.nodes, straight_fallback.nodes);
    assert_eq!(
        balanced.edges[2].points,
        vec![
            Point {
                x: 2053.0,
                y: 1371.0
            },
            Point {
                x: 2203.0,
                y: 1371.0
            }
        ]
    );
    assert_eq!(
        balanced.edges[4].points,
        vec![
            Point {
                x: 1952.0,
                y: 1272.0
            },
            Point {
                x: 1952.0,
                y: 1122.0
            }
        ]
    );
    assert_eq!(
        balanced.edges[5].points,
        vec![
            Point {
                x: 2003.0,
                y: 1416.0
            },
            Point {
                x: 2003.0,
                y: 1491.0
            },
            Point {
                x: 2620.0,
                y: 1491.0
            },
            Point {
                x: 2620.0,
                y: 1380.0
            },
        ]
    );
    assert_eq!(
        balanced.edges[6].points,
        vec![
            Point {
                x: 1852.0,
                y: 1371.0
            },
            Point {
                x: 1702.0,
                y: 1371.0
            }
        ]
    );
    let fixed_cluster = layout_snapshot(&input, 1, LayoutStage::FixClusterEdgeBranching);
    assert_eq!(fixed_cluster.nodes, balanced.nodes);
    assert_eq!(fixed_cluster.edges, balanced.edges);
    let traced = layout_snapshot(&input, 1, LayoutStage::TraceEdgesToShapeBorder);
    assert_eq!(traced.nodes, fixed_cluster.nodes);
    assert_eq!(traced.edges, fixed_cluster.edges);

    let reordered = layout_snapshot(&input, 1, LayoutStage::ReorderDuplicates);
    assert_eq!(reordered.nodes, traced.nodes);
    assert_eq!(reordered.edges, traced.edges);
    let labelled = layout_snapshot(&input, 1, LayoutStage::PlaceLabels);
    assert_eq!(labelled.nodes, reordered.nodes);
    assert_eq!(labelled.edges, reordered.edges);

    let normalized = layout_snapshot(&input, 1, LayoutStage::Normalize);
    assert_eq!(
        normalized
            .nodes
            .iter()
            .map(|node| node.position.unwrap())
            .collect::<Vec<_>>(),
        vec![
            Point { x: 458.0, y: 294.0 },
            Point { x: 809.0, y: 330.0 },
            Point { x: 462.0, y: 588.0 },
            Point { x: 448.0, y: 0.0 },
            Point {
                x: 1153.0,
                y: 294.0
            },
            Point { x: 0.0, y: 330.0 },
        ]
    );
    for (normalized_edge, labelled_edge) in normalized.edges.iter().zip(&labelled.edges) {
        assert_eq!(
            normalized_edge.points,
            labelled_edge
                .points
                .iter()
                .map(|point| Point {
                    x: point.x - 1394.0,
                    y: point.y - 978.0,
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn locked_constraints_are_unpositioned_until_layout_but_preserved_for_routing() {
    // D2 v0.9.0 adapter.go assigns FixedTopLeft while NewNode leaves TopLeft nil.
    let mut input = Graph::default();
    let mut anchor = node("anchor", 96.0, 66.0);
    let fixed = Point { x: 520.0, y: 140.0 };
    anchor.locked_position = Some(fixed);
    let id = input.add_node(anchor);
    let mut layout = Pipeline::new(&input, 1, false, false);
    assert_eq!(
        layout.graph.nodes[id.0 as usize].fixed_top_left,
        Some(fixed)
    );
    assert_eq!(layout.graph.active_node_position(id), None);
    layout.run_prescale();
    assert_eq!(layout.graph.active_node_position(id), None);
    let routing = Pipeline::new(&input, 1, true, true);
    assert_eq!(routing.graph.active_node_position(id), Some(fixed));
}

#[test]
fn fixed_anchor_translation_moves_external_scope_children() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", 360.0, 300.0));
    let mut anchor_node = node("anchor", 98.0, 66.0);
    anchor_node.locked_position = Some(Point { x: 600.0, y: 80.0 });
    let anchor = input.add_node(anchor_node);
    let mut arena = ArenaGraph::from_input(&input);
    arena.nodes[parent.0 as usize].is_container = true;
    arena.set_position(parent, Point { x: 297.0, y: 0.0 });
    arena.set_position(anchor, Point { x: 3.0, y: 1.0 });
    let parent_tala_id = arena.nodes[parent.0 as usize].tala_id;
    let mut child = arena.nodes[parent.0 as usize].clone();
    child.input_id = NodeId(2);
    child.tala_id = 123456789;
    child.is_container = false;
    child.container = Some(parent);
    child.position = Some(Point { x: 477.0, y: 170.0 });
    arena
        .transaction_external_container_children
        .insert(parent_tala_id, vec![child]);

    arena.anchor_fixed_positions(true);
    assert_eq!(arena.position(anchor), Some(Point { x: 600.0, y: 80.0 }));
    assert_eq!(arena.position(parent), Some(Point { x: 894.0, y: 79.0 }));
    assert_eq!(
        arena.transaction_external_container_children[&parent_tala_id][0].position,
        Some(Point {
            x: 1074.0,
            y: 249.0
        })
    );
}

#[test]
fn hierarchy_restoration_reorders_only_the_restored_endpoint() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", 50.0, 50.0));
    let second = input.add_node(node("second", 50.0, 50.0));
    let third = input.add_node(node("third", 50.0, 50.0));
    let restored = input.add_edge(Edge {
        source: first,
        target: second,
    });
    let local = input.add_edge(Edge {
        source: first,
        target: third,
    });
    let peer = input.add_edge(Edge {
        source: second,
        target: third,
    });
    let mut graph = ArenaGraph::from_input(&input);
    graph
        .restored_hierarchy_endpoint_order
        .push((first, restored));
    graph.sync_edge_adjacency_order();
    assert_eq!(graph.nodes[first.0 as usize].edges, vec![local, restored]);
    assert_eq!(graph.nodes[second.0 as usize].edges, vec![restored, peer]);
}

fn assign_test_parent_herds(pipeline: &mut Pipeline, root: NodeId) {
    let parent = pipeline.graph.nodes[root.0 as usize].container;
    let (_, prepared) = pipeline
        .prepare_placement_scope_after(parent, &BTreeSet::new())
        .unwrap()
        .unwrap();
    let prepared = prepared.unwrap();
    pipeline
        .assign_herds_for_scope(root, OwnerGraphId(0), &prepared.abductions)
        .unwrap();
    pipeline.restore_prepared_owner_scope(&prepared).unwrap();
}

#[test]
fn unpositioned_node_constructor_equals_one_node_arena_construction() {
    for (size, tala_id) in [
        (Size::default(), 7_i64),
        (
            Size {
                width: 128.0,
                height: 276.0,
            },
            -42,
        ),
    ] {
        let mut input = Graph::default();
        let local = input.add_node(Node::new(String::new(), size));
        input.tala_id_overrides[local.0 as usize] = Some(tala_id);
        let expected = ArenaGraph::from_input(&input).nodes.remove(0);
        let actual = ArenaNode::new_unpositioned(size, tala_id);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        assert_eq!(actual.nears.is_nil(), expected.nears.is_nil());
    }
}
