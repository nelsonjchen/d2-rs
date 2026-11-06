// SPDX-License-Identifier: MPL-2.0
//! Runs the native SVG visibility port on Go hook dumps
//! (<board>.visibility.json: Go's faces and visible fragments) and reports
//! exact equality per board.

use d2_rs_isometric::img::svg_visibility::{SvgPoint, SvgVisibilityFace, svg_visible_faces};

fn point(value: &serde_json::Value) -> SvgPoint {
    SvgPoint {
        x: value[0].as_f64().unwrap(),
        y: value[1].as_f64().unwrap(),
        z: value[2].as_f64().unwrap(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut matched, mut total) = (0, 0);
    for path in std::env::args().skip(1) {
        let dump: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let faces: Vec<SvgVisibilityFace> = dump["faces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| SvgVisibilityFace {
                points: f["points"].as_array().unwrap().iter().map(point).collect(),
                opaque: f["opaque"].as_bool().unwrap_or(false),
                group: f["group"].as_u64().map(|v| v as usize),
                owner: f["owner"].as_u64().map(|v| v as usize),
                contour: f["contour"].as_bool().unwrap_or(false),
                order: f["order"].as_i64().unwrap(),
            })
            .collect();
        total += 1;
        let result = svg_visible_faces(&faces);
        let ok = match (&result, dump["error"].as_str()) {
            (Err(e), Some(expected)) => e == expected,
            (Ok(visible), None) => {
                let expected = dump["visible"].as_array().unwrap();
                visible.len() == expected.len()
                    && visible.iter().zip(expected).all(|(ours, theirs)| {
                        let theirs = theirs.as_array().unwrap();
                        ours.len() == theirs.len()
                            && ours.iter().zip(theirs).all(|(a, b)| {
                                let b = b.as_array().unwrap();
                                a.len() == b.len() && a.iter().zip(b).all(|(p, q)| *p == point(q))
                            })
                    })
            }
            _ => false,
        };
        if ok {
            matched += 1;
        } else {
            println!("mismatch {path}");
        }
    }
    println!("visibility match {matched}/{total}");
    Ok(())
}
