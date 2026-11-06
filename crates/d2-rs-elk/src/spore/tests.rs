// Copyright 2026 Nelson Chen. MPL-2.0.
use super::*;
fn compare(actual: &Value, expected: &Value, path: &str) -> Result<(), String> {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if a.to_bits() != b.to_bits() {
                return Err(format!(
                    "{path}: {a:?} [{:016x}] != {b:?} [{:016x}]",
                    a.to_bits(),
                    b.to_bits()
                ));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!("{path}: inventory {} != {}", a.len(), b.len()));
            }
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"))?;
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            const KEYS: &[&str] = &[
                "x",
                "y",
                "width",
                "height",
                "id",
                "text",
                "container",
                "sources",
                "targets",
                "children",
                "edges",
                "ports",
                "labels",
                "sections",
                "startPoint",
                "endPoint",
                "bendPoints",
                "junctionPoints",
                "incomingShape",
                "outgoingShape",
            ];
            for key in KEYS {
                let a = a.get(*key);
                let b = b.get(*key);
                match (a, b) {
                    (Some(a), Some(b)) => compare(a, b, &format!("{path}.{key}"))?,
                    (None, None) => {}
                    _ => return Err(format!("{path}.{key}: field presence {a:?} != {b:?}")),
                }
            }
        }
        _ => {
            if actual != expected {
                return Err(format!("{path}: {actual} != {expected}"));
            }
        }
    }
    Ok(())
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_spore_geometry() {
    exercise(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_spore.json"),
        (82, 0),
        "D2RS_SPORE_INITIAL_RECORDS",
    )
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_spore_fractional_delegation_and_sections() {
    exercise(
        d2_rs_testdata::oracle_fixture!("src/fixtures/original_spore_expanded.json"),
        (144, 2),
        "D2RS_SPORE_EXPANDED_RECORDS",
    )
}
fn exercise(text: &str, want: (usize, usize), record_key: &str) {
    let rows: Value = serde_json::from_str(text).unwrap();
    let mut failures = vec![];
    let mut records = vec![];
    let (mut successes, mut errors) = (0, 0);
    for row in rows.as_array().unwrap() {
        #[cfg(feature = "diagnostic-traces")]
        TRACE_ACTIVE.with(|v| {
            v.set(
                std::env::var("D2RS_SPORE_TRACE_FILTER")
                    .ok()
                    .is_some_and(|names| names.split(',').any(|n| Some(n) == row["name"].as_str())),
            )
        });

        #[cfg(feature = "diagnostic-traces")]
        if std::env::var_os("D2RS_SPORE_TRACE").is_some() {
            eprintln!(
                "{}",
                serde_json::json!({"trace":"spore-case","name":row["name"]})
            );
        }

        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        let alg = ElkAlgorithm::parse(
            row["input"]["layoutOptions"]["elk.algorithm"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let outcome = if alg == ElkAlgorithm::SporeOverlap {
            crate::layout_elk_graph_with_spore_overlap_pipeline(&mut graph, ElkOptions::default())
        } else {
            crate::layout_elk_graph_with_spore_compaction_pipeline(
                &mut graph,
                ElkOptions::default(),
            )
        };
        records.push(serde_json::json!({"name":row["name"],"actual":graph,"error":outcome.as_ref().err().map(ToString::to_string)}));
        if row["source_exit"] == 0 {
            successes += 1;
            let mismatch = outcome.map_err(|e| e.to_string()).and_then(|_| {
                compare(
                    &serde_json::to_value(graph).unwrap(),
                    &row["expected"],
                    "root",
                )
            });
            if let Err(e) = mismatch {
                failures.push(format!("{}: {e}", row["name"]))
            }
        } else {
            errors += 1;
            assert_eq!(row["source_exit"], 1);
            match outcome {
                Err(LayeringError::Provider { algorithm, message }) => assert_eq!(
                    row["source_stderr"],
                    format!("layout ELK graph: layout algorithm {algorithm}: {message}\n")
                ),
                other => panic!(
                    "{}: expected source provider rejection, got {other:?}",
                    row["name"]
                ),
            }
        }
    }
    if let Ok(path) = std::env::var(record_key) {
        std::fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap()
    }
    assert_eq!((successes, errors), want);
    assert!(
        failures.is_empty(),
        "{} source differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_layered_root_transfer_minimum_and_fixed_dimensions() {
    let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_spore_root_transfer.json"
    ))
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 6);
    for row in rows.as_array().unwrap() {
        assert_eq!(row["source_exit"], 0);
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        crate::layout_elk_graph_with_layered_pipeline(
            &mut graph,
            ElkOptions {
                algorithm: ElkAlgorithm::Layered,
                node_spacing: 20.,
                edge_node_spacing: 10.,
                edge_edge_spacing: 10.0,
                padding: 12.,
                self_loop_spacing: 10.,
            },
        )
        .unwrap();
        compare(
            &serde_json::to_value(graph).unwrap(),
            &row["expected"],
            "root",
        )
        .unwrap();
    }
}
