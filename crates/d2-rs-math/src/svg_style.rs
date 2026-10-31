// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/style.go computeStyle.
use super::{
    svg_color, svg_length,
    svg_primitive::{Paint, Style as PaintStyle},
    svg_resources::{self, Resource},
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Style {
    pub paint: PaintStyle,
    pub opacity: f64,
    pub display: bool,
    pub clip_rule: u8,
    pub clip_path: String,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            paint: PaintStyle {
                visible: true,
                fill: Some(Paint::Solid {
                    color: [0, 0, 0, 255],
                }),
                stroke: None,
                fill_current: false,
                stroke_current: false,
                color: [0, 0, 0, 255],
                fill_opacity: 1.,
                stroke_opacity: 1.,
                stroke_width: 1.,
                cap: 0,
                join: 0,
                miter_limit: 4.,
                dashes: Vec::new(),
                dash_offset: 0.,
                fill_rule: 0,
            },
            opacity: 1.,
            display: true,
            clip_rule: 0,
            clip_path: String::new(),
        }
    }
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn keyword(value: &str) -> Result<String, String> {
    if value.len() > 64 {
        return Err("keyword is too long".into());
    }
    Ok(String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes())).unwrap())
}
pub fn compute(
    source: &str,
    name: &str,
    parent: &Style,
    values: &BTreeMap<String, String>,
    resources: &BTreeMap<String, Resource>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Style, String> {
    let mut style = parent.clone();
    style.opacity = 1.;
    style.display = true;
    style.clip_path.clear();
    let error = |property: &str, reason: String| {
        format!("d2svgimport: {source}: element <{name}> has invalid {property}: {reason}")
    };
    let inherit = |value: &str| value.eq_ignore_ascii_case("inherit");
    if let Some(value) = values.get("color") {
        if !inherit(value) && !value.eq_ignore_ascii_case("currentColor") {
            style.paint.color = svg_color::solid(value).map_err(|e| error("color", e))?;
        }
    }
    for property in ["fill", "stroke"] {
        if let Some(value) = values.get(property) {
            if !inherit(value) {
                let (paint, current) = svg_resources::paint(value, resources, cancel)
                    .map_err(|e| error(property, e))?;
                if property == "fill" {
                    style.paint.fill = paint;
                    style.paint.fill_current = current;
                } else {
                    style.paint.stroke = paint;
                    style.paint.stroke_current = current;
                }
            }
        }
    }
    for property in ["fill-opacity", "stroke-opacity"] {
        if let Some(value) = values.get(property) {
            if !inherit(value) {
                let opacity = svg_length::unit_interval(value).map_err(|e| error(property, e))?;
                if property == "fill-opacity" {
                    style.paint.fill_opacity = opacity;
                } else {
                    style.paint.stroke_opacity = opacity;
                }
            }
        }
    }
    if let Some(value) = values.get("stroke-width") {
        if !inherit(value) {
            let width = svg_length::length(value, true).map_err(|e| error("stroke-width", e))?;
            if width < 0. {
                return Err(error("stroke-width", "must not be negative".into()));
            }
            style.paint.stroke_width = width;
        }
    }
    for property in ["stroke-linecap", "stroke-linejoin"] {
        if let Some(value) = values.get(property) {
            let word = keyword(value).map_err(|e| error(property, e))?;
            let result = if property == "stroke-linecap" {
                match word.as_str() {
                    "inherit" => parent.paint.cap,
                    "butt" => 0,
                    "round" => 1,
                    "square" => 2,
                    _ => return Err(error(property, "unsupported value".into())),
                }
            } else {
                match word.as_str() {
                    "inherit" => parent.paint.join,
                    "miter" => 0,
                    "round" => 1,
                    "bevel" => 2,
                    _ => return Err(error(property, "unsupported value".into())),
                }
            };
            if property == "stroke-linecap" {
                style.paint.cap = result;
            } else {
                style.paint.join = result;
            }
        }
    }
    if let Some(value) = values.get("stroke-miterlimit") {
        if !inherit(value) {
            let limit = svg_length::number(value).map_err(|e| error("stroke-miterlimit", e))?;
            if limit < 1. {
                return Err(error("stroke-miterlimit", "must be at least 1".into()));
            }
            style.paint.miter_limit = limit;
        }
    }
    if let Some(value) = values.get("stroke-dasharray") {
        if !inherit(value) {
            style.paint.dashes = if value.eq_ignore_ascii_case("none") {
                Vec::new()
            } else {
                svg_length::dashes(value, cancel)
                    .map_err(|e| {
                        context(cancel)
                            .err()
                            .unwrap_or_else(|| error("stroke-dasharray", e))
                    })?
                    .unwrap_or_default()
            };
        }
    }
    if let Some(value) = values.get("stroke-dashoffset") {
        if !inherit(value) {
            style.paint.dash_offset =
                svg_length::length(value, true).map_err(|e| error("stroke-dashoffset", e))?;
        }
    }
    for property in ["fill-rule", "clip-rule"] {
        if let Some(value) = values.get(property) {
            let word = keyword(value).map_err(|e| error(property, e))?;
            let rule = match word.as_str() {
                "inherit" => {
                    if property == "fill-rule" {
                        parent.paint.fill_rule
                    } else {
                        parent.clip_rule
                    }
                }
                "nonzero" => 0,
                "evenodd" => 1,
                _ => return Err(error(property, "unsupported value".into())),
            };
            if property == "fill-rule" {
                style.paint.fill_rule = rule;
            } else {
                style.clip_rule = rule;
            }
        }
    }
    if let Some(value) = values.get("clip-path") {
        style.clip_path = if value.eq_ignore_ascii_case("none") {
            String::new()
        } else {
            svg_resources::clip(value, resources, cancel).map_err(|e| error("clip-path", e))?
        };
    }
    if let Some(value) = values.get("opacity") {
        style.opacity = if inherit(value) {
            parent.opacity
        } else {
            svg_length::unit_interval(value).map_err(|e| error("opacity", e))?
        };
    }
    if let Some(value) = values.get("display") {
        let word = keyword(value).map_err(|e| error("display", e))?;
        style.display = match word.as_str() {
            "inherit" => parent.display,
            "none" => false,
            "inline" | "block" => true,
            _ => return Err(error("display", "unsupported value".into())),
        };
    }
    if let Some(value) = values.get("visibility") {
        let word = keyword(value).map_err(|e| error("visibility", e))?;
        style.paint.visible = match word.as_str() {
            "inherit" => parent.paint.visible,
            "visible" => true,
            "hidden" | "collapse" => false,
            _ => return Err(error("visibility", "unsupported value".into())),
        };
    }
    Ok(style)
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    use crate::svg_primitive::Stop;
    #[test]
    fn original_computed_style_values_inheritance_and_error_order() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-style-v090.json"
        ))
        .unwrap();
        let resources = BTreeMap::from([
            (
                "g".into(),
                Resource {
                    name: "linearGradient".into(),
                    gradient: Some(Paint::Linear {
                        start: [0., 0.],
                        end: [1., 0.],
                        stops: vec![
                            Stop {
                                offset: 0.,
                                color: [255, 0, 0, 255],
                            },
                            Stop {
                                offset: 1.,
                                color: [0, 0, 255, 255],
                            },
                        ],
                        units: 0,
                        transform: [1., 0., 0., 1., 0., 0.],
                        spread: 0,
                    }),
                },
            ),
            (
                "c".into(),
                Resource {
                    name: "clipPath".into(),
                    gradient: None,
                },
            ),
            (
                "🙂".into(),
                Resource {
                    name: "clipPath".into(),
                    gradient: None,
                },
            ),
        ]);
        for row in rows.as_array().unwrap() {
            let parent: Style = serde_json::from_value(row["parent"].clone()).unwrap();
            let values = serde_json::from_value(row["values"].clone()).unwrap();
            let result = compute("math", "path", &parent, &values, &resources, None);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(style) = result {
                let expected: Style = serde_json::from_value(row["style"].clone()).unwrap();
                let actual = serde_json::to_value(style).unwrap();
                let expected = serde_json::to_value(expected).unwrap();
                assert_eq!(actual, expected, "{row}");
            }
        }
    }
}
