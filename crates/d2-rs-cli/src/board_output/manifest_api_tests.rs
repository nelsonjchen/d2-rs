// SPDX-License-Identifier: MPL-2.0
use super::*;

fn observe(calls: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let scratch = std::env::temp_dir().join(format!("d2rs-manifest-api-{}", std::process::id()));
    std::fs::create_dir(&scratch).unwrap();
    let results = calls
        .iter()
        .map(|call| {
            std::fs::write(
                scratch.join(BOARD_OUTPUT_MANIFEST_NAME),
                serde_json::to_vec(&call["manifest"]).unwrap(),
            )
            .unwrap();
            let error = Manifest::read(&scratch).err().unwrap_or_default();
            serde_json::json!({"error": error})
        })
        .collect();
    std::fs::remove_dir_all(scratch).unwrap();
    results
}

#[test]
#[ignore = "input-only external API replay"]
fn replay_manifest_calls() {
    let request = std::env::var_os("D2RS_MANIFEST_REQUEST").expect("request path");
    let output = std::env::var_os("D2RS_MANIFEST_RESPONSE").expect("response path");
    let calls: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    std::fs::write(output, serde_json::to_vec(&observe(&calls)).unwrap()).unwrap();
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_manifest_admission() {
    let fixture: serde_json::Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/board-manifest-api.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert!(!calls.is_empty());
    let expected: Vec<_> = calls
        .iter()
        .map(|call| serde_json::json!({"error": call["error"]}))
        .collect();
    assert_eq!(observe(calls), expected);
}
