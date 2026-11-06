// SPDX-License-Identifier: MPL-2.0
#[test]
fn master_urlenc_scoreboard() {
    if let Ok(path) = std::env::var("D2RS_REMAINING_REQUEST") {
        let request: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let result = evaluate(&request);
        std::fs::write(
            std::env::var("D2RS_REMAINING_RESULT").unwrap(),
            serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();
    }
}

fn evaluate(request: &serde_json::Value) -> serde_json::Value {
    let input = request["input"].as_str().unwrap();
    let result = match request["op"].as_str().unwrap() {
        "encode" => Ok(d2_rs_lib::urlenc::encode(input)),
        "decode" => d2_rs_lib::urlenc::decode(input),
        _ => panic!("unknown URL codec operation"),
    };
    let got = match result {
        Ok(value) => serde_json::json!({"result":value,"error":""}),
        Err(error) => serde_json::json!({"result":"","error":error}),
    };
    serde_json::json!({"got":got,"observations":null})
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_master_size_limit() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-urlenc.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut request = case["request"].clone();
        if request["op"] == "encode" {
            request["input"] = serde_json::json!(
                request["repeat"]
                    .as_str()
                    .unwrap()
                    .repeat(request["count"].as_u64().unwrap() as usize)
            );
        }
        assert_eq!(evaluate(&request)["got"], case["got"], "{}", case["name"]);
    }
}
