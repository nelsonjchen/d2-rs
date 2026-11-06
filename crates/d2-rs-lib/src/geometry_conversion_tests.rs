// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::wire_geometry_tests::{geometry, run};
use super::*;

fn wire_fields(graph: &Value) -> Value {
    let fields = |record: &Value, field: &str| -> Value {
        Value::Object(
            record
                .as_object()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(field))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    };
    json!({
        "boxes": graph["objects"].as_array().unwrap().iter()
            .map(|o| json!({"id":o["AbsID"],"geometry":fields(o,"box")})).collect::<Vec<_>>(),
        "routes": graph["edges"].as_array().unwrap().iter()
            .map(|e| fields(e,"route")).collect::<Vec<_>>(),
    })
}

#[test]
fn complete_geometry_conversion_source_contract() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/geometry-conversion-source-v090.json"
    ))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 2168);
    let mut differences = Vec::new();
    let mut successes = 0;
    let mut results = Vec::new();
    for row in rows {
        let result = run(&row["input"]);
        let error = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let name = &row["input"]["name"];
        if error != row["error"] {
            differences.push(format!(
                "{name}: error {error:?}, expected {}",
                row["error"]
            ));
        } else if let Ok(actual) = &result {
            successes += 1;
            if wire_fields(actual) != wire_fields(&row["graph"]) {
                differences.push(format!(
                    "{name}: serialized geometry {} != {}",
                    wire_fields(actual),
                    wire_fields(&row["graph"])
                ));
            }
            match std::panic::catch_unwind(|| geometry(actual)) {
                Ok(actual_geometry) if actual_geometry == geometry(&row["graph"]) => {}
                Ok(actual_geometry) => differences.push(format!(
                    "{name}: geometry {actual_geometry} != {}",
                    geometry(&row["graph"])
                )),
                Err(_) => {
                    differences.push(format!("{name}: incomplete geometry inventory: {actual}"))
                }
            }
        }
        results.push(json!({"name":name,"error":error,"graph":result.ok()}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_GEOMETRY_CONVERSION_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, rows.iter().filter(|r| r["error"] == "").count());
}
