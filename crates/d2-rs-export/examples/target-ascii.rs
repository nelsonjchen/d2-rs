// SPDX-License-Identifier: MPL-2.0
//! Compare source-exported target ASCII without compiling or laying out source.
use d2_rs_render::target::TargetDiagram;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let inputs: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut rows = Vec::new();
    for row in inputs {
        let target: TargetDiagram = serde_json::from_value(row["diagram"].clone())?;
        let options = &row["options"];
        let output = d2_rs_export::ascii_target::render(
            &target.without_children(),
            options["asciiMode"].as_str().unwrap_or("extended"),
            options["scale"].as_f64(),
        );
        rows.push(match output {
            Ok(output) => serde_json::json!({"name":row["name"],"output":output}),
            Err(error) => serde_json::json!({"name":row["name"],"error":error}),
        });
    }
    std::fs::write(&args[2], serde_json::to_vec_pretty(&rows)?)?;
    println!("ASCII target outcomes {}", rows.len());
    Ok(())
}
