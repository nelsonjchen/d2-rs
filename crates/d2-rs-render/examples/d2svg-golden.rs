// SPDX-License-Identifier: MPL-2.0
//! Render upstream e2e `board.exp.json` targets with the d2svg port.
//!
//! Input: a JSON manifest `[{"input": path, "output": path, "theme_id": n?}]`.
//! Each board is rendered as the e2e test does (pad 0, the case theme, the
//! diagram config applied like d2lib.applyConfigs). Errors are written to
//! `<output>.err` so the comparison script can report them per board.

fn main() {
    let manifest = std::env::args().nth(1).expect("usage: d2svg-golden <manifest.json>");
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(manifest).expect("read manifest")).expect("manifest");
    for row in rows {
        let input = row["input"].as_str().expect("input");
        let output = row["output"].as_str().expect("output");
        let theme_id = row["theme_id"].as_i64();
        let result = std::fs::read(input)
            .map_err(|error| error.to_string())
            .and_then(|bytes| d2_rs_render::render_d2svg_e2e(&bytes, theme_id));
        match result {
            Ok(svg) => std::fs::write(output, svg).expect("write svg"),
            Err(error) => std::fs::write(format!("{output}.err"), error).expect("write error"),
        }
    }
}
