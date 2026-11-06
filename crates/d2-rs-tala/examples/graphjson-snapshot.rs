// SPDX-License-Identifier: MPL-2.0
//! GraphJSON stage snapshot helper.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use d2_rs_tala::{LayoutStage, diagnostic};
    use std::io::Read;

    let mut args = std::env::args().skip(1);
    let stage = args
        .next()
        .ok_or("usage: graphjson-snapshot STAGE [SEED] < graph.input.json")?;
    let seed = args
        .next()
        .map_or(Ok(1_i64), |value| value.parse::<i64>())?;
    let stage: LayoutStage = serde_json::from_str(&format!("\"{stage}\""))?;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let graph = d2_rs_tala::graphjson::graphjson_to_graph(&input)?;
    let snapshot = diagnostic::layout_snapshot(&graph, seed, stage);
    println!("{}", serde_json::to_string(&snapshot)?);
    Ok(())
}
