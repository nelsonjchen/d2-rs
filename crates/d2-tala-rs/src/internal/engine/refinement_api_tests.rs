// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Input-only replay of the original optional-refinement calls. Graph payloads
//! are opaque: this observes acceptance/publication, not layout or serialization.
use super::*;
use serde_json::{Value, json};

fn score(value: &Value) -> evaluation::RaceSelectionScore {
    evaluation::RaceSelectionScore {
        penalty: value["penalty"].as_f64().unwrap(),
        area: value["area"].as_f64().unwrap(),
    }
}

fn replay_call(request: &Value) -> Value {
    let incumbent = request["incumbent"].clone();
    let token = CancellationToken::new();
    if request["context_before"] == "context canceled" {
        token.cancel();
    }
    let result = consider_optional_proposal(Some(&token), || {
        if request["context_after"] == "context canceled" {
            token.cancel();
        }
        if request["panic"].as_bool().unwrap() {
            panic!("original optional routing invariant");
        }
        match request["build_error"].as_str().unwrap() {
            "" => {}
            "context canceled" => return Err(LayoutError::Cancelled),
            "routing: context canceled" => {
                return Err(LayoutError::Context {
                    source: Box::new(LayoutError::Cancelled),
                    context: "routing".into(),
                });
            }
            "context deadline exceeded" => return Err(LayoutError::DeadlineExceeded),
            text => return Err(LayoutError::EngineValidation(text.into())),
        }
        let candidate = request["candidate"].clone();
        // build_compound_candidate performs this same production score comparison
        // before publishing an ordinary proposal. Routing refinement has no gate.
        if request["ordinary"].as_bool().unwrap()
            && score(&candidate).compare(score(&incumbent)) != CandidateOrdering::Less
        {
            Ok(None)
        } else {
            Ok(Some(candidate))
        }
    });
    match result {
        Ok(selected) => json!({"selected": selected.unwrap_or_else(|| incumbent.clone()),
            "error": "", "incumbent_after": incumbent}),
        Err(error) => json!({"selected": null, "error": error.to_string(),
            "incumbent_after": incumbent}),
    }
}

#[test]
#[ignore = "explicit original-call replay; requires input/output paths"]
fn replay() {
    let input = std::env::var_os("D2RS_TALA_REFINEMENT_REQUEST").unwrap();
    let output = std::env::var_os("D2RS_TALA_REFINEMENT_RESPONSE").unwrap();
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let results: Vec<_> = requests.iter().map(replay_call).collect();
    std::fs::write(output, serde_json::to_vec(&results).unwrap()).unwrap();
}
