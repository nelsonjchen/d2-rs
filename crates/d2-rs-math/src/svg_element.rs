// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/compile.go compileElement through
// nested viewport admission; geometry/use compilation follows this boundary.
use super::{
    svg_compile::non_rendering,
    svg_declarations::{self, Rule},
    svg_geometry::trim_space,
    svg_gradient_tree::{self, Element},
    svg_length, svg_stylesheet, svg_transform,
    svg_viewport::{self, Viewport},
};
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct State {
    pub transform: [f64; 6],
    pub viewport: Option<Viewport>,
}
#[derive(Debug, Default)]
pub struct Work {
    pub transforms: usize,
    pub mathjax_root: bool,
}
#[derive(Debug, PartialEq)]
pub enum Continuation {
    Complete,
    Geometry,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn allowed(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "svg" => &[
            "width",
            "height",
            "viewBox",
            "preserveAspectRatio",
            "version",
            "x",
            "y",
            "enable-background",
        ],
        "g" | "defs" => &[],
        "style" => &["type"],
        "title" | "metadata" => &["id"],
        "rdf:RDF" | "cc:Work" | "dc:format" | "dc:type" => &[],
        "sodipodi:namedview" => &[
            "id",
            "pagecolor",
            "bordercolor",
            "borderopacity",
            "objecttolerance",
            "gridtolerance",
            "guidetolerance",
            "fit-margin-bottom",
            "fit-margin-left",
            "fit-margin-right",
            "fit-margin-top",
            "showgrid",
        ],
        "path" => &["d"],
        "rect" => &["x", "y", "width", "height", "rx", "ry"],
        "circle" => &["cx", "cy", "r"],
        "ellipse" => &["cx", "cy", "rx", "ry"],
        "line" => &["x1", "y1", "x2", "y2"],
        "polyline" | "polygon" => &["points"],
        "image" => &["x", "y", "width", "height", "href", "preserveAspectRatio"],
        "text" => &["font-size", "font-family"],
        "use" => &["href", "x", "y", "overflow"],
        "clipPath" => &["id", "clipPathUnits"],
        "linearGradient" => &[
            "id",
            "x1",
            "y1",
            "x2",
            "y2",
            "gradientUnits",
            "gradientTransform",
            "spreadMethod",
        ],
        "stop" => &["offset", "class", "style", "stop-color", "stop-opacity"],
        _ => return None,
    })
}
/// Connect admission to existing declaration/transform/gradient/viewport stages.
/// Mutated fields and counters survive errors. Complete means the source returns
/// before geometry; Geometry requires the subsequent compileGeometry/use stage.
pub fn admit(
    source: &str,
    element: &mut Element,
    state: &mut State,
    root: bool,
    rendering_children: bool,
    rules: &[Rule],
    max_bytes: usize,
    max_transforms: usize,
    work: &mut Work,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Continuation, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let name = element.name.as_str();
    let specific =
        allowed(name).ok_or_else(|| error(format!("internal unsupported element <{name}>")))?;
    if matches!(name, "linearGradient" | "stop") {
        svg_gradient_tree::compile(
            source,
            element,
            rules,
            max_bytes,
            max_transforms,
            &mut work.transforms,
            cancel,
        )?;
        return Ok(Continuation::Complete);
    }
    for attribute in &element.order {
        context(cancel)?;
        if name != "style"
            && !non_rendering(name)
            && (matches!(attribute.as_str(), "id" | "class" | "style")
                || svg_declarations::PROPERTIES.contains(&attribute.as_str()))
        {
            continue;
        }
        if !specific.contains(&attribute.as_str()) {
            return Err(error(format!(
                "element <{name}> has unsupported attribute {:?}",
                if attribute.len() > 64 {
                    "<oversized-name>"
                } else {
                    attribute
                }
            )));
        }
    }
    if non_rendering(name) {
        state.transform = [1., 0., 0., 1., 0., 0.];
        return Ok(Continuation::Complete);
    }
    if !matches!(name, "svg" | "g" | "defs" | "clipPath") && rendering_children {
        return Err(error(format!(
            "element <{name}> cannot contain child elements"
        )));
    }
    if name == "style" {
        state.transform = [1., 0., 0., 1., 0., 0.];
        return Ok(Continuation::Complete);
    }
    if name == "svg" {
        if root {
            if let Some(raw) = element.attrs.get("version") {
                if !matches!(trim_space(raw, cancel)?, "1.0" | "1.1") {
                    return Err(error("root <svg> has unsupported version".into()));
                }
            }
            for name in ["x", "y"] {
                if let Some(raw) = element.attrs.get(name) {
                    if !matches!(svg_length::length(raw,true),Ok(value) if value==0.) {
                        return Err(error(format!(
                            "root <svg> {name} must be a zero absolute length"
                        )));
                    }
                }
            }
            if let Some(raw) = element.attrs.get("enable-background") {
                svg_declarations::background(raw, cancel).map_err(|reason| {
                    context(cancel).err().unwrap_or_else(|| {
                        error(format!(
                            "root <svg> has invalid enable-background: {reason}"
                        ))
                    })
                })?;
            }
        } else if element.attrs.contains_key("version") {
            return Err(error("nested <svg> version is unsupported".into()));
        } else if element.attrs.contains_key("enable-background") {
            return Err(error(
                "nested <svg> enable-background is unsupported".into(),
            ));
        }
    }
    let (classes, set) =
        svg_stylesheet::classes(element.attrs.get("class").map(String::as_str), cancel)?;
    element.classes = classes;
    element.class_set = set;
    element.declarations = svg_declarations::collect(
        source,
        name,
        root,
        &element.attrs,
        &element.class_set,
        rules,
        &mut work.mathjax_root,
        cancel,
    )?;
    for property in element.declarations.keys() {
        if matches!(property.as_str(), "stop-color" | "stop-opacity") {
            return Err(error(format!(
                "element <{name}> has unsupported style property {property:?}"
            )));
        }
        if name == "defs" && property == "clip-path" {
            return Err(error(
                "element <defs> cannot carry clip-path because it is not an emitted element".into(),
            ));
        }
    }
    state.transform = [1., 0., 0., 1., 0., 0.];
    if let Some(raw) = element
        .declarations
        .get("transform")
        .filter(|raw| !raw.is_empty() && !raw.eq_ignore_ascii_case("none"))
    {
        if name == "svg" {
            return Err(error(if root{"root <svg> transform is unsupported because its outer transform order and origin cannot be represented"}else{"nested <svg> transform is unsupported"}.into()));
        }
        if work.transforms >= max_transforms {
            return Err(error(format!(
                "transform function count exceeds limit {max_transforms}"
            )));
        }
        match svg_transform::parse(
            source,
            raw,
            max_bytes,
            max_transforms - work.transforms,
            cancel,
        ) {
            Ok((matrix, count)) => {
                state.transform = matrix;
                work.transforms += count;
            }
            Err(reason) => {
                state.transform = [0.; 6];
                return Err(reason);
            }
        }
    }
    if name == "svg" && !root {
        if element.declarations.contains_key("clip-path") {
            return Err(error(
                "nested <svg> clip-path is unsupported because the viewport clip is already active"
                    .into(),
            ));
        }
        state.viewport = Some(svg_viewport::compile(
            source,
            &element.attrs,
            true,
            false,
            cancel,
        )?);
    }
    Ok(Continuation::Geometry)
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_element_admission_and_partial_state() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-element-v090.json"
        ))
        .unwrap();
        let rules = vec![Rule {
            class: "a".into(),
            declarations: std::collections::BTreeMap::from([("fill".into(), "red".into())]),
        }];
        for row in rows.as_array().unwrap() {
            let mut element = Element {
                name: row["name"].as_str().unwrap().into(),
                attrs: serde_json::from_value(row["attrs"].clone()).unwrap(),
                order: serde_json::from_value(row["order"].clone()).unwrap(),
                ..Default::default()
            };
            let mut state = State::default();
            let mut work = Work {
                transforms: row["used"].as_u64().unwrap() as usize,
                ..Default::default()
            };
            let result = admit(
                "math",
                &mut element,
                &mut state,
                row["root"].as_bool().unwrap(),
                row["child"] == "path",
                &rules,
                10000,
                2,
                &mut work,
                None,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(
                serde_json::to_value(element.classes).unwrap(),
                row["classes"],
                "{row}"
            );
            assert_eq!(
                serde_json::to_value(element.declarations).unwrap(),
                row["declarations"],
                "{row}"
            );
            let expected: State = serde_json::from_value(row["state"].clone()).unwrap();
            assert_eq!(state, expected, "{row}");
            let expected: Option<super::super::svg_primitive::Paint> =
                serde_json::from_value(row["gradient"].clone()).unwrap();
            assert_eq!(element.gradient, expected, "{row}");
            let expected: Option<super::super::svg_primitive::Stop> =
                serde_json::from_value(row["stop"].clone()).unwrap();
            assert_eq!(element.stop, expected, "{row}");
            assert_eq!(
                work.transforms as u64,
                row["after_transforms"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                work.mathjax_root,
                row["mathjax"].as_bool().unwrap(),
                "{row}"
            );
            if let Ok(next) = result {
                assert_eq!(
                    if next == Continuation::Complete {
                        "complete"
                    } else {
                        "geometry"
                    },
                    row["continuation"].as_str().unwrap(),
                    "{row}"
                );
            }
        }
    }
}
