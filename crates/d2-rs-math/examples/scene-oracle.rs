// SPDX-License-Identifier: MPL-2.0
//! Batch source-SVG scene contract qualification; not a production runtime.
fn main() {
    let rows: Vec<serde_json::Value> = serde_json::from_reader(std::io::stdin()).unwrap();
    let out:Vec<_>=rows.into_iter().map(|row| {
        let scene=d2_rs_math::import_scene_svg(row["svg"].as_str().unwrap());
        serde_json::json!({"expression":row["expression"],"svg":scene.as_ref().ok(),"error":scene.err().unwrap_or_default()})
    }).collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}
