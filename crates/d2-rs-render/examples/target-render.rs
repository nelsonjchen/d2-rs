// SPDX-License-Identifier: MPL-2.0
//! Probe public target rendering without any compiler or layout call.
use d2_rs_render::{RenderOptions, target::TargetDiagram};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let out = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(out)?;
    let mut report = Vec::new();
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let diagram: TargetDiagram = serde_json::from_value(row["diagram"].clone())?;
        let diagram = diagram.without_children();
        let hash = d2_rs_render::target_hash_id_from_input(&diagram.hash_input()?, None);
        let options = RenderOptions {
            pad: row["options"]["pad"].as_f64().unwrap_or(100.),
            scale: row["options"]["scale"].as_f64(),
            center: row["options"]["center"].as_bool().unwrap_or(false),
            theme_id: row["options"]["themeID"].as_i64(),
            dark_theme_id: row["options"]["darkThemeID"].as_i64(),
            theme_overrides: row["options"]["themeOverrides"]
                .as_object()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|(k, v)| {
                            v.as_str().map(|v| (k.to_ascii_uppercase(), v.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            dark_theme_overrides: row["options"]["darkThemeOverrides"]
                .as_object()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|(k, v)| {
                            v.as_str().map(|v| (k.to_ascii_uppercase(), v.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ..Default::default()
        };
        let sketch = row["options"]["sketch"].as_bool().unwrap_or(false);
        match diagram.render_d2svg(&options, sketch, false, 0, false) {
            Ok(bytes) => {
                std::fs::write(out.join(format!("{name}.svg")), bytes)?;
                report.push(serde_json::json!({"name":name,"hash":hash,"success":true}));
            }
            Err(error) => report
                .push(serde_json::json!({"name":name,"hash":hash,"success":false,"error":error})),
        }
    }
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{} targets", report.len());
    Ok(())
}
