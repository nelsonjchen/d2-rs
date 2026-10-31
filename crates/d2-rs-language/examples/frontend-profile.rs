// SPDX-License-Identifier: MPL-2.0
//! Stage timings for the native front end, excluding layout and rendering.
//! Usage: frontend-profile input.d2 [iterations]. Every iteration must agree.
use d2_rs_core::ast::Shared;
use d2_rs_language::{compiler, parse};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("input path")?;
    let iterations: usize = std::env::args().nth(2).unwrap_or("30".into()).parse()?;
    assert!(iterations > 0);
    let source = std::fs::read(&path)?;
    let mut samples = vec![];
    let mut expected = None;
    for _ in 0..iterations {
        let start = Instant::now();
        let parsed = parse(&path, &source, None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let ast = Shared::new(parsed.map);
        let parsed_at = Instant::now();
        let (compiler, root) = compiler::compile_ir(ast.clone())?;
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
        let compiled_at = Instant::now();
        let graph = compiler::source_graph_json(&compiler, root, ast)?;
        let lowered_at = Instant::now();
        if let Some(expected) = &expected {
            assert_eq!(
                &graph, expected,
                "front end output changed between iterations"
            );
        } else {
            expected = Some(graph);
        }
        samples.push([
            (parsed_at - start).as_secs_f64(),
            (compiled_at - parsed_at).as_secs_f64(),
            (lowered_at - compiled_at).as_secs_f64(),
        ]);
    }
    let stages = ["parse", "compile_ir", "source_graph_json"];
    let mut times = serde_json::Map::new();
    for (index, stage) in stages.iter().enumerate() {
        let mut values: Vec<_> = samples.iter().map(|sample| sample[index]).collect();
        values.sort_by(f64::total_cmp);
        times.insert(
            stage.to_string(),
            serde_json::json!(values[values.len() / 2]),
        );
    }
    println!(
        "{}",
        serde_json::json!({"input":path, "iterations":iterations,
        "scope":"native API stages; no layout, actual text measurement or rendering",
        "median_seconds":times, "samples_seconds":samples})
    );
    Ok(())
}
