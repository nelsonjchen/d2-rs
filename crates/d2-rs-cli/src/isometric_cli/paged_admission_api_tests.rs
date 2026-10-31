// SPDX-License-Identifier: MPL-2.0
//! Input-only transport for TestIsometricPagedAdmissionAndBoundedWriter.
use super::*;
use base64::Engine as _;
use serde_json::json;
use std::io::Write as _;

#[test]
#[ignore = "executed explicitly by the original paged-admission scoreboard"]
fn replay() {
    let requests: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_PAGED_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let mut results = Vec::new();
    for input in requests {
        let token = d2_rs_core::CancellationToken::new();
        if input["canceled"].as_bool().unwrap() {
            token.cancel();
        }
        let _guard = watch::track_cancellation(token);
        let result = match input["kind"].as_str().unwrap() {
            "page" => {
                let b = input["budget"].as_array().unwrap();
                let mut budget = Budget {
                    pixels: b[0].as_i64().unwrap(),
                    encoded: b[1].as_u64().unwrap() as usize,
                    links: b[2].as_u64().unwrap() as usize,
                    link_bytes: b[3].as_u64().unwrap() as usize,
                };
                let page = &input["page"];
                let page = img::page::Page {
                    width: page["Width"].as_i64().unwrap(),
                    height: page["Height"].as_i64().unwrap(),
                    png: base64::engine::general_purpose::STANDARD
                        .decode(page["PNG"].as_str().unwrap())
                        .unwrap(),
                    links: serde_json::from_value(page["Links"].clone()).unwrap(),
                };
                let error = budget
                    .admit(
                        &page,
                        input["width"].as_i64().unwrap(),
                        input["height"].as_i64().unwrap(),
                    )
                    .err()
                    .unwrap_or_default();
                json!({"error":error,"budget":[budget.pixels,budget.encoded as i64,budget.links as i64,budget.link_bytes as i64]})
            }
            "write" => {
                let mut writer = DocumentWriter {
                    output: Vec::new(),
                    remaining: input["remaining"].as_u64().unwrap() as usize,
                };
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(input["data"].as_str().unwrap())
                    .unwrap();
                let (n, error) = match writer.write(&bytes) {
                    Ok(n) => (n, String::new()),
                    Err(error) => (0, error.to_string()),
                };
                // Record every output byte independently; a failed write cannot
                // pass merely by returning the expected error.
                json!({"error":error,"remaining":writer.remaining,"n":n,"output":base64::engine::general_purpose::STANDARD.encode(&writer.output)})
            }
            other => panic!("unknown original operation {other}"),
        };
        results.push(result);
    }
    std::fs::write(
        std::env::var_os("D2RS_PAGED_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}

#[test]
fn owned_document_admission_preserves_bytes_and_context_precedence() {
    let bytes = vec![1, 2, 3];
    let allocation = bytes.as_ptr();
    let admitted = DocumentWriter {
        output: Vec::new(),
        remaining: 3,
    }
    .admit_owned(bytes)
    .unwrap();
    assert_eq!(admitted, [1, 2, 3]);
    assert_eq!(admitted.as_ptr(), allocation);
    let error = DocumentWriter {
        output: Vec::new(),
        remaining: 2,
    }
    .admit_owned(admitted)
    .unwrap_err();
    assert!(error.to_string().contains("output limit"));
    let token = d2_rs_core::CancellationToken::new();
    token.cancel();
    let _guard = watch::track_cancellation(token);
    assert_eq!(
        DocumentWriter {
            output: Vec::new(),
            remaining: 0
        }
        .admit_owned(vec![1])
        .unwrap_err()
        .to_string(),
        "context canceled"
    );
}
