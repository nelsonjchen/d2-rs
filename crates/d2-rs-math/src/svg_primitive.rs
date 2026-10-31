// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2svgimport/compile.go svgGeometry.primitive,
// style.go paintWithOpacity and d2scene/paint.go.
use super::svg_geometry::Geometry;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Stop {
    pub offset: f64,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum Paint {
    #[serde(rename = "solid")]
    Solid { color: [u8; 4] },
    #[serde(rename = "linear")]
    Linear {
        start: [f64; 2],
        end: [f64; 2],
        stops: Vec<Stop>,
        units: u8,
        transform: [f64; 6],
        spread: u8,
    },
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Style {
    pub visible: bool,
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub fill_current: bool,
    pub stroke_current: bool,
    pub color: [u8; 4],
    pub fill_opacity: f64,
    pub stroke_opacity: f64,
    pub stroke_width: f64,
    pub cap: u8,
    pub join: u8,
    pub miter_limit: f64,
    pub dashes: Vec<f64>,
    pub dash_offset: f64,
    pub fill_rule: u8,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    pub cap: u8,
    pub join: u8,
    pub miter_limit: f64,
    pub dashes: Option<Vec<f64>>,
    pub dash_offset: f64,
}
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Primitive {
    pub geometry: Geometry,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    pub fill_rule: u8,
}
/// Independent emitted-scene counters. Path work commits before node admission,
/// matching instantiate's ordering, including a later node-budget failure.
#[derive(Debug, Default)]
pub struct Budget {
    pub max_commands: usize,
    pub max_elements: usize,
    pub commands: usize,
    pub elements: usize,
}
impl Budget {
    pub fn admit(&mut self, source: &str, commands: Option<usize>) -> Result<(), String> {
        if let Some(commands) = commands {
            if self.commands > self.max_commands || commands > self.max_commands - self.commands {
                return Err(format!(
                    "d2svgimport: {source}: emitted path command count exceeds limit {}",
                    self.max_commands
                ));
            }
            self.commands += commands;
        }
        if self.elements >= self.max_elements {
            return Err(format!(
                "d2svgimport: {source}: emitted element count exceeds limit {}",
                self.max_elements
            ));
        }
        self.elements += 1;
        Ok(())
    }
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
/// Apply already admitted [0,1] opacity without mutating retained paint data.
/// Alpha-zero paint remains present, as it does in the original scene.
pub fn opacity(
    paint: Option<&Paint>,
    value: f64,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Option<Paint>, String> {
    let Some(paint) = paint else {
        return Ok(None);
    };
    match paint {
        Paint::Solid { color } => {
            let mut color = *color;
            color[3] = (f64::from(color[3]) * value).round() as u8;
            Ok(Some(Paint::Solid { color }))
        }
        Paint::Linear {
            start,
            end,
            stops,
            units,
            transform,
            spread,
        } => {
            context(cancel)?;
            let mut copied = Vec::with_capacity(stops.len());
            for (index, stop) in stops.iter().enumerate() {
                if index & 255 == 0 {
                    context(cancel)?;
                }
                copied.push(stop.clone());
            }
            for (index, stop) in copied.iter_mut().enumerate() {
                if index & 255 == 0 {
                    context(cancel)?;
                }
                stop.color[3] = (f64::from(stop.color[3]) * value).round() as u8;
            }
            Ok(Some(Paint::Linear {
                start: *start,
                end: *end,
                stops: copied,
                units: *units,
                transform: *transform,
                spread: *spread,
            }))
        }
    }
}
/// Emit admitted geometry using computed source style. Images retain their asset
/// identity/box/aspect and bypass fill/stroke paint, matching source primitives.
pub fn emit(
    geometry: &Geometry,
    style: &Style,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(Option<Primitive>, usize), String> {
    if !style.visible {
        return Ok((None, 0));
    }
    if geometry.kind == 4 {
        return Ok((
            Some(Primitive {
                geometry: geometry.clone(),
                fill: None,
                stroke: None,
                fill_rule: 0,
            }),
            0,
        ));
    }
    let current = Paint::Solid { color: style.color };
    let mut fill = opacity(
        if style.fill_current {
            Some(&current)
        } else {
            style.fill.as_ref()
        },
        style.fill_opacity,
        cancel,
    )?;
    if geometry.force_no_fill {
        fill = None;
    }
    let stroke = opacity(
        if style.stroke_current {
            Some(&current)
        } else {
            style.stroke.as_ref()
        },
        style.stroke_opacity,
        cancel,
    )?;
    let stroke = stroke
        .filter(|_| style.stroke_width > 0.)
        .map(|paint| Stroke {
            paint,
            width: style.stroke_width,
            cap: style.cap,
            join: style.join,
            miter_limit: style.miter_limit,
            dashes: (!style.dashes.is_empty()).then(|| style.dashes.clone()),
            dash_offset: style.dash_offset,
        });
    let commands = match geometry.kind {
        1 if geometry.bounds[2] != 0. && geometry.bounds[3] != 0. => 0,
        2 if geometry.radii[0] != 0. && geometry.radii[1] != 0. => 0,
        3 if !geometry.commands.is_empty() => geometry.commands.len(),
        _ => return Ok((None, 0)),
    };
    if fill.is_none() && stroke.is_none() {
        return Ok((None, 0));
    }
    let owned = geometry.clone();
    Ok((
        Some(Primitive {
            geometry: owned,
            fill,
            stroke,
            fill_rule: if geometry.kind == 3 {
                style.fill_rule
            } else {
                0
            },
        }),
        commands,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_emitted_budget_order_and_partial_charges() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-emitted-budget-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut budget = Budget {
                max_commands: row["maximum"].as_u64().unwrap() as usize,
                max_elements: row["maximum_nodes"].as_u64().unwrap() as usize,
                commands: row["used"].as_u64().unwrap() as usize,
                elements: row["nodes"].as_u64().unwrap() as usize,
            };
            let error = budget
                .admit(
                    "math",
                    match row["name"].as_str().unwrap() {
                        "path" => Some(2),
                        "rect" => Some(0),
                        _ => None,
                    },
                )
                .err()
                .unwrap_or_default();
            assert_eq!(error, row["error"].as_str().unwrap(), "{row}");
            assert_eq!(
                budget.commands as u64,
                row["after_commands"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                budget.elements as u64,
                row["after_nodes"].as_u64().unwrap(),
                "{row}"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_typed_primitives_opacity_and_owned_paint() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-primitive-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let geometry: Geometry = serde_json::from_value(row["geometry"].clone()).unwrap();
            let style: Style = serde_json::from_value(row["style"].clone()).unwrap();
            let result = emit(&geometry, &style, None);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok((primitive, count)) = result {
                let expected: Option<Primitive> =
                    serde_json::from_value(row["primitive"].clone()).unwrap();
                assert_eq!(primitive, expected, "{row}");
                assert_eq!(count as u64, row["count"].as_u64().unwrap(), "{row}");
            }
            let after_fill: Option<Paint> =
                serde_json::from_value(row["after_fill"].clone()).unwrap();
            let after_stroke: Option<Paint> =
                serde_json::from_value(row["after_stroke"].clone()).unwrap();
            assert_eq!(style.fill, after_fill, "{row}");
            assert_eq!(style.stroke, after_stroke, "{row}");
        }
    }
}
