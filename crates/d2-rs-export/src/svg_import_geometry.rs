// SPDX-License-Identifier: MPL-2.0
//! D2 v0.9.0 style.go numeric lengths, geometry.go primitive lengths and
//! viewport.go admission. Raster construction remains in the native pipeline.
use std::collections::HashMap;
pub(super) type Attrs = HashMap<String, String>;
fn number(input: &str) -> Result<f64, &'static str> {
    if input.len() > 512 {
        return Err("number is too long");
    }
    let value = input.trim();
    if value.is_empty() {
        return Err("empty number");
    }
    let bytes = value.as_bytes();
    let mut at = 0;
    if matches!(bytes[at], b'+' | b'-') {
        at += 1;
    }
    let mut digits = 0;
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        at += 1;
        digits += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
            digits += 1;
        }
    }
    let invalid = "expected a finite unitless number or px length";
    if digits == 0 {
        return Err(invalid);
    }
    if bytes.get(at).is_some_and(|c| matches!(c, b'e' | b'E')) {
        at += 1;
        if bytes.get(at).is_some_and(|c| matches!(c, b'+' | b'-')) {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at == start {
            return Err(invalid);
        }
    }
    if at != bytes.len() {
        return Err(invalid);
    }
    let result = value.parse::<f64>().map_err(|_| invalid)?;
    if !result.is_finite() {
        return Err(invalid);
    }
    Ok(result)
}
fn length(input: &str) -> Result<f64, &'static str> {
    if input.len() > 512 {
        return Err("number is too long");
    }
    let mut value = input.trim();
    if value.contains('%') {
        return Err("percentages are unsupported");
    }
    let mut scale = 1.0;
    for (unit, factor) in [
        ("px", 1.0),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
        ("q", 96.0 / 101.6),
    ] {
        if value.len() >= unit.len()
            && value
                .get(value.len() - unit.len()..)
                .is_some_and(|end| end.eq_ignore_ascii_case(unit))
        {
            value = &value[..value.len() - unit.len()];
            if value.trim() != value {
                return Err("units must immediately follow their number");
            }
            scale = factor;
            break;
        }
    }
    let value = number(value)? * scale;
    if !value.is_finite() {
        return Err("absolute length must be finite");
    }
    Ok(value)
}
fn ex_length(input: &str) -> Result<f64, &'static str> {
    if input.len() > 512 {
        return Err("number is too long");
    }
    let value = input.trim();
    if value.len() <= 2
        || !value
            .get(value.len() - 2..)
            .is_some_and(|unit| unit.eq_ignore_ascii_case("ex"))
    {
        return Err("expected a finite ex length");
    }
    let n = &value[..value.len() - 2];
    if n.trim() != n {
        return Err("units must immediately follow their number");
    }
    let result = number(n).map_err(|_| "expected a finite ex length")? * 8.0;
    if !result.is_finite() {
        return Err("ex length must be finite");
    }
    Ok(result)
}
fn property(element: &str, name: &str, error: &str) -> String {
    format!("element <{element}> has invalid {name}: {error}")
}
fn attribute(
    element: &str,
    attrs: &Attrs,
    name: &str,
    nonnegative: bool,
    radius: bool,
) -> Result<(), String> {
    let Some(raw) = attrs.get(name) else {
        return Ok(());
    };
    if radius && raw.len() <= 16 && raw.trim().eq_ignore_ascii_case("auto") {
        return Ok(());
    }
    let value = length(raw).map_err(|error| property(element, name, error))?;
    if nonnegative && value < 0.0 {
        return Err(property(element, name, "must not be negative"));
    }
    Ok(())
}
fn number_list(input: &str) -> Option<Vec<f64>> {
    let bytes = input.as_bytes();
    let mut result = Vec::new();
    let mut start = None;
    let mut need = true;
    for at in 0..bytes.len() {
        let c = bytes[at];
        if matches!(c, b'+' | b'-') && start.is_some() && !matches!(bytes[at - 1], b'e' | b'E') {
            result.push(number(&input[start.take()?..at]).ok()?);
            need = false;
            start = Some(at);
            continue;
        }
        if c == b',' || matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c') {
            if let Some(begin) = start.take() {
                result.push(number(&input[begin..at]).ok()?);
                need = false;
            }
            if c == b',' {
                if need {
                    return None;
                }
                need = true;
            }
            continue;
        }
        if start.is_none() {
            start = Some(at);
        }
    }
    if let Some(begin) = start {
        result.push(number(&input[begin..]).ok()?);
        need = false;
    }
    if need && !result.is_empty() {
        return None;
    }
    Some(result)
}
fn mathjax(attrs: &Attrs) -> bool {
    attrs.get("style").is_some_and(|style| {
        style
            .split(';')
            .filter_map(|part| part.split_once(':'))
            .any(|(name, value)| {
                name.trim_matches([' ', '\t', '\r', '\n'])
                    .eq_ignore_ascii_case("vertical-align")
                    && ex_length(value).is_ok()
            })
    })
}
fn viewport(attrs: &Attrs, root: bool) -> Result<(), String> {
    let scope = if root { "root" } else { "nested" };
    let mut viewbox = None;
    if let Some(raw) = attrs.get("viewBox") {
        let values = number_list(raw)
            .filter(|values| values.len() == 4)
            .ok_or_else(|| {
                format!("{scope} <svg> viewBox must contain four finite unitless numbers")
            })?;
        if values[2] <= 0.0 || values[3] <= 0.0 {
            return Err(format!(
                "{scope} <svg> viewBox width and height must be positive"
            ));
        }
        viewbox = Some([values[0], values[1], values[2], values[3]]);
    }
    if !root && viewbox.is_none() {
        return Err("nested <svg> requires an explicit viewBox".into());
    }
    let dimension = |name: &str| -> Result<Option<f64>, String> {
        let Some(raw) = attrs.get(name) else {
            return Ok(None);
        };
        let value = if root && mathjax(attrs) {
            ex_length(raw).or_else(|_| length(raw))
        } else {
            length(raw)
        };
        value
            .map(Some)
            .map_err(|error| property("svg", name, error))
    };
    let mut width = dimension("width")?;
    let mut height = dimension("height")?;
    if !root && (width.is_none() || height.is_none()) {
        return Err("nested <svg> requires explicit width and height".into());
    }
    if width.is_none() || height.is_none() {
        let Some(vb) = viewbox else {
            return Err(format!(
                "root <svg> requires {} or viewBox",
                if width.is_none() { "width" } else { "height" }
            ));
        };
        match (width, height) {
            (None, None) => {
                width = Some(vb[2]);
                height = Some(vb[3]);
            }
            (None, Some(h)) => width = Some(h * vb[2] / vb[3]),
            (Some(w), None) => height = Some(w * vb[3] / vb[2]),
            _ => {}
        }
    }
    let width = width.unwrap();
    let height = height.unwrap();
    if width <= 0.0 || height <= 0.0 {
        return Err(format!(
            "{scope} <svg> viewport width and height must be positive"
        ));
    }
    let vb = viewbox.unwrap_or([0.0, 0.0, width, height]);
    let sx = width / vb[2];
    let sy = height / vb[3];
    if root && (sx <= 0.0 || sy <= 0.0 || sx.is_infinite() || sy.is_infinite()) {
        return Err("root <svg> viewport scale is zero or non-finite".into());
    }
    if !root {
        attribute("svg", attrs, "x", false, false)?;
        attribute("svg", attrs, "y", false, false)?;
    }
    let raw = attrs
        .get("preserveAspectRatio")
        .map(String::as_str)
        .unwrap_or("");
    if raw.len() > 128 {
        return Err(format!("{scope} <svg> preserveAspectRatio is too long"));
    }
    let fields: Vec<_> = raw.split_whitespace().collect();
    let align = fields.first().copied().unwrap_or("xMidYMid");
    if align == "defer" {
        return Err(format!(
            "{scope} <svg> preserveAspectRatio defer is unsupported"
        ));
    }
    let (ax, ay) = match align {
        "none" => (0.0, 0.0),
        "xMinYMin" => (0.0, 0.0),
        "xMidYMin" => (0.5, 0.0),
        "xMaxYMin" => (1.0, 0.0),
        "xMinYMid" => (0.0, 0.5),
        "xMidYMid" => (0.5, 0.5),
        "xMaxYMid" => (1.0, 0.5),
        "xMinYMax" => (0.0, 1.0),
        "xMidYMax" => (0.5, 1.0),
        "xMaxYMax" => (1.0, 1.0),
        _ => {
            return Err(format!(
                "{scope} <svg> has unsupported preserveAspectRatio alignment"
            ));
        }
    };
    if fields.len() > 2 {
        return Err(format!("{scope} <svg> has malformed preserveAspectRatio"));
    }
    let slice = match fields.get(1).copied().unwrap_or("meet") {
        "meet" => false,
        "slice" => true,
        _ => {
            return Err(format!(
                "{scope} <svg> has unsupported preserveAspectRatio fit"
            ));
        }
    };
    let (a, d, tx, ty) = if align == "none" {
        (sx, sy, -vb[0] * sx, -vb[1] * sy)
    } else {
        let scale = if slice { sx.max(sy) } else { sx.min(sy) };
        (
            scale,
            scale,
            (width - vb[2] * scale) * ax - vb[0] * scale,
            (height - vb[3] * scale) * ay - vb[1] * scale,
        )
    };
    if ![width, height, a, d, tx, ty].iter().all(|v| v.is_finite()) {
        return Err(format!("{scope} <svg> viewport transform is non-finite"));
    }
    Ok(())
}
pub(super) fn compile(element: &str, attrs: &Attrs, root: bool) -> Result<(), String> {
    if element == "svg" {
        if root {
            if attrs
                .get("version")
                .is_some_and(|v| !matches!(v.trim_matches([' ', '\t', '\r', '\n']), "1.0" | "1.1"))
            {
                return Err("root <svg> has unsupported version".into());
            }
            for name in ["x", "y"] {
                if attrs.get(name).is_some_and(|v| length(v).ok() != Some(0.0)) {
                    return Err(format!("root <svg> {name} must be a zero absolute length"));
                }
            }
        } else {
            if attrs.contains_key("version") {
                return Err("nested <svg> version is unsupported".into());
            }
            if attrs.contains_key("enable-background") {
                return Err("nested <svg> enable-background is unsupported".into());
            }
            viewport(attrs, false)?;
        }
    }
    // Source style.go parses each MathJax marker declaration in authored order.
    if let Some(style) = attrs.get("style") {
        for (name, value) in style.split(';').filter_map(|part| part.split_once(':')) {
            if name
                .trim_matches([' ', '\t', '\r', '\n'])
                .eq_ignore_ascii_case("vertical-align")
            {
                if !root {
                    return Err(format!(
                        "element <{element}> has unsupported vertical-align outside the root <svg>"
                    ));
                }
                ex_length(value.trim_matches([' ', '\t', '\r', '\n'])).map_err(|error| {
                    format!("root <svg> has invalid MathJax vertical-align: {error}")
                })?;
            }
        }
    }
    let fields: &[(&str, bool, bool)] = match element {
        "rect" => &[
            ("x", false, false),
            ("y", false, false),
            ("width", true, false),
            ("height", true, false),
            ("rx", true, true),
            ("ry", true, true),
        ],
        "circle" => &[
            ("cx", false, false),
            ("cy", false, false),
            ("r", true, false),
        ],
        "ellipse" => &[
            ("cx", false, false),
            ("cy", false, false),
            ("rx", true, true),
            ("ry", true, true),
        ],
        "line" => &[
            ("x1", false, false),
            ("y1", false, false),
            ("x2", false, false),
            ("y2", false, false),
        ],
        _ => &[],
    };
    for &(name, nonnegative, radius) in fields {
        attribute(element, attrs, name, nonnegative, radius)?;
    }
    Ok(())
}
pub(super) fn root_viewport(attrs: &Attrs) -> Result<(), String> {
    viewport(attrs, true)
}
