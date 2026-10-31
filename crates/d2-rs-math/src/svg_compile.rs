// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/compile.go, boilerplate.go,
// stylesheet.go. Parsed-tree compilation admission, before compileElement.
use super::{
    svg_clip, svg_declarations::Rule, svg_geometry::trim_space, svg_gradient_tree, svg_stylesheet,
    svg_xml_document::Document,
};
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn non_rendering(name: &str) -> bool {
    matches!(
        name,
        "title"
            | "metadata"
            | "rdf:RDF"
            | "cc:Work"
            | "dc:format"
            | "dc:type"
            | "sodipodi:namedview"
    )
}
fn display(name: &str) -> &str {
    if name.len() > 64 {
        "<oversized-name>"
    } else {
        name
    }
}
/// The source performs topology/text validation in child order, then a separate
/// cardinality walk whose sibling order is reversed. Keep their error precedence.
pub fn validate_metadata(
    source: &str,
    document: &Document,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let mut stack = vec![(0, None)];
    while let Some((index, parent)) = stack.pop() {
        context(cancel)?;
        let element = &document.elements[index];
        let parent_name = parent.map(|i: usize| document.elements[i].name.as_str());
        if let Some(name) = parent_name {
            match name {
                "metadata" if element.name != "rdf:RDF" => {
                    return Err(error(format!(
                        "element <metadata> has unsupported child <{}>",
                        display(&element.name)
                    )));
                }
                "rdf:RDF" if element.name != "cc:Work" => {
                    return Err(error(format!(
                        "element <rdf:RDF> has unsupported child <{}>",
                        display(&element.name)
                    )));
                }
                "cc:Work" if !matches!(element.name.as_str(), "dc:format" | "dc:type") => {
                    return Err(error(format!(
                        "element <cc:Work> has unsupported child <{}>",
                        display(&element.name)
                    )));
                }
                "title" | "dc:format" | "dc:type" | "sodipodi:namedview" => {
                    return Err(error(format!(
                        "non-rendering element <{name}> cannot contain child elements"
                    )));
                }
                _ => {}
            }
        }
        match element.name.as_str() {
            "metadata" | "sodipodi:namedview" if parent != Some(0) => {
                return Err(error(format!(
                    "non-rendering element <{}> must be a direct child of the root <svg>",
                    element.name
                )));
            }
            "rdf:RDF" if parent_name != Some("metadata") => {
                return Err(error(
                    "element <rdf:RDF> is only supported inside <metadata>".into(),
                ));
            }
            "cc:Work" if parent_name != Some("rdf:RDF") => {
                return Err(error(
                    "element <cc:Work> is only supported inside <rdf:RDF>".into(),
                ));
            }
            "dc:format" | "dc:type" if parent_name != Some("cc:Work") => {
                return Err(error(format!(
                    "element <{}> is only supported inside <cc:Work>",
                    element.name
                )));
            }
            _ => {}
        }
        if element.name == "dc:format" {
            let text = String::from_utf8_lossy(&element.text);
            if trim_space(&text, cancel)? != "image/svg+xml" {
                return Err(error(
                    "element <dc:format> has unsupported metadata text".into(),
                ));
            }
        }
        stack.extend(
            element
                .children
                .iter()
                .rev()
                .map(|&child| (child, Some(index))),
        );
    }
    let mut stack = vec![0];
    while let Some(index) = stack.pop() {
        context(cancel)?;
        let element = &document.elements[index];
        let name = |position: usize| document.elements[element.children[position]].name.as_str();
        match element.name.as_str() {
            "metadata" if element.children.len() != 1 || name(0) != "rdf:RDF" => {
                return Err(error(
                    "element <metadata> must contain exactly one <rdf:RDF>".into(),
                ));
            }
            "rdf:RDF" if element.children.len() != 1 || name(0) != "cc:Work" => {
                return Err(error(
                    "element <rdf:RDF> must contain exactly one <cc:Work>".into(),
                ));
            }
            "cc:Work"
                if element.children.len() != 2
                    || !matches!(
                        (name(0), name(1)),
                        ("dc:format", "dc:type") | ("dc:type", "dc:format")
                    ) =>
            {
                return Err(error(
                    "element <cc:Work> must contain exactly one <dc:format> and one <dc:type>"
                        .into(),
                ));
            }
            _ => {}
        }
        stack.extend(element.children.iter().copied());
    }
    Ok(())
}
/// Source compile's first four passes. Earlier stylesheet rules survive a later
/// failure, as in the importer. Returned resource records retain parsed identity,
/// attribute order and IDs for subsequent element/resource compilation.
pub fn prepare(
    source: &str,
    document: &Document,
    rules: &mut Vec<Rule>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<svg_gradient_tree::Element>, String> {
    validate_metadata(source, document, cancel)?;
    let mut elements = Vec::with_capacity(document.elements.len());
    for node in &document.elements {
        context(cancel)?;
        elements.push(svg_gradient_tree::Element {
            name: node.name.clone(),
            id: node.attributes.id.clone(),
            children: node.children.clone(),
            attrs: node.attributes.values.clone(),
            order: node.attributes.order.clone(),
            ..Default::default()
        });
    }
    svg_gradient_tree::validate(source, &elements, 0, cancel)?;
    svg_clip::validate_tree(source, &elements, 0, cancel)?;
    let mut stack = vec![0];
    while let Some(index) = stack.pop() {
        context(cancel)?;
        let node = &document.elements[index];
        if node.name == "style" {
            svg_stylesheet::validate_element(
                source,
                &node.attributes.values,
                &node.attributes.order,
                node.children.len(),
                cancel,
            )?;
            let text = String::from_utf8_lossy(&node.text);
            let parsed = svg_stylesheet::parse(source, &text, cancel)?;
            rules.extend(parsed);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    Ok(elements)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_parsed_tree_compilation_admission() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-compile-admission-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut rules = Vec::new();
            let result = super::super::svg_xml_document::parse(
                "math",
                row["input"].as_str().unwrap(),
                [10000, 100, 100, 100, 10000, 100, 100, 100, 100],
                None,
            )
            .and_then(|document| prepare("math", &document, &mut rules, None));
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(serde_json::to_value(rules).unwrap(), row["rules"], "{row}");
            if let Ok(elements) = result {
                assert_eq!(
                    elements.len() as u64,
                    row["elements"].as_u64().unwrap(),
                    "{row}"
                );
            }
        }
    }
}
/// Owned state of the source element pass. An image continuation preserves the
/// exact point where asset resolution must run before later element work.
pub struct Compilation {
    pub image_assets: Option<super::svg_image::Assets>,
    pub clips: std::collections::BTreeMap<usize, super::svg_clip::Template>,
    pub elements: Vec<svg_gradient_tree::Element>,
    pub states: Vec<super::svg_element::State>,
    pub geometry: Vec<super::svg_geometry::Geometry>,
    pub use_targets: Vec<Option<usize>>,
    pub use_positions: Vec<[f64; 2]>,
    pub rules: Vec<Rule>,
    pub work: super::svg_element::Work,
    pub parsed_commands: usize,
}
#[derive(Debug, PartialEq)]
pub enum Progress {
    Complete,
    Image(usize),
}
impl Compilation {
    pub fn new(document: &Document) -> Self {
        Self {
            image_assets: None,
            clips: Default::default(),
            elements: document
                .elements
                .iter()
                .map(|node| svg_gradient_tree::Element {
                    name: node.name.clone(),
                    id: node.attributes.id.clone(),
                    children: node.children.clone(),
                    attrs: node.attributes.values.clone(),
                    order: node.attributes.order.clone(),
                    ..Default::default()
                })
                .collect(),
            states: (0..document.elements.len())
                .map(|_| Default::default())
                .collect(),
            geometry: (0..document.elements.len())
                .map(|_| Default::default())
                .collect(),
            use_targets: vec![None; document.elements.len()],
            use_positions: vec![[0.; 2]; document.elements.len()],
            rules: Vec::new(),
            work: Default::default(),
            parsed_commands: 0,
        }
    }
}
impl Compilation {
    /// Complete image admission; `new` retains the historical phase boundary.
    pub fn with_images(document: &Document) -> Self {
        let mut output = Self::new(document);
        output.image_assets = Some(Default::default());
        output
    }
}
pub fn compile_elements(
    source: &str,
    document: &Document,
    output: &mut Compilation,
    limits: [usize; 9],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Progress, String> {
    output.elements = prepare(source, document, &mut output.rules, cancel)?;
    let ids = document
        .ids
        .iter()
        .map(|(id, &index)| (id.clone(), (index, document.elements[index].name.clone())))
        .collect();
    let mut stack = vec![0];
    while let Some(index) = stack.pop() {
        context(cancel)?;
        let rendering = document.elements[index]
            .children
            .iter()
            .any(|&child| !non_rendering(&document.elements[child].name));
        let next = super::svg_element::admit(
            source,
            &mut output.elements[index],
            &mut output.states[index],
            index == 0,
            rendering,
            &output.rules,
            limits[4],
            limits[6],
            &mut output.work,
            cancel,
        )?;
        if next == super::svg_element::Continuation::Geometry {
            let name = output.elements[index].name.as_str();
            if name == "image" && output.image_assets.is_none() {
                return Ok(Progress::Image(index));
            }
            output.geometry[index] = if name == "image" {
                let image = super::svg_image::compile(
                    source,
                    &output.elements[index].attrs,
                    document.elements[index].children.len(),
                    limits[0],
                    limits[8],
                    document.ids.len(),
                    output.image_assets.as_mut().unwrap(),
                    cancel,
                )?;
                super::svg_geometry::Geometry {
                    kind: 4,
                    bounds: image.bounds,
                    asset: image.content_id,
                    aspect: image.aspect,
                    ..Default::default()
                }
            } else if name == "text" {
                let commands = super::svg_frozen_text::commands(
                    source,
                    document,
                    &output.elements,
                    &output.states,
                    output.work.mathjax_root,
                    index,
                    limits[5],
                    cancel,
                )?;
                super::svg_geometry::Geometry {
                    kind: 3,
                    commands,
                    ..Default::default()
                }
            } else {
                super::svg_geometry::compile(
                    source,
                    name,
                    &output.elements[index].attrs,
                    limits[4],
                    limits[5],
                    &mut output.parsed_commands,
                    cancel,
                )?
            };
            if name == "use" {
                let attrs = &output.elements[index].attrs;
                let target = super::svg_use::resolve(
                    source,
                    attrs.get("href").map(String::as_str),
                    &ids,
                    cancel,
                )?;
                output.use_targets[index] = Some(target);
                for (axis, key) in ["x", "y"].iter().enumerate() {
                    output.use_positions[index][axis] = 0.;
                    if let Some(raw) = attrs.get(*key) {
                        output.use_positions[index][axis]=super::svg_length::length(raw,true).map_err(|reason|format!("d2svgimport: {source}: element <use> has invalid {key}: {reason}"))?;
                    }
                }
                if let Some(raw) = attrs.get("overflow") {
                    if trim_space(raw, cancel)? != "visible" {
                        return Err(format!(
                            "d2svgimport: {source}: element <use> supports only overflow=\"visible\" inside a clipping resource"
                        ));
                    }
                }
            }
        }
        stack.extend(document.elements[index].children.iter().rev().copied());
    }
    Ok(Progress::Complete)
}
#[cfg(test)]
mod elements_pass_tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_ordered_element_geometry_use_pass_and_partial_state() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-elements-pass-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let maximum = row["maximum"].as_u64().unwrap() as usize;
            let limits = [10000, 100, 100, 100, 10000, maximum, maximum, 100, 100];
            let document = super::super::svg_xml_document::parse(
                "math",
                row["input"].as_str().unwrap(),
                limits,
                None,
            )
            .unwrap();
            let mut output = Compilation::new(&document);
            let result = compile_elements("math", &document, &mut output, limits, None);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            let pending = match result {
                Ok(Progress::Image(index)) => index as i64,
                _ => -1,
            };
            assert_eq!(pending, row["pending_image"].as_i64().unwrap(), "{row}");
            assert_eq!(
                output.parsed_commands as u64,
                row["commands"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                output.work.transforms as u64,
                row["transforms"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                output.work.mathjax_root,
                row["mathjax"].as_bool().unwrap(),
                "{row}"
            );
            for (index, expected) in row["states"].as_array().unwrap().iter().enumerate() {
                let element = &output.elements[index];
                assert_eq!(
                    serde_json::to_value(&element.classes).unwrap(),
                    expected["classes"],
                    "{row}: {index}"
                );
                assert_eq!(
                    serde_json::to_value(&element.declarations).unwrap(),
                    expected["declarations"],
                    "{row}: {index}"
                );
                let state: super::super::svg_element::State =
                    serde_json::from_value(expected["state"].clone()).unwrap();
                assert_eq!(output.states[index], state, "{row}: {index}");
                let geometry: super::super::svg_geometry::Geometry =
                    serde_json::from_value(expected["geometry"].clone()).unwrap();
                assert_eq!(output.geometry[index], geometry, "{row}: {index}");
                let target: Option<usize> =
                    serde_json::from_value(expected["target"].clone()).unwrap();
                assert_eq!(output.use_targets[index], target, "{row}: {index}");
                let position: [f64; 2] =
                    serde_json::from_value(expected["position"].clone()).unwrap();
                assert_eq!(output.use_positions[index], position, "{row}: {index}");
                let gradient: Option<super::super::svg_primitive::Paint> =
                    serde_json::from_value(expected["gradient"].clone()).unwrap();
                assert_eq!(element.gradient, gradient, "{row}: {index}");
                let stop: Option<super::super::svg_primitive::Stop> =
                    serde_json::from_value(expected["stop"].clone()).unwrap();
                assert_eq!(element.stop, stop, "{row}: {index}");
            }
        }
    }
}
/// Complete source compile stages after the ordered element pass. Images remain
/// an explicit asset-resolution continuation, with no later work performed.
pub fn compile(
    source: &str,
    document: &Document,
    output: &mut Compilation,
    limits: [usize; 9],
    current_color: [u8; 4],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Progress, String> {
    let progress = compile_elements(source, document, output, limits, cancel)?;
    if progress != Progress::Complete {
        return Ok(progress);
    }
    let mut stack = vec![0];
    while let Some(index) = stack.pop() {
        context(cancel)?;
        if output.elements[index].name == "linearGradient" {
            let stops = output.elements[index]
                .children
                .iter()
                .map(|&child| output.elements[child].stop.clone())
                .collect::<Vec<_>>();
            let gradient = output.elements[index].gradient.as_mut().ok_or_else(|| {
                format!("d2svgimport: {source}: internal uncompiled linear gradient")
            })?;
            super::svg_gradient::attach_stops(source, gradient, &stops, cancel)?;
        }
        stack.extend(document.elements[index].children.iter().rev().copied());
    }
    let resources = document
        .ids
        .iter()
        .map(|(id, &index)| {
            (
                id.clone(),
                super::svg_resources::Resource {
                    name: output.elements[index].name.clone(),
                    gradient: output.elements[index].gradient.clone(),
                },
            )
        })
        .collect();
    super::svg_gradient_stop::validate_rules(source, &output.rules, &resources, cancel)?;
    let uses = output
        .elements
        .iter()
        .enumerate()
        .map(|(index, element)| super::svg_use::Element {
            children: element.children.clone(),
            is_use: element.name == "use",
            target: output.use_targets[index],
        })
        .collect::<Vec<_>>();
    super::svg_use::validate(source, &uses, 0, limits[7], cancel)?;
    validate_style_tree(source, document, output, &resources, current_color, cancel)?;
    let inputs = output
        .elements
        .iter()
        .enumerate()
        .map(|(index, element)| super::svg_clip::Input {
            name: element.name.clone(),
            id: element.id.clone(),
            children: element.children.clone(),
            declarations: element.declarations.clone(),
            transform: output.states[index].transform,
            geometry: output.geometry[index].clone(),
            href: if output.use_targets[index].is_some() {
                element.attrs["href"][1..].into()
            } else {
                String::new()
            },
            use_position: output.use_positions[index],
        })
        .collect::<Vec<_>>();
    super::svg_clip::compile_tree(
        source,
        &inputs,
        0,
        &document.ids,
        &mut output.clips,
        limits[7],
        limits[8],
        limits[5],
        cancel,
    )?;
    Ok(Progress::Complete)
}
fn validate_style_tree(
    source: &str,
    document: &Document,
    output: &Compilation,
    resources: &std::collections::BTreeMap<String, super::svg_resources::Resource>,
    current_color: [u8; 4],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let mut parent = super::svg_style::Style::default();
    parent.paint.color = current_color;
    let mut stack = vec![(0, parent)];
    while let Some((index, parent)) = stack.pop() {
        context(cancel)?;
        let element = &output.elements[index];
        let name = element.name.as_str();
        if non_rendering(name) || matches!(name, "linearGradient" | "stop" | "clipPath") {
            continue;
        }
        let style = super::svg_style::compute(
            source,
            name,
            &parent,
            &element.declarations,
            resources,
            cancel,
        )?;
        if name == "text" {
            super::svg_frozen_text::validate_style(source, &style)?;
        }
        validate_stroke_caps(source, name, &output.geometry[index], &style, cancel)?;
        for &child in document.elements[index].children.iter().rev() {
            stack.push((child, style.clone()));
        }
    }
    Ok(())
}
/// Source rejects zero-length open stroked subpaths for round/square caps.
/// Closed paths and curves whose control points have extent remain supported.
pub fn validate_stroke_caps(
    source: &str,
    name: &str,
    geometry: &super::svg_geometry::Geometry,
    style: &super::svg_style::Style,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    if geometry.kind != 3
        || style.paint.cap == 0
        || style.paint.stroke_width <= 0.
        || (style.paint.stroke.is_none() && !style.paint.stroke_current)
    {
        return Ok(());
    }
    let error = || {
        format!(
            "d2svgimport: {source}: element <{name}> has a zero-length open stroked subpath with unsupported round or square linecap"
        )
    };
    let mut current = [0.; 2];
    let mut start = [0.; 2];
    let mut have = false;
    let mut extent = false;
    let mut closed = false;
    for (index, command) in geometry.commands.iter().enumerate() {
        if index & 255 == 0 {
            context(cancel)?;
        }
        match command.kind {
            0 => {
                if have && !extent && !closed {
                    return Err(error());
                }
                current = command.points[0];
                start = current;
                have = true;
                extent = false;
                closed = false;
            }
            1 | 4 => {
                extent |= command.points[0] != current;
                current = command.points[0];
            }
            2 => {
                extent |= command.points[0] != current || command.points[1] != current;
                current = command.points[1];
            }
            3 => {
                extent |= command.points.iter().any(|point| *point != current);
                current = command.points[2];
            }
            5 => {
                extent |= current != start;
                current = start;
                closed = true;
            }
            _ => {}
        }
    }
    if have && !extent && !closed {
        return Err(error());
    }
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod pipeline_tests {
    use super::*;
    #[test]
    fn original_connected_compile_pipeline_and_partial_resources() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-compile-pipeline-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let maximum = row["maximum"].as_u64().unwrap() as usize;
            let limits = [10000, 100, 100, 100, 10000, maximum, maximum, maximum, 100];
            let document = super::super::svg_xml_document::parse(
                "math",
                row["input"].as_str().unwrap(),
                limits,
                None,
            )
            .unwrap();
            let mut output = Compilation::new(&document);
            let result = compile("math", &document, &mut output, limits, [1, 2, 3, 200], None);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            let pending = match result {
                Ok(Progress::Image(index)) => index as i64,
                _ => -1,
            };
            assert_eq!(pending, row["pending_image"].as_i64().unwrap(), "{row}");
            assert_eq!(
                output.parsed_commands as u64,
                row["commands"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                output.work.transforms as u64,
                row["transforms"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                output.work.mathjax_root,
                row["mathjax"].as_bool().unwrap(),
                "{row}"
            );
            for (index, expected) in row["states"].as_array().unwrap().iter().enumerate() {
                let element = &output.elements[index];
                let clip: Option<super::super::svg_clip::Template> =
                    serde_json::from_value(expected["clip"].clone()).unwrap();
                assert_eq!(output.clips.get(&index), clip.as_ref(), "{row}: {index}");
                assert_eq!(
                    serde_json::to_value(&element.classes).unwrap(),
                    expected["classes"],
                    "{row}: {index}"
                );
                assert_eq!(
                    serde_json::to_value(&element.declarations).unwrap(),
                    expected["declarations"],
                    "{row}: {index}"
                );
                let state: super::super::svg_element::State =
                    serde_json::from_value(expected["state"].clone()).unwrap();
                assert_eq!(output.states[index], state, "{row}: {index}");
                let geometry: super::super::svg_geometry::Geometry =
                    serde_json::from_value(expected["geometry"].clone()).unwrap();
                assert_eq!(output.geometry[index], geometry, "{row}: {index}");
                let target: Option<usize> =
                    serde_json::from_value(expected["target"].clone()).unwrap();
                assert_eq!(output.use_targets[index], target, "{row}: {index}");
                let position: [f64; 2] =
                    serde_json::from_value(expected["position"].clone()).unwrap();
                assert_eq!(output.use_positions[index], position, "{row}: {index}");
                let gradient: Option<super::super::svg_primitive::Paint> =
                    serde_json::from_value(expected["gradient"].clone()).unwrap();
                assert_eq!(element.gradient, gradient, "{row}: {index}");
                let stop: Option<super::super::svg_primitive::Stop> =
                    serde_json::from_value(expected["stop"].clone()).unwrap();
                assert_eq!(element.stop, stop, "{row}: {index}");
            }
        }
    }
}
