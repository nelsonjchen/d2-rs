// SPDX-License-Identifier: MPL-2.0
//! Replay original visibility calls, including typed nonfinite coordinates.
use d2_rs_core::CancellationToken;
use d2_rs_isometric::img::{
    context::RenderContext,
    svg_visibility::{
        SVG_DEFAULT_VISIBILITY_LIMITS, SvgPoint, SvgVisibilityFace, SvgVisibilityLimits,
        svg_visibility_work_limit, svg_visible_faces_with_context,
    },
};
use serde_json::{Value, json};

fn number(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        Some(other) => panic!("unknown scalar {other}"),
        None => v.as_f64().unwrap(),
    }
}

fn replay(op: &Value) -> Value {
    if op["op"] == "work_limit" {
        return json!(svg_visibility_work_limit(
            op["faces"].as_u64().unwrap() as usize
        ));
    }
    assert_eq!(op["op"], "visibility");
    let faces: Vec<_> = op["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| SvgVisibilityFace {
            points: f["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| SvgPoint {
                    x: number(&p[0]),
                    y: number(&p[1]),
                    z: number(&p[2]),
                })
                .collect(),
            opaque: f["opaque"].as_bool().unwrap(),
            group: f["group"].as_u64().map(|n| n as usize),
            owner: f["owner"].as_u64().map(|n| n as usize),
            contour: f["contour"].as_bool().unwrap(),
            order: f["order"].as_i64().unwrap(),
        })
        .collect();
    let l = &op["limits"];
    let limits = if l.is_null() {
        SvgVisibilityLimits {
            work: svg_visibility_work_limit(faces.len()),
            ..SVG_DEFAULT_VISIBILITY_LIMITS
        }
    } else {
        SvgVisibilityLimits {
            faces: l["faces"].as_u64().unwrap() as usize,
            vertices: l["vertices"].as_u64().unwrap() as usize,
            fragments: l["fragments"].as_u64().unwrap() as usize,
            work: l["work"].as_u64().unwrap() as usize,
            grid_references: l["grid_references"].as_u64().unwrap() as usize,
        }
    };
    let token = CancellationToken::new();
    if op["canceled"].as_bool().unwrap() {
        token.cancel();
    }
    match svg_visible_faces_with_context(&faces, limits, &RenderContext::from_token(&token)) {
        Err(error) => json!({"visible": null, "error": error}),
        Ok(visible) => {
            // Go allocates the outer result, leaving absent face fragments nil.
            let output: Vec<Option<Vec<Vec<[f64; 3]>>>> = visible
                .iter()
                .map(|fragments| {
                    (!fragments.is_empty()).then(|| {
                        fragments
                            .iter()
                            .map(|points| points.iter().map(|p| [p.x, p.y, p.z]).collect())
                            .collect()
                    })
                })
                .collect();
            json!({"visible": output, "error": ""})
        }
    }
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
