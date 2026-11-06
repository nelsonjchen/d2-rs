// SPDX-License-Identifier: MPL-2.0
//! Diagnostic graphjson reconstruction and layout adapter.
//!
//! This is intentionally separate from the public plugin protocol. It accepts
//! the internal v0.9.0 graphjson fixture format on stdin, reconstructs the
//! shared graph model, and serializes the completed live graph. The old compact
//! geometry projection remains below only for its existing regression tests.

#[cfg(all(test, feature = "oracle-fixtures"))]
fn layout(input: &[u8], seed: i64) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use d2_rs_tala::{LabelPosition, LayoutStage, Point, diagnostic};
    use serde::Serialize;
    use std::collections::BTreeMap;

    #[derive(Serialize)]
    struct OutputNode {
        id: i64,
        width: f64,
        height: f64,
        top_left: Option<Point>,
        label: Option<OutputLabel>,
        icon: Option<OutputIcon>,
    }
    #[derive(Serialize)]
    struct OutputEdge {
        id: i64,
        from: i64,
        to: i64,
        points: Vec<Point>,
        label_percentage: f64,
        label: Option<OutputLabel>,
    }
    #[derive(Serialize)]
    struct OutputLabel {
        position: String,
        width: f64,
        height: f64,
    }
    #[derive(Serialize)]
    struct OutputIcon {
        position: String,
    }

    let graph = d2_rs_tala::graphjson::graphjson_to_graph(input)?;
    // The Go graphjson oracle calls internal/engine.Layout for one seed. Use
    // the corresponding Rust single-seed boundary; Engine::layout adds D2's
    // post-selection CompoundCandidate adapter pass after this point.
    let snapshot = diagnostic::layout_snapshot(&graph, seed, LayoutStage::Normalize);
    if let Some(error) = snapshot.layout_error() {
        return Err(error.clone().into());
    }
    let node_states = snapshot
        .nodes
        .iter()
        .map(|state| (state.node, state))
        .collect::<BTreeMap<_, _>>();
    let edge_states = snapshot
        .edges
        .iter()
        .map(|state| (state.edge, state))
        .collect::<BTreeMap<_, _>>();
    let nodes = graph
        .nodes()
        .map(|(dense, node)| {
            let id = graph.tala_id_overrides[dense.0 as usize].unwrap_or(dense.0 as i64);
            let state = node_states[&dense];
            let label = state.label_size.map(|size| OutputLabel {
                position: state.label_position.d2_name().to_owned(),
                width: size.width,
                height: size.height,
            });
            OutputNode {
                id,
                width: state.rect.size.width,
                height: state.rect.size.height,
                top_left: state.position,
                label,
                icon: node.has_icon.then(|| OutputIcon {
                    position: state
                        .icon_position
                        .unwrap_or(LabelPosition::Unset)
                        .d2_name()
                        .to_owned(),
                }),
            }
        })
        .collect::<Vec<_>>();
    let edges = graph
        .edges()
        .filter_map(|(dense, _edge)| {
            let state = edge_states.get(&dense)?;
            let label = state.label.as_ref().map(|label| OutputLabel {
                position: label.position.d2_name().to_owned(),
                width: label.size.width,
                height: label.size.height,
            });
            let label_percentage = state.label.as_ref().map_or(0.0, |label| label.percentage);
            Some(OutputEdge {
                id: graph.tala_edge_ids[dense.0 as usize].unwrap_or(dense.0 as i64),
                from: graph.tala_id_overrides[state.source_node.0 as usize]
                    .unwrap_or(state.source_node.0 as i64),
                to: graph.tala_id_overrides[state.target_node.0 as usize]
                    .unwrap_or(state.target_node.0 as i64),
                points: state.points.clone(),
                label_percentage,
                label,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_vec(&serde_json::json!({
        "nodes": nodes,
        "edges": edges,
    }))?)
}

fn run() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let seed = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".to_owned())
        .parse::<i64>()?;
    d2_rs_tala::graphjson::graphjson_layout(&input, seed)
}

fn main() {
    match run() {
        Ok(output) => print!("{}", String::from_utf8_lossy(&output)),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    #[test]
    fn original_stage_dimension_errors_are_not_published_as_geometry() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/graphjson-layout-errors-source-v090.json"
        ))
        .unwrap();
        let cases = cases.as_array().unwrap();
        assert_eq!(cases.len(), 16);
        for case in cases {
            let input = serde_json::to_vec(&case["input"]).unwrap();
            let result = super::layout(&input, case["seed"].as_i64().unwrap());
            if case["source_exit"] == 0 {
                let output = result.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
                let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
                assert_eq!(output["nodes"].as_array().unwrap().len(), 1);
            } else {
                let error = result.expect_err("failed layout must not publish a partial snapshot");
                assert_eq!(
                    format!("{error}\n"),
                    case["source_stderr"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
    }
    #[test]
    fn original_cluster_hub_retry_and_transpose_keep_live_owner_context() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cluster-hub-owner-source-v090.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let input = serde_json::to_vec(&case["input"]).unwrap();
            let output = super::layout(&input, case["seed"].as_i64().unwrap()).unwrap();
            let native: serde_json::Value = serde_json::from_slice(&output).unwrap();
            for (source_key, native_key) in [("nodes", "nodes"), ("edges", "edges")] {
                let original = case["source"][source_key].as_array().unwrap();
                let actual = native[native_key].as_array().unwrap();
                assert_eq!(original.len(), actual.len());
                for (source, native) in original.iter().zip(actual) {
                    let fields: &[(&str, &str)] = if source_key == "nodes" {
                        &[
                            ("id", "id"),
                            ("width", "width"),
                            ("height", "height"),
                            ("topLeft", "top_left"),
                        ]
                    } else {
                        &[
                            ("id", "id"),
                            ("from", "from"),
                            ("to", "to"),
                            ("points", "points"),
                            ("labelPercentage", "label_percentage"),
                        ]
                    };
                    for &(source_key, native_key) in fields {
                        assert_exact_number_tree(&source[source_key], &native[native_key]);
                    }
                }
            }
        }
    }

    #[test]
    fn original_container_direct_geometry_survives_later_independent_components() {
        let case: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/container-direct-owner-source-v090.json"
        ))
        .unwrap();
        assert_individual_geometry(&case);
    }

    #[test]
    fn original_container_refit_keeps_children_in_place_for_later_medians() {
        let case: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/container-refit-owner-source-v090.json"
        ))
        .unwrap();
        assert_individual_geometry(&case);
    }

    #[test]
    fn original_cluster_restores_each_member_edge_to_the_container_child() {
        let case: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cluster-owner-restore-source-v090.json"
        ))
        .unwrap();
        assert_individual_geometry(&case);
    }

    #[test]
    fn original_nested_packing_ignores_retired_vessel_obstacles() {
        let case: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/packing-member-obstacles-source-v090.json"
        ))
        .unwrap();
        assert_individual_geometry(&case);
    }

    #[test]
    fn original_table_port_balancing_ignores_retired_vessel_obstacles() {
        let case: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/table-port-member-obstacles-source-v090.json"
        ))
        .unwrap();
        assert_individual_geometry(&case);
    }

    fn assert_individual_geometry(case: &serde_json::Value) {
        let output = super::layout(
            &serde_json::to_vec(&case["input"]).unwrap(),
            case["seed"].as_i64().unwrap(),
        )
        .unwrap();
        let native: serde_json::Value = serde_json::from_slice(&output).unwrap();
        // This contract is acceptance and individual geometry/routes. The
        // separate complete-output gate retains the known node/edge order
        // and metadata failures; this test does not qualify serialization.
        for kind in ["nodes", "edges"] {
            let actual = native[kind].as_array().unwrap();
            let original = case["source"][kind].as_array().unwrap();
            assert_eq!(actual.len(), original.len());
            let by_id = actual
                .iter()
                .map(|n| (n["id"].as_i64().unwrap(), n))
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(by_id.len(), actual.len());
            for source in original {
                let n = by_id[&source["id"].as_i64().unwrap()];
                let fields: &[(&str, &str)] = if kind == "nodes" {
                    &[
                        ("width", "width"),
                        ("height", "height"),
                        ("topLeft", "top_left"),
                    ]
                } else {
                    &[
                        ("from", "from"),
                        ("to", "to"),
                        ("points", "points"),
                        ("labelPercentage", "label_percentage"),
                    ]
                };
                for &(sk, nk) in fields {
                    assert_exact_number_tree(&source[sk], &n[nk]);
                }
            }
        }
    }

    fn assert_exact_number_tree(source: &serde_json::Value, native: &serde_json::Value) {
        use serde_json::Value;
        match (source, native) {
            (Value::Number(a), Value::Number(b)) => {
                if a.is_i64() && b.is_i64() {
                    assert_eq!(a.as_i64(), b.as_i64());
                } else {
                    assert_eq!(a.as_f64().unwrap().to_bits(), b.as_f64().unwrap().to_bits());
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                assert_eq!(a.len(), b.len());
                for (a, b) in a.iter().zip(b) {
                    assert_exact_number_tree(a, b);
                }
            }
            (Value::Object(a), Value::Object(b)) => {
                assert_eq!(a.len(), b.len());
                for (key, a) in a {
                    assert_exact_number_tree(a, &b[key]);
                }
            }
            _ => assert_eq!(source, native),
        }
    }
}
