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
fn original_go_force_and_stress_geometry() {
    let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_force_stress.json"
    ))
    .unwrap();
    exercise(rows.as_array().unwrap());
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_force_and_stress_expanded_source_successes() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "src/fixtures/original_force_stress_expanded.json"
    ))
    .unwrap();
    let rows = fixture["source_successes"].as_array().unwrap();
    assert_eq!(rows.len(), 118);
    let timeouts = fixture["unqualified_source_timeouts"].as_array().unwrap();
    assert_eq!(timeouts.len(), 14);
    assert!(timeouts.iter().all(|row| row["source_exit"] == 124));
    eprintln!(
        "14 additional original-Go timeout outcomes remain unqualified; they are not geometry matches"
    );
    exercise(rows);
}
fn exercise(rows: &[Value]) {
    let mut failures = vec![];
    let mut records = Vec::new();
    for row in rows {
        assert_eq!(
            row["source_exit"], 0,
            "source failure retained, not a passing geometry control"
        );
        let mut graph: ElkGraph = serde_json::from_value(row["input"].clone()).unwrap();
        let result = if row["name"].as_str().unwrap().starts_with("stress-") {
            crate::layout_elk_graph_with_stress_pipeline(&mut graph, ElkOptions::default())
        } else {
            crate::layout_elk_graph_with_force_pipeline(&mut graph, ElkOptions::default())
        };
        records.push(
            serde_json::json!({"name":row["name"],"actual":serde_json::to_value(&graph).unwrap()}),
        );
        let mismatch = result.map_err(|e| e.to_string()).and_then(|_| {
            compare(
                &serde_json::to_value(graph).unwrap(),
                &row["expected"],
                "root",
            )
        });
        if let Err(error) = mismatch {
            failures.push(format!("{}: {error}", row["name"]));
        }
    }
    if let Ok(path) = std::env::var("D2RS_FORCE_STRESS_RECORDS") {
        std::fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} source differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
