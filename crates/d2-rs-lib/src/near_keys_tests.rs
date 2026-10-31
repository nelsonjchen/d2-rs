// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use super::{public_records_tests::difference, wire_geometry_tests::run};

#[test]
fn complete_public_near_keys_source_contract() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/near-keys-source-v090.json")).unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1936);
    let mut differences = Vec::new();
    let mut successes = 0;
    let mut results = Vec::new();
    for row in rows {
        assert!(row.get("panic").is_none(), "unexpected source panic");
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
            if let Some(diff) = difference(&row["graph"], actual, "$") {
                differences.push(format!("{name}: {diff}"));
            } else {
                successes += 1;
            }
        }
        results.push(json!({"name":name,"error":error,"graph":result.ok()}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_NEAR_KEYS_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, 489);
}
