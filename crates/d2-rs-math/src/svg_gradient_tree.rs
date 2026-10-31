// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/gradient.go
// validateGradientTree,compileGradientElement,compileGradientStop.
use super::{
    svg_declarations::{self, Rule},
    svg_gradient, svg_gradient_stop, svg_length,
    svg_primitive::{Paint, Stop},
    svg_stylesheet,
};
use std::collections::{BTreeMap, HashSet};
#[derive(Default)]
pub struct Element {
    pub name: String,
    pub id: String,
    pub children: Vec<usize>,
    pub attrs: BTreeMap<String, String>,
    pub order: Vec<String>,
    pub classes: Vec<String>,
    pub class_set: HashSet<String>,
    pub declarations: BTreeMap<String, String>,
    pub gradient: Option<Paint>,
    pub stop: Option<Stop>,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn validate(
    source: &str,
    elements: &[Element],
    root: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let mut stack = vec![(root, None)];
    while let Some((index, parent)) = stack.pop() {
        context(cancel)?;
        let element = &elements[index];
        let parent = parent.map(|i: usize| elements[i].name.as_str());
        if element.name == "linearGradient" {
            if parent != Some("defs") {
                return Err(error(
                    "element <linearGradient> must be a direct child of <defs>".into(),
                ));
            }
            if element.id.is_empty() {
                return Err(error("element <linearGradient> must declare an id".into()));
            }
            if element.children.is_empty() {
                return Err(error(
                    "element <linearGradient> must contain at least one <stop>".into(),
                ));
            }
            for &child in &element.children {
                let name = &elements[child].name;
                if name != "stop" {
                    return Err(error(format!(
                        "element <linearGradient> has unsupported child <{}>",
                        if name.len() > 64 {
                            "<oversized-name>"
                        } else {
                            name
                        }
                    )));
                }
            }
        } else if element.name == "stop" {
            if parent != Some("linearGradient") {
                return Err(error(
                    "element <stop> is only supported inside <linearGradient>".into(),
                ));
            }
            if !element.children.is_empty() {
                return Err(error("element <stop> cannot contain child elements".into()));
            }
        }
        for &child in element.children.iter().rev() {
            stack.push((child, Some(index)));
        }
    }
    Ok(())
}
pub fn compile(
    source: &str,
    element: &mut Element,
    rules: &[Rule],
    max_bytes: usize,
    max_functions: usize,
    used: &mut usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let allowed: &[&str] = match element.name.as_str() {
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
        _ => &[],
    };
    for name in &element.order {
        context(cancel)?;
        if !allowed.contains(&name.as_str()) {
            return Err(error(format!(
                "element <{}> has unsupported attribute {:?}",
                element.name,
                if name.len() > 64 {
                    "<oversized-name>"
                } else {
                    name
                }
            )));
        }
    }
    match element.name.as_str() {
        "linearGradient" => {
            element.gradient = Some(svg_gradient::compile(
                source,
                &element.attrs,
                max_bytes,
                max_functions,
                used,
                cancel,
            )?)
        }
        "stop" => {
            let (classes, set) =
                svg_stylesheet::classes(element.attrs.get("class").map(String::as_str), cancel)?;
            element.classes = classes;
            element.class_set = set;
            let mut mathjax = false;
            element.declarations = svg_declarations::collect(
                source,
                "stop",
                false,
                &element.attrs,
                &element.class_set,
                rules,
                &mut mathjax,
                cancel,
            )?;
            for property in element.declarations.keys() {
                if !matches!(property.as_str(), "stop-color" | "stop-opacity") {
                    return Err(error(format!(
                        "element <stop> has unsupported style property {property:?}"
                    )));
                }
            }
            let raw = element
                .attrs
                .get("offset")
                .ok_or_else(|| error("element <stop> is missing offset".into()))?;
            let offset = svg_length::unit_interval(raw)
                .map_err(|e| error(format!("element <stop> has invalid offset: {e}")))?;
            let mut stop = svg_gradient_stop::from_declarations(source, &element.declarations)?;
            stop.offset = offset;
            element.stop = Some(stop);
        }
        _ => {
            return Err(error(format!(
                "internal unsupported gradient element <{}>",
                element.name
            )));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_gradient_tree_admission() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-gradient-tree-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let child = row["child"].as_str().unwrap();
            let elements = [
                Element {
                    name: row["parent"].as_str().unwrap().into(),
                    children: vec![1],
                    ..Default::default()
                },
                Element {
                    name: "linearGradient".into(),
                    id: row["id"].as_str().unwrap().into(),
                    children: if child == "absent" {
                        Vec::new()
                    } else {
                        vec![2]
                    },
                    ..Default::default()
                },
                Element {
                    name: child.into(),
                    ..Default::default()
                },
            ];
            assert_eq!(
                validate("math", &elements, 0, None)
                    .err()
                    .unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{row}"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_stop_element_order_and_metadata() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-stop-element-v090.json"))
                .unwrap();
        for row in rows.as_array().unwrap() {
            let mut element = Element {
                name: "stop".into(),
                attrs: serde_json::from_value(row["attrs"].clone()).unwrap(),
                order: serde_json::from_value(row["order"].clone()).unwrap(),
                ..Default::default()
            };
            let rules: Vec<Rule> = serde_json::from_value(row["rules"].clone()).unwrap();
            let mut used = 0;
            let result = compile("math", &mut element, &rules, 10000, 100, &mut used, None);
            assert_eq!(
                result.err().unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(
                serde_json::to_value(element.classes).unwrap(),
                row["classes"],
                "{row}"
            );
            let expected: Option<Stop> = serde_json::from_value(row["stop"].clone()).unwrap();
            assert_eq!(element.stop, expected, "{row}");
            if !row["declarations"].is_null() {
                assert_eq!(
                    serde_json::to_value(element.declarations).unwrap(),
                    row["declarations"],
                    "{row}"
                );
            }
        }
    }
}
