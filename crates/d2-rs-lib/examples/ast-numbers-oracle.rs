// SPDX-License-Identifier: MPL-2.0
use d2_tala_rs::ast::*;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Request {
    raw: String,
    numerator: Option<String>,
    denominator: String,
}

fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows: Vec<_> = requests
        .into_iter()
        .map(|request| {
            let number = Number {
                raw: request.raw,
                value: request.numerator.map(|numerator| {
                    Shared::new(BigRational::new(
                        numerator.parse().unwrap(),
                        request.denominator.parse().unwrap(),
                    ))
                }),
                ..Default::default()
            };
            json!({"node": number, "type": number.node_type(), "scalar": number.scalar_string(),
            "children_count": number.children().len()})
        })
        .collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
