// SPDX-License-Identifier: MPL-2.0
use d2_rs_tala::ast::raw_string_box;
use serde::Deserialize;

#[derive(Deserialize)]
struct Request {
    text: String,
    in_key: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin())?;
    let result = requests
        .into_iter()
        .map(|request| raw_string_box(request.text, request.in_key))
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
