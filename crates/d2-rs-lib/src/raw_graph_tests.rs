// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::public_records_tests::difference;
use super::*;

#[test]
fn complete_public_raw_graph_source_contract() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/raw-graph-source-v090.json")).unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 586);
    let mut differences = Vec::new();
    let mut successes = 0;
    let mut results = Vec::new();
    for row in rows {
        assert!(row.get("panic").is_none(), "unexpected source panic");
        let input = &row["input"];
        let options = D2Options {
            seeds: serde_json::from_value(input["seeds"].clone()).unwrap(),
            max_concurrency: 1,
        };
        let cancellation = CancellationToken::new();
        if input["cancel"].as_bool().unwrap() {
            cancellation.cancel();
        }
        let raw_result = layout_json_with_cancellation(
            input["raw_graph"].as_str().unwrap().as_bytes(),
            &options,
            &cancellation,
        );
        let graph_raw = raw_result
            .as_ref()
            .ok()
            .map(|raw| std::str::from_utf8(raw).unwrap().to_owned());
        let result = raw_result.map(|raw| serde_json::from_slice::<Value>(&raw).unwrap());
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
        results.push(json!({"name":name,"error":error,"graph":result.ok(),"graph_raw":graph_raw}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_RAW_GRAPH_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, 115);
}
