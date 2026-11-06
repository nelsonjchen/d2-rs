// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source-map contracts from D2 v0.9.0 loops.computeOffsets, Node.deltaTo,
//! graphjson.deserializeNode and transactionSweepPadding. These assert map
//! content/cardinality and direct API behavior, not Go map traversal order.

#![cfg(test)]

use crate::internal::RoutePoints;
use crate::internal::{
    ArenaGraph, Orientation, Pipeline, limits, loop_cardinal_extents, loop_offset_toward,
};
use crate::{Edge, Graph, Node, NodeId, Point, Size};
use d2_rs_core::TalaLoopOffsets;

fn one_box(with_loop: bool) -> (Graph, NodeId) {
    let mut graph = Graph::default();
    let node = graph.add_node(Node::new(
        "box",
        Size {
            width: 100.0,
            height: 100.0,
        },
    ));
    if with_loop {
        graph.add_edge(Edge {
            source: node,
            target: node,
        });
    }
    (graph, node)
}

fn raw_entries(offsets: &TalaLoopOffsets) -> Vec<(i64, u64)> {
    offsets
        .iter()
        .map(|(&key, value)| (key, value.to_bits()))
        .collect()
}

#[test]
fn directional_loop_lookup_preserves_authored_keys_and_raw_bits() {
    // Go orientation numbering differs from the Rust enum declaration order.
    // Diagonals are independent authored values, not reconstructed maxima.
    let minus_zero = f64::from_bits(0x8000_0000_0000_0000);
    let nan_payload = f64::from_bits(0x7ff8_1234_5678_9abc);
    let offsets = TalaLoopOffsets::from([
        (-31, 999.0),
        (0, minus_zero),
        (1, -7.0),
        (2, 19.0),
        (3, nan_payload),
        (4, minus_zero),
        (5, 12.0),
        (6, 23.0),
        (7, 34.0),
        (8, minus_zero),
        (91, f64::INFINITY),
    ]);
    for (side, expected_bits) in [
        (Orientation::TopLeft, minus_zero.to_bits()),
        (Orientation::TopRight, (-7.0_f64).to_bits()),
        (Orientation::BottomLeft, 19.0_f64.to_bits()),
        (Orientation::BottomRight, nan_payload.to_bits()),
        (Orientation::Top, minus_zero.to_bits()),
        (Orientation::Right, 12.0_f64.to_bits()),
        (Orientation::Bottom, 23.0_f64.to_bits()),
        (Orientation::Left, 34.0_f64.to_bits()),
        (Orientation::None, minus_zero.to_bits()),
    ] {
        assert_eq!(
            loop_offset_toward(Some(&offsets), side).to_bits(),
            expected_bits,
            "authored lookup for {side:?}"
        );
    }
    assert_eq!(
        loop_cardinal_extents(Some(&offsets)).map(f64::to_bits),
        [
            minus_zero.to_bits(),
            12.0_f64.to_bits(),
            23.0_f64.to_bits(),
            34.0_f64.to_bits()
        ]
    );
    let only_unknown = TalaLoopOffsets::from([(91, 999.0)]);
    assert_eq!(loop_cardinal_extents(Some(&only_unknown)), [0.0; 4]);
    assert_eq!(
        loop_offset_toward(Some(&only_unknown), Orientation::None).to_bits(),
        0
    );
    assert_eq!(loop_offset_toward(None, Orientation::TopLeft).to_bits(), 0);
    assert_eq!(offsets.len(), 11, "unknown keys must remain map entries");
}

#[test]
fn input_adapter_retains_nil_empty_and_arbitrary_loop_map_contents() {
    let mut input = Graph::default();
    let nodes: Vec<_> = (0..3)
        .map(|index| {
            input.add_node(Node::new(
                format!("n{index}"),
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        })
        .collect();
    assert_eq!(input.tala_node_loop_offsets.len(), 3);
    assert!(input.tala_node_loop_offsets.iter().all(Option::is_none));
    let authored = TalaLoopOffsets::from([
        (-7, f64::from_bits(0x7ff8_1234_5678_9abc)),
        (0, 39.0),
        (8, -0.0),
        (1234, -17.0),
    ]);
    input.tala_node_loop_offsets = vec![None, Some(TalaLoopOffsets::new()), Some(authored.clone())];
    let arena = ArenaGraph::from_input(&input);
    assert!(arena.nodes[nodes[0].0 as usize].loop_offsets.is_none());
    assert!(
        arena.nodes[nodes[1].0 as usize]
            .loop_offsets
            .as_ref()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        raw_entries(
            arena.nodes[nodes[2].0 as usize]
                .loop_offsets
                .as_ref()
                .unwrap()
        ),
        raw_entries(&authored)
    );
    assert_eq!(arena.loop_spacing_extents(nodes[0]), None);
    assert_eq!(arena.loop_spacing_extents(nodes[1]), Some([0.0; 4]));
    assert_eq!(
        arena.loop_extent_toward(nodes[2], Orientation::TopLeft),
        39.0
    );
    assert_eq!(
        arena
            .loop_extent_toward(nodes[2], Orientation::None)
            .to_bits(),
        (-0.0_f64).to_bits()
    );
}

#[test]
fn graph_wire_roundtrip_omits_loop_metadata_and_starts_decoded_nodes_nil() {
    // Original graphjson.SerializedNode has no LoopOffsets field, and its
    // decoder calls NewNode without Prepare even when self-edges are present.
    let (mut input, node) = one_box(true);
    input.tala_node_loop_offsets[node.0 as usize] = Some(TalaLoopOffsets::from([
        (0, 999.0),
        (8, -0.0),
        (42, f64::from_bits(0x7ff8_1234_5678_9abc)),
    ]));
    let with_metadata = serde_json::to_value(&input).unwrap();
    assert!(
        !with_metadata
            .as_object()
            .unwrap()
            .contains_key("tala_node_loop_offsets")
    );
    input.tala_node_loop_offsets[node.0 as usize] = None;
    assert_eq!(with_metadata, serde_json::to_value(&input).unwrap());
    let decoded: Graph = serde_json::from_value(with_metadata).unwrap();
    assert!(decoded.tala_node_loop_offsets.is_empty());
    let arena = ArenaGraph::from_input(&decoded);
    assert!(arena.nodes[node.0 as usize].loop_offsets.is_none());
    assert_eq!(arena.loop_spacing_extents(node), None);
}

#[test]
fn prepare_replaces_authored_offsets_with_nonnil_empty_map_without_loops() {
    // computeOffsets allocates a new map before the unchanged-envelope return.
    let (mut input, node) = one_box(false);
    input.tala_node_loop_offsets[node.0 as usize] =
        Some(TalaLoopOffsets::from([(0, 700.0), (8, 99.0), (1234, 88.0)]));
    let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
    pipeline
        .graph
        .set_position(node, Point { x: 10.0, y: 20.0 });
    let position = pipeline.graph.position(node);
    pipeline.run_preprocess();
    let offsets = pipeline.graph.nodes[node.0 as usize]
        .loop_offsets
        .as_ref()
        .unwrap();
    assert!(
        offsets.is_empty(),
        "Prepare must replace rather than extend authored state"
    );
    assert_eq!(pipeline.graph.loop_spacing_extents(node), Some([0.0; 4]));
    assert_eq!(pipeline.graph.position(node), position);
}

#[test]
fn prepare_keeps_all_eight_computed_orientation_keys_including_zero_values() {
    // A first plain-box loop occupies the left/top slot. The unchanged right
    // and bottom sides still have explicit entries in the source eight-key map.
    let (mut input, node) = one_box(true);
    input.tala_node_loop_offsets[node.0 as usize] =
        Some(TalaLoopOffsets::from([(8, 999.0), (1234, 999.0)]));
    let mut pipeline = Pipeline::new_untraced(&input, 1, false, true);
    pipeline.graph.set_position(node, Point { x: 0.0, y: 0.0 });
    pipeline.run_preprocess();
    let offsets = pipeline.graph.nodes[node.0 as usize]
        .loop_offsets
        .as_ref()
        .unwrap();
    assert_eq!(
        offsets.keys().copied().collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
    assert!(
        offsets
            .values()
            .all(|value| value.is_finite() && *value >= 0.0)
    );
    assert!(offsets[&4] > 0.0 && offsets[&7] > 0.0);
    assert_eq!(offsets[&5].to_bits(), 0);
    assert_eq!(offsets[&6].to_bits(), 0);
    assert!(!offsets.contains_key(&8) && !offsets.contains_key(&1234));
}

#[test]
fn cached_loop_spacing_does_not_route_a_self_edge_or_reconstruct_diagonals() {
    let (input, node) = one_box(true);
    let mut arena = ArenaGraph::from_input(&input);
    assert!(arena.nodes[node.0 as usize].loop_offsets.is_none());
    assert_eq!(arena.loop_spacing_extents(node), None);
    assert_eq!(arena.loop_extent_toward(node, Orientation::TopLeft), 0.0);
    arena.nodes[node.0 as usize].loop_offsets = Some(TalaLoopOffsets::new());
    assert_eq!(arena.loop_spacing_extents(node), Some([0.0; 4]));
    arena.nodes[node.0 as usize].loop_offsets = Some(TalaLoopOffsets::from([(0, 73.0), (8, 11.0)]));
    assert_eq!(arena.loop_spacing_extents(node), Some([0.0; 4]));
    assert_eq!(arena.loop_extent_toward(node, Orientation::TopLeft), 73.0);
    assert_eq!(arena.loop_extent_toward(node, Orientation::None), 11.0);
}

#[test]
fn update_offsets_preserves_nil_position_and_discards_old_absolute_loop_routes() {
    let (input, node) = one_box(true);
    let mut arena = ArenaGraph::from_input(&input);
    arena.edge_records[0]
        .route
        .replace(RoutePoints::literal(&[Point { x: 900.0, y: 800.0 }]));
    arena.edge_records[0].label = Some(crate::EdgeLabel {
        text: "loop".into(),
        size: Size {
            width: 20.0,
            height: 10.0,
        },
        position: crate::LabelPosition::UnlockedBottom,
        percentage: 0.0,
    });
    assert!(arena.position(node).is_none());
    arena.update_loop_offsets(node);
    assert!(arena.position(node).is_none());
    assert!(arena.edge_records[0].route.is_empty());
    assert_eq!(
        arena.edge_records[0].label.as_ref().unwrap().position,
        crate::LabelPosition::OutsideTopCenter
    );
    assert_eq!(
        arena.nodes[node.0 as usize]
            .loop_offsets
            .as_ref()
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn spacing_truncates_each_authored_diagonal_or_none_lookup_before_adding() {
    // Node.deltaTo converts each source map lookup to int independently:
    // NodeGap20 + int(3.75) + int(4.75) =27, rather than int(20+8.5)=28.
    let size = Size {
        width: 10.0,
        height: 10.0,
    };
    for (proposed, other_position, node_key, other_key) in [
        (Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 100.0 }, 3, 0),
        (Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }, 8, 8),
        (Point { x: 100.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }, 7, 5),
    ] {
        // Large unrelated cardinal entries must not replace an authored
        // diagonal or NONE value with a reconstructed maximum.
        let mut first = TalaLoopOffsets::from([(4, 100.0), (5, 100.0), (6, 100.0), (7, 100.0)]);
        let mut second = first.clone();
        first.insert(node_key, 3.75);
        second.insert(other_key, 4.75);
        assert_eq!(
            ArenaGraph::spacing_delta_with_loop_geometry(
                20.0,
                proposed,
                size,
                other_position,
                size,
                Some(&first),
                Some(&second)
            ),
            27.0
        );
        first.insert(node_key, -3.75);
        assert_eq!(
            ArenaGraph::spacing_delta_with_loop_geometry(
                20.0,
                proposed,
                size,
                other_position,
                size,
                Some(&first),
                Some(&second)
            ),
            21.0,
            "negative source int conversions truncate toward zero"
        );
    }
}

#[test]
fn spacing_invalid_pinned_go_int_conversions_do_not_saturate_positive() {
    // The pinned Go 1.27 darwin/arm64 controls return MinInt64 from these int
    // conversions. A single invalid lookup therefore cannot enlarge NodeGap.
    // Rust's saturating float-to-int cast would incorrectly enlarge +Inf.
    let size = Size {
        width: 10.0,
        height: 10.0,
    };
    for value in [
        f64::from_bits(0x7ff8_1234_5678_9abc),
        f64::INFINITY,
        f64::NEG_INFINITY,
        9_223_372_036_854_775_808.0,
    ] {
        let offsets = TalaLoopOffsets::from([(7, value)]);
        assert_eq!(
            ArenaGraph::spacing_delta_with_loop_geometry(
                20.0,
                Point { x: 100.0, y: 0.0 },
                size,
                Point { x: 0.0, y: 0.0 },
                size,
                Some(&offsets),
                None,
            ),
            20.0
        );
    }
}

struct Background;

impl limits::Context for Background {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<limits::ContextError> {
        None
    }
}

fn one_transaction_node() -> (ArenaGraph, NodeId) {
    let (input, node) = one_box(false);
    let mut arena = ArenaGraph::from_input(&input);
    // Match original NewGraph/NewNode controls without an adapter root key.
    arena.containers.clear();
    arena.nodes[node.0 as usize].tala_id = 101;
    arena.set_position(node, Point { x: 0.0, y: 0.0 });
    (arena, node)
}

#[test]
fn initial_overlap_work_counts_zero_entries_and_unknown_keys_individually() {
    let (mut arena, node) = one_transaction_node();
    // One node/no edges charges padding1 + collection1 + presort1 + sweep1.
    // An actual map entry adds one Step, even for NONE/unknown or a zero value.
    for (offsets, expected) in [
        (None, 4),
        (Some(TalaLoopOffsets::new()), 4),
        (Some(TalaLoopOffsets::from([(1234, -0.0)])), 5),
        (Some((0..8).map(|key| (key, 0.0)).collect()), 12),
    ] {
        arena.nodes[node.0 as usize].loop_offsets = offsets;
        let mut guard =
            limits::WorkGuard::new(Some(&Background), "LoopOffsetsTransaction", 100).unwrap();
        arena
            .gap_transaction_initial_overlap_work(&mut guard)
            .unwrap();
        assert_eq!(guard.used(), expected);
    }
    let mut guard = limits::WorkGuard::new(Some(&Background), "LoopOffsetsTransaction", 8).unwrap();
    let error = arena
        .gap_transaction_initial_overlap_work(&mut guard)
        .unwrap_err();
    assert_eq!(
        guard.used(),
        9,
        "retain the first rejected source work unit"
    );
    assert_eq!(
        error.to_string(),
        "TALA LoopOffsetsTransaction work exceeds limit 8"
    );
}

#[test]
fn initial_overlap_work_rejects_nonfinite_values_at_any_authored_key() {
    let (mut arena, node) = one_transaction_node();
    // Singleton maps avoid any assumption about Go randomized map order.
    // Padding charges the node and offending entry before finite validation.
    for key in [-99, 0, 8, 1234] {
        for value in [
            f64::from_bits(0x7ff8_1234_5678_9abc),
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            arena.nodes[node.0 as usize].loop_offsets = Some(TalaLoopOffsets::from([(key, value)]));
            let mut guard =
                limits::WorkGuard::new(Some(&Background), "LoopOffsetsTransaction", 100).unwrap();
            let error = arena
                .gap_transaction_initial_overlap_work(&mut guard)
                .unwrap_err();
            assert_eq!(guard.used(), 2);
            assert_eq!(
                error.to_string(),
                "layout invariant violated: transaction node 101 has a non-finite loop offset"
            );
        }
    }
}
