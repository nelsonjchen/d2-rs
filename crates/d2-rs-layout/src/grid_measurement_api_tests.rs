// SPDX-License-Identifier: MPL-2.0
//! Numerical contract of the original Go row measurement tests. Cache layout is
//! an implementation detail; complete float bits and interval bounds compare.
use super::*;
use serde_json::{Value, json};

fn decode(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap())
}

#[test]
#[ignore = "executed explicitly by grid-measurement-scoreboard.py"]
fn replay() {
    let calls: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_GRID_MEASUREMENT_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<Value> = calls
        .iter()
        .map(|call| {
            let sizes: Vec<f64> = call["sizes"]
                .as_array()
                .unwrap()
                .iter()
                .map(decode)
                .collect();
            let gap = decode(&call["gap"]);
            Value::Array(
                call["ranges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|range| {
                        let start = range[0].as_u64().unwrap() as usize;
                        let end = range[1].as_u64().unwrap() as usize;
                        let size = grid_row_size(&sizes, start, end, 0.0);
                        let with_gap = grid_row_with_gap(&sizes, start, end, gap);
                        json!([
                            start,
                            end,
                            format!("{:016x}", size.to_bits()),
                            format!("{:016x}", with_gap.to_bits())
                        ])
                    })
                    .collect(),
            )
        })
        .collect();
    std::fs::write(
        std::env::var_os("D2RS_GRID_MEASUREMENT_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}
