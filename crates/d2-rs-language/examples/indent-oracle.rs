// SPDX-License-Identifier: MPL-2.0
use d2_rs_language::parser::indent::*;
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
struct Request {
    hex: String,
    max: i64,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows: Vec<_> = requests.into_iter().map(|r| {
        let bytes: Vec<u8> = r.hex.as_bytes().as_chunks::<2>().0.iter()
            .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(),16).unwrap()).collect();
        let (indent,remainder) = split_leading_indent(&bytes,r.max);
        json!({"indent":hex(&indent),"remainder":hex(remainder),
            "trim":hex(&trim_indent(&bytes,&vec![b' '; (r.max+1) as usize])),
            "common":hex(&trim_common_indent(&bytes)),"last":hex(&trim_space_after_last_newline(&bytes))})
    }).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
