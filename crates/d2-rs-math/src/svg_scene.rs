// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/compile.go instantiate/newNode/
// nestedViewportClip. Owned arena avoids recursion during scene construction.
use super::{
    svg_clip::Template,
    svg_compile::Compilation,
    svg_path::Command,
    svg_primitive::{Budget, Primitive},
    svg_style::Style,
    svg_xml_document::Document,
};
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Clip {
    pub commands: Vec<Command>,
    pub fill_rule: u8,
    pub transform: [f64; 6],
}
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Node {
    pub id: String,
    pub classes: Vec<String>,
    pub transform: [f64; 6],
    pub opacity: f64,
    pub clip: Option<Clip>,
    pub primitive: Option<Primitive>,
    pub children: Vec<usize>,
}
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Scene {
    pub nodes: Vec<Node>,
    pub root: Option<usize>,
}
#[derive(Debug, Default)]
pub struct Work {
    pub budget: Budget,
    pub uses: usize,
    pub max_uses: usize,
    pub max_depth: usize,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn node(
    scene: &mut Scene,
    work: &mut Work,
    source: &str,
    primitive: Option<Primitive>,
    commands: usize,
) -> Result<usize, String> {
    work.budget
        .admit(source, primitive.as_ref().map(|_| commands))?;
    let index = scene.nodes.len();
    scene.nodes.push(Node {
        id: String::new(),
        classes: Vec::new(),
        transform: [1., 0., 0., 1., 0., 0.],
        opacity: 1.,
        clip: None,
        primitive,
        children: Vec::new(),
    });
    Ok(index)
}
fn clip(
    source: &str,
    id: &str,
    templates: &std::collections::BTreeMap<String, Template>,
    work: &mut Work,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Option<Clip>, String> {
    super::svg_clip::instantiate(
        source,
        id,
        templates,
        work.max_uses,
        &mut work.uses,
        work.budget.max_commands,
        &mut work.budget.commands,
        cancel,
    )
    .map(|clip| {
        clip.map(|clip| Clip {
            commands: clip.commands,
            fill_rule: clip.fill_rule,
            transform: clip.transform,
        })
    })
}
struct Frame {
    index: usize,
    inherited: Style,
    style: Style,
    prefix: String,
    depth: usize,
    referenced: bool,
    phase: u8,
    next: usize,
    node: Option<usize>,
    child_node: Option<usize>,
    target: Option<usize>,
}
impl Frame {
    fn new(index: usize, inherited: Style, prefix: String, depth: usize, referenced: bool) -> Self {
        Self {
            index,
            inherited,
            style: Default::default(),
            prefix,
            depth,
            referenced,
            phase: 0,
            next: 0,
            node: None,
            child_node: None,
            target: None,
        }
    }
}
/// Instantiate an already compiled subtree. Counters retain source partial
/// charges on failure; scene.root is returned only on complete success.
pub fn instantiate(
    source: &str,
    document: &Document,
    compiled: &Compilation,
    root: usize,
    inherited: Style,
    prefix: String,
    depth: usize,
    referenced: bool,
    scene: &mut Scene,
    work: &mut Work,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    scene.root = None;
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let resources = document
        .ids
        .iter()
        .map(|(id, &index)| {
            (
                id.clone(),
                super::svg_resources::Resource {
                    name: compiled.elements[index].name.clone(),
                    gradient: compiled.elements[index].gradient.clone(),
                },
            )
        })
        .collect();
    let templates = document
        .ids
        .iter()
        .filter_map(|(id, index)| {
            compiled
                .clips
                .get(index)
                .map(|template| (id.clone(), template.clone()))
        })
        .collect();
    let mut stack = vec![Frame::new(root, inherited, prefix, depth, referenced)];
    let mut active = std::collections::HashSet::new();
    let mut returned = None;
    let mut returning = false;
    while !stack.is_empty() {
        context(cancel)?;
        if returning {
            let frame = stack.last_mut().unwrap();
            match frame.phase {
                1 => {
                    if let Some(child) = returned {
                        scene.nodes[frame.child_node.unwrap()].children.push(child);
                    }
                    returned = None;
                    returning = false;
                    continue;
                }
                2 => {
                    active.remove(&frame.target.unwrap());
                    if let Some(child) = returned {
                        let index = node(scene, work, source, None, 0)?;
                        let element = &compiled.elements[frame.index];
                        let target = &mut scene.nodes[index];
                        target.id = frame.prefix.clone() + &element.id;
                        target.classes = element.classes.clone();
                        target.transform = super::svg_transform::mul(
                            compiled.states[frame.index].transform,
                            [
                                1.,
                                0.,
                                0.,
                                1.,
                                compiled.use_positions[frame.index][0],
                                compiled.use_positions[frame.index][1],
                            ],
                        );
                        target.opacity = frame.style.opacity;
                        target.clip =
                            clip(source, &frame.style.clip_path, &templates, work, cancel)?;
                        target.children.push(child);
                        returned = Some(index);
                    }
                    stack.pop();
                    continue;
                }
                _ => return Err(error("internal scene instantiation state".into())),
            }
        }
        let frame = stack.last_mut().unwrap();
        let index = frame.index;
        let element = &compiled.elements[index];
        let name = element.name.as_str();
        match frame.phase {
            0 => {
                if super::svg_compile::non_rendering(name)
                    || matches!(name, "linearGradient" | "stop" | "clipPath")
                {
                    stack.pop();
                    returned = None;
                    returning = true;
                    continue;
                }
                let style = super::svg_style::compute(
                    source,
                    name,
                    &frame.inherited,
                    &element.declarations,
                    &resources,
                    cancel,
                )?;
                super::svg_compile::validate_stroke_caps(
                    source,
                    name,
                    &compiled.geometry[index],
                    &style,
                    cancel,
                )?;
                frame.style = style.clone();
                if !style.display || (name == "defs" && !frame.referenced) {
                    stack.pop();
                    returned = None;
                    returning = true;
                    continue;
                }
                if name == "use" {
                    if frame.depth >= work.max_depth {
                        return Err(error(format!(
                            "local <use> expansion depth exceeds limit {}",
                            work.max_depth
                        )));
                    }
                    let target = compiled.use_targets[index].ok_or_else(|| {
                        error("element <use> references an unknown local id".into())
                    })?;
                    if active.contains(&target) {
                        return Err(error("local <use> reference cycle detected".into()));
                    }
                    if work.uses >= work.max_uses {
                        return Err(error(format!(
                            "expanded local-use resource count exceeds limit {}",
                            work.max_uses
                        )));
                    }
                    work.uses += 1;
                    let instance = if element.id.is_empty() {
                        format!("@use{}", work.uses)
                    } else {
                        element.id.clone()
                    };
                    let prefix = frame.prefix.clone() + &instance + "/";
                    let depth = frame.depth + 1;
                    frame.phase = 2;
                    frame.target = Some(target);
                    active.insert(target);
                    stack.push(Frame::new(target, style, prefix, depth, true));
                    continue;
                }
                if name == "svg" && frame.referenced {
                    return Err(error("referenced <svg> resources are unsupported".into()));
                }
                let (primitive, commands) =
                    super::svg_primitive::emit(&compiled.geometry[index], &style.paint, cancel)?;
                if !matches!(name, "svg" | "g") && primitive.is_none() {
                    stack.pop();
                    returned = None;
                    returning = true;
                    continue;
                }
                let node_index = node(scene, work, source, primitive, commands)?;
                scene.nodes[node_index].id = frame.prefix.clone() + &element.id;
                scene.nodes[node_index].classes = element.classes.clone();
                scene.nodes[node_index].opacity = style.opacity;
                if name == "svg" && index != 0 {
                    let viewport = compiled.states[index]
                        .viewport
                        .as_ref()
                        .ok_or_else(|| error("internal uncompiled nested <svg> viewport".into()))?;
                    if work.budget.commands > work.budget.max_commands
                        || 5 > work.budget.max_commands - work.budget.commands
                    {
                        return Err(error(format!(
                            "emitted path command count exceeds limit {}",
                            work.budget.max_commands
                        )));
                    }
                    let content = node(scene, work, source, None, 0)?;
                    work.budget.commands += 5;
                    scene.nodes[node_index].transform =
                        [1., 0., 0., 1., viewport.position[0], viewport.position[1]];
                    let point = |kind, xy| Command {
                        kind,
                        points: [xy, [0.; 2], [0.; 2]],
                        ..Default::default()
                    };
                    scene.nodes[node_index].clip = Some(Clip {
                        commands: vec![
                            point(0, [0., 0.]),
                            point(1, [viewport.width, 0.]),
                            point(1, [viewport.width, viewport.height]),
                            point(1, [0., viewport.height]),
                            Command {
                                kind: 5,
                                ..Default::default()
                            },
                        ],
                        fill_rule: 0,
                        transform: [1., 0., 0., 1., 0., 0.],
                    });
                    scene.nodes[content].transform = viewport.transform;
                    scene.nodes[node_index].children.push(content);
                    frame.child_node = Some(content);
                } else {
                    scene.nodes[node_index].transform = compiled.states[index].transform;
                    scene.nodes[node_index].clip =
                        clip(source, &style.clip_path, &templates, work, cancel)?;
                    frame.child_node = Some(node_index);
                }
                frame.node = Some(node_index);
                frame.phase = 1;
            }
            1 => {
                if frame.next == element.children.len() {
                    returned = frame.node;
                    stack.pop();
                    returning = true;
                    continue;
                }
                let child = element.children[frame.next];
                frame.next += 1;
                let child = Frame::new(
                    child,
                    frame.style.clone(),
                    frame.prefix.clone(),
                    frame.depth,
                    false,
                );
                stack.push(child);
            }
            _ => return Err(error("internal scene instantiation state".into())),
        }
    }
    scene.root = returned;
    Ok(())
}
#[cfg(all(test, feature = "oracle-fixtures"))]
pub(crate) mod tests {
    use super::*;
    #[derive(Debug, serde::Deserialize, PartialEq)]
    pub(crate) struct Tree {
        id: String,
        classes: Vec<String>,
        transform: [f64; 6],
        opacity: f64,
        clip: Option<Clip>,
        primitive: Option<Leaf>,
        children: Vec<Tree>,
    }
    #[derive(Debug, serde::Deserialize, PartialEq)]
    #[serde(tag = "kind")]
    enum Leaf {
        #[serde(rename = "image")]
        Image {
            bounds: [f64; 4],
            asset: String,
            aspect: [u8; 2],
        },
        #[serde(rename = "rect")]
        Rect {
            bounds: [f64; 4],
            radii: [f64; 2],
            fill: Option<super::super::svg_primitive::Paint>,
            stroke: Option<super::super::svg_primitive::Stroke>,
        },
        #[serde(rename = "ellipse")]
        Ellipse {
            center: [f64; 2],
            radii: [f64; 2],
            fill: Option<super::super::svg_primitive::Paint>,
            stroke: Option<super::super::svg_primitive::Stroke>,
        },
        #[serde(rename = "path")]
        Path {
            commands: Vec<Command>,
            fill_rule: u8,
            fill: Option<super::super::svg_primitive::Paint>,
            stroke: Option<super::super::svg_primitive::Stroke>,
        },
    }
    pub(crate) fn tree(scene: &Scene, index: usize) -> Tree {
        let node = &scene.nodes[index];
        let primitive = node.primitive.as_ref().map(|primitive| {
            let geometry = &primitive.geometry;
            let fill = primitive.fill.clone();
            let stroke = primitive.stroke.clone();
            match geometry.kind {
                4 => Leaf::Image {
                    bounds: geometry.bounds,
                    asset: geometry.asset.clone(),
                    aspect: geometry.aspect,
                },
                1 => Leaf::Rect {
                    bounds: geometry.bounds,
                    radii: geometry.radii,
                    fill,
                    stroke,
                },
                2 => Leaf::Ellipse {
                    center: geometry.center,
                    radii: geometry.radii,
                    fill,
                    stroke,
                },
                3 => Leaf::Path {
                    commands: geometry.commands.clone(),
                    fill_rule: primitive.fill_rule,
                    fill,
                    stroke,
                },
                _ => panic!("unsupported fixture primitive"),
            }
        });
        Tree {
            id: node.id.clone(),
            classes: node.classes.clone(),
            transform: node.transform,
            opacity: node.opacity,
            clip: node.clip.as_ref().map(|clip| Clip {
                commands: clip.commands.clone(),
                fill_rule: clip.fill_rule,
                transform: clip.transform,
            }),
            primitive,
            children: node
                .children
                .iter()
                .map(|&child| tree(scene, child))
                .collect(),
        }
    }
    #[test]
    fn original_owned_scene_trees_ids_paint_clips_and_partial_work() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-scene-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let document = super::super::svg_xml_document::parse(
                "math",
                row["input"].as_str().unwrap(),
                [10000, 100, 100, 100, 10000, 100, 100, 100, 100],
                None,
            )
            .unwrap();
            let mut compiled = Compilation::new(&document);
            let maximum: [usize; 4] = serde_json::from_value(row["maximum"].clone()).unwrap();
            let used = row["used"].as_u64().unwrap() as usize;
            let mut work = Work {
                budget: Budget {
                    max_elements: maximum[0],
                    max_commands: maximum[1],
                    elements: used,
                    commands: used,
                },
                max_uses: maximum[2],
                max_depth: maximum[3],
                uses: used,
            };
            let mut scene = Scene::default();
            let result = super::super::svg_compile::compile(
                "math",
                &document,
                &mut compiled,
                [10000, 100, 100, 100, 10000, 100, 100, 100, 100],
                [1, 2, 3, 200],
                None,
            )
            .and_then(|progress| {
                assert_eq!(progress, super::super::svg_compile::Progress::Complete);
                let mut inherited = Style::default();
                inherited.paint.color = [1, 2, 3, 200];
                instantiate(
                    "math",
                    &document,
                    &compiled,
                    0,
                    inherited,
                    "pre/".into(),
                    0,
                    false,
                    &mut scene,
                    &mut work,
                    None,
                )
            });
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(
                work.budget.elements as u64,
                row["nodes"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(
                work.budget.commands as u64,
                row["commands"].as_u64().unwrap(),
                "{row}"
            );
            assert_eq!(work.uses as u64, row["uses"].as_u64().unwrap(), "{row}");
            let actual = scene.root.map(|index| tree(&scene, index));
            let expected: Option<Tree> = serde_json::from_value(row["tree"].clone()).unwrap();
            assert_eq!(actual, expected, "{row}");
        }
    }
}
