// SPDX-License-Identifier: MPL-2.0
//! Batch boundary used by the v0.9.0 MathJax qualification harness.
//!
//! This is deliberately an example rather than a production dependency: it
//! keeps the corpus comparator single-process and makes the exact SVG emitted
//! by the Rust boundary inspectable without changing the public CLI protocol.

use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Request {
    expression: String,
}

fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows = requests
        .into_iter()
        .map(|request| {
            let measured = d2_rs_math::measure(&request.expression);
            let rendered = d2_rs_math::render(&request.expression);
            let exact = d2_rs_math::render_exact(&request.expression);
            json!({
                "expression": request.expression,
                "width": measured.as_ref().map(|size| size.width).unwrap_or(0.0),
                "height": measured.as_ref().map(|size| size.height).unwrap_or(0.0),
                "svg": rendered.as_ref().ok(),
                "measure_error": measured.err().map(|error| error.to_string()),
                "render_error": rendered.err().map(|error| error.to_string()),
                "exact_svg": exact.as_ref().ok(),
                "exact_error": exact.err().map(|error| error.to_string()),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
