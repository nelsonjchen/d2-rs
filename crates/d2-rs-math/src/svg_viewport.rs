// SPDX-License-Identifier: MPL-2.0
// Source: D2 master 6a7b48378 d2renderers/d2svgimport/viewport.go and d2scene/aspect.go.
use super::{svg_geometry, svg_length};
use std::collections::BTreeMap;
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Viewport {
    pub view_box: [f64; 4],
    pub width: f64,
    pub height: f64,
    pub aspect: [u8; 2],
    pub transform: [f64; 6],
    pub position: [f64; 2],
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn aspect(source: &str, raw: &str, scope: &str) -> Result<[u8; 2], String> {
    let error = |reason: &str| format!("d2svgimport: {source}: {scope} <svg> {reason}");
    if raw.len() > 128 {
        return Err(error("preserveAspectRatio is too long"));
    }
    let fields = raw.split_whitespace().collect::<Vec<_>>();
    if fields.is_empty() {
        return Ok([5, 0]);
    }
    if fields[0] == "defer" {
        return Err(error("preserveAspectRatio defer is unsupported"));
    }
    let align = match fields[0] {
        "none" => 0,
        "xMinYMin" => 1,
        "xMidYMin" => 2,
        "xMaxYMin" => 3,
        "xMinYMid" => 4,
        "xMidYMid" => 5,
        "xMaxYMid" => 6,
        "xMinYMax" => 7,
        "xMidYMax" => 8,
        "xMaxYMax" => 9,
        _ => return Err(error("has unsupported preserveAspectRatio alignment")),
    };
    if fields.len() > 2 {
        return Err(error("has malformed preserveAspectRatio"));
    }
    let fit = match fields.get(1).copied() {
        None | Some("meet") => 0,
        Some("slice") => 1,
        _ => return Err(error("has unsupported preserveAspectRatio fit")),
    };
    Ok([align, fit])
}
pub fn aspect_matrix(
    source: [f64; 4],
    destination: [f64; 4],
    aspect: [u8; 2],
) -> Result<[f64; 6], String> {
    let valid = |b: [f64; 4]| b.iter().all(|v| v.is_finite()) && b[2] >= 0. && b[3] >= 0.;
    if !valid(source) || source[2] == 0. || source[3] == 0. {
        return Err("d2scene: aspect-ratio source box must have finite positive dimensions".into());
    }
    if !valid(destination) {
        return Err(
            "d2scene: aspect-ratio destination box must have finite non-negative dimensions".into(),
        );
    }
    let [align, fit] = aspect;
    if align > 9 || fit > 1 {
        return Err(format!(
            "d2scene: invalid aspect-ratio policy align={align} fit={fit}"
        ));
    }
    let sx = destination[2] / source[2];
    let sy = destination[3] / source[3];
    if !sx.is_finite() || !sy.is_finite() {
        return Err("d2scene: aspect-ratio scale is non-finite".into());
    }
    let (dx, dy, sx, sy) = if align == 0 {
        (destination[0], destination[1], sx, sy)
    } else {
        let mut scale = sx;
        if sy < scale {
            scale = sy;
        }
        if fit == 1 {
            scale = if sy > sx { sy } else { sx };
        }
        let ax = ((align - 1) % 3) as f64 * 0.5;
        let ay = ((align - 1) / 3) as f64 * 0.5;
        let extra_x = destination[2] - source[2] * scale;
        let extra_y = destination[3] - source[3] * scale;
        (
            destination[0] + extra_x * ax,
            destination[1] + extra_y * ay,
            scale,
            scale,
        )
    };
    // Master aspect.go explicitly rounds each product; no FMA is permitted.
    let matrix = [sx, 0., 0., sy, dx - source[0] * sx, dy - source[1] * sy];
    if !matrix.iter().all(|v| v.is_finite()) {
        return Err("d2scene: aspect-ratio transform is non-finite".into());
    }
    Ok(matrix)
}
pub(crate) fn view_box(
    source: &str,
    attrs: &BTreeMap<String, String>,
    scope: &str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Option<[f64; 4]>, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {scope} <svg> {reason}");
    Ok(if let Some(raw) = attrs.get("viewBox") {
        let invalid = || error("viewBox must contain four finite unitless numbers".into());
        let parts = svg_geometry::list(raw, 4, cancel)
            .map_err(|_| context(cancel).err().unwrap_or_else(invalid))?;
        if parts.len() != 4 {
            context(cancel)?;
            return Err(invalid());
        }
        let mut values = [0.; 4];
        for (index, part) in parts.iter().enumerate() {
            values[index] = svg_length::number(part).map_err(|_| invalid())?;
        }
        if values[2] <= 0. || values[3] <= 0. {
            return Err(error("viewBox width and height must be positive".into()));
        }
        Some(values)
    } else {
        None
    })
}
pub fn compile(
    source: &str,
    attrs: &BTreeMap<String, String>,
    nested: bool,
    mathjax: bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Viewport, String> {
    let scope = if nested { "nested" } else { "root" };
    let error = |reason: String| format!("d2svgimport: {source}: {scope} <svg> {reason}");
    let mut view_box = view_box(source, attrs, scope, cancel)?;
    if nested && view_box.is_none() {
        return Err(error("requires an explicit viewBox".into()));
    }
    let length = |name: &str, allow_mathjax: bool| -> Result<Option<f64>, String> {
        let Some(raw) = attrs.get(name) else {
            return Ok(None);
        };
        let result = svg_length::length(raw, true);
        let result = if allow_mathjax && mathjax {
            svg_length::mathjax_ex(raw).map(Ok).unwrap_or(result)
        } else {
            result
        };
        result.map(Some).map_err(|reason| {
            format!("d2svgimport: {source}: element <svg> has invalid {name}: {reason}")
        })
    };
    let width = length("width", true)?;
    let height = length("height", true)?;
    if nested && (width.is_none() || height.is_none()) {
        return Err(error("requires explicit width and height".into()));
    }
    let (width, height) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (None, None) => {
            let b = view_box.ok_or_else(|| error("requires width or viewBox".into()))?;
            (b[2], b[3])
        }
        (None, Some(h)) => {
            let b = view_box.ok_or_else(|| error("requires width or viewBox".into()))?;
            (h * b[2] / b[3], h)
        }
        (Some(w), None) => {
            let b = view_box.ok_or_else(|| error("requires height or viewBox".into()))?;
            (w, w * b[3] / b[2])
        }
    };
    if width <= 0. || height <= 0. {
        return Err(error("viewport width and height must be positive".into()));
    }
    let b = *view_box.get_or_insert([0., 0., width, height]);
    let position = if nested {
        [
            length("x", false)?.unwrap_or(0.),
            length("y", false)?.unwrap_or(0.),
        ]
    } else {
        [0.; 2]
    };
    if !nested {
        let sx = width / b[2];
        let sy = height / b[3];
        if sx <= 0. || sy <= 0. || sx.is_infinite() || sy.is_infinite() {
            return Err(error("viewport scale is zero or non-finite".into()));
        }
    }
    let aspect = aspect(
        source,
        attrs
            .get("preserveAspectRatio")
            .map(String::as_str)
            .unwrap_or(""),
        scope,
    )?;
    let transform = aspect_matrix(b, [0., 0., width, height], aspect)
        .map_err(|_| error("viewport transform is non-finite".into()))?;
    Ok(Viewport {
        view_box: b,
        width,
        height,
        aspect,
        transform,
        position,
    })
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_root_nested_viewports_and_aspect_matrix_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-viewport-master.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let result = if row["operation"] == "viewport" {
                let attrs = serde_json::from_value(row["attrs"].clone()).unwrap();
                compile(
                    "math",
                    &attrs,
                    row["nested"].as_bool().unwrap(),
                    row["mathjax"].as_bool().unwrap(),
                    None,
                )
                .map(|viewport| {
                    let expected: Viewport = serde_json::from_value(row["output"].clone()).unwrap();
                    assert_eq!(viewport, expected, "{row}");
                    viewport.transform
                })
            } else {
                aspect_matrix(
                    serde_json::from_value(row["source"].clone()).unwrap(),
                    serde_json::from_value(row["destination"].clone()).unwrap(),
                    serde_json::from_value(row["aspect"].clone()).unwrap(),
                )
            };
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(matrix) = result {
                let bits = matrix.map(|v| format!("{:016x}", v.to_bits()));
                let expected: Vec<String> =
                    serde_json::from_value(row["matrix_bits"].clone()).unwrap();
                assert_eq!(bits.as_slice(), expected, "{row}");
            }
        }
    }
}
