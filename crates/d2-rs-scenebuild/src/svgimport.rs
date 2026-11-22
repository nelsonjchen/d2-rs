// SPDX-License-Identifier: MPL-2.0
//! Native d2svgimport arena -> d2scene ownership bridge. Source: master
//! assets.go remainingSVGImportLimits/reserveSVGImport/mergeSVGImportAssets.
//! Parsing, compilation and bounded work reuse the native math importer.
use crate::{Builder, util::*};
use d2_rs_math::{
    svg_import::{Imported, Outcome},
    svg_path::Command,
    svg_primitive,
};
use d2_rs_scene::*;

fn point(p: [f64; 2]) -> Point {
    Point::new(p[0], p[1])
}
fn matrix(m: [f64; 6]) -> Matrix {
    Matrix {
        a: m[0],
        b: m[1],
        c: m[2],
        d: m[3],
        e: m[4],
        f: m[5],
    }
}
fn bounds(b: [f64; 4]) -> Box {
    Box {
        x: b[0],
        y: b[1],
        width: b[2],
        height: b[3],
    }
}
fn color(c: [u8; 4]) -> Nrgba {
    Nrgba::new(c[0], c[1], c[2], c[3])
}
fn paint(p: &svg_primitive::Paint) -> Paint {
    match p {
        svg_primitive::Paint::Solid { color: c } => Paint::solid(color(*c)),
        svg_primitive::Paint::Linear {
            start,
            end,
            stops,
            units,
            transform,
            spread,
            ..
        } => Paint::LinearGradient(LinearGradient {
            start: point(*start),
            end: point(*end),
            stops: stops
                .iter()
                .map(|s| GradientStop {
                    offset: s.offset,
                    color: color(s.color),
                })
                .collect(),
            units: PaintUnits(*units),
            transform: matrix(*transform),
            spread: SpreadMethod(*spread),
        }),
        svg_primitive::Paint::Radial {
            center,
            focal,
            radius,
            focal_radius,
            stops,
            units,
            transform,
            spread,
            ..
        } => Paint::RadialGradient(RadialGradient {
            center: point(*center),
            focal: point(*focal),
            radius: *radius,
            focal_radius: *focal_radius,
            stops: stops
                .iter()
                .map(|s| GradientStop {
                    offset: s.offset,
                    color: color(s.color),
                })
                .collect(),
            units: PaintUnits(*units),
            transform: matrix(*transform),
            spread: SpreadMethod(*spread),
        }),
    }
}
fn commands(cs: &[Command]) -> Vec<PathCommand> {
    cs.iter()
        .map(|c| PathCommand {
            kind: PathCommandKind(c.kind),
            p1: point(c.points[0]),
            p2: point(c.points[1]),
            p3: point(c.points[2]),
            radius_x: c.radii[0],
            radius_y: c.radii[1],
            rotation: c.rotation,
            large_arc: c.large_arc,
            sweep: c.sweep,
        })
        .collect()
}
fn primitive(p: &svg_primitive::Primitive) -> Primitive {
    let fill = p.fill.as_ref().map(paint);
    let stroke = p.stroke.as_ref().map(|s| Stroke {
        paint: Some(paint(&s.paint)),
        width: s.width,
        cap: LineCap(s.cap),
        join: LineJoin(s.join),
        miter_limit: s.miter_limit,
        dashes: s.dashes.clone(),
        dash_offset: s.dash_offset,
    });
    let g = &p.geometry;
    match g.kind {
        1 => Primitive::Rect(Rect {
            r#box: bounds(g.bounds),
            radius_x: g.radii[0],
            radius_y: g.radii[1],
            fill,
            stroke,
        }),
        2 => Primitive::Ellipse(Ellipse {
            center: point(g.center),
            radius_x: g.radii[0],
            radius_y: g.radii[1],
            fill,
            stroke,
        }),
        3 => Primitive::Path(Path {
            commands: commands(&g.commands),
            fill_rule: FillRule(p.fill_rule),
            fill,
            stroke,
        }),
        4 => Primitive::Image(Image {
            asset: g.asset.clone(),
            r#box: bounds(g.bounds),
            aspect: AspectRatio {
                align: AspectAlign(g.aspect[0]),
                fit: AspectFit(g.aspect[1]),
            },
        }),
        _ => unreachable!("importer emitted an unknown primitive"),
    }
}

/// Consume the arena iteratively. Reverse traversal guarantees children are
/// moved exactly once, without recursive cloning or a source-depth stack.
pub(crate) fn root(scene: d2_rs_math::svg_scene::Scene) -> Node {
    let mut mapped: Vec<Option<Node>> = (0..scene.nodes.len()).map(|_| None).collect();
    let root = scene.root.expect("importer always emits a root");
    let mut stack = vec![(root, false)];
    while let Some((index, visited)) = stack.pop() {
        let n = &scene.nodes[index];
        if !visited {
            stack.push((index, true));
            stack.extend(n.children.iter().rev().map(|&c| (c, false)));
            if let Some(mask) = &n.mask {
                stack.push((mask.root, false));
            }
            continue;
        }
        let mut out = Node::new(n.primitive.as_ref().map(primitive));
        out.id = n.id.clone();
        out.classes = (!n.classes.is_empty()).then(|| n.classes.clone());
        out.transform = matrix(n.transform);
        out.opacity = n.opacity;
        out.clip = n.clip.as_ref().map(|c| Clip {
            path: Path {
                commands: commands(&c.commands),
                fill_rule: FillRule(c.fill_rule),
                ..Default::default()
            },
            transform: matrix(c.transform),
        });
        out.filters = (!n.filters.is_empty()).then(|| {
            n.filters
                .iter()
                .map(|b| Filter::GaussianBlur {
                    sigma_x: b.sigma_x,
                    sigma_y: b.sigma_y,
                    linear_rgb: b.linear_rgb,
                })
                .collect()
        });
        out.mask = n.mask.as_ref().map(|m| Mask {
            r#type: MaskType(m.kind),
            transform: Matrix::identity(),
            root: Some(std::sync::Arc::new(
                mapped[m.root].take().expect("owned mask"),
            )),
        });
        out.children = (!n.children.is_empty()).then(|| {
            n.children
                .iter()
                .map(|&c| mapped[c].take().expect("owned child"))
                .collect()
        });
        mapped[index] = Some(out);
    }
    mapped[root].take().unwrap()
}

impl Builder<'_> {
    pub(crate) fn remaining_svg_limits(&mut self, object: &str) -> Result<[usize; 9]> {
        let assets = self
            .options
            .assets
            .as_ref()
            .expect("asset options admitted");
        let b = assets.svg_import_budget;
        self.svg_budget.maximum = [
            b.max_source_bytes,
            b.max_elements,
            b.max_attributes,
            b.max_attribute_bytes,
            b.max_path_commands,
            b.max_transform_functions,
            b.max_declared_resources,
            b.max_expanded_use_instances,
        ]
        .map(|v| v.max(0) as usize);
        let l = assets.svg_import_limits;
        let mut limits = [
            l.max_bytes,
            l.max_depth,
            l.max_elements,
            l.max_attributes,
            l.max_attribute_bytes,
            l.max_path_commands,
            l.max_transform_functions,
            l.max_use_depth,
            l.max_resources,
        ]
        .map(|v| v.max(0) as usize);
        let remaining = self
            .svg_budget
            .remaining(
                object,
                [
                    limits[0], limits[2], limits[3], limits[4], limits[5], limits[6], limits[8],
                ],
            )
            .map_err(Error)?;
        for (index, value) in [0, 2, 3, 4, 5, 6, 8].into_iter().zip(remaining) {
            limits[index] = value;
        }
        Ok(limits)
    }
    pub(crate) fn reserve_svg_import(&mut self, object: &str, imported: &Imported) -> Result<()> {
        let m = &imported.metrics;
        self.svg_budget
            .reserve(
                object,
                [
                    m.source_bytes,
                    m.parsed_elements,
                    m.parsed_attributes,
                    m.parsed_attribute_bytes,
                    m.parsed_path_commands,
                    m.parsed_transform_funcs,
                    m.declared_resources,
                    m.expanded_use_instances,
                ],
                m.emitted_elements,
                m.emitted_path_commands,
            )
            .map_err(Error)?;
        let mut pending = Vec::new();
        for (id, asset) in &imported.assets.entries {
            let Some(a) = asset else {
                return err(format!(
                    "scene: {object} SVG asset {} has unsupported imported resource type <nil>",
                    q(id)
                ));
            };
            let asset = Asset::RasterAsset {
                mime_type: a.mime.clone(),
                data: a.data.clone(),
                pixel_width: a.width,
                pixel_height: a.height,
                decoded_bytes: a.decoded_bytes,
            };
            if let Some(existing) = self.assets.get(id) {
                if existing != &asset {
                    return err(format!(
                        "scene: {object} SVG asset {} collides with a different document resource",
                        q(id)
                    ));
                }
            } else {
                pending.push((id.clone(), asset));
            }
        }
        self.assets.extend(pending);
        Ok(())
    }
    pub(crate) fn import_svg(
        &mut self,
        object: &str,
        source: &str,
        data: &[u8],
        current: Option<[u8; 4]>,
        latex: bool,
    ) -> Result<Imported> {
        let budget_object = if latex {
            format!("{object} latex")
        } else {
            object.to_owned()
        };
        let limits = self.remaining_svg_limits(&budget_object)?;
        let decoded_limit = self
            .options
            .assets
            .as_ref()
            .unwrap()
            .svg_import_limits
            .max_decoded_raster_bytes;
        let outcome = if decoded_limit < 0 {
            Err("d2svgimport: all import limits must be positive".to_owned())
        } else {
            d2_rs_math::svg_import::import_labeled_with_decoded_limit(
                source,
                data,
                limits,
                current,
                decoded_limit as usize,
                None,
            )
        }
        .map_err(|e| {
            Error(if latex {
                format!("scene: {object} latex SVG: {e}")
            } else {
                format!("scene: {object} SVG asset: {e}")
            })
        })?;
        let Outcome::Imported(imported) = outcome else {
            unreachable!("native importer handles embedded raster resources");
        };
        self.reserve_svg_import(&budget_object, &imported)?;
        Ok(imported)
    }
}

pub(crate) fn vector_asset(id: &str, imported: Imported) -> Asset {
    let viewport = Box {
        width: imported.viewport.width,
        height: imported.viewport.height,
        ..Default::default()
    };
    let mut content = Node::new(None);
    content.id = format!("{id}:content");
    content.transform = matrix(imported.viewport.transform);
    content.children = Some(vec![root(imported.scene)]);
    let mut wrapper = Node::new(None);
    wrapper.id = format!("{id}:viewport");
    wrapper.clip = Some(Clip {
        path: Path {
            commands: vec![
                PathCommand::move_to(0., 0.),
                PathCommand::line_to(viewport.width, 0.),
                PathCommand::line_to(viewport.width, viewport.height),
                PathCommand::line_to(0., viewport.height),
                PathCommand::close(),
            ],
            ..Default::default()
        },
        transform: Matrix::identity(),
    });
    wrapper.children = Some(vec![content]);
    Asset::VectorAsset {
        view_box: viewport,
        root: Some(std::boxed::Box::new(wrapper)),
    }
}

pub(crate) fn redact_source(source: &str) -> String {
    if source.len() > 4096 {
        return "<oversized source locator>".into();
    }
    let source = source.trim();
    if source.is_empty() {
        return "SVG asset".into();
    }
    if source
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
    {
        return "data URI".into();
    }
    let source = source.split(['?', '#']).next().unwrap_or(source);
    let scheme = source.split_once(':').filter(|(scheme, _)| {
        !scheme.is_empty()
            && scheme.as_bytes()[0].is_ascii_alphabetic()
            && scheme
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
    });
    if let Some((scheme, rest)) = scheme {
        if !rest.starts_with('/') {
            return format!("{scheme}:<redacted>");
        }
    }
    let authority_start = if let Some((scheme, rest)) = scheme {
        rest.starts_with("//").then_some(scheme.len() + 3)
    } else {
        source.starts_with("//").then_some(2)
    };
    if let Some(start) = authority_start {
        let end = source[start..]
            .find('/')
            .map_or(source.len(), |end| start + end);
        if let Some(at) = source[start..end].rfind('@') {
            return format!("{}{}", &source[..start], &source[start + at + 1..]);
        }
    }
    source.into()
}

#[cfg(test)]
mod icon_effect_tests {
    use super::*;
    fn imported(svg: &str) -> d2_rs_math::svg_import::Imported {
        let Outcome::Imported(out) = d2_rs_math::svg_import::import_labeled(
            "icon.svg",
            svg.as_bytes(),
            [
                1_000_000, 100, 10_000, 100_000, 1_000_000, 1_000_000, 10_000, 100, 10_000,
            ],
            None,
            None,
        )
        .unwrap() else {
            panic!("unexpected image continuation")
        };
        out
    }
    fn frame(svg: &str) -> d2_rs_raster::NrgbaImage {
        let out = imported(svg);
        let doc = Document {
            logical_width: out.viewport.width,
            logical_height: out.viewport.height,
            view_box: Box {
                width: out.viewport.width,
                height: out.viewport.height,
                ..Default::default()
            },
            root: Some(std::boxed::Box::new(root(out.scene))),
            ..Default::default()
        };
        d2_rs_raster::render(
            &doc,
            &d2_rs_raster::FrameOptions {
                scale: 1.,
                max_width: 1000,
                max_height: 1000,
                max_pixels: 1_000_000,
                max_nodes: 10_000,
                max_depth: 100,
                max_path_commands: 1_000_000,
                max_animation_tracks: 10_000,
                max_animation_keyframes: 100_000,
                max_assets: 100,
                max_asset_bytes: 64 * 1024 * 1024,
                max_decoded_asset_bytes: 64 * 1024 * 1024,
                max_import_depth: 100,
                max_offscreen_bytes: 64 * 1024 * 1024,
                max_scanline_work: 1_000_000_000,
                max_even_odd_clip_work: 1_000_000_000,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn pixel(frame: &d2_rs_raster::NrgbaImage, x: usize, y: usize) -> [u8; 4] {
        frame.pix[y * frame.stride + x * 4..y * frame.stride + x * 4 + 4]
            .try_into()
            .unwrap()
    }
    // Upstream icon_test.go TestIconMaskRegionContentAndType.
    #[test]
    fn mask_region_content_and_type() {
        for (mode, alpha) in [("", 0), (" mask-type=\"alpha\"", 255)] {
            let svg = format!(
                r##"<svg viewBox="0 0 32 16"><mask id="m" x="25%" width="50%" maskContentUnits="objectBoundingBox"{mode}><rect width=".5" height="1" fill="white"/><rect x=".5" width=".5" height="1" fill="black"/></mask><g transform="translate(4 0)" mask="url(#m)"><rect x="2" y="2" width="16" height="12" fill="red"/></g></svg>"##
            );
            let f = frame(&svg);
            assert_eq!(pixel(&f, 9, 8)[3], 0);
            assert_eq!(pixel(&f, 11, 8), [255, 0, 0, 255]);
            assert_eq!(pixel(&f, 16, 8)[3], alpha);
            assert_eq!(pixel(&f, 18, 8)[3], 0);
        }
    }
    // Upstream icon_test.go TestIconGaussianBlurColorSpaceAndRegion.
    #[test]
    fn gaussian_blur_color_space_and_region() {
        let mut colors = Vec::new();
        for space in ["", " color-interpolation-filters=\"sRGB\""] {
            let svg = format!(
                r##"<svg viewBox="0 0 36 24"><filter id="blur" x="0" y="-50%" width="1" height="200%"{space}><feGaussianBlur stdDeviation="2"/></filter><g filter="url(#blur)"><rect x="6" y="6" width="12" height="12" fill="red"/><rect x="18" y="6" width="12" height="12" fill="blue"/></g></svg>"##
            );
            let f = frame(&svg);
            let edge = pixel(&f, 12, 4);
            assert!(edge[0] >= 240 && edge[3] > 0 && edge[3] < 255, "{edge:?}");
            assert_eq!(pixel(&f, 5, 12)[3], 0);
            colors.push(pixel(&f, 17, 12));
        }
        assert!(
            colors[0][0] > colors[1][0] + 20 && colors[0][2] > colors[1][2] + 20,
            "{colors:?}"
        );
    }
    #[test]
    fn gaussian_filter_admission_and_primitive_units() {
        for (body, expected) in [
            (
                r##"<filter id="f"><feGaussianBlur in="BackgroundImage"/></filter>"##,
                "SourceGraphic",
            ),
            (
                r##"<filter id="f"><feGaussianBlur stdDeviation="-1"/></filter>"##,
                "deviation",
            ),
            (
                r##"<filter id="f"><feGaussianBlur/><feGaussianBlur/></filter>"##,
                "one feGaussianBlur",
            ),
            (
                r##"<filter id="f" style="opacity:1"><feGaussianBlur/></filter>"##,
                "unsupported attribute",
            ),
        ] {
            let svg = format!("<svg width=\"8\" height=\"8\">{body}</svg>");
            let error = d2_rs_math::svg_import::import_labeled(
                "icon.svg",
                svg.as_bytes(),
                [10000, 100, 1000, 1000, 10000, 10000, 1000, 100, 1000],
                None,
                None,
            )
            .err()
            .expect("rejected filter");
            assert!(error.contains(expected), "{error}");
        }
        let out = imported(
            r##"<svg width="40" height="20"><filter id="f" primitiveUnits="objectBoundingBox" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation=".1 .2" color-interpolation-filters="inherit"/></filter><rect width="30" height="10" filter="url(#f)"/></svg>"##,
        );
        let blur = out
            .scene
            .nodes
            .iter()
            .find_map(|n| n.filters.first())
            .unwrap();
        assert_eq!(
            (blur.sigma_x, blur.sigma_y, blur.linear_rgb),
            (3., 2., false)
        );
    }
    #[test]
    fn mask_definition_inherits_its_ancestors() {
        let f = frame(
            r##"<svg viewBox="0 0 8 8"><defs fill="white"><mask id="m"><rect width="8" height="8"/></mask></defs><rect width="8" height="8" fill="red" mask="url(#m)"/></svg>"##,
        );
        assert_eq!(pixel(&f, 4, 4), [255, 0, 0, 255]);
    }
    #[test]
    fn mask_references_are_bounded_and_typed() {
        for (body, expected) in [
            (
                r##"<mask id="m"><rect width="8" height="8" mask="url(#m)"/></mask><rect width="8" height="8" mask="url(#m)"/>"##,
                "nested mask",
            ),
            (r##"<mask id="m"/><use href="#m"/>"##, "unsupported <mask>"),
            (
                r##"<g id="g"/><rect width="8" height="8" mask="url(#g)"/>"##,
                "<mask> resource",
            ),
            (r##"<mask id="m" width="-1"/>"##, "negative region"),
        ] {
            let svg = format!("<svg width=\"8\" height=\"8\">{body}</svg>");
            let e = d2_rs_math::svg_import::import_labeled(
                "icon.svg",
                svg.as_bytes(),
                [10000, 100, 1000, 1000, 10000, 10000, 1000, 100, 1000],
                None,
                None,
            )
            .err()
            .expect("rejected mask");
            assert!(e.contains(expected), "{e}");
        }
    }
}
