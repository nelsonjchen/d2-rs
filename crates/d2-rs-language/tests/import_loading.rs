// SPDX-License-Identifier: MPL-2.0
// This fixture uses the pinned Unix oracle. Windows filepath.IsAbs qualification
// requires corresponding Windows reference observations.
#![cfg(unix)]
#![cfg(feature = "oracle-fixtures")]
#[path = "../examples/import-loading-oracle.rs"]
#[allow(dead_code)]
mod oracle;
#[test]
fn import_path_and_parsed_cache_match_pinned_go() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/import-loading.json"
    ))
    .unwrap();
    let requests = fixture["requests"].as_array().unwrap();
    let reference = fixture["reference"].as_array().unwrap();
    assert_eq!(requests.len(), reference.len());
    for (i, (request, expected)) in requests.iter().zip(reference).enumerate() {
        assert_eq!(
            &oracle::evaluate(serde_json::from_value(request.clone()).unwrap()),
            expected,
            "case {i}: {request}"
        );
    }
}
