// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Rust port of TALA, the layout engine bundled in upstream D2, producing the
//! same layouts as Go D2.
//!
//! [`Graph`] stores nodes, edges, and adapter metadata. [`Engine`] turns that
//! model into node rectangles, edge routes, label placements, and a diagnostic
//! [`LayoutReport`]. A single seed is deterministic; with several seeds, the
//! engine races their candidates within its time budget. The
//! `diagnostic-traces` feature enables traces, `checked-cells` checks internal
//! cell borrows, and `oracle-fixtures` compiles tests that read regenerated
//! Go-oracle artifacts.
//!
//! # Where to begin
//!
//! - [`guide::quick_start`] builds and lays out a graph from scratch.
//! - [`guide::graph_model`] explains identity, containers, constraints, and
//!   edge metadata.
//! - [`guide::architecture`] follows data through the complete engine.
//! - [`guide::placement`] and [`guide::routing`] explain the two major
//!   geometry phases step by step.
//! - [`guide::seeds_and_scoring`] defines reproducibility and candidate
//!   selection precisely.
//! - [`guide::diagnostics`] describes intermediate snapshots and traces.
//!
//! # Minimal example
//!
//! ```
//! use d2_rs_tala::{Edge, Engine, Graph, LayoutOptions, Node, Size};
//!
//! let mut graph = Graph::default();
//! let a = graph.add_node(Node::new("a", Size { width: 80.0, height: 40.0 }));
//! let b = graph.add_node(Node::new("b", Size { width: 80.0, height: 40.0 }));
//! graph.add_edge(Edge { source: a, target: b });
//!
//! let result = Engine.layout(&graph, &LayoutOptions { seeds: vec![1], max_concurrency: 0 })?;
//! assert_eq!(result.boxes.len(), 2);
//! assert_eq!(result.routes.len(), 1);
//! # Ok::<(), d2_rs_tala::LayoutError>(())
//! ```
#![warn(missing_docs)]

mod internal;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod route_validation_tests;
use internal as engine;

pub mod guide;

/// Decode, validate, and reconstruct D2 v0.9.0's measured graphjson format.
///
/// This adapter is useful for native callers that already have TALA's
/// serialized graph representation. It does not invoke an external plugin or
/// require tracing to be enabled.
pub mod graphjson {
    pub use crate::internal::graphjson::{
        graphjson_layout, graphjson_preflight, graphjson_to_graph, graphjson_wire_roundtrip,
    };
}

/// Diagnostic stage snapshots for compatibility analysis.
///
/// These types expose implementation boundaries rather than the stable layout
/// result contract. They are useful for locating the first divergent stage,
/// inspecting hierarchy decisions, and verifying random-stream consumption.
/// They are observations, not resumable checkpoints, and may evolve when the
/// internal pipeline changes. See [`guide::diagnostics`] before using them.
pub mod diagnostic {
    pub use crate::engine::{
        LayoutEdgeState, LayoutHierarchyAlignmentDirection, LayoutHierarchyAlignmentNodeState,
        LayoutHierarchyAlignmentState, LayoutHierarchyAssignmentTrace, LayoutHierarchyOrderStage,
        LayoutHierarchyOrderState, LayoutHierarchyPlacementLevelState,
        LayoutHierarchyPlacementNodeState, LayoutHierarchyPlacementTrace, LayoutNodeState,
        LayoutSnapshot, LayoutStage, layout_snapshot,
    };
    pub use crate::graphjson::{graphjson_preflight, graphjson_to_graph, graphjson_wire_roundtrip};
    #[cfg(feature = "diagnostic-traces")]
    pub use crate::internal::diagnostic_combine_work_probe;
    #[cfg(feature = "diagnostic-traces")]
    pub use crate::internal::diagnostic_gap_stage_probe;
    #[cfg(feature = "diagnostic-traces")]
    pub use crate::internal::diagnostic_loop_offsets_probe;
    #[cfg(feature = "diagnostic-traces")]
    pub use crate::internal::diagnostic_node_bounds_probe;
}

// Preserve the original root paths for diagnostic tooling while routing new
// rustdoc readers through the explicitly unstable `diagnostic` namespace.
#[doc(hidden)]
pub use diagnostic::{
    LayoutHierarchyAlignmentDirection, LayoutHierarchyAlignmentNodeState,
    LayoutHierarchyAlignmentState, LayoutHierarchyAssignmentTrace, LayoutHierarchyOrderStage,
    LayoutHierarchyOrderState, LayoutHierarchyPlacementLevelState,
    LayoutHierarchyPlacementNodeState, LayoutHierarchyPlacementTrace, LayoutNodeState,
    LayoutSnapshot, LayoutStage, layout_snapshot,
};

pub use d2_rs_core::*;

/// Installs a callback that runs on the routing thread after each edge
/// routing pass has dropped its visibility graph and search state, before
/// another seed of the same race may start routing. The callback cannot
/// observe or change any layout data; an allocator can use it to return the
/// freed pages promptly. The first installation wins.
pub fn set_routing_release_hook(hook: fn()) {
    internal::set_routing_release_hook(hook);
}

#[derive(Clone, Debug, Default)]
/// Stateless entry point for layout and standalone edge routing.
pub struct Engine;

impl Engine {
    /// Places nodes, routes edges, and positions labels.
    ///
    /// The input is borrowed and never mutated. Every requested seed runs the
    /// complete pipeline on an owned internal copy; the returned
    /// [`LayoutResult`] contains only the selected candidate.
    ///
    /// # Errors
    ///
    /// Returns a validation variant of [`LayoutError`] for malformed graph
    /// references or dimensions, [`LayoutError::NoSeeds`] for an empty seed
    /// list, or a routing/timeout variant when the pipeline cannot finish.
    pub fn layout(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
    ) -> Result<LayoutResult, LayoutError> {
        engine::layout_result(graph, options)
    }

    /// Places nodes and routes edges while observing a caller-owned
    /// cancellation token.
    ///
    /// The token is checked before work starts and cooperatively at the same
    /// stage, routing, and work-accounting boundaries used by the v0.9.0
    /// implementation. The input graph is never mutated.
    pub fn layout_with_cancellation(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        cancellation: &CancellationToken,
    ) -> Result<LayoutResult, LayoutError> {
        engine::layout_result_with_cancellation(graph, options, Some(cancellation))
    }

    /// Routes a graph at its existing node coordinates without running node
    /// placement. This is the D2 `routeedges` contract.
    ///
    /// Every edge is rerouted in stable ID order. Existing coordinates come
    /// from [`Graph::set_initial_position`] or [`Node::locked_position`]; routes stored with
    /// [`Graph::set_edge_route`] provide interaction context.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError`] when validation fails or
    /// at least one requested route cannot be produced.
    pub fn route_edges(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
    ) -> Result<LayoutResult, LayoutError> {
        let edges = graph.edges().map(|(id, _)| id).collect::<Vec<_>>();
        engine::route_existing(graph, options, &edges)
    }

    /// Routes every edge while observing a caller-owned cancellation token.
    pub fn route_edges_with_cancellation(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        cancellation: &CancellationToken,
    ) -> Result<LayoutResult, LayoutError> {
        let edges = graph.edges().map(|(id, _)| id).collect::<Vec<_>>();
        engine::route_existing_with_cancellation(graph, options, &edges, Some(cancellation))
    }

    /// Routes only the requested edges, in the supplied order, while treating
    /// every other imported route as fixed standalone-routing topology.
    ///
    /// The order of `edges` is significant because each accepted route can
    /// influence the cost of later routes.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError`] when the selection contains duplicates or
    /// out-of-inventory IDs, exceeds D2's edge limits, graph validation fails,
    /// or a requested route cannot be produced. Routing has no seed plan and
    /// accepts an empty caller seed list.
    /// Selection validation precedes cancellation and graph validation.
    /// IDs refer to this graph's indexed slots; an ID copied from another
    /// graph with the same numeric slot is indistinguishable in this API.
    pub fn route_edges_selected(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        edges: &[EdgeId],
    ) -> Result<LayoutResult, LayoutError> {
        engine::route_existing(graph, options, edges)
    }

    /// Routes selected edges while observing a caller-owned cancellation
    /// token.
    pub fn route_edges_selected_with_cancellation(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        edges: &[EdgeId],
        cancellation: &CancellationToken,
    ) -> Result<LayoutResult, LayoutError> {
        engine::route_existing_with_cancellation(graph, options, edges, Some(cancellation))
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::{CancellationToken, Engine, Graph, LayoutError, LayoutOptions, Node, Size};

    #[test]
    fn source_seed_plan_is_checked_after_cancellation_before_graph_validation() {
        let mut graph = Graph::default();
        graph.add_node(Node::new("invalid-size", Size::default()));
        graph.tala_seed_data = Some(Err(
            "invalid tala seed at index 0 (nope): must be a signed 64-bit integer".into(),
        ));
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            Engine.layout_with_cancellation(&graph, &LayoutOptions::default(), &token),
            Err(LayoutError::Cancelled)
        ));
        assert!(matches!(
            Engine.layout(&graph, &LayoutOptions::default()),
            Err(LayoutError::InvalidSeedData(_))
        ));
        graph.tala_seed_data = Some(Ok(Vec::new()));
        assert!(matches!(
            Engine.layout(&graph, &LayoutOptions::default()),
            Err(LayoutError::NoSeeds)
        ));
        graph.tala_seed_data = Some(Ok(vec![7]));
        let invalid_caller = LayoutOptions {
            seeds: Vec::new(),
            max_concurrency: 0,
        };
        assert!(matches!(
            Engine.layout(&graph, &invalid_caller),
            Err(LayoutError::InvalidSize { .. })
        ));
    }

    #[test]
    fn cancelled_layout_is_rejected_before_pipeline_mutation() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            Engine.layout_with_cancellation(&graph, &LayoutOptions::default(), &token),
            Err(LayoutError::Cancelled)
        ));
    }

    #[test]
    fn cancelled_route_is_rejected_before_pipeline_mutation() {
        let graph = Graph::default();
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            Engine.route_edges_with_cancellation(&graph, &LayoutOptions::default(), &token),
            Err(LayoutError::Cancelled)
        ));
    }

    #[cfg(feature = "oracle-fixtures")]
    fn check_source_requested_routes(cancelled_only: bool) {
        use super::{Edge, EdgeId, Point};
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/requested-routing-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 30);
        let mut checked = 0;
        let mut mismatches = Vec::new();
        for case in cases {
            let input = &case["input"];
            let cancelled = input["cancel"].as_bool().unwrap();
            if cancelled_only && !cancelled {
                continue;
            }
            let mut graph = Graph::default();
            let nodes = [("a", 10.0), ("b", 250.0)].map(|(name, x)| {
                let mut node = Node::new(
                    name,
                    Size {
                        width: 100.0,
                        height: 60.0,
                    },
                );
                node.locked_position = Some(Point { x, y: 20.0 });
                graph.add_node(node)
            });
            for _ in 0..input["graph_edges"].as_u64().unwrap() {
                let edge = graph.add_edge(Edge {
                    source: nodes[0],
                    target: nodes[1],
                });
                graph.set_edge_route(
                    edge,
                    vec![Point { x: 110.0, y: 50.0 }, Point { x: 250.0, y: 50.0 }],
                );
            }
            let requested = if input["oversized"].as_bool().unwrap() {
                vec![EdgeId(0); 50_001]
            } else {
                input["requested"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| EdgeId(value.as_u64().unwrap() as u32))
                    .collect()
            };
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let actual = Engine
                .route_edges_selected_with_cancellation(
                    &graph,
                    &LayoutOptions::default(),
                    &requested,
                    &token,
                )
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            let expected = case["error"].as_str().unwrap();
            checked += 1;
            if actual != expected {
                mismatches.push(format!(
                    "{}: actual={actual:?}, source={expected:?}",
                    input["name"]
                ));
            }
        }
        assert_eq!(checked, if cancelled_only { 15 } else { 30 });
        assert!(
            mismatches.is_empty(),
            "{} / {checked} requested-routing mismatches:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_requested_route_cancellation_preflight() {
        check_source_requested_routes(true);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_requested_route_preflight_contract() {
        check_source_requested_routes(false);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_existing_route_geometry_contract() {
        use super::{Edge, EdgeId, NodeId, Point};
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/route-geometry-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 258);
        for case in cases {
            let input = &case["input"];
            let mut graph = Graph::default();
            let number =
                |value: &serde_json::Value| value.as_str().unwrap().parse::<f64>().unwrap();
            for data in input["nodes"].as_array().unwrap() {
                let mut node = Node::new(
                    data["id"].as_str().unwrap(),
                    Size {
                        width: number(&data["width"]),
                        height: number(&data["height"]),
                    },
                );
                if let Some(attrs) = data["attrs"].as_object() {
                    let bytes = |key| {
                        attrs
                            .get(key)
                            .and_then(|v| v.as_str())
                            .map(|s| s.as_bytes().to_vec())
                    };
                    node.d2_authored_geometry = Some(Box::new(d2_rs_core::D2AuthoredGeometry {
                        object_id: data["id"].as_str().unwrap().as_bytes().to_vec(),
                        width: bytes("width"),
                        height: bytes("height"),
                        top: bytes("top"),
                        left: bytes("left"),
                        input_box: None,
                        attributes: None,
                    }));
                }
                if let Some(position) = data["position"].as_array() {
                    node.locked_position = Some(Point {
                        x: number(&position[0]),
                        y: number(&position[1]),
                    });
                }
                graph.add_node(node);
            }
            graph.hierarchy_children_order.insert(
                None,
                input["visit_order"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| NodeId(value.as_u64().unwrap() as u32))
                    .collect(),
            );
            let edge = graph.add_edge(Edge {
                source: NodeId(0),
                target: NodeId(1),
            });
            graph.set_edge_arrows(
                edge,
                super::EdgeArrows {
                    source: false,
                    target: true,
                },
            );
            graph.set_edge_route(
                edge,
                vec![Point { x: 110.0, y: 50.0 }, Point { x: 250.0, y: 50.0 }],
            );
            let requested = input["requested"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| EdgeId(value.as_u64().unwrap() as u32))
                .collect::<Vec<_>>();
            let options = LayoutOptions {
                seeds: input["seeds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap())
                    .collect(),
                max_concurrency: 1,
            };
            let token = CancellationToken::new();
            if input["cancel"].as_bool().unwrap() {
                token.cancel();
            }
            for retained_position in [false, true] {
                if retained_position {
                    for id in [NodeId(0), NodeId(1)] {
                        let position = graph.node_mut(id).unwrap().locked_position.take();
                        graph.set_initial_position(id, position);
                    }
                }
                let result = Engine
                    .route_edges_selected_with_cancellation(&graph, &options, &requested, &token);
                let actual = result
                    .as_ref()
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default();
                assert_eq!(
                    actual,
                    case["error"].as_str().unwrap(),
                    "{} retained={retained_position}",
                    input["name"]
                );
                if let Ok(result) = result {
                    let source = &case["graph"];
                    let objects = source["objects"].as_array().unwrap();
                    assert_eq!(result.boxes.len(), objects.len());
                    for (index, object) in objects.iter().enumerate() {
                        let actual = result.boxes[&NodeId(index as u32)];
                        let expected = &object["box"];
                        for (left, right) in [
                            (actual.origin.x, expected["TopLeft"]["x"].as_f64().unwrap()),
                            (actual.origin.y, expected["TopLeft"]["y"].as_f64().unwrap()),
                            (actual.size.width, expected["Width"].as_f64().unwrap()),
                            (actual.size.height, expected["Height"].as_f64().unwrap()),
                        ] {
                            assert_eq!(
                                left.to_bits(),
                                right.to_bits(),
                                "{} box {index}",
                                input["name"]
                            );
                        }
                    }
                    let edges = source["edges"].as_array().unwrap();
                    assert_eq!(result.routes.len(), edges.len());
                    for (index, edge) in edges.iter().enumerate() {
                        let actual = &result.routes[&EdgeId(index as u32)];
                        let expected = edge["route"].as_array().unwrap();
                        assert_eq!(actual.len(), expected.len());
                        for (left, right) in actual.iter().zip(expected) {
                            assert_eq!(
                                left.x.to_bits(),
                                right["x"].as_f64().unwrap().to_bits(),
                                "{} route x",
                                input["name"]
                            );
                            assert_eq!(
                                left.y.to_bits(),
                                right["y"].as_f64().unwrap().to_bits(),
                                "{} route y",
                                input["name"]
                            );
                        }
                    }
                }
            }
        }
    }
}
