// SPDX-License-Identifier: MPL-2.0
use d2_rs_language::compiler::import_urls::{html_unescape, is_remote};
pub fn evaluate(request: &str) -> serde_json::Value {
    let bytes: Vec<_> = (0..request.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&request[i..i + 2], 16).unwrap())
        .collect();
    let decoded = html_unescape(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    serde_json::json!({"decoded":decoded,"remote":is_remote(&bytes)})
}
fn main() {
    let requests: Vec<String> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&requests.iter().map(|r| evaluate(r)).collect::<Vec<_>>()).unwrap()
    );
}
