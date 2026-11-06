// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/mathjax_text.go.
use super::{
    svg_element::State,
    svg_gradient_tree::Element,
    svg_length,
    svg_path::Command,
    svg_primitive::Paint,
    svg_style::Style,
    svg_viewport,
    svg_xml_document::{Document, Element as Node},
};
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn only(node: &Node, names: &[&str]) -> bool {
    node.attributes.values.len() == names.len()
        && names
            .iter()
            .all(|name| node.attributes.values.contains_key(*name))
}
fn vertical_align(raw: &str) -> bool {
    let Some(value) = raw.trim().strip_prefix("vertical-align:") else {
        return false;
    };
    let value = value.trim();
    let value = value.strip_suffix(';').unwrap_or(value).trim();
    !value.contains(';') && svg_length::mathjax_ex(value).is_ok()
}
fn frozen_root(
    source: &str,
    doc: &Document,
    elements: &[Element],
    mathjax: bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> bool {
    let root = &doc.elements[0];
    let attrs = &root.attributes.values;
    let metadata = &root.attributes.metadata;
    if !mathjax
        || root.name != "svg"
        || doc.namespace != super::svg_xml_admission::SVG
        || root.attributes.order.len() != 4
        || !only(root, &["style", "width", "height", "viewBox"])
        || metadata.len() != 2
        || metadata.get("role").map(String::as_str) != Some("img")
        || metadata.get("focusable").map(String::as_str) != Some("false")
        || !vertical_align(&attrs["style"])
    {
        return false;
    }
    let width = svg_length::mathjax_ex(&attrs["width"]);
    let height = svg_length::mathjax_ex(&attrs["height"]);
    let view_box = svg_viewport::view_box(source, attrs, "root", cancel);
    if !matches!(width,Ok(w) if w>0.)
        || !matches!(height,Ok(h) if h>0.)
        || !matches!(view_box,Ok(Some(b)) if b[0]==0.)
        || root.children.len() != 1
    {
        return false;
    }
    let wi = root.children[0];
    let wrapper = &doc.elements[wi];
    let attrs = &wrapper.attributes.values;
    if wrapper.name != "g"
        || wrapper.attributes.order.len() != 4
        || !only(wrapper, &["stroke", "fill", "stroke-width", "transform"])
        || attrs["stroke"] != "currentColor"
        || attrs["fill"] != "currentColor"
        || attrs["stroke-width"] != "0"
        || attrs["transform"] != "scale(1,-1)"
        || !wrapper.attributes.metadata.is_empty()
        || !wrapper.attributes.id.is_empty()
        || !elements[wi].classes.is_empty()
        || wrapper.children.len() != 1
    {
        return false;
    }
    let mi = wrapper.children[0];
    let node = &doc.elements[mi];
    node.name == "g"
        && node.attributes.order.is_empty()
        && node.attributes.metadata.len() == 1
        && node
            .attributes
            .metadata
            .get("data-mml-node")
            .map(String::as_str)
            == Some("math")
        && node.attributes.id.is_empty()
        && elements[mi].classes.is_empty()
}
pub fn validate(
    source: &str,
    doc: &Document,
    elements: &[Element],
    states: &[State],
    mathjax: bool,
    index: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: &str| format!("d2svgimport: {source}: {reason}");
    let node = &doc.elements[index];
    if node.name != "text" {
        return Err(error(
            "internal MathJax text validation on a non-text element",
        ));
    }
    if !node.children.is_empty() {
        return Err(error(
            "frozen MathJax <text> fallback cannot contain child elements",
        ));
    }
    if node.text != "µ".as_bytes() {
        return Err(error(
            "frozen MathJax <text> fallback supports only one U+00B5 micro sign",
        ));
    }
    if node.attributes.order.len() != 3 || !only(node, &["transform", "font-size", "font-family"]) {
        return Err(error(
            "frozen MathJax <text> fallback has unsupported layout or presentation attributes",
        ));
    }
    let attrs = &node.attributes.values;
    if attrs["transform"] != "scale(1,-1)"
        || attrs["font-size"] != "884px"
        || attrs["font-family"] != "serif"
    {
        return Err(error(
            "frozen MathJax <text> fallback must use scale(1,-1), 884px, and serif",
        ));
    }
    if node.attributes.metadata.len() != 1
        || node
            .attributes
            .metadata
            .get("data-variant")
            .map(String::as_str)
            != Some("normal")
    {
        return Err(error(
            "frozen MathJax <text> fallback must declare data-variant=\"normal\"",
        ));
    }
    if !frozen_root(source, doc, elements, mathjax, cancel) {
        context(cancel)?;
        return Err(error(
            "painted <text> is supported only inside a verified frozen MathJax root",
        ));
    }
    let unit = node.parent.ok_or_else(|| {
        error("frozen MathJax <text> fallback must be the sole child of an mi.MathML-Unit group")
    })?;
    let parent = &doc.elements[unit];
    let attrs = &parent.attributes.values;
    let optional = (attrs.len() == 1 && attrs.contains_key("class"))
        || (attrs.len() == 2 && attrs.contains_key("class") && attrs.contains_key("transform"));
    if parent.name != "g"
        || parent.children.len() != 1
        || elements[unit].class_set.len() != 1
        || !elements[unit].class_set.contains("MathML-Unit")
        || parent.attributes.metadata.len() != 1
        || parent
            .attributes
            .metadata
            .get("data-mml-node")
            .map(String::as_str)
            != Some("mi")
        || !optional
    {
        return Err(error(
            "frozen MathJax <text> fallback must be the sole child of an mi.MathML-Unit group",
        ));
    }
    if let Some(raw) = attrs.get("transform") {
        let [a, b, c, d, e, f] = states[unit].transform;
        if a != 1.
            || b != 0.
            || c != 0.
            || d != 1.
            || e < 0.
            || f != 0.
            || !states[unit].transform.iter().all(|v| v.is_finite())
            || !raw.starts_with("translate(")
        {
            return Err(error(
                "frozen MathJax unit group has unsupported placement transform",
            ));
        }
    }
    let math_node = doc.elements[doc.elements[0].children[0]].children[0];
    let mut ancestor = parent.parent;
    let mut inside = false;
    while let Some(index) = ancestor {
        if index == 0 {
            break;
        }
        if index == math_node {
            inside = true;
            break;
        }
        ancestor = doc.elements[index].parent;
    }
    if !inside {
        return Err(error(
            "frozen MathJax <text> fallback is outside the root math node",
        ));
    }
    context(cancel)
}
pub fn commands(
    source: &str,
    doc: &Document,
    elements: &[Element],
    states: &[State],
    mathjax: bool,
    index: usize,
    max_commands: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<Command>, String> {
    validate(source, doc, elements, states, mathjax, index, cancel)?;
    context(cancel)?;
    if 16 > max_commands {
        return Err(format!(
            "d2svgimport: {source}: emitted path command count exceeds limit {max_commands}"
        ));
    }
    context(cancel)?;
    let point = |kind, x, y| Command {
        kind,
        points: [[x, y], [0.; 2], [0.; 2]],
        ..Default::default()
    };
    let cubic = |a, b, c| Command {
        kind: 3,
        points: [a, b, c],
        ..Default::default()
    };
    Ok(vec![
        point(0, 56., -360.),
        point(1, 136., -360.),
        point(1, 136., -92.),
        cubic([136., -35.], [172., 0.], [224., 0.]),
        cubic([276., 0.], [316., -35.], [316., -92.]),
        point(1, 316., -360.),
        point(1, 396., -360.),
        point(1, 396., -84.),
        cubic([396., -38.], [416., -16.], [456., -16.]),
        point(1, 456., 58.),
        cubic([407., 58.], [371., 39.], [346., 9.]),
        cubic([316., 53.], [267., 76.], [210., 76.]),
        cubic([181., 76.], [156., 68.], [136., 52.]),
        point(1, 136., 188.),
        point(1, 56., 188.),
        Command {
            kind: 5,
            ..Default::default()
        },
    ])
}
pub fn validate_style(source: &str, style: &Style) -> Result<(), String> {
    if !style.display
        || !style.paint.visible
        || (!style.paint.fill_current && !matches!(style.paint.fill, Some(Paint::Solid { .. })))
        || style.paint.fill_opacity != 1.
        || style.paint.stroke_width != 0.
        || style.paint.fill_rule != 0
        || !style.clip_path.is_empty()
        || style.opacity != 1.
    {
        return Err(format!(
            "d2svgimport: {source}: frozen MathJax <text> fallback has unsupported inherited text paint or visibility"
        ));
    }
    Ok(())
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_frozen_mathjax_micro_tree_outline_context_and_paint() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-frozen-text-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            if row["operation"] == "style" {
                let mut parent = Style::default();
                parent.paint.stroke_width = 0.;
                let values = std::collections::BTreeMap::from([(
                    row["property"].as_str().unwrap().into(),
                    row["value"].as_str().unwrap().into(),
                )]);
                let result = super::super::svg_style::compute(
                    "math",
                    "text",
                    &parent,
                    &values,
                    &Default::default(),
                    None,
                )
                .and_then(|style| {
                    let expected: Style = serde_json::from_value(row["style"].clone()).unwrap();
                    assert_eq!(
                        serde_json::to_value(&style).unwrap(),
                        serde_json::to_value(expected).unwrap(),
                        "{row}"
                    );
                    validate_style("math", &style)
                });
                assert_eq!(
                    result.err().unwrap_or_default(),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                continue;
            }
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
            let result = super::super::svg_xml_document::parse(
                "math",
                row["input"].as_str().unwrap(),
                [10000, 100, 100, 100, 10000, 100, 100, 100, 100],
                None,
            )
            .and_then(|doc| {
                let mut rules = Vec::new();
                let mut elements =
                    super::super::svg_compile::prepare("math", &doc, &mut rules, None)?;
                let mut states = (0..elements.len())
                    .map(|_| State::default())
                    .collect::<Vec<_>>();
                let mut work = super::super::svg_element::Work::default();
                // Phase oracle admits parent state before text geometry; these
                // documents contain only groups/text and non-rendering children.
                for index in 0..elements.len() {
                    let rendering = doc.elements[index].children.iter().any(|&child| {
                        !super::super::svg_compile::non_rendering(&doc.elements[child].name)
                    });
                    super::super::svg_element::admit(
                        "math",
                        &mut elements[index],
                        &mut states[index],
                        index == 0,
                        rendering,
                        &rules,
                        10000,
                        100,
                        &mut work,
                        None,
                    )?;
                }
                let index = doc
                    .elements
                    .iter()
                    .position(|node| node.name == "text")
                    .unwrap();
                commands(
                    "math",
                    &doc,
                    &elements,
                    &states,
                    work.mathjax_root,
                    index,
                    row["maximum"].as_u64().unwrap() as usize,
                    cancel,
                )
            });
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(output) = result {
                let expected: Vec<Command> =
                    serde_json::from_value(row["commands"].clone()).unwrap();
                assert_eq!(output, expected, "{row}");
            }
        }
    }
}
