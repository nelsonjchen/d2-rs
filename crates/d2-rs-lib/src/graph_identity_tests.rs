// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::public_records_tests::difference;
use super::*;

fn run(input: &Value) -> Result<Vec<u8>, D2Error> {
    let options = D2Options {
        seeds: serde_json::from_value(input["seeds"].clone()).unwrap(),
        max_concurrency: 1,
    };
    let token = CancellationToken::new();
    if input["cancel"].as_bool().unwrap() {
        token.cancel();
    }
    let raw = input["raw_graph"].as_str().unwrap();
    if input["mode"] == "layout" {
        return layout_json_with_cancellation(raw.as_bytes(), &options, &token);
    }
    let graph: Value = serde_json::from_str(raw).unwrap();
    let selected: Vec<usize> = serde_json::from_value(input["requested"].clone()).unwrap();
    let mut requested = graph.clone();
    requested["edges"] = json!(
        selected
            .iter()
            .map(|i| graph["edges"][*i].clone())
            .collect::<Vec<_>>()
    );
    let envelope = json!({"g":BASE64.encode(raw),"gEdges":BASE64.encode(serde_json::to_vec(&requested).unwrap())});
    route_edges_json_with_cancellation(
        &serde_json::to_vec(&envelope).unwrap(),
        &options,
        Some(&token),
    )
}

#[test]
fn complete_public_graph_identity_source_contract() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/graph-identity-source-v090.json"))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 3006);
    let mut differences = Vec::new();
    let mut results = Vec::new();
    let mut successes = 0;
    for row in rows {
        assert!(row.get("panic").is_none());
        let result = run(&row["input"]);
        let message = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let raw = result
            .as_ref()
            .ok()
            .map(|raw| std::str::from_utf8(raw).unwrap());
        let name = &row["input"]["name"];
        if message != row["error"] {
            differences.push(format!("{name}: {message:?}, expected {}", row["error"]));
        } else if let Some(raw) = raw {
            let graph: Value = serde_json::from_str(raw).unwrap();
            if let Some(diff) = difference(&row["graph"], &graph, "$") {
                differences.push(format!("{name}: {diff}"));
            } else {
                successes += 1;
            }
        }
        results.push(json!({"name":name,"error":message,"result_raw":raw}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_GRAPH_IDENTITY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, 747);
}
