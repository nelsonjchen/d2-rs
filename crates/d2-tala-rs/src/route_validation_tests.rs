// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use serde_json::Value;

fn source_cases() -> Vec<Value> {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/route-validation-source-v090.json"
    ))
    .unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

fn input_graph(input: &Value) -> Graph {
    let mut graph = Graph::default();
    for (index, name) in ["a", "b"].iter().enumerate() {
        let mut node = Node::new(
            *name,
            Size {
                width: 100.0,
                height: 60.0,
            },
        );
        // adapter.go uses obj.Text().FontSize even without a visible label,
        // and retains the D2 ID independently from authored scalar geometry.
        node.font_size = Some(16);
        node.declared_size = Some(node.size);
        node.d2_authored_geometry = Some(Box::new(D2AuthoredGeometry {
            object_id: name.as_bytes().to_vec(),
            width: None,
            height: None,
            top: None,
            left: None,
            input_box: None,
            attributes: None,
        }));
        let id = graph.add_node(node);
        // Layout translation discards Box.TopLeft; standalone routing loads it.
        if !input["layout"].as_bool().unwrap()
            && (index != 0 || !input["missing_position"].as_bool().unwrap())
        {
            graph.set_initial_position(
                id,
                Some(Point {
                    x: 10.0 + index as f64 * 240.0,
                    y: 20.0,
                }),
            );
        }
    }
    graph.tala_id_overrides = identity::allocate(&[b"a", b"b"])
        .unwrap()
        .into_iter()
        .map(Some)
        .collect();
    let point = |v: &Value| Point {
        x: v[0].as_str().unwrap().parse().unwrap(),
        y: v[1].as_str().unwrap().parse().unwrap(),
    };
    let specs = input["routes"].as_array().unwrap();
    let names = (0..specs.len())
        .map(|index| format!("(a -> b)[{index}]"))
        .collect::<Vec<_>>();
    for spec in specs {
        let id = graph.add_edge(Edge {
            source: NodeId(0),
            target: NodeId(1),
        });
        graph.set_edge_arrows(
            id,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        let mut points = (0..spec["count"].as_u64().unwrap())
            .map(|i| Point {
                x: 110.0 + (i % 2) as f64 * 140.0,
                y: 50.0,
            })
            .collect::<Vec<_>>();
        points.extend(spec["points"].as_array().unwrap().iter().map(point));
        if !spec["tail"].is_null() {
            *points.last_mut().unwrap() = point(&spec["tail"]);
        }
        graph.set_edge_route(id, points);
    }
    graph.tala_edge_ids =
        identity::allocate(&names.iter().map(|s| s.as_bytes()).collect::<Vec<_>>())
            .unwrap()
            .into_iter()
            .map(Some)
            .collect();
    if !input["arrowhead"].as_str().unwrap().is_empty() {
        graph.set_edge_arrowhead_declarations(
            EdgeId(0),
            EdgeArrowheadDeclarations {
                source: Some(input["arrowhead"].as_str().unwrap().to_owned()),
                target: None,
            },
        );
    }
    graph
}

fn run(input: &Value) -> Result<LayoutResult, LayoutError> {
    let graph = input_graph(input);
    let token = CancellationToken::new();
    if input["cancel"].as_bool().unwrap() {
        token.cancel();
    }
    let options = LayoutOptions {
        seeds: vec![1],
        max_concurrency: 1,
    };
    if input["layout"].as_bool().unwrap() {
        Engine.layout_with_cancellation(&graph, &options, &token)
    } else {
        let requested = input["requested"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| EdgeId(v.as_u64().unwrap() as u32))
            .collect::<Vec<_>>();
        Engine.route_edges_selected_with_cancellation(&graph, &options, &requested, &token)
    }
}

#[test]
fn source_single_route_point_is_rejected() {
    let case = source_cases()
        .into_iter()
        .find(|v| v["input"]["name"] == "single-first-empty-False")
        .unwrap();
    let actual = run(&case["input"])
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert_eq!(actual, case["error"].as_str().unwrap());
}

#[test]
fn source_edge_route_validation_and_geometry_contract() {
    let cases = source_cases();
    assert_eq!(cases.len(), 504);
    let mut mismatches = Vec::new();
    let mut successes = 0;
    for case in cases {
        let input = &case["input"];
        let result = run(input);
        let actual = result
            .as_ref()
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        let expected = case["error"].as_str().unwrap();
        if actual != expected {
            mismatches.push(format!(
                "{}: expected {expected:?}, actual {actual:?}",
                input["name"]
            ));
            continue;
        }
        if let Ok(result) = result {
            successes += 1;
            let source = &case["graph"];
            let objects = source["objects"].as_array().unwrap();
            assert_eq!(result.boxes.len(), objects.len(), "{}", input["name"]);
            for (index, object) in objects.iter().enumerate() {
                let actual = result.boxes[&NodeId(index as u32)];
                let expected = &object["box"];
                for (label, a, b) in [
                    (
                        "x",
                        actual.origin.x,
                        expected["TopLeft"]["x"].as_f64().unwrap(),
                    ),
                    (
                        "y",
                        actual.origin.y,
                        expected["TopLeft"]["y"].as_f64().unwrap(),
                    ),
                    ("w", actual.size.width, expected["Width"].as_f64().unwrap()),
                    (
                        "h",
                        actual.size.height,
                        expected["Height"].as_f64().unwrap(),
                    ),
                ] {
                    if a.to_bits() != b.to_bits() {
                        mismatches
                            .push(format!("{} box {index} {label}: {a} != {b}", input["name"]));
                    }
                }
            }
            let edges = source["edges"].as_array().unwrap();
            assert_eq!(result.routes.len(), edges.len(), "{}", input["name"]);
            for (index, edge) in edges.iter().enumerate() {
                let actual = &result.routes[&EdgeId(index as u32)];
                let empty = Vec::new();
                let expected = edge["route"].as_array().unwrap_or(&empty);
                if actual.len() != expected.len() {
                    mismatches.push(format!(
                        "{} edge {index}: route length {} != {}",
                        input["name"],
                        actual.len(),
                        expected.len()
                    ));
                    continue;
                }
                for (a, b) in actual.iter().zip(expected) {
                    if a.x.to_bits() != b["x"].as_f64().unwrap().to_bits()
                        || a.y.to_bits() != b["y"].as_f64().unwrap().to_bits()
                    {
                        mismatches.push(format!(
                            "{} edge {index}: point {a:?} != {b}",
                            input["name"]
                        ));
                    }
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert_eq!(successes, 65);
}
