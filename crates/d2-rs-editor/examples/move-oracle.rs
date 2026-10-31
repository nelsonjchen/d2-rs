// SPDX-License-Identifier: MPL-2.0
//! Capture complete native editor results for the pinned Go move corpus.
use serde::Deserialize;
use std::io::{self, Read};

#[derive(Deserialize)]
struct Input {
    name: String,
    source: String,
    old_key: String,
    new_key: String,
    include_descendants: bool,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    files: std::collections::HashMap<String, String>,
    #[serde(default)]
    board_path: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let inputs: Vec<Input> = serde_json::from_str(&input)?;
    let results = inputs.into_iter().map(|input| {
        let options = d2_rs_editor::MoveOptions {path:input.path.unwrap_or_else(||"-".into()),files:input.files,board_path:input.board_path};
        let result = d2_rs_editor::move_object_with_options(&input.source, &input.old_key, &input.new_key, input.include_descendants,&options);
        match result {
            Ok(result) => serde_json::json!({ "name": input.name, "error": "", "result_raw": serde_json::to_string(&result).unwrap() }),
            Err(error) => serde_json::json!({ "name": input.name, "error": format!("could not move object: {error}"), "result_raw": null }),
        }
    }).collect::<Vec<_>>();
    serde_json::to_writer_pretty(io::stdout(), &results)?;
    Ok(())
}
