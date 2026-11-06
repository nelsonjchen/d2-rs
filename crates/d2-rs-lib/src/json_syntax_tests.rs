// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::public_records_tests::difference;
use super::*;

// Decode each graph as raw JSON so the source's nested-data controls are not
// restricted by serde's default Value recursion limit in the test harness.
#[derive(Deserialize)]
struct Fixture {
    cases: Vec<SourceRow>,
}
#[derive(Deserialize)]
struct SourceRow {
    input: Value,
    error: Option<String>,
    panic: Option<String>,
    graph: Option<Box<serde_json::value::RawValue>>,
}
fn graph_value(raw: &serde_json::value::RawValue) -> Value {
    use serde_json::value::RawValue;
    match raw.get().as_bytes()[0] {
        b'{' => {
            let entries: BTreeMap<String, Box<RawValue>> = serde_json::from_str(raw.get()).unwrap();
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, graph_value(&value)))
                    .collect(),
            )
        }
        b'[' => {
            let entries: Vec<Box<RawValue>> = serde_json::from_str(raw.get()).unwrap();
            Value::Array(entries.iter().map(|value| graph_value(value)).collect())
        }
        _ => serde_json::from_str(raw.get()).unwrap(),
    }
}

#[test]
fn complete_public_json_syntax_source_contract() {
    let fixture: Fixture = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/json-syntax-source-v090.json"
    ))
    .unwrap();
    let cases = &fixture.cases;
    assert_eq!(cases.len(), 3820);
    let mut differences = Vec::new();
    let mut results = Vec::new();
    let (mut successes, mut byte_cases, mut source_panics) = (0, 0, 0);
    for row in cases {
        let input = &row.input;
        let bytes = if let Some(raw) = input["raw_bytes"].as_str() {
            byte_cases += 1;
            BASE64.decode(raw).unwrap()
        } else {
            input["raw_graph"].as_str().unwrap().as_bytes().to_vec()
        };
        let options = D2Options {
            seeds: vec![1],
            max_concurrency: 1,
        };
        let token = CancellationToken::new();
        if input["cancel"].as_bool().unwrap() {
            token.cancel();
        }
        let result = layout_json_with_cancellation(&bytes, &options, &token);
        let message = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let raw = result
            .as_ref()
            .ok()
            .map(|raw| std::str::from_utf8(raw).unwrap());
        let name = &input["name"];
        // Preserve and expose the source panic as an open compatibility gap;
        // the safe native rejection is not counted as a passing source match.
        if row.panic.is_some() {
            source_panics += 1;
            assert_eq!(bytes, b"null");
            assert!(matches!(result, Err(D2Error::RootType)));
        } else if Some(message.as_str()) != row.error.as_deref() {
            differences.push(format!(
                "{name}: {message:?}, expected {}",
                row.error.as_deref().unwrap()
            ));
        } else if let Some(raw) = raw {
            let raw: Box<serde_json::value::RawValue> = serde_json::from_str(raw).unwrap();
            let graph = graph_value(&raw);
            if let Some(diff) = difference(&graph_value(row.graph.as_ref().unwrap()), &graph, "$") {
                differences.push(format!("{name}: {diff}"));
            } else {
                successes += 1;
            }
        }
        results
            .push(json!({"name":name,"error":message,"result_raw":raw,"source_panic":row.panic}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_JSON_SYNTAX_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert_eq!(source_panics, 2);
    assert_eq!(byte_cases, 1096);
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, 576);
}
