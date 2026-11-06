// SPDX-License-Identifier: MPL-2.0
//! Small JSON bridge for differential testing Markdown substitutions.

use d2_rs_text::markdown::substitute::replace_substitutions;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read},
};

#[derive(Deserialize)]
struct Request {
    #[serde(rename = "Text")]
    text: String,
    #[serde(rename = "Variables")]
    variables: BTreeMap<String, String>,
    #[serde(rename = "Repeats", default)]
    repeats: usize,
}

#[derive(Serialize)]
struct Run {
    output: String,
}

fn main() {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .expect("read requests");
    let requests: Vec<Request> = serde_json::from_str(&input).expect("decode requests");
    let results: Vec<Vec<Run>> = requests
        .into_iter()
        .map(|request| {
            let repeats = request.repeats.max(1);
            (0..repeats)
                .map(|_| Run {
                    output: replace_substitutions(request.text.as_bytes(), &request.variables),
                })
                .collect()
        })
        .collect();
    serde_json::to_writer(io::stdout(), &results).expect("encode results");
}
