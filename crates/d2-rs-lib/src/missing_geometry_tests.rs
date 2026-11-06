// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::wire_geometry_tests::{geometry, run};
use super::*;

#[test]
fn complete_missing_geometry_source_contract() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/missing-geometry-source-v090.json"
    ))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 744);
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
    if let Some(path) = std::env::var_os("D2_RS_TEST_MISSING_GEOMETRY_OUTPUT") {
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
