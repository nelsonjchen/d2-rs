// SPDX-License-Identifier: MPL-2.0
use d2_rs_language::parser::numeric::parse_rational;
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
struct Request {
    hex: String,
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows: Vec<_> = requests
        .into_iter()
        .map(|r| {
            let bytes: Vec<u8> = r
                .hex
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
                .collect();
            let rational = std::str::from_utf8(&bytes).ok().and_then(parse_rational);
            json!({"accepted":rational.is_some(),"rational":rational.map(|r|r.to_string())})
        })
        .collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
