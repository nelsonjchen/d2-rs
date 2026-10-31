// SPDX-License-Identifier: MPL-2.0
#[path = "../examples/compile-ir-oracle.rs"]
#[allow(dead_code)]
mod oracle;
/// Compares every request with its expected value. The oracle captures are
/// trace runs (`ir-*-trace.json`, recipes `lang-ir-compiler` and
/// `lang-ir-import`); the plain view is the same capture without the `trace`
/// key, which the generators assert against an uninstrumented Go build, so it
/// is not cached twice.
fn compare(input: &str, retain_trace: bool) {
    let fixture: serde_json::Value = serde_json::from_str(input).unwrap();
    let requests = fixture["requests"].as_array().unwrap();
    let expected = fixture["reference"].as_array().unwrap();
    assert_eq!(requests.len(), expected.len());
    for (index, (request, expected)) in requests.iter().zip(expected).enumerate() {
        let mut actual = oracle::evaluate(serde_json::from_value(request.clone()).unwrap());
        let mut expected = expected.clone();
        if !retain_trace {
            actual.as_object_mut().unwrap().remove("trace");
            expected.as_object_mut().unwrap().remove("trace");
        }
        assert_eq!(actual, expected, "compiler case {index}: {request}");
    }
}
#[test]
#[cfg(feature = "oracle-fixtures")]
fn pinned_go_ir_compiler() {
    compare(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/ir-compiler-trace.json"),
        false,
    );
}
#[test]
#[cfg(all(feature = "oracle-fixtures", feature = "compiler-trace"))]
fn pinned_go_ir_compiler_key_replay_trace() {
    compare(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/ir-compiler-trace.json"),
        true,
    );
}
#[test]
#[cfg(feature = "oracle-fixtures")]
fn pinned_go_ir_import_compilation() {
    compare(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/ir-import-trace.json"),
        false,
    );
}
#[test]
#[cfg(all(feature = "oracle-fixtures", feature = "compiler-trace"))]
fn pinned_go_ir_import_key_trace() {
    compare(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/ir-import-trace.json"),
        true,
    );
}
#[test]
#[cfg(feature = "oracle-fixtures")]
fn pinned_go_nil_import_ir_with_unprojectable_ast() {
    compare(
        d2_rs_testdata::oracle_fixture!("tests/fixtures/ir-import-projection.json"),
        false,
    );
}
#[test]
#[cfg(feature = "oracle-fixtures")]
fn reviewed_upstream_nil_import_panics() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/ir-import-panics.json"
    ))
    .unwrap();
    for case in cases {
        assert_eq!(
            oracle::evaluate(serde_json::from_value(case["request"].clone()).unwrap()),
            serde_json::json!({"panic":true}),
            "{}",
            case["request"]
        );
    }
}
