// SPDX-License-Identifier: MPL-2.0
//! Native library source adapter. Oracle results never enter native calls.
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_language::{ParseContextError, compiler::CompileLimits};
use d2_rs_lib::source::{SourceError, SourceOptions, compile_source_ir, parse_source};
use serde_json::{Value, json};

pub fn evaluate(request: &Value) -> Value {
    if request["has_fs"] == true {
        return json!({"unsupported":"filesystem serialization"});
    }
    let options = SourceOptions {
        input_path: request["path"].as_str().unwrap().into(),
        utf16_positions: request["utf16"].as_bool().unwrap(),
        limits: CompileLimits {
            max_variable_expansion: request["max_variable_expansion"].as_i64().unwrap(),
            max_glob_expansion: request["max_glob_expansion"].as_i64().unwrap(),
            max_edge_expansion: request["max_edge_expansion"].as_i64().unwrap(),
            max_edge_expansion_work: request["max_edge_expansion_work"].as_i64().unwrap(),
        },
        ..Default::default()
    };
    let token = CancellationToken::new();
    if request["canceled"] == true {
        token.cancel();
    }
    let input = request["input"].as_str().unwrap().as_bytes();
    match request["op"].as_str().unwrap() {
        "source-parse" => {
            let parsed = parse_source(input, &options, Some(&token));
            json!({"error":parsed.error.as_ref().map(ToString::to_string).unwrap_or_default(),
                "canceled":matches!(parsed.error,Some(ParseContextError::Context(LayoutError::Cancelled))),
                "ast_nil":false, "ast":parsed.map})
        }
        "source-compile" => match compile_source_ir(input, options, Some(&token)) {
            Err(error) => json!({"error":error.to_string(),
                "canceled":matches!(error,SourceError::Context(LayoutError::Cancelled)),
                "diagram_nil":true,"graph_nil":true}),
            // This runner observes admission, not graph/layout/render success.
            Ok(_) => json!({"unsupported":"successful graph/layout/render result"}),
        },
        _ => json!({"unsupported":"unknown operation"}),
    }
}

#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!(
        "{}",
        serde_json::to_string(&requests.iter().map(evaluate).collect::<Vec<_>>())?
    );
    Ok(())
}
