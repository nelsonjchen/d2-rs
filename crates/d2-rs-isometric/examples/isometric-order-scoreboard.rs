// SPDX-License-Identifier: MPL-2.0
//! Replay the ordering projection of original source paint units. Native code
//! borrows every polygon and returns a permutation; opacity/decal data is owned
//! by the caller and never enters the ordering algorithm's mutation surface.
use d2_rs_core::CancellationToken;
use d2_rs_isometric::img::{
    context::RenderContext,
    svg_order::{OrderUnit, order_paint_units_with_context},
    svg_visibility::{SvgPoint, SvgVisibilityLimits},
};
use serde_json::{Value, json};

fn replay(op: &Value) -> Value {
    assert_eq!(op["op"], "order");
    let inputs = op["units"].as_array().unwrap();
    let units: Vec<_> = inputs
        .iter()
        .map(|u| OrderUnit {
            first: u["first"].as_i64().unwrap(),
            opaque: u["opaque"].as_bool().unwrap(),
            depth: u["depth"].as_f64().unwrap(),
            polygons: u["batches"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|b| b["polygons"].as_array().unwrap())
                .map(|points| {
                    points
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| SvgPoint {
                            x: p[0].as_f64().unwrap(),
                            y: p[1].as_f64().unwrap(),
                            z: p[2].as_f64().unwrap(),
                        })
                        .collect()
                })
                .collect(),
        })
        .collect();
    let l = &op["limits"];
    let limits = SvgVisibilityLimits {
        faces: l["faces"].as_u64().unwrap() as usize,
        vertices: l["vertices"].as_u64().unwrap() as usize,
        fragments: l["fragments"].as_u64().unwrap() as usize,
        work: l["work"].as_u64().unwrap() as usize,
        grid_references: l["grid_references"].as_u64().unwrap() as usize,
    };
    let token = CancellationToken::new();
    if op["canceled"].as_bool().unwrap() {
        token.cancel();
    }
    let (ordered, stats, error) = match order_paint_units_with_context(
        &units,
        limits,
        &RenderContext::from_token(&token),
    ) {
        Ok((order, stats)) => (
            order.iter().map(|&i| inputs[i].clone()).collect::<Vec<_>>(),
            json!({"relations": stats.relations, "cyclic_groups": stats.cyclic_groups, "conflicting_pairs": stats.conflicting_pairs}),
            String::new(),
        ),
        // The native Result API does not expose partial statistics on failure.
        // Report that limitation explicitly rather than fabricating zero stats.
        Err(error) => {
            if op["with_stats"] == true {
                return json!({"unsupported": "partial paint-order stats on error", "error": error});
            }
            (inputs.clone(), Value::Null, error)
        }
    };
    json!({"units": ordered, "stats": if op["with_stats"] == true {stats} else {Value::Null}, "error": error})
}

fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let output: Vec<Vec<Value>> = request["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            case["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(replay)
                .collect()
        })
        .collect();
    println!("{}", json!(output));
}
