// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/gradient.go compileLinearGradient,
// affineInverseIsFinite and compiled stop assembly.
use super::{
    svg_geometry::trim_space,
    svg_length,
    svg_primitive::{Paint, Stop},
    svg_transform,
};
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::collections::BTreeMap;
pub fn inverse_is_finite(matrix: [f64; 6]) -> bool {
    if !matrix.iter().all(|v| v.is_finite()) {
        return false;
    }
    let values: Vec<_> = matrix
        .into_iter()
        .map(|v| BigRational::from_float(v).unwrap())
        .collect();
    let [a, b, c, d, e, f] = values.as_slice() else {
        unreachable!()
    };
    let determinant = a * d - b * c;
    if determinant == BigRational::from_integer(0.into()) {
        return false;
    }
    [
        d.clone(),
        -b.clone(),
        -c.clone(),
        a.clone(),
        c * f - d * e,
        b * e - a * f,
    ]
    .into_iter()
    .all(|n| (n / &determinant).to_f64().is_some_and(f64::is_finite))
}
pub fn compile(
    source: &str,
    attrs: &BTreeMap<String, String>,
    maximum_bytes: usize,
    maximum_functions: usize,
    used: &mut usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Paint, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let property = |name: &str, reason: String| {
        error(format!(
            "element <linearGradient> has invalid {name}: {reason}"
        ))
    };
    let units = match attrs.get("gradientUnits").map(|v| v.trim()).unwrap_or("") {
        "" | "objectBoundingBox" => 0,
        "userSpaceOnUse" => 1,
        _ => {
            return Err(error(
                "element <linearGradient> has invalid gradientUnits".into(),
            ));
        }
    };
    let spread = match attrs.get("spreadMethod").map(|v| v.trim()).unwrap_or("") {
        "" | "pad" => 0,
        "reflect" => 1,
        "repeat" => 2,
        _ => {
            return Err(error(
                "element <linearGradient> has invalid spreadMethod".into(),
            ));
        }
    };
    let mut coordinates = [0., 0., 1., 0.];
    let mut percentages = [false, false, true, false];
    for (index, name) in ["x1", "y1", "x2", "y2"].iter().enumerate() {
        if let Some(raw) = attrs.get(*name) {
            let raw = raw.trim();
            let (number, percent) = raw.strip_suffix('%').map_or((raw, false), |n| (n, true));
            if percent && number.trim() != number {
                return Err(property(
                    name,
                    "percentage sign must follow its number".into(),
                ));
            }
            let value = svg_length::length(number, !percent && units == 1)
                .map_err(|e| property(name, e))?;
            coordinates[index] = if percent { value / 100. } else { value };
            percentages[index] = percent;
        }
    }
    let start = [coordinates[0], coordinates[1]];
    let end = [coordinates[2], coordinates[3]];
    let mut transform = [1., 0., 0., 1., 0., 0.];
    if let Some(raw) = attrs.get("gradientTransform") {
        let raw = trim_space(raw, cancel)?;
        if raw.is_empty() {
            return Err(error(
                "element <linearGradient> has an empty gradientTransform".into(),
            ));
        }
        if *used >= maximum_functions {
            return Err(error(format!(
                "transform function count exceeds limit {maximum_functions}"
            )));
        }
        let (matrix, count) = svg_transform::parse(
            source,
            raw,
            maximum_bytes,
            maximum_functions - *used,
            cancel,
        )?;
        transform = matrix;
        *used += count;
    }
    if !inverse_is_finite(transform) {
        return Err(error(
            "element <linearGradient> has a singular or unrepresentable gradientTransform".into(),
        ));
    }
    Ok(Paint::Linear {
        percentages,
        start,
        end,
        stops: Vec::new(),
        units,
        transform,
        spread,
    })
}
pub fn attach_stops(
    source: &str,
    paint: &mut Paint,
    stops: &[Option<Stop>],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let mut previous = -1.;
    let mut compiled = Vec::with_capacity(stops.len());
    if let Some(cause) = cancel.and_then(d2_rs_core::CancellationToken::error) {
        return Err(
            if matches!(cause, d2_rs_core::LayoutError::DeadlineExceeded) {
                "context deadline exceeded"
            } else {
                "context canceled"
            }
            .into(),
        );
    }
    if !matches!(paint, Paint::Linear { .. }) {
        return Err(error("internal uncompiled linear gradient".into()));
    }
    for stop in stops {
        if let Some(cause) = cancel.and_then(d2_rs_core::CancellationToken::error) {
            return Err(
                if matches!(cause, d2_rs_core::LayoutError::DeadlineExceeded) {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                }
                .into(),
            );
        }
        let stop = stop
            .as_ref()
            .ok_or_else(|| error("internal uncompiled gradient stop".into()))?;
        let mut stop = stop.clone();
        stop.offset = stop.offset.max(previous);
        previous = stop.offset;
        compiled.push(stop);
    }
    if let Paint::Linear { stops, .. } = paint {
        *stops = compiled;
        Ok(())
    } else {
        Err(error("internal uncompiled linear gradient".into()))
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_stop_assembly_and_context() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-gradient-stops-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let stops: Vec<Option<Stop>> = serde_json::from_value(row["stops"].clone()).unwrap();
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let mut paint = Paint::Linear {
                percentages: [false; 4],
                start: [0.; 2],
                end: [0.; 2],
                stops: vec![Stop {
                    offset: 0.5,
                    color: [0, 2, 0, 200],
                }],
                units: 0,
                transform: [1., 0., 0., 1., 0., 0.],
                spread: 0,
            };
            let result = attach_stops("math", &mut paint, &stops, cancel);
            assert_eq!(
                result.err().unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            let expected: Paint = serde_json::from_value(row["after"].clone()).unwrap();
            assert_eq!(paint, expected, "{row}");
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_gradient_template_and_transform_counter() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-gradient-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let attrs = serde_json::from_value(row["attrs"].clone()).unwrap();
            let mut used = row["used"].as_u64().unwrap() as usize;
            let result = compile(
                "math",
                &attrs,
                10000,
                row["maximum"].as_u64().unwrap() as usize,
                &mut used,
                None,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(used as u64, row["after"].as_u64().unwrap(), "{row}");
            if let Ok(paint) = result {
                let expected: Paint = serde_json::from_value(row["paint"].clone()).unwrap();
                assert_eq!(paint, expected, "{row}");
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_exact_rational_inverse_admission() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-inverse-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let matrix = serde_json::from_value(row["matrix"].clone()).unwrap();
            assert_eq!(
                inverse_is_finite(matrix),
                row["finite"].as_bool().unwrap(),
                "{row}"
            );
        }
    }
}

/// Source d2raster/vector_validation.go finiteAffineInverse: ordinary source
/// matrix inverse first, exact rational fallback for finite extreme transforms.
pub fn finite_inverse(m: [f64; 6]) -> Option<[f64; 6]> {
    if !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    let sub = |a: f64, b: f64, c: f64, d: f64| {
        if cfg!(target_arch = "aarch64") {
            (-c).mul_add(d, a * b)
        } else {
            a * b - c * d
        }
    };
    let [a, b, c, d, e, f] = m;
    let determinant = sub(a, d, b, c);
    if determinant != 0. && determinant.is_finite() {
        let out = [
            d / determinant,
            -b / determinant,
            -c / determinant,
            a / determinant,
            sub(c, f, d, e) / determinant,
            sub(b, e, a, f) / determinant,
        ];
        if out.iter().all(|v| v.is_finite()) {
            return Some(out);
        }
    }
    let values: Vec<_> = m
        .into_iter()
        .map(|v| BigRational::from_float(v).unwrap())
        .collect();
    let [a, b, c, d, e, f] = values.as_slice() else {
        unreachable!()
    };
    let determinant = a * d - b * c;
    if determinant == BigRational::from_integer(0.into()) {
        return None;
    }
    let out: Vec<_> = [
        d.clone(),
        -b.clone(),
        -c.clone(),
        a.clone(),
        c * f - d * e,
        b * e - a * f,
    ]
    .into_iter()
    .map(|v| (v / &determinant).to_f64())
    .collect::<Option<_>>()?;
    let out: [f64; 6] = out.try_into().ok()?;
    out.iter().all(|v| v.is_finite()).then_some(out)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod raster_inverse_tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_raster_inverse_standard_and_exact_fallback_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/raster-affine-inverse-v090.json"
        ))
        .unwrap();
        for (i, row) in rows.as_array().unwrap().iter().enumerate() {
            let input: [f64; 6] = row["input"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            let result = finite_inverse(input);
            assert_eq!(
                result.is_some(),
                row["invertible"].as_bool().unwrap(),
                "control {i}"
            );
            if let Some(out) = result {
                let bits: Vec<_> = out
                    .iter()
                    .map(|v| format!("{:016x}", v.to_bits()))
                    .collect();
                assert_eq!(
                    serde_json::to_value(bits).unwrap(),
                    row["output"],
                    "control {i}"
                );
            }
        }
    }
}

/// Resolve a retained paint template in the referencing element's viewport.
pub fn resolve(paint: &mut Option<Paint>, viewport: [f64; 4]) -> Result<(), String> {
    let Some(Paint::Linear {
        start,
        end,
        percentages,
        units,
        transform,
        stops,
        ..
    }) = paint
    else {
        return Ok(());
    };
    if stops.is_empty() {
        *paint = None;
        return Ok(());
    }
    let extent = if *units == 0 {
        [1., 1.]
    } else {
        [viewport[2], viewport[3]]
    };
    for (index, coordinate) in start.iter_mut().chain(end.iter_mut()).enumerate() {
        if percentages[index] {
            *coordinate *= extent[index % 2];
        }
        if !coordinate.is_finite() {
            return Err("invalid gradient coordinates".into());
        }
    }
    *percentages = [false; 4];
    let delta = [end[0] - start[0], end[1] - start[1]];
    let direction = if cfg!(target_arch = "aarch64") {
        [
            transform[0].mul_add(delta[0], transform[2] * delta[1]),
            transform[1].mul_add(delta[0], transform[3] * delta[1]),
        ]
    } else {
        [
            transform[0] * delta[0] + transform[2] * delta[1],
            transform[1] * delta[0] + transform[3] * delta[1],
        ]
    };
    if !direction.iter().all(|v| v.is_finite()) {
        return Err("invalid transformed gradient geometry".into());
    }
    Ok(())
}
