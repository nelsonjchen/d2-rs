// SPDX-License-Identifier: MPL-2.0
//! Development-only probe for RaceSeeds selection diagnostics.

use d2_rs_lib::{D2Options, layout_json_with_report};
use serde_json::json;
use std::io::Read;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--tala-seeds" {
        return Err("usage: tala-seed-report --tala-seeds 1[,2,...] < measured-graph.json".into());
    }
    let seeds = args[1]
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<i64>, _>>()?;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let output = layout_json_with_report(
        &input,
        &D2Options {
            seeds,
            max_concurrency: 1,
        },
    )?;
    println!(
        "{}",
        json!({
            "selected_seed": output.report.selected_seed,
            "candidates": output.report.candidates,
            "warnings": output.report.warnings,
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
