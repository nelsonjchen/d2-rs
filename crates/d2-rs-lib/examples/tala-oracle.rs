// SPDX-License-Identifier: MPL-2.0
//! Development-only serialized measured-graph oracle. This is not the D2 CLI.
use d2_rs_lib::{D2Options, layout_json, route_edges_json};
use std::io::{Read, Write};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 || args[1] != "--tala-seeds" {
        return Err(
            "usage: tala-oracle {layout|routeedges} --tala-seeds 1[,2,...] < measured-graph.json"
                .into(),
        );
    }
    let seeds = args[2]
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<i64>, _>>()?;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let options = D2Options {
        seeds,
        max_concurrency: 0,
    };
    let output = match args[0].as_str() {
        "layout" => layout_json(&input, &options)?,
        "routeedges" => route_edges_json(&input, &options)?,
        _ => return Err("unknown oracle operation".into()),
    };
    std::io::stdout().write_all(&output)?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
