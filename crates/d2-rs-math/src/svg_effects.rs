// SPDX-License-Identifier: MPL-2.0
//! Upstream 8558856dc d2svgimport/effects.go: SVG effect regions and masks.
use super::{
    svg_compile::Compilation,
    svg_gradient_tree::Element,
    svg_scene::{Clip, Mask, Scene, Work},
    svg_style::Style,
    svg_xml_document::Document,
};
const IDENTITY: [f64; 6] = [1., 0., 0., 1., 0., 0.];
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Region {
    pub units: u8,
    pub content_units: u8,
    pub values: [f64; 4],
    pub percentages: [bool; 4],
}
fn units(raw: &str, fallback: u8) -> Result<u8, String> {
    match raw.trim() {
        "" => Ok(fallback),
        "objectBoundingBox" => Ok(0),
        "userSpaceOnUse" => Ok(1),
        _ => Err("expected objectBoundingBox or userSpaceOnUse".into()),
    }
}
impl Region {
    pub fn compile(
        source: &str,
        el: &Element,
        units_name: &str,
        content_name: &str,
    ) -> Result<Self, String> {
        let property = |key: &str, reason: String| {
            format!(
                "d2svgimport: {source}: element <{}> has invalid {key}: {reason}",
                el.name
            )
        };
        let mut out = Self {
            units: units(
                el.attrs.get(units_name).map(String::as_str).unwrap_or(""),
                0,
            )
            .map_err(|e| property(units_name, e))?,
            content_units: units(
                el.attrs.get(content_name).map(String::as_str).unwrap_or(""),
                1,
            )
            .map_err(|e| property(content_name, e))?,
            values: [-0.1, -0.1, 1.2, 1.2],
            percentages: [true; 4],
        };
        for (n, key) in ["x", "y", "width", "height"].iter().enumerate() {
            if let Some(raw) = el.attrs.get(*key) {
                let raw = raw.trim();
                let (number, percent) = raw.strip_suffix('%').map_or((raw, false), |n| (n, true));
                if percent && number.trim() != number {
                    return Err(property(
                        key,
                        "percentage sign must follow its number".into(),
                    ));
                }
                let value = super::svg_length::length(number, !percent && out.units == 1)
                    .map_err(|e| property(key, e))?;
                out.values[n] = if percent { value / 100. } else { value };
                out.percentages[n] = percent;
            }
        }
        if out.values[2] < 0. || out.values[3] < 0. {
            return Err(format!(
                "d2svgimport: {source}: element <{}> has negative region dimensions",
                el.name
            ));
        }
        Ok(out)
    }
    pub fn bounds(&self, object: [f64; 4], viewport: [f64; 4]) -> [f64; 4] {
        let mut out = self.values;
        for n in 0..4 {
            if self.units == 0 {
                out[n] *= object[2 + n % 2];
                if n < 2 {
                    out[n] += object[n];
                }
            } else if self.percentages[n] {
                out[n] *= viewport[2 + n % 2];
            }
        }
        out
    }
}
pub fn compile_mask(source: &str, el: &Element) -> Result<Region, String> {
    if !matches!(
        el.attrs.get("mask-type").map(|s| s.trim()).unwrap_or(""),
        "" | "luminance" | "alpha"
    ) {
        return Err(format!(
            "d2svgimport: {source}: element <mask> has unsupported mask-type"
        ));
    }
    Region::compile(source, el, "maskUnits", "maskContentUnits")
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn object_bounds(
    source: &str,
    scene: &Scene,
    root: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<[f64; 4], String> {
    let mut bounds = super::svg_bounds::Bounds::default();
    let mut stack = vec![(root, IDENTITY)];
    while let Some((index, transform)) = stack.pop() {
        context(cancel)?;
        let node = &scene.nodes[index];
        if let Some(p) = &node.primitive {
            let mut p = p.clone();
            if matches!(p.geometry.kind, 1..=3) {
                p.fill = Some(super::svg_primitive::Paint::Solid { color: [0; 4] });
                p.stroke = None;
            }
            let b = super::svg_bounds::primitive(&p, transform)
                .map_err(|_| format!("d2svgimport: {source}: invalid effect geometry bounds"))?;
            if !b.finite() {
                return Err(format!(
                    "d2svgimport: {source}: invalid effect geometry bounds"
                ));
            }
            bounds.union(b);
        }
        for &child in &node.children {
            stack.push((
                child,
                super::svg_transform::mul(transform, scene.nodes[child].transform),
            ));
        }
    }
    Ok(if bounds.valid {
        [
            bounds.min[0],
            bounds.min[1],
            bounds.max[0] - bounds.min[0],
            bounds.max[1] - bounds.min[1],
        ]
    } else {
        [0.; 4]
    })
}
pub fn region_clip(source: &str, region: [f64; 4], work: &mut Work) -> Result<Clip, String> {
    if 5 > work
        .budget
        .max_commands
        .saturating_sub(work.budget.commands)
    {
        return Err(format!(
            "d2svgimport: {source}: emitted path command count exceeds limit {}",
            work.budget.max_commands
        ));
    }
    work.budget.commands += 5;
    let [x, y, w, h] = region;
    let point = |kind, xy| super::svg_path::Command {
        kind,
        points: [xy, [0.; 2], [0.; 2]],
        ..Default::default()
    };
    Ok(Clip {
        commands: vec![
            point(0, [0., 0.]),
            point(1, [w, 0.]),
            point(1, [w, h]),
            point(1, [0., h]),
            point(5, [0., 0.]),
        ],
        fill_rule: 0,
        transform: [1., 0., 0., 1., x, y],
    })
}
pub fn apply(
    source: &str,
    document: &Document,
    compiled: &Compilation,
    scene: &mut Scene,
    index: usize,
    style: &Style,
    root_color: [u8; 4],
    building_mask: bool,
    work: &mut Work,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<usize, String> {
    if style.mask.is_empty() {
        return Ok(index);
    }
    if building_mask {
        return Err(format!(
            "d2svgimport: {source}: nested mask references are unsupported"
        ));
    }
    let mask = document.ids[&style.mask];
    let spec = compiled.states[mask]
        .mask_region
        .as_ref()
        .expect("compiled mask region");
    let object = object_bounds(source, scene, index, cancel)?;
    let region = spec.bounds(object, style.viewport);
    if region[2] <= 0. || region[3] <= 0. {
        scene.nodes[index].opacity = 0.;
        return Ok(index);
    }
    let resources = document
        .ids
        .iter()
        .map(|(id, &i)| {
            (
                id.clone(),
                super::svg_resources::Resource {
                    name: compiled
                        .elements
                        .get(i)
                        .map(|e| e.name.clone())
                        .unwrap_or_default(),
                    gradient: compiled.elements.get(i).and_then(|e| e.gradient.clone()),
                },
            )
        })
        .collect();
    let mut inherited = Style::default();
    inherited.paint.color = root_color;
    inherited.viewport = style.viewport;
    let mut ancestors = vec![];
    let mut parent = document.elements[mask].parent;
    while let Some(i) = parent {
        ancestors.push(i);
        parent = document.elements[i].parent;
    }
    for &i in ancestors.iter().rev() {
        inherited = super::svg_style::compute(
            source,
            &compiled.elements[i].name,
            &inherited,
            &compiled.elements[i].declarations,
            &resources,
            cancel,
        )?;
    }
    inherited.viewport = if spec.content_units == 0 {
        [0., 0., 1., 1.]
    } else {
        style.viewport
    };
    let old_root = scene.root;
    super::svg_scene::instantiate_effects(
        source,
        document,
        compiled,
        mask,
        inherited,
        String::new(),
        0,
        true,
        scene,
        work,
        cancel,
        true,
    )?;
    let content = scene.root;
    scene.root = old_root;
    let Some(content) = content else {
        scene.nodes[index].opacity = 0.;
        return Ok(index);
    };
    if spec.content_units == 0 {
        scene.nodes[content].transform = super::svg_transform::mul(
            [object[2], 0., 0., object[3], object[0], object[1]],
            scene.nodes[content].transform,
        );
    }
    let wrapper = super::svg_scene::node(scene, work, source, None, 0)?;
    scene.nodes[wrapper].children.push(content);
    scene.nodes[wrapper].clip = Some(region_clip(source, region, work)?);
    let kind = if compiled.elements[mask]
        .attrs
        .get("mask-type")
        .is_some_and(|v| v.trim() == "alpha")
    {
        0
    } else {
        1
    };
    scene.nodes[index].mask = Some(Mask {
        kind,
        root: wrapper,
    });
    Ok(index)
}
