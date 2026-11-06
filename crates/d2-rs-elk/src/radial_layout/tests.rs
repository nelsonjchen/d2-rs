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
fn original_go_radial_geometry_and_provider_errors() {
    let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_radial.json"
    ))
    .unwrap();
    exercise(rows.as_array().unwrap(), (67, 2));
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_radial_option_combinations() {
    let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_radial_expanded.json"
    ))
    .unwrap();
    exercise(rows.as_array().unwrap(), (81, 0));
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_radial_fractional_sizes_and_labels() {
    let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_radial_fractional.json"
    ))
    .unwrap();
    exercise(rows.as_array().unwrap(), (100, 0));
}
fn exercise(rows: &[Value], expected_counts: (usize, usize)) {
    let mut failures = vec![];
    let mut records = vec![];
    let (mut successes, mut errors) = (0, 0);
    for row in rows {
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        let outcome =
            crate::layout_elk_graph_with_radial_pipeline(&mut graph, ElkOptions::default());
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
            assert_eq!(
                row["source_exit"], 1,
                "source timeout/panic is not a passing error control"
            );
            let source = row["source_stderr"].as_str().unwrap();
            assert_eq!(
                source,
                "layout ELK graph: layout algorithm org.eclipse.elk.radial: the given graph is not a tree\n"
            );
            assert!(
                matches!(outcome,Err(LayeringError::Provider { algorithm: "org.eclipse.elk.radial", ref message }) if message=="the given graph is not a tree"),
                "{}: {:?}",
                row["name"],
                outcome
            );
        }
    }
    if let Ok(path) = std::env::var("D2RS_RADIAL_RECORDS") {
        std::fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap()
    }
    assert_eq!((successes, errors), expected_counts);
    assert!(
        failures.is_empty(),
        "{} source differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
