// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{Graph, LayoutOptions, Point, Rect, ast::Shared};
use d2_rs_elk::{ElkOptions, codec::graph_to_elk_graph, layout};
use d2_rs_language::{compiler, parse};

fn compile(source: &str) -> Graph {
    let parsed = parse("main.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
    compiler::lower_graph(&compiler, root).unwrap()
}

fn crosses_interior(rect: Rect, start: Point, end: Point) -> bool {
    let mut lo: f64 = 0.;
    let mut hi: f64 = 1.;
    for (a, b, min, max) in [
        (
            start.x,
            end.x,
            rect.origin.x + 1.,
            rect.origin.x + rect.size.width - 1.,
        ),
        (
            start.y,
            end.y,
            rect.origin.y + 1.,
            rect.origin.y + rect.size.height - 1.,
        ),
    ] {
        let delta = b - a;
        if delta == 0. {
            if a < min || a > max {
                return false;
            }
        } else {
            let first = (min - a) / delta;
            let last = (max - a) / delta;
            lo = lo.max(first.min(last));
            hi = hi.min(first.max(last));
            if lo > hi {
                return false;
            }
        }
    }
    true
}

// f3da1baea TestMrTreeRoutesAvoidShapes, using measured D2 input.
#[test]
fn mr_tree_routes_avoid_shapes() {
    for direction in ["right", "left", "down", "up"] {
        for order in ["dfs", "bfs"] {
            let source = format!(
                "direction: {direction}\nvars: {{d2-config: {{data: {{elk-algorithm: mrtree; elk-mrtreeSearchOrder: {order}}}}}}}\nRoot -> API -> Cache -> Store\nRoot -> Store\nRoot -> Web\n"
            );
            let graph = compile(&source);
            assert_eq!(graph.d2_config_data["elk-algorithm"], "mrtree");
            assert_eq!(graph.d2_config_data["elk-mrtreeSearchOrder"], order);
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            for (id, edge) in graph.edges() {
                let route = &result.routes[&id];
                assert!(route.len() >= 2);
                let contains = |node, p: Point| {
                    let rect = result.boxes[&node];
                    p.x >= rect.origin.x
                        && p.x <= rect.origin.x + rect.size.width
                        && p.y >= rect.origin.y
                        && p.y <= rect.origin.y + rect.size.height
                };
                assert!(
                    !contains(edge.source, route[1]),
                    "{direction}/{order}/{id:?}"
                );
                assert!(
                    !contains(edge.target, route[route.len() - 2]),
                    "{direction}/{order}/{id:?}"
                );
                for pair in route.windows(2) {
                    assert_ne!(pair[0], pair[1]);
                    for (node, obj) in graph.nodes() {
                        assert!(
                            !crosses_interior(result.boxes[&node], pair[0], pair[1]),
                            "{direction}/{order}/{id:?} {pair:?} crosses {} {:?}",
                            obj.external_id,
                            result.boxes[&node]
                        );
                    }
                }
            }
        }
    }
}

// e1922797b: d2elklayout.Layout uppercases the object property on every node.
#[test]
fn authored_layer_constraint_reaches_wire_and_changes_layout() {
    let baseline = compile("direction: right\na -> b -> c\n");
    let ordinary = layout(&baseline, &LayoutOptions::default()).unwrap();
    for constraint in ["fIrSt", "lAsT"] {
        let graph = compile(&format!(
            "direction: right\na -> b -> c\nb.elk-layer-constraint: {constraint}\n"
        ));
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|n| n.external_id == "b")
                .unwrap()
                .elk_layer_constraint,
            constraint.to_uppercase()
        );
        let wire = graph_to_elk_graph(&graph, &graph.node_labels, ElkOptions::default());
        let b = wire.children.iter().find(|n| n.id == "b").unwrap();
        assert_eq!(
            b.layout_options.as_ref().unwrap()["elk.layered.layering.layerConstraint"],
            constraint.to_uppercase()
        );
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        let b_id = graph.nodes().find(|(_, n)| n.external_id == "b").unwrap().0;
        let bx = result.boxes[&b_id].origin.x;
        for (id, _) in graph.nodes() {
            if constraint.eq_ignore_ascii_case("first") {
                assert!(bx <= result.boxes[&id].origin.x);
            } else {
                assert!(bx >= result.boxes[&id].origin.x);
            }
        }
        assert_ne!(result.boxes, ordinary.boxes);
    }
}

#[test]
fn authored_container_layer_constraint_reaches_wire() {
    let graph = compile("a: {child}\na.elk-layer-constraint: last\nb -> a\n");
    let wire = graph_to_elk_graph(&graph, &graph.node_labels, ElkOptions::default());
    let a = wire.children.iter().find(|n| n.id == "a").unwrap();
    assert!(!a.children.is_empty());
    assert_eq!(
        a.layout_options.as_ref().unwrap()["elk.layered.layering.layerConstraint"],
        "LAST"
    );
}

// elk-go 8b285820520e p3order/TestModelOrderBarycenterKeepsRealNodeOrder:
// model order wins over the opposing barycenters of crossed edges.
#[test]
fn forced_model_order_preserves_authored_nodes_across_crossings() {
    for direction in ["right", "down", "left", "up"] {
        let graph = compile(&format!(
            "direction: {direction}\nvars: {{d2-config: {{data: {{elk-forceNodeModelOrder: true}}}}}}\na\nb\nc\nd\na -> d\nb -> c\n"
        ));
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        let cross_position = |name: &str| {
            let id = graph
                .nodes()
                .find(|(_, n)| n.external_id == name)
                .unwrap()
                .0;
            let rect = result.boxes[&id];
            if direction == "right" || direction == "left" {
                rect.origin.y
            } else {
                rect.origin.x
            }
        };
        assert!(
            cross_position("a") < cross_position("b"),
            "{direction}: a before b"
        );
        assert!(
            cross_position("c") < cross_position("d"),
            "{direction}: c before d"
        );
        assert_eq!(result.routes.len(), 2);
    }
}

// elk-go configurator.go + p5edges/orthogonal_edge_router.go: feedback ports
// retain the original flow sides even when P1 reverses an edge in the cycle.
#[test]
fn feedback_edges_keep_forward_endpoint_approaches() {
    for direction in ["right", "down", "left", "up"] {
        for body in ["a -> b -> c -> a", "container: {a -> b -> c -> a}"] {
            let graph = compile(&format!(
                "direction: {direction}\nvars: {{d2-config: {{data: {{elk-feedbackEdges: true}}}}}}\n{body}\n"
            ));
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            for route in result.routes.values() {
                let along = |p: Point| match direction {
                    "right" => p.x,
                    "left" => -p.x,
                    "down" => p.y,
                    _ => -p.y,
                };
                assert!(along(route[1]) > along(route[0]), "{direction}: {route:?}");
                let end = route.len() - 1;
                assert!(
                    along(route[end]) > along(route[end - 1]),
                    "{direction}: {route:?}"
                );
                for pair in route.windows(2) {
                    assert!(pair[0].x == pair[1].x || pair[0].y == pair[1].y);
                }
            }
            assert_eq!(result.routes.len(), 3);
        }
    }
}

#[test]
fn single_edge_wrapping_routes_d2_paths_in_all_directions() {
    for direction in ["right", "left", "down", "up"] {
        let body = (0..11)
            .map(|i| format!("n{i} -> n{}\n", i + 1))
            .collect::<String>();
        let ordinary = compile(&format!("direction: {direction}\n{body}"));
        let wrapped = compile(&format!(
            "direction: {direction}\nvars: {{d2-config: {{data: {{elk-wrappingStrategy: single_edge}}}}}}\n{body}"
        ));
        let baseline = layout(&ordinary, &LayoutOptions::default()).unwrap();
        let result = layout(&wrapped, &LayoutOptions::default()).unwrap();
        assert_eq!(result.boxes.len(), 12);
        assert_eq!(result.routes.len(), 11);
        let first = wrapped
            .nodes()
            .find(|(_, n)| n.external_id == "n0")
            .unwrap()
            .0;
        let last = wrapped
            .nodes()
            .find(|(_, n)| n.external_id == "n11")
            .unwrap()
            .0;
        let distance = |boxes: &std::collections::BTreeMap<_, Rect>| {
            let a = boxes[&first].origin;
            let b = boxes[&last].origin;
            if matches!(direction, "right" | "left") {
                (a.x - b.x).abs()
            } else {
                (a.y - b.y).abs()
            }
        };
        assert!(
            distance(&result.boxes) < distance(&baseline.boxes),
            "{direction}"
        );
        for route in result.routes.values() {
            assert!(route.len() >= 2);
            assert!(route.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
            for pair in route.windows(2) {
                assert!(
                    pair[0].x == pair[1].x || pair[0].y == pair[1].y,
                    "{direction}: {pair:?}"
                );
            }
        }
    }
}
