// SPDX-License-Identifier: MPL-2.0
//! Input-only observations of original SVG relinking and rendering calls.
use super::*;

#[test]
#[ignore = "executed explicitly by cli-mode-scoreboard.py --suite svg-links"]
fn replay() {
    let requests: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_SVG_LINK_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<_> = requests
        .iter()
        .map(|input| {
            let source_root = input["source_root"].as_str().unwrap();
            let selected = source_root.strip_prefix("root.").unwrap_or("");
            // Go's helper receives local IDs after subtree selection; the
            // production Rust loop uses the corresponding full source IDs.
            let full_id = |local: &str| {
                let suffix = local.strip_prefix("root").unwrap();
                format!("{selected}{suffix}")
                    .trim_start_matches('.')
                    .to_owned()
            };
            let paths = input["paths"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| (full_id(key), PathBuf::from(value.as_str().unwrap())))
                .collect();
            let original = input["board"].clone();
            let mut board = original.clone();
            relink_svg_board(
                &mut board,
                &full_id(input["key"].as_str().unwrap()),
                selected,
                &paths,
            );
            let mut svg = None;
            let mut render_error = String::new();
            if !input["render"].is_null() {
                let options = img::render::Options {
                    format: input["render"]["format"].as_str().unwrap().into(),
                    width: input["render"]["width"].as_i64().unwrap(),
                    height: input["render"]["height"].as_i64().unwrap(),
                    ..Default::default()
                };
                let diagram: Diagram = serde_json::from_value(board.clone()).unwrap();
                match img::render::render_with_context(None, Some(&diagram), Some(&options)) {
                    Ok(bytes) => svg = Some(String::from_utf8(bytes).unwrap()),
                    Err(error) => render_error = error.to_string(),
                }
            }
            serde_json::json!({"board":board,"input_board":original,"error":"",
                "svg":svg,"render_error":render_error})
        })
        .collect();
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var_os("D2RS_SVG_LINK_RESPONSE").unwrap())
        .unwrap();
    serde_json::to_writer(&mut output, &results).unwrap();
}
