// SPDX-License-Identifier: MPL-2.0
//! d2raster/colrv1.go: compile authenticated color-font paint plans into the
//! same prepared tree as scene primitives, including isolated composites.

use std::rc::Rc;

use d2_rs_fontface::{
    COLRv1Affine, COLRv1ClipBox, COLRv1ColorLine, COLRv1CompositeMode, COLRv1LinearGradient,
    COLRv1Paint, COLRv1Plan, COLRv1RadialGradient,
};
use d2_rs_scene::{
    Bounds, Box as SceneBox, FillRule, GradientStop, LinearGradient, MaskType, Matrix, Nrgba,
    PaintUnits, Point, RadialGradient, SpreadMethod,
};

use crate::fma::{finite, go_fmadd, go_fmsub, mmul, mpoint};
use crate::geometry::{GEOMETRY_EPSILON, Subpath, flatten_tolerance, subpath_pixel_bounds};
use crate::paint::{
    PaintKind, PreparedPaint, average_colrv1_gradient_color, prepare_linear_gradient,
    prepare_radial_gradient,
};
use crate::pixels::{Rect, union_rect};
use crate::raster::{
    PREPARED_COLRV1_SOFT_LIGHT, Preflight, PreparedClip, PreparedMask, PreparedNode,
    validate_raster_point,
};
use crate::text::{PreparedFont, flatten_glyph};

impl<'d> Preflight<'d> {
    pub(crate) fn colrv1_text_glyph(
        &mut self,
        node_id: &str,
        font: &PreparedFont,
        plan: &COLRv1Plan,
        origin: Point,
        ppem: i32,
        transform: &Matrix,
    ) -> Result<(Option<PreparedNode>, Bounds), String> {
        self.add_prepared_nodes(plan.usage.paint_nodes as i64)
            .map_err(|e| {
                format!(
                    "d2raster: node {node_id:?} COLRv1 glyph {}: {e}",
                    plan.glyph_id
                )
            })?;
        let units_per_em = font.outline.units_per_em();
        if units_per_em <= 0 {
            return Err(format!(
                "d2raster: node {node_id:?} COLRv1 font has invalid units per em {units_per_em}"
            ));
        }
        let scale = (ppem as f64 / 64.0) / units_per_em as f64;
        let font_to_text = mmul(
            &Matrix::translate(origin.x, origin.y),
            &Matrix::scale(scale, -scale),
        );
        let mut compiler = TextCompiler {
            preflight: self,
            node_id,
            font,
            clip_box: plan.clip.as_ref(),
            font_to_device: mmul(transform, &font_to_text),
        };
        let mut root = compiler
            .paint(&plan.root, &Matrix::identity())
            .map_err(|e| {
                format!(
                    "d2raster: node {node_id:?} COLRv1 glyph {}: {e}",
                    plan.glyph_id
                )
            })?;
        let mut local_bounds = Bounds::default();
        if let Some(clip_box) = &plan.clip {
            let clip = compiler.clip(clip_box, &Matrix::identity()).map_err(|e| {
                format!(
                    "d2raster: node {node_id:?} COLRv1 glyph {} clip: {e}",
                    plan.glyph_id
                )
            })?;
            if let Some(root) = &mut root {
                root.bounds = root.bounds.intersect(clip.bounds);
                root.content_bounds = root.bounds;
                root.clip = Some(clip);
            }
            local_bounds = transformed_clip_bounds(clip_box, &font_to_text);
        }
        Ok((root, local_bounds))
    }
}

struct TextCompiler<'p, 'd, 'f> {
    preflight: &'p mut Preflight<'d>,
    node_id: &'f str,
    font: &'f PreparedFont,
    clip_box: Option<&'f COLRv1ClipBox>,
    font_to_device: Matrix,
}

impl TextCompiler<'_, '_, '_> {
    fn paint(
        &mut self,
        paint: &COLRv1Paint,
        current: &Matrix,
    ) -> Result<Option<PreparedNode>, String> {
        match paint {
            COLRv1Paint::Layers(layers) => {
                let mut root = PreparedNode::group(Vec::new(), None);
                for (index, paint) in layers.paints.iter().enumerate() {
                    if let Some(child) = self
                        .paint(paint, current)
                        .map_err(|e| format!("layer {index}: {e}"))?
                    {
                        append_prepared_child(&mut root, child);
                    }
                }
                Ok(nonempty(root))
            }
            COLRv1Paint::Glyph(glyph) => {
                let fill = self
                    .glyph_fill(&glyph.paint, current)
                    .map_err(|e| format!("glyph {} fill: {e}", glyph.glyph_id))?;
                self.glyph(glyph.glyph_id, fill, current)
            }
            COLRv1Paint::Transform(transform) => {
                let next = mmul(current, &affine(transform.matrix));
                if !next.is_finite() {
                    return Err("non-finite composed transform".into());
                }
                self.paint(&transform.paint, &next)
            }
            COLRv1Paint::Composite(composite) => {
                let backdrop = self
                    .paint(&composite.backdrop, current)
                    .map_err(|e| format!("composite backdrop: {e}"))?;
                let source = self
                    .paint(&composite.source, current)
                    .map_err(|e| format!("composite source: {e}"))?;
                match composite.mode {
                    COLRv1CompositeMode::SrcIn => {
                        let (Some(source), Some(backdrop)) = (source, backdrop) else {
                            return Ok(None);
                        };
                        let mut root = PreparedNode::group(Vec::new(), None);
                        root.content_bounds = source.bounds;
                        root.bounds = source.bounds.intersect(backdrop.bounds);
                        root.children.push(source);
                        root.mask = Some(Box::new(PreparedMask {
                            kind: MaskType::ALPHA,
                            root: backdrop,
                        }));
                        Ok(nonempty(root))
                    }
                    COLRv1CompositeMode::SoftLight => {
                        let Some(backdrop) = backdrop else {
                            return Ok(source);
                        };
                        let Some(mut source) = source else {
                            return Ok(Some(backdrop));
                        };
                        source.blend = PREPARED_COLRV1_SOFT_LIGHT;
                        let mut root = PreparedNode::group(Vec::new(), None);
                        root.isolated = true;
                        append_prepared_child(&mut root, backdrop);
                        append_prepared_child(&mut root, source);
                        Ok(Some(root))
                    }
                }
            }
            _ => self.unbounded_fill(paint, current),
        }
    }

    fn unbounded_fill(
        &mut self,
        paint: &COLRv1Paint,
        current: &Matrix,
    ) -> Result<Option<PreparedNode>, String> {
        let clip_box = self.clip_box.ok_or_else(|| {
            format!(
                "unbounded {} paint has no static root clip",
                paint_go_type(paint)
            )
        })?;
        let fill = self.glyph_fill(paint, current)?;
        let points = clip_points(clip_box);
        for _ in &points {
            self.preflight.add_path_segment()?;
        }
        self.primitive(
            vec![Subpath {
                points: points.to_vec(),
                closed: true,
            }],
            &self.font_to_device.clone(),
            fill,
        )
    }

    fn glyph(
        &mut self,
        glyph_id: u32,
        fill: PreparedPaint,
        current: &Matrix,
    ) -> Result<Option<PreparedNode>, String> {
        if glyph_id == 0
            || glyph_id > u16::MAX as u32
            || glyph_id as usize >= self.font.num_glyphs()
        {
            return Err(format!("glyph ID {glyph_id} is out of range"));
        }
        let units_per_em = self.font.outline.units_per_em();
        let segments = self
            .font
            .outline
            .load_glyph(glyph_id as u16, units_per_em.wrapping_shl(6))
            .map_err(|e| format!("load glyph {glyph_id}: {e}"))?;
        if segments.is_empty() {
            return Ok(None);
        }
        // sfnt outlines are y-down; paint coordinates are y-up.
        let to_device = mmul(
            &mmul(&self.font_to_device, current),
            &Matrix::scale(1.0, -1.0),
        );
        let paths = flatten_glyph(
            &segments,
            Point::default(),
            flatten_tolerance(&to_device),
            &mut || self.preflight.add_path_segment(),
        )
        .map_err(|e| format!("flatten glyph {glyph_id}: {e}"))?;
        self.primitive(paths, &to_device, fill)
    }

    fn primitive(
        &mut self,
        paths: Vec<Subpath>,
        transform: &Matrix,
        fill: PreparedPaint,
    ) -> Result<Option<PreparedNode>, String> {
        let primitive =
            self.preflight
                .finish_primitive(self.node_id, paths, transform, Some(fill), None)?;
        if primitive.bounds.empty() {
            return Ok(None);
        }
        let mut node = PreparedNode::group(Vec::new(), None);
        node.bounds = primitive.bounds;
        node.content_bounds = primitive.bounds;
        node.primitive = Some(Box::new(primitive));
        Ok(Some(node))
    }

    fn glyph_fill(
        &mut self,
        paint: &COLRv1Paint,
        current: &Matrix,
    ) -> Result<PreparedPaint, String> {
        match paint {
            COLRv1Paint::Solid(solid) => Ok(PreparedPaint::solid(Nrgba::new(
                solid.color.r,
                solid.color.g,
                solid.color.b,
                solid.color.a,
            ))),
            COLRv1Paint::LinearGradient(gradient) => {
                prepare_color_linear_gradient(gradient, &mmul(&self.font_to_device, current))
            }
            COLRv1Paint::RadialGradient(gradient) => {
                prepare_color_radial_gradient(gradient, &mmul(&self.font_to_device, current))
            }
            COLRv1Paint::Transform(transform) => {
                let next = mmul(current, &affine(transform.matrix));
                if !next.is_finite() {
                    return Err("non-finite composed paint transform".into());
                }
                self.glyph_fill(&transform.paint, &next)
            }
            _ => Err(format!("unsupported glyph fill {}", paint_go_type(paint))),
        }
    }

    fn clip(&mut self, clip_box: &COLRv1ClipBox, current: &Matrix) -> Result<PreparedClip, String> {
        let to_device = mmul(&self.font_to_device, current);
        let points = clip_points(clip_box).map(|point| mpoint(&to_device, point));
        for point in &points {
            validate_raster_point(*point)?;
            self.preflight.add_path_segment()?;
        }
        let paths = vec![Subpath {
            points: points.to_vec(),
            closed: true,
        }];
        let bounds = subpath_pixel_bounds(
            &paths,
            &Matrix::identity(),
            0.0,
            self.preflight.preparation_bounds(),
        );
        Ok(PreparedClip {
            reference_bounds: Rect::default(),
            subpaths: paths,
            fill_rule: FillRule::NON_ZERO,
            bounds,
            edges: points.len() as i64,
        })
    }
}

fn prepare_color_linear_gradient(
    gradient: &COLRv1LinearGradient,
    to_device: &Matrix,
) -> Result<PreparedPaint, String> {
    let p0 = Point::new(gradient.x0, gradient.y0);
    let v = Point::new(gradient.x1 - p0.x, gradient.y1 - p0.y);
    let r = Point::new(gradient.x2 - p0.x, gradient.y2 - p0.y);
    // Pinned ARM64 colrv1.go:272/276 fuse the first product; :278 the second.
    let r2 = go_fmadd(r.x, r.x, r.y * r.y);
    if r2 == 0.0 || !finite(r2) {
        return Err("ill-formed linear gradient rotation vector".into());
    }
    let projection = go_fmadd(v.x, r.x, v.y * r.y) / r2;
    let w = Point::new(
        go_fmsub(projection, r.x, v.x),
        go_fmsub(projection, r.y, v.y),
    );
    if go_fmadd(w.y, w.y, w.x * w.x) <= GEOMETRY_EPSILON * GEOMETRY_EPSILON {
        return Err("ill-formed linear gradient projection".into());
    }
    let mut paint = prepare_linear_gradient(
        &LinearGradient {
            start: p0,
            end: Point::new(p0.x + w.x, p0.y + w.y),
            stops: color_line(&gradient.color_line),
            units: PaintUnits::USER_SPACE_ON_USE,
            transform: Matrix::identity(),
            spread: SpreadMethod::PAD,
        },
        SceneBox::default(),
        to_device,
    )?;
    if paint.kind != PaintKind::Solid {
        Rc::make_mut(&mut paint.gradient).colrv1_interpolation = true;
    }
    Ok(paint)
}

fn prepare_color_radial_gradient(
    gradient: &COLRv1RadialGradient,
    to_device: &Matrix,
) -> Result<PreparedPaint, String> {
    let mut paint = prepare_radial_gradient(
        &RadialGradient {
            focal: Point::new(gradient.x0, gradient.y0),
            focal_radius: gradient.radius0,
            center: Point::new(gradient.x1, gradient.y1),
            radius: gradient.radius1,
            stops: color_line(&gradient.color_line),
            units: PaintUnits::USER_SPACE_ON_USE,
            transform: Matrix::identity(),
            spread: SpreadMethod::PAD,
        },
        SceneBox::default(),
        to_device,
    )?;
    if paint.kind != PaintKind::Solid {
        let gradient = Rc::make_mut(&mut paint.gradient);
        gradient.colrv1_interpolation = true;
        gradient.radial_average = average_colrv1_gradient_color(&gradient.stops);
    }
    Ok(paint)
}

fn color_line(line: &COLRv1ColorLine) -> Vec<GradientStop> {
    line.stops
        .iter()
        .map(|stop| GradientStop {
            offset: stop.offset,
            color: Nrgba::new(stop.color.r, stop.color.g, stop.color.b, stop.color.a),
        })
        .collect()
}

fn affine(matrix: COLRv1Affine) -> Matrix {
    Matrix {
        a: matrix.xx,
        b: matrix.yx,
        c: matrix.xy,
        d: matrix.yy,
        e: matrix.dx,
        f: matrix.dy,
    }
}

pub(crate) fn append_prepared_child(parent: &mut PreparedNode, child: PreparedNode) {
    if child.bounds.empty() {
        return;
    }
    parent.bounds = union_rect(parent.bounds, child.bounds);
    parent.content_bounds = parent.bounds;
    parent.children.push(child);
}

fn nonempty(node: PreparedNode) -> Option<PreparedNode> {
    if node.bounds.empty() {
        None
    } else {
        Some(node)
    }
}

fn clip_points(clip_box: &COLRv1ClipBox) -> [Point; 4] {
    [
        Point::new(clip_box.x_min, clip_box.y_min),
        Point::new(clip_box.x_max, clip_box.y_min),
        Point::new(clip_box.x_max, clip_box.y_max),
        Point::new(clip_box.x_min, clip_box.y_max),
    ]
}

fn transformed_clip_bounds(clip_box: &COLRv1ClipBox, transform: &Matrix) -> Bounds {
    Bounds::from_points(&clip_points(clip_box).map(|point| mpoint(transform, point)))
}

fn paint_go_type(paint: &COLRv1Paint) -> &'static str {
    match paint {
        COLRv1Paint::Layers(_) => "fontface.COLRv1Layers",
        COLRv1Paint::Solid(_) => "fontface.COLRv1Solid",
        COLRv1Paint::LinearGradient(_) => "fontface.COLRv1LinearGradient",
        COLRv1Paint::RadialGradient(_) => "fontface.COLRv1RadialGradient",
        COLRv1Paint::Glyph(_) => "fontface.COLRv1Glyph",
        COLRv1Paint::Transform(_) => "fontface.COLRv1Transform",
        COLRv1Paint::Composite(_) => "fontface.COLRv1Composite",
    }
}
