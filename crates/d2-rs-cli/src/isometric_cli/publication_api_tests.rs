// SPDX-License-Identifier: MPL-2.0
//! Input-only transport for the original atomic replacement assertions.
use super::*;
use base64::Engine as _;
#[test]
#[ignore = "invoked by the original Go atomic replacement test"]
fn replay_atomic() {
    let request: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_PUBLICATION_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let token = d2_rs_core::CancellationToken::new();
    if request["canceled"].as_bool().unwrap() {
        token.cancel();
    }
    let _guard = watch::track_cancellation(token);
    let data = base64::engine::general_purpose::STANDARD
        .decode(request["data"].as_str().unwrap())
        .unwrap();
    let result = atomic_image(Path::new(request["path"].as_str().unwrap()), &data);
    std::fs::write(
        std::env::var_os("D2RS_PUBLICATION_RESPONSE").unwrap(),
        serde_json::to_vec(&serde_json::json!({"error":result.err().unwrap_or_default()})).unwrap(),
    )
    .unwrap();
}
