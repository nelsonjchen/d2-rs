// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of upstream's original bounded grid-search tests.
use super::*;
use serde_json::Value;

fn evaluate(call: &Value) -> Value {
    let values: Vec<f64> = call["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect();
    let ids = call["ids"].as_array().unwrap();
    assert_eq!(values.len(), ids.len());
    let cuts = best_grid_cuts(
        &values,
        call["groups"].as_u64().unwrap() as usize,
        call["gap"].as_f64().unwrap(),
        call["target"].as_f64().unwrap(),
    );
    let mut start = 0;
    Value::Array(
        cuts.into_iter()
            .chain(std::iter::once(values.len() - 1))
            .map(|cut| {
                let row = Value::Array(ids[start..=cut].to_vec());
                start = cut + 1;
                row
            })
            .collect(),
    )
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_grid_search_regressions() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/grid-search-master.json"
    ))
    .unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 5);
    for call in calls {
        assert_eq!(evaluate(&call["input"]), call["result"], "{}", call["test"]);
    }
}

#[test]
#[ignore = "executed explicitly by grid-search-scoreboard.py"]
fn replay() {
    let calls: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_GRID_SEARCH_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<Value> = calls.iter().map(evaluate).collect();
    std::fs::write(
        std::env::var_os("D2RS_GRID_SEARCH_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}
