// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/geometry.go and style.go.
use super::{svg_length, svg_path::Command};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Geometry {
    pub kind: u8,
    pub bounds: [f64; 4],
    pub center: [f64; 2],
    pub radii: [f64; 2],
    pub commands: Vec<Command>,
    pub force_no_fill: bool,
    #[serde(default)]
    pub asset: String,
    #[serde(default)]
    pub aspect: [u8; 2],
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub(crate) fn trim_space<'a>(
    raw: &'a str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<&'a str, String> {
    let data = raw.as_bytes();
    let mut start = 0;
    let mut end = data.len();
    while start < end && b" \t\n\r\x0c".contains(&data[start]) {
        start += 1;
        if start & 4095 == 0 {
            context(cancel)?;
        }
    }
    while end > start && b" \t\n\r\x0c".contains(&data[end - 1]) {
        end -= 1;
        if (data.len() - end) & 4095 == 0 {
            context(cancel)?;
        }
    }
    context(cancel)?;
    Ok(&raw[start..end])
}
pub(crate) fn list<'a>(
    value: &'a str,
    maximum: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<&'a str>, String> {
    fn flush<'a>(
        value: &'a str,
        start: &mut Option<usize>,
        end: usize,
        fields: &mut Vec<&'a str>,
        maximum: usize,
        need: &mut bool,
    ) -> Result<(), String> {
        if let Some(from) = start.take() {
            if fields.len() >= maximum {
                return Err("SVG number list exceeds item limit".into());
            }
            fields.push(&value[from..end]);
            *need = false;
        }
        Ok(())
    }
    let data = value.as_bytes();
    let mut fields = Vec::new();
    let mut start = None;
    let mut need = true;
    for (index, &byte) in data.iter().enumerate() {
        if index & 4095 == 0 {
            context(cancel)?;
        }
        if matches!(byte, b'+' | b'-') && start.is_some() && !matches!(data[index - 1], b'e' | b'E')
        {
            flush(value, &mut start, index, &mut fields, maximum, &mut need)?;
            start = Some(index);
            continue;
        }
        if byte == b',' {
            flush(value, &mut start, index, &mut fields, maximum, &mut need)?;
            if need {
                return Err("empty comma-separated number".into());
            }
            need = true;
            continue;
        }
        if b" \t\n\r\x0c".contains(&byte) {
            flush(value, &mut start, index, &mut fields, maximum, &mut need)?;
            continue;
        }
        if start.is_none() {
            start = Some(index);
        }
    }
    flush(
        value,
        &mut start,
        data.len(),
        &mut fields,
        maximum,
        &mut need,
    )?;
    if need && !fields.is_empty() {
        return Err("trailing comma".into());
    }
    Ok(fields)
}
/// Compile non-image/non-text geometry after XML/style admission. Text fallback
/// and embedded images use their independent importer stages.
pub fn compile(
    source: &str,
    name: &str,
    attrs: &BTreeMap<String, String>,
    max_bytes: usize,
    max_commands: usize,
    used: &mut usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Geometry, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let property =
        |key: &str, reason: String| error(format!("element <{name}> has invalid {key}: {reason}"));
    let length = |key: &str, nonnegative: bool| -> Result<f64, String> {
        let Some(raw) = attrs.get(key) else {
            return Ok(0.);
        };
        let value = svg_length::length(raw, true).map_err(|e| property(key, e))?;
        if nonnegative && value < 0. {
            return Err(property(key, "must not be negative".into()));
        }
        Ok(value)
    };
    let radius = |key: &str| -> Result<(f64, bool), String> {
        let Some(raw) = attrs.get(key) else {
            return Ok((0., false));
        };
        if raw.len() <= 16 && raw.trim().eq_ignore_ascii_case("auto") {
            return Ok((0., false));
        }
        Ok((length(key, true)?, true))
    };
    let mut geometry = Geometry::default();
    match name {
        "svg" | "g" | "defs" | "style" | "use" | "clipPath" => {}
        "rect" => {
            geometry.bounds = [
                length("x", false)?,
                length("y", false)?,
                length("width", true)?,
                length("height", true)?,
            ];
            let (mut rx, has_rx) = radius("rx")?;
            let (mut ry, has_ry) = radius("ry")?;
            if has_rx && !has_ry {
                ry = rx;
            } else if has_ry && !has_rx {
                rx = ry;
            }
            if rx > geometry.bounds[2] / 2. {
                rx = geometry.bounds[2] / 2.;
            }
            if ry > geometry.bounds[3] / 2. {
                ry = geometry.bounds[3] / 2.;
            }
            geometry.kind = 1;
            geometry.radii = [rx, ry];
        }
        "circle" | "ellipse" => {
            geometry.center = [length("cx", false)?, length("cy", false)?];
            geometry.radii = if name == "circle" {
                let r = length("r", true)?;
                [r, r]
            } else {
                let (mut rx, has_rx) = radius("rx")?;
                let (mut ry, has_ry) = radius("ry")?;
                if has_rx && !has_ry {
                    ry = rx;
                } else if has_ry && !has_rx {
                    rx = ry;
                }
                [rx, ry]
            };
            geometry.kind = 2;
        }
        "line" => {
            let start = [length("x1", false)?, length("y1", false)?];
            let end = [length("x2", false)?, length("y2", false)?];
            if 2 > max_commands.saturating_sub(*used) {
                return Err(error(format!(
                    "element <line> makes path command count exceed limit {max_commands}"
                )));
            }
            geometry.commands = vec![
                Command {
                    points: [start, [0.; 2], [0.; 2]],
                    ..Default::default()
                },
                Command {
                    kind: 1,
                    points: [end, [0.; 2], [0.; 2]],
                    ..Default::default()
                },
            ];
            *used += 2;
            geometry.kind = 3;
            geometry.force_no_fill = true;
        }
        "path" => {
            let raw = attrs.get("d").map(String::as_str).unwrap_or("");
            if trim_space(raw, cancel)?.is_empty() {
                geometry.kind = 3;
                return Ok(geometry);
            }
            let remaining = max_commands.saturating_sub(*used);
            if remaining == 0 {
                return Err(error(format!(
                    "path command count exceeds limit {max_commands}"
                )));
            }
            let (commands, count) =
                super::svg_path::parse(source, raw, max_bytes, remaining, cancel)?;
            *used += count;
            if !commands.is_empty() && commands.iter().all(|c| c.kind == 0) {
                return Ok(geometry);
            }
            geometry.kind = 3;
            geometry.commands = commands;
        }
        "polyline" | "polygon" => {
            let raw = attrs.get("points").map(String::as_str).unwrap_or("");
            let remaining = max_commands.saturating_sub(*used);
            let pairs = remaining.saturating_sub(usize::from(name == "polygon" && remaining > 0));
            let maximum = if pairs <= raw.len() / 2 {
                pairs * 2
            } else {
                raw.len()
            };
            let reason = |e: String| {
                if e == "SVG number list exceeds item limit" {
                    format!("path command count exceeds limit {max_commands}")
                } else {
                    e
                }
            };
            let point_error = |e: String| {
                if let Err(context_error) = context(cancel) {
                    context_error
                } else {
                    property("points", reason(e))
                }
            };
            let fields = list(raw, maximum, cancel).map_err(point_error)?;
            geometry.kind = 3;
            if fields.is_empty() {
                return Ok(geometry);
            }
            if fields.len() % 2 != 0 {
                return Err(point_error("expected complete coordinate pairs".into()));
            }
            let count = fields.len() / 2 + usize::from(name == "polygon");
            if count > remaining {
                return Err(point_error(format!(
                    "path command count exceeds limit {max_commands}"
                )));
            }
            for pair in fields.chunks_exact(2) {
                context(cancel)?;
                let x = svg_length::number(pair[0])
                    .map_err(|_| point_error("expected finite unitless coordinates".into()))?;
                let y = svg_length::number(pair[1])
                    .map_err(|_| point_error("expected finite unitless coordinates".into()))?;
                geometry.commands.push(Command {
                    kind: u8::from(!geometry.commands.is_empty()),
                    points: [[x, y], [0.; 2], [0.; 2]],
                    ..Default::default()
                });
            }
            if name == "polygon" {
                geometry.commands.push(Command {
                    kind: 5,
                    ..Default::default()
                });
            }
            *used += geometry.commands.len();
        }
        _ => return Err(error(format!("internal unsupported geometry <{name}>"))),
    }
    Ok(geometry)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_geometry_fields_commands_counters_and_errors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-geometry-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let attrs = serde_json::from_value(row["attrs"].clone()).unwrap();
            let mut used = row["used"].as_u64().unwrap() as usize;
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap_or("") {
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
            let result = compile(
                "math",
                row["name"].as_str().unwrap(),
                &attrs,
                row["max_bytes"].as_u64().unwrap() as usize,
                row["maximum"].as_u64().unwrap() as usize,
                &mut used,
                cancel,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(used as u64, row["after"].as_u64().unwrap(), "{row}");
            if let Ok(g) = result {
                let expected: Geometry = serde_json::from_value(row["geometry"].clone()).unwrap();
                assert_eq!(g, expected, "{row}");
                let bits: Vec<_> = g
                    .bounds
                    .iter()
                    .chain(g.center.iter())
                    .chain(g.radii.iter())
                    .map(|v| v.to_bits())
                    .collect();
                assert_eq!(
                    serde_json::to_value(bits).unwrap(),
                    row["geometry"]["bits"],
                    "{row}"
                );
                for (command, expected) in g
                    .commands
                    .iter()
                    .zip(row["geometry"]["commands"].as_array().unwrap())
                {
                    let bits: Vec<_> = command
                        .points
                        .iter()
                        .flatten()
                        .chain(command.radii.iter())
                        .chain(std::iter::once(&command.rotation))
                        .map(|v| v.to_bits())
                        .collect();
                    assert_eq!(
                        serde_json::to_value(bits).unwrap(),
                        expected["bits"],
                        "{row}"
                    );
                }
            }
        }
    }
}
