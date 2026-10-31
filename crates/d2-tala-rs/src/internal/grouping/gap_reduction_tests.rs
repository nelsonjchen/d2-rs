// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use crate::{ContentAlignment, Edge, Insets, LabelPosition, Node};

fn node(name: &str, shape: ShapeKind) -> Node {
    Node {
        d2_near_key: None,
        external_id: name.to_owned(),
        size: Size {
            width: 100.0,
            height: 80.0,
        },
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
        person: shape == ShapeKind::Person,
        is_3d: false,
        is_multiple: false,
        shape,
    }
}

#[test]
fn cluster_gap_fixed_origin_rejects_connected_branch_move() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", ShapeKind::Rectangle));
    let second = input.add_node(node("second", ShapeKind::Rectangle));
    let ahead = input.add_node(node("ahead", ShapeKind::Rectangle));
    let branch = input.add_node(node("branch", ShapeKind::Rectangle));
    let mut peer_node = node("fixed-peer", ShapeKind::Rectangle);
    peer_node.locked_position = Some(Point { x: 0.0, y: 1_000.0 });
    let peer = input.add_node(peer_node);
    // Equal-size cluster members have identical connections. Parallel edges
    // leave the ordinary topology unpeeled by eager tree reconstruction.
    for source in [first, first, second, second, branch, branch] {
        input.add_edge(Edge {
            source,
            target: ahead,
        });
    }
    let mut graph = ArenaGraph::from_input(&input);
    assert!(graph.tree_routing_nodes.is_empty());
    graph.cell_size = 20.0;
    for member in [first, second] {
        graph.nodes[member.0 as usize].cluster = Some(0);
    }
    graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Row,
        desired_arrangement: ClusterArrangement::Row,
        padding: 60.0,
        vessel_tala_id: 6,
        fixed_size: false,
    });
    graph.node_order.retain(|id| *id != second);
    graph.rebuild_active_aggregate_node_order();
    for (id, position) in [
        (first, Point::default()),
        (second, Point { x: 160.0, y: 0.0 }),
        (ahead, Point { x: 600.0, y: 0.0 }),
        (branch, Point { x: 0.0, y: 200.0 }),
        (peer, Point { x: 0.0, y: 1_000.0 }),
    ] {
        graph.set_position(id, position);
    }
    let baseline_positions = graph
        .nodes
        .iter()
        .map(|node| node.position)
        .collect::<Vec<_>>();
    graph.sync_clusters();
    graph.reposition_ordinary_containers();
    assert_eq!(
        graph
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>(),
        baseline_positions
    );
    assert_eq!(
        graph.active_node_size(first),
        Size {
            width: 260.0,
            height: 80.0
        }
    );
    assert!(graph.transaction_containment_is_valid());
    assert!(graph.transaction_fixed_origin_is_valid());
    assert!(graph.existing_overlap_pairs().is_empty());

    // The vessel stays at the origin. Shrinking its 340-unit gap to 150
    // translates the connected ahead/branch side by -190. The fixed peer
    // never moves, but Go rejects the branch crossing its root origin.
    let mut unfixed = graph.clone();
    unfixed.nodes[peer.0 as usize].fixed_top_left = None;
    assert!(unfixed.reduce_cluster_gap_to_neighbors(0, true, true, false));
    assert_eq!(unfixed.position(ahead), Some(Point { x: 410.0, y: 0.0 }));
    assert_eq!(
        unfixed.position(branch),
        Some(Point {
            x: -190.0,
            y: 200.0
        })
    );
    assert_eq!(unfixed.position(first), Some(Point::default()));
    assert_eq!(unfixed.position(second), Some(Point { x: 160.0, y: 0.0 }));
    assert_eq!(unfixed.position(peer), graph.position(peer));
    let mut crossed = unfixed.clone();
    crossed.nodes[peer.0 as usize].fixed_top_left = graph.nodes[peer.0 as usize].fixed_top_left;
    assert!(!crossed.transaction_fixed_origin_is_valid());

    assert!(!graph.reduce_cluster_gap_to_neighbors(0, true, true, false));
    assert_eq!(
        graph
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>(),
        baseline_positions
    );
}

#[test]
fn recovered_inner_boxes_include_every_non_base_shape_formula() {
    let cases = [
        (
            ShapeKind::Circle,
            Size {
                width: 100.0,
                height: 100.0,
            },
            Rect {
                origin: Point { x: 25.0, y: 35.0 },
                size: Size {
                    width: 70.0,
                    height: 70.0,
                },
            },
        ),
        (
            ShapeKind::Queue,
            Size {
                width: 40.0,
                height: 80.0,
            },
            Rect {
                origin: Point { x: 30.0, y: 20.0 },
                size: Size {
                    width: -20.0,
                    height: 80.0,
                },
            },
        ),
        (
            ShapeKind::Cylinder,
            Size {
                width: 100.0,
                height: 40.0,
            },
            Rect {
                origin: Point { x: 10.0, y: 60.0 },
                size: Size {
                    width: 100.0,
                    height: -20.0,
                },
            },
        ),
        (
            ShapeKind::Document,
            Size {
                width: 100.0,
                height: 100.0,
            },
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 100.0,
                    height: 74.0,
                },
            },
        ),
        (
            ShapeKind::Page,
            Size {
                width: 100.0,
                height: 50.0,
            },
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 79.0,
                    height: 50.0,
                },
            },
        ),
        (
            ShapeKind::Callout,
            Size {
                width: 100.0,
                height: 80.0,
            },
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 100.0,
                    height: 40.0,
                },
            },
        ),
        (
            ShapeKind::Person,
            Size {
                width: 100.0,
                height: 80.0,
            },
            Rect {
                origin: Point { x: 40.0, y: 20.0 },
                size: Size {
                    width: 41.0,
                    height: 80.0,
                },
            },
        ),
        (
            ShapeKind::C4Person,
            Size {
                width: 100.0,
                height: 100.0,
            },
            Rect {
                origin: Point { x: 15.0, y: 63.0 },
                size: Size {
                    width: 90.0,
                    height: 54.0,
                },
            },
        ),
    ];

    for (index, (shape, size, expected)) in cases.into_iter().enumerate() {
        let mut input = Graph::default();
        let id = input.add_node(node(&format!("shape-{index}"), shape));
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[id.0 as usize].rect.size = size;
        graph.set_position(id, Point { x: 10.0, y: 20.0 });
        assert_eq!(graph.shape_inner_box(id), Some(expected), "{shape:?}");
    }
}

#[test]
fn recovered_container_padding_considers_child_margins_and_icons() {
    let mut input = Graph::default();
    let mut parent_node = node("parent", ShapeKind::Rectangle);
    parent_node.content_insets = Insets {
        top: 70.0,
        right: 80.0,
        bottom: 90.0,
        left: 100.0,
    };
    let parent = input.add_node(parent_node);
    let mut child_node = node("child", ShapeKind::Rectangle);
    child_node.parent = Some(parent);
    child_node.layout_margins = Insets {
        top: 11.0,
        right: 12.0,
        bottom: 13.0,
        left: 14.0,
    };
    child_node.icon_position = Some(LabelPosition::InsideTopLeft);
    input.add_node(child_node);
    let mut graph = ArenaGraph::from_input(&input);
    graph.nodes[parent.0 as usize].node_padding = Insets {
        top: 70.0,
        right: 80.0,
        bottom: 90.0,
        left: 100.0,
    };

    assert_eq!(
        graph.shape_fit_padding_with_children(parent, true),
        Insets {
            top: 81.0,
            right: 92.0,
            bottom: 103.0,
            left: 114.0,
        }
    );
}

#[test]
fn recovered_container_padding_does_not_add_the_spacing_floor_to_child_margins() {
    let mut input = Graph::default();
    let parent = input.add_node(node("parent", ShapeKind::Rectangle));
    let mut child_node = node("child", ShapeKind::Rectangle);
    child_node.parent = Some(parent);
    child_node.layout_margins.bottom = 113.0;
    input.add_node(child_node);
    let graph = ArenaGraph::from_input(&input);

    assert_eq!(
        graph.shape_fit_padding_with_children(parent, true),
        Insets {
            top: 60.0,
            right: 60.0,
            bottom: 113.0,
            left: 60.0,
        }
    );
}

#[test]
fn nested_cluster_gap_uses_the_sibling_clearance_fallback() {
    let mut input = Graph::default();
    let mut container_node = node("container", ShapeKind::Rectangle);
    container_node.size = Size {
        width: 800.0,
        height: 400.0,
    };
    let container = input.add_node(container_node);

    let mut first_node = node("first", ShapeKind::Rectangle);
    first_node.parent = Some(container);
    let first = input.add_node(first_node);
    let mut second_node = node("second", ShapeKind::Rectangle);
    second_node.parent = Some(container);
    let second = input.add_node(second_node);
    let mut sibling_node = node("sibling", ShapeKind::Rectangle);
    sibling_node.parent = Some(container);
    let sibling = input.add_node(sibling_node);
    let nearest = input.add_node(node("nearest", ShapeKind::Diamond));
    input.tala_id_overrides = vec![Some(10), Some(1), Some(2), Some(3), Some(4)];
    input.add_edge(Edge {
        source: first,
        target: nearest,
    });

    let mut graph = ArenaGraph::from_input(&input);
    graph.cell_size = 20.0;
    // The source graph lazily computes TurnCost from its single 800-unit
    // connection; seed that cached value in this isolated placement fixture.
    graph.set_turn_cost(100.0);
    for member in [first, second] {
        graph.nodes[member.0 as usize].cluster = Some(0);
    }
    graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: 5,
        fixed_size: false,
    });
    graph.node_order.retain(|node| *node != second);
    graph.rebuild_active_aggregate_node_order();
    graph.set_position(container, Point { x: 0.0, y: 0.0 });
    graph.set_position(first, Point { x: 100.0, y: 80.0 });
    graph.set_position(second, Point { x: 100.0, y: 180.0 });
    graph.set_position(sibling, Point { x: 500.0, y: 80.0 });
    graph.set_position(
        nearest,
        Point {
            x: 1_000.0,
            y: 80.0,
        },
    );

    assert!(graph.reduce_cluster_gap_to_neighbors(0, true, true, false));

    // Moving to the padded inner boundary would overlap the sibling. TALA
    // rolls that trial back, then stops at the first 150-unit sibling boundary.
    assert_eq!(graph.position(first), Some(Point { x: 250.0, y: 80.0 }));
    assert_eq!(graph.position(second), Some(Point { x: 250.0, y: 180.0 }));
    assert_eq!(graph.position(sibling), Some(Point { x: 500.0, y: 80.0 }));
    assert_eq!(graph.position(nearest), Some(Point { x: 950.0, y: 80.0 }));
    assert_eq!(graph.position(container), Some(Point { x: 190.0, y: 20.0 }));
    assert_eq!(
        graph
            .global_sized_edge_length_with_direction(false)
            .to_bits(),
        0x409a_9013_b13b_13b1
    );
}

#[test]
fn cluster_gap_nearest_ahead_uses_the_live_vessel_box() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", ShapeKind::Rectangle));
    let second = input.add_node(node("second", ShapeKind::Rectangle));
    let nearest = input.add_node(node("nearest", ShapeKind::Diamond));
    input.add_edge(Edge {
        source: first,
        target: nearest,
    });
    input.add_edge(Edge {
        source: second,
        target: nearest,
    });
    let mut graph = ArenaGraph::from_input(&input);
    for member in [first, second] {
        graph.nodes[member.0 as usize].cluster = Some(0);
    }
    graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: 1,
        fixed_size: false,
    });
    graph.node_order.retain(|node| *node != second);
    graph.rebuild_active_aggregate_node_order();
    graph.set_position(first, Point { x: 0.0, y: 0.0 });
    graph.set_position(second, Point { x: 0.0, y: 100.0 });
    graph.set_position(nearest, Point { x: 400.0, y: 0.0 });

    // The temporary vessel may be offset from stale retained-member boxes.
    // Nearest-ahead must read its live top-left, like Go's Node.TopLeft.
    graph
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 500.0, y: 0.0 });
    assert_eq!(graph.cluster_nearest_connected_ahead(0, true, true), None);
    graph
        .pending_cluster_vessel_positions
        .insert(0, Point { x: 0.0, y: 0.0 });
    assert_eq!(
        graph.cluster_nearest_connected_ahead(0, true, true),
        Some(nearest)
    );
}

#[test]
fn cluster_gap_movement_stops_before_a_fixed_connected_node() {
    let mut input = Graph::default();
    let first = input.add_node(node("first", ShapeKind::Rectangle));
    let second = input.add_node(node("second", ShapeKind::Rectangle));
    let nearest = input.add_node(node("nearest", ShapeKind::Diamond));
    let fixed = input.add_node(node("fixed", ShapeKind::Rectangle));
    input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4)];
    for (source, target) in [(first, nearest), (second, nearest), (nearest, fixed)] {
        input.add_edge(Edge { source, target });
    }
    let mut graph = ArenaGraph::from_input(&input);
    graph.cell_size = 20.0;
    for member in [first, second] {
        graph.nodes[member.0 as usize].cluster = Some(0);
    }
    graph.clusters.push(ClusterState {
        members: vec![first, second],
        arrangement: ClusterArrangement::Column,
        desired_arrangement: ClusterArrangement::Column,
        padding: 20.0,
        vessel_tala_id: 5,
        fixed_size: false,
    });
    graph.node_order.retain(|node| *node != second);
    graph.rebuild_active_aggregate_node_order();
    graph.set_position(first, Point { x: 0.0, y: 0.0 });
    graph.set_position(second, Point { x: 0.0, y: 100.0 });
    graph.set_position(nearest, Point { x: 400.0, y: 0.0 });
    graph.set_position(fixed, Point { x: 600.0, y: 0.0 });
    graph.nodes[fixed.0 as usize].fixed_top_left = Some(Point { x: 600.0, y: 0.0 });

    assert_eq!(
        graph.connected_node_views(
            ConnectedNode::Active(nearest),
            &[ConnectedNode::Active(first), ConnectedNode::Active(fixed)],
        ),
        vec![ConnectedNode::Active(nearest)],
        "fixed graph nodes stay connected for gap ordering but are excluded from movement"
    );
    assert!(graph.reduce_cluster_gap_to_neighbors(0, true, true, false));
    assert_eq!(graph.position(nearest), Some(Point { x: 250.0, y: 0.0 }));
    assert_eq!(graph.position(fixed), Some(Point { x: 600.0, y: 0.0 }));
    assert_eq!(
        graph
            .global_sized_edge_length_with_direction(false)
            .to_bits(),
        0x4094_5027_6276_2762,
        "fixed-node exclusion preserves the D2 v0.9.0 accepted score"
    );
}

#[test]
fn cluster_gap_matches_source_in_all_axis_directions() {
    struct Case {
        name: &'static str,
        horizontal: bool,
        forwards: bool,
        arrangement: ClusterArrangement,
        first: Point,
        second: Point,
        vessel_size: Size,
        nearest: Point,
        tail: Point,
        expected_nearest: Point,
        expected_tail: Point,
        expected_score_bits: u64,
    }

    let cases = [
        Case {
            name: "east",
            horizontal: true,
            forwards: true,
            arrangement: ClusterArrangement::Column,
            first: Point { x: 0.0, y: 0.0 },
            second: Point { x: 0.0, y: 100.0 },
            vessel_size: Size {
                width: 100.0,
                height: 180.0,
            },
            nearest: Point { x: 400.0, y: 0.0 },
            tail: Point { x: 600.0, y: 0.0 },
            expected_nearest: Point { x: 250.0, y: 0.0 },
            expected_tail: Point { x: 450.0, y: 0.0 },
            expected_score_bits: 0x408f_404e_c4ec_4ec4,
        },
        Case {
            name: "west",
            horizontal: true,
            forwards: false,
            arrangement: ClusterArrangement::Column,
            first: Point { x: 600.0, y: 0.0 },
            second: Point { x: 600.0, y: 100.0 },
            vessel_size: Size {
                width: 100.0,
                height: 180.0,
            },
            nearest: Point { x: 200.0, y: 0.0 },
            tail: Point { x: 0.0, y: 0.0 },
            expected_nearest: Point { x: 350.0, y: 0.0 },
            expected_tail: Point { x: 150.0, y: 0.0 },
            expected_score_bits: 0x408f_404e_c4ec_4ec4,
        },
        Case {
            name: "south",
            horizontal: false,
            forwards: true,
            arrangement: ClusterArrangement::Row,
            first: Point { x: 0.0, y: 0.0 },
            second: Point { x: 100.0, y: 0.0 },
            vessel_size: Size {
                width: 180.0,
                height: 80.0,
            },
            nearest: Point { x: 0.0, y: 400.0 },
            tail: Point { x: 0.0, y: 600.0 },
            expected_nearest: Point { x: 0.0, y: 230.0 },
            expected_tail: Point { x: 0.0, y: 430.0 },
            expected_score_bits: 0x4090_e026_6666_6666,
        },
        Case {
            name: "north",
            horizontal: false,
            forwards: false,
            arrangement: ClusterArrangement::Row,
            first: Point { x: 0.0, y: 600.0 },
            second: Point { x: 100.0, y: 600.0 },
            vessel_size: Size {
                width: 180.0,
                height: 80.0,
            },
            nearest: Point { x: 0.0, y: 200.0 },
            tail: Point { x: 0.0, y: 0.0 },
            expected_nearest: Point { x: 0.0, y: 370.0 },
            expected_tail: Point { x: 0.0, y: 170.0 },
            expected_score_bits: 0x4090_e026_6666_6666,
        },
    ];

    for case in cases {
        let mut input = Graph::default();
        let first = input.add_node(node("first", ShapeKind::Rectangle));
        let second = input.add_node(node("second", ShapeKind::Rectangle));
        let nearest = input.add_node(node("nearest", ShapeKind::Diamond));
        let tail = input.add_node(node("tail", ShapeKind::Rectangle));
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4)];
        for (source, target) in [(first, nearest), (second, nearest), (nearest, tail)] {
            input.add_edge(Edge { source, target });
        }

        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        for member in [first, second] {
            graph.nodes[member.0 as usize].cluster = Some(0);
        }
        graph.clusters.push(ClusterState {
            members: vec![first, second],
            arrangement: case.arrangement,
            desired_arrangement: case.arrangement,
            padding: 20.0,
            vessel_tala_id: 5,
            fixed_size: false,
        });
        graph.node_order.retain(|member| *member != second);
        graph.rebuild_active_aggregate_node_order();
        graph.set_position(first, case.first);
        graph.set_position(second, case.second);
        graph.set_position(nearest, case.nearest);
        graph.set_position(tail, case.tail);

        assert!(
            graph.reduce_cluster_gap_to_neighbors(0, case.horizontal, case.forwards, false),
            "{} should reduce the connected-side gap",
            case.name
        );
        assert_eq!(
            graph.position(nearest),
            Some(case.expected_nearest),
            "nearest node moved incorrectly in {}",
            case.name
        );
        assert_eq!(
            graph.position(tail),
            Some(case.expected_tail),
            "connected tail moved incorrectly in {}",
            case.name
        );
        assert_eq!(
            graph
                .global_sized_edge_length_with_direction(false)
                .to_bits(),
            case.expected_score_bits,
            "accepted edge-length score differs from the Go oracle in {}",
            case.name
        );
    }
}
