// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/clip.go.
use super::{
    svg_geometry::{Geometry, trim_space},
    svg_gradient_tree::Element,
    svg_path::Command,
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Template {
    pub commands: Vec<Command>,
    pub fill_rule: u8,
    pub transform: [f64; 6],
    pub uses: usize,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn supported(name: &str) -> bool {
    matches!(
        name,
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
    )
}
pub fn validate_tree(
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
        if element.name == "clipPath" {
            if element.id.is_empty() {
                return Err(error("element <clipPath> must declare an id".into()));
            }
            if !matches!(parent, Some("svg" | "g" | "defs")) {
                return Err(error(
                    "element <clipPath> is only supported under <svg>, <g>, or <defs>".into(),
                ));
            }
            if let Some(raw) = element.attrs.get("clipPathUnits") {
                if trim_space(raw, cancel)? != "userSpaceOnUse" {
                    return Err(error(
                        "element <clipPath> supports only clipPathUnits=\"userSpaceOnUse\"".into(),
                    ));
                }
            }
            if element.children.len() != 1 {
                return Err(error(
                    "element <clipPath> must contain exactly one supported geometry or local <use>"
                        .into(),
                ));
            }
            let child = &elements[element.children[0]].name;
            if child != "use" && !supported(child) {
                return Err(error(format!(
                    "element <clipPath> has unsupported child <{}>",
                    if child.len() > 64 {
                        "<oversized-name>"
                    } else {
                        child
                    }
                )));
            }
        }
        if element.attrs.contains_key("overflow")
            && (element.name != "use" || parent != Some("clipPath"))
        {
            return Err(error(
                "overflow metadata is supported only on a direct <use> child of <clipPath>".into(),
            ));
        }
        for &child in element.children.iter().rev() {
            stack.push((child, Some(index)));
        }
    }
    Ok(())
}
pub fn commands(
    name: &str,
    geometry: &Geometry,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<Command>, String> {
    let point = |kind, xy| Command {
        kind,
        points: [xy, [0.; 2], [0.; 2]],
        ..Default::default()
    };
    let arc = |xy, rx, ry| Command {
        kind: 4,
        points: [xy, [0.; 2], [0.; 2]],
        radii: [rx, ry],
        sweep: true,
        ..Default::default()
    };
    let close = || Command {
        kind: 5,
        ..Default::default()
    };
    match geometry.kind {
        3 => {
            let mut copy = Vec::with_capacity(geometry.commands.len());
            for chunk in geometry.commands.chunks(256) {
                context(cancel)?;
                copy.extend_from_slice(chunk);
            }
            context(cancel)?;
            Ok(copy)
        }
        1 => {
            let [x, y, w, h] = geometry.bounds;
            let [rx, ry] = geometry.radii;
            if w == 0. || h == 0. {
                return Ok(Vec::new());
            }
            if rx == 0. || ry == 0. {
                return Ok(vec![
                    point(0, [x, y]),
                    point(1, [x + w, y]),
                    point(1, [x + w, y + h]),
                    point(1, [x, y + h]),
                    close(),
                ]);
            }
            Ok(vec![
                point(0, [x + rx, y]),
                point(1, [x + w - rx, y]),
                arc([x + w, y + ry], rx, ry),
                point(1, [x + w, y + h - ry]),
                arc([x + w - rx, y + h], rx, ry),
                point(1, [x + rx, y + h]),
                arc([x, y + h - ry], rx, ry),
                point(1, [x, y + ry]),
                arc([x + rx, y], rx, ry),
                close(),
            ])
        }
        2 => {
            let [x, y] = geometry.center;
            let [rx, ry] = geometry.radii;
            if rx == 0. || ry == 0. {
                return Ok(Vec::new());
            }
            Ok(vec![
                point(0, [x + rx, y]),
                arc([x - rx, y], rx, ry),
                arc([x + rx, y], rx, ry),
                close(),
            ])
        }
        _ => Err(format!(
            "d2svgimport: internal unsupported clip geometry <{name}>"
        )),
    }
}
pub fn instantiate(
    source: &str,
    id: &str,
    templates: &BTreeMap<String, Template>,
    max_uses: usize,
    used_uses: &mut usize,
    max_commands: usize,
    used_commands: &mut usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Option<Template>, String> {
    context(cancel)?;
    if id.is_empty() {
        return Ok(None);
    }
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let template = templates
        .get(id)
        .ok_or_else(|| error("internal uncompiled clip-path resource".into()))?;
    if *used_uses > max_uses || template.uses > max_uses - *used_uses {
        return Err(error(format!(
            "expanded local-use resource count exceeds limit {max_uses}"
        )));
    }
    if *used_commands > max_commands || template.commands.len() > max_commands - *used_commands {
        return Err(error(format!(
            "emitted path command count exceeds limit {max_commands}"
        )));
    }
    let mut copied = Vec::with_capacity(template.commands.len());
    for chunk in template.commands.chunks(256) {
        context(cancel)?;
        copied.extend_from_slice(chunk);
    }
    context(cancel)?;
    *used_uses += template.uses;
    *used_commands += template.commands.len();
    Ok(Some(Template {
        commands: copied,
        fill_rule: template.fill_rule,
        transform: template.transform,
        uses: template.uses,
    }))
}
/// Parsed clip input after tree/geometry/declaration admission. Indices preserve
/// parsed-element identity, including distinct elements with equal IDs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Input {
    pub name: String,
    pub id: String,
    pub children: Vec<usize>,
    pub declarations: BTreeMap<String, String>,
    pub transform: [f64; 6],
    pub geometry: Geometry,
    pub href: String,
    pub use_position: [f64; 2],
}
fn validate_declarations(
    source: &str,
    element: &Input,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    for property in element.declarations.keys() {
        context(cancel)?;
        if property != "transform" && property != "clip-rule" {
            return Err(format!(
                "d2svgimport: {source}: element <{}> has unsupported clipping-resource property {property:?}",
                element.name
            ));
        }
    }
    Ok(())
}
/// Source validatePrimitiveBounds for the unpainted clip Path. PrimitiveBounds
/// short-circuits absent fill/stroke; no command geometry is inspected. Preserve
/// the importer's chunk checkpoints even though each nonempty chunk has no bounds.
fn validate_unpainted_path(
    commands: &[Command],
    transform: [f64; 6],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    for _ in commands.chunks(256) {
        context(cancel)?;
        if !transform.iter().all(|v| v.is_finite()) {
            return Err("invalid path bounds".into());
        }
        context(cancel)?;
    }
    Ok(())
}
pub fn compile(
    source: &str,
    elements: &[Input],
    root: usize,
    ids: &BTreeMap<String, usize>,
    max_depth: usize,
    max_resources: usize,
    max_commands: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Template, String> {
    use super::{svg_style, svg_transform::mul};
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    // The declaration gate permits only transform/clip-rule; neither reads paints.
    let resources = BTreeMap::new();
    let element = &elements[root];
    validate_declarations(source, element, cancel)?;
    let mut inherited = svg_style::compute(
        source,
        &element.name,
        &Default::default(),
        &element.declarations,
        &resources,
        cancel,
    )?;
    if !inherited.clip_path.is_empty() {
        return Err(error("nested clip-path resources are unsupported".into()));
    }
    let mut current = element.children[0];
    let mut transform = element.transform;
    let mut active = std::collections::HashSet::new();
    let mut uses = 0;
    loop {
        context(cancel)?;
        let node = &elements[current];
        validate_declarations(source, node, cancel)?;
        let style = svg_style::compute(
            source,
            &node.name,
            &inherited,
            &node.declarations,
            &resources,
            cancel,
        )?;
        if !style.clip_path.is_empty() {
            return Err(error(
                if style.clip_path == element.id {
                    "local clip-path reference cycle detected"
                } else {
                    "nested clip-path resources are unsupported"
                }
                .into(),
            ));
        }
        transform = mul(transform, node.transform);
        if !transform.iter().all(|v| v.is_finite()) {
            return Err(error(
                "clip-path has a non-finite composed transform".into(),
            ));
        }
        if node.name != "use" {
            if !supported(&node.name) {
                return Err(error(format!(
                    "clip-path local <use> resolves to unsupported <{}> resource",
                    node.name
                )));
            }
            let commands = commands(&node.name, &node.geometry, cancel)?;
            if commands.len() > max_commands {
                return Err(error(format!(
                    "clip-path command count exceeds limit {max_commands}"
                )));
            }
            if validate_unpainted_path(&commands, transform, cancel).is_err() {
                context(cancel)?;
                return Err(error("clip-path has invalid or non-finite geometry".into()));
            }
            return Ok(Template {
                commands,
                fill_rule: style.clip_rule,
                transform,
                uses,
            });
        }
        if uses >= max_depth {
            return Err(error(format!(
                "local <use> expansion depth exceeds limit {max_depth}"
            )));
        }
        uses += 1;
        if uses > max_resources {
            return Err(error(format!(
                "expanded local-use resource count exceeds limit {max_resources}"
            )));
        }
        transform = mul(
            transform,
            [1., 0., 0., 1., node.use_position[0], node.use_position[1]],
        );
        if !transform.iter().all(|v| v.is_finite()) {
            return Err(error(
                "clip-path has a non-finite composed transform".into(),
            ));
        }
        let target = *ids
            .get(&node.href)
            .ok_or_else(|| error("element <use> references an unknown local id".into()))?;
        if !active.insert(target) {
            return Err(error("local <use> reference cycle detected".into()));
        }
        inherited = style;
        current = target;
    }
}
/// Compile in source child order, retaining earlier templates on a later error.
pub fn compile_tree(
    source: &str,
    elements: &[Input],
    root: usize,
    ids: &BTreeMap<String, usize>,
    templates: &mut BTreeMap<usize, Template>,
    max_depth: usize,
    max_resources: usize,
    max_commands: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let mut stack = vec![root];
    while let Some(index) = stack.pop() {
        context(cancel)?;
        let node = &elements[index];
        if node.name == "clipPath" {
            templates.insert(
                index,
                compile(
                    source,
                    elements,
                    index,
                    ids,
                    max_depth,
                    max_resources,
                    max_commands,
                    cancel,
                )?,
            );
        }
        stack.extend(node.children.iter().rev().copied());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_connected_clip_templates() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-clip-compile-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let elements: Vec<Input> = serde_json::from_value(row["elements"].clone()).unwrap();
            let ids = elements
                .iter()
                .enumerate()
                .map(|(index, e)| (e.id.clone(), index))
                .collect();
            let result = compile(
                "math",
                &elements,
                0,
                &ids,
                row["depth"].as_u64().unwrap() as usize,
                row["resources"].as_u64().unwrap() as usize,
                row["maximum_commands"].as_u64().unwrap() as usize,
                None,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(output) = result {
                if row["mode"] == "overflow-geometry" {
                    assert!(
                        output
                            .commands
                            .iter()
                            .flat_map(|c| c.points)
                            .flatten()
                            .any(|v| !v.is_finite())
                    );
                } else {
                    let expected: Template = serde_json::from_value(row["output"].clone()).unwrap();
                    assert_eq!(output, expected, "{row}");
                }
            }
        }
    }
    #[test]
    fn clip_tree_retains_prior_template_and_context_precedes_declaration() {
        let make = |name: &str, children: Vec<usize>| Input {
            name: name.into(),
            id: String::new(),
            children,
            declarations: BTreeMap::new(),
            transform: [1., 0., 0., 1., 0., 0.],
            geometry: Geometry::default(),
            href: String::new(),
            use_position: [0.; 2],
        };
        let mut elements = vec![
            make("svg", vec![1, 3]),
            make("clipPath", vec![2]),
            make("path", vec![]),
            make("clipPath", vec![4]),
            make("path", vec![]),
        ];
        elements[2].geometry.kind = 3;
        elements[4].declarations.insert("fill".into(), "red".into());
        let mut templates = BTreeMap::new();
        let ids = BTreeMap::new();
        assert_eq!(
            compile_tree("math", &elements, 0, &ids, &mut templates, 4, 4, 20, None).unwrap_err(),
            "d2svgimport: math: element <path> has unsupported clipping-resource property \"fill\""
        );
        assert_eq!(templates.len(), 1);
        assert!(templates.contains_key(&1));
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            compile("math", &elements, 3, &ids, 4, 4, 20, Some(&token)).unwrap_err(),
            "context canceled"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.expire();
        assert_eq!(
            compile_tree(
                "math",
                &elements,
                0,
                &ids,
                &mut templates,
                4,
                4,
                20,
                Some(&token)
            )
            .unwrap_err(),
            "context deadline exceeded"
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_clip_commands_and_expansion_charges() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-clip-v090.json")).unwrap();
        let command = |kind, xy| Command {
            kind,
            points: [xy, [0.; 2], [0.; 2]],
            ..Default::default()
        };
        let templates = BTreeMap::from([(
            "c".into(),
            Template {
                commands: vec![command(0, [1., 2.]), command(1, [3., 4.])],
                fill_rule: 1,
                transform: [1., 0., 0., 1., 0., 0.],
                uses: 2,
            },
        )]);
        for row in rows.as_array().unwrap() {
            if row["operation"] == "commands" {
                let geometry: Geometry = serde_json::from_value(row["geometry"].clone()).unwrap();
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
                let result = commands("probe", &geometry, cancel);
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                if let Ok(commands) = result {
                    let expected: Vec<Command> =
                        serde_json::from_value(row["commands"].clone()).unwrap();
                    assert_eq!(commands, expected, "{row}");
                }
            } else {
                let mut uses = row["used"].as_u64().unwrap() as usize;
                let mut count = uses;
                let result = instantiate(
                    "math",
                    row["id"].as_str().unwrap(),
                    &templates,
                    row["maximum"].as_u64().unwrap() as usize,
                    &mut uses,
                    row["maximum_commands"].as_u64().unwrap() as usize,
                    &mut count,
                    None,
                );
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                assert_eq!(uses as u64, row["after_uses"].as_u64().unwrap(), "{row}");
                assert_eq!(
                    count as u64,
                    row["after_commands"].as_u64().unwrap(),
                    "{row}"
                );
                if let Ok(output) = result {
                    let expected: Option<Template> =
                        serde_json::from_value(row["output"].clone()).unwrap();
                    assert_eq!(output, expected, "{row}");
                }
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_clip_tree_admission() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-clip-tree-v090.json"))
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
                    name: "clipPath".into(),
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
                validate_tree("math", &elements, 0, None)
                    .err()
                    .unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{row}"
            );
        }
    }
}
