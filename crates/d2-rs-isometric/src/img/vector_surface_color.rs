// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/vector_surface_color.go: bounded COLRv1 paint plans.
use super::vector_surface_svg::{SurfaceSvg, box_attrs, matrix};
use d2_rs_fontface::{
    COLRv1ColorLine, COLRv1CompositeMode, COLRv1Paint as C, COLRv1Plan, ParsedFace,
};
use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use d2_rs_scene::*;
impl SurfaceSvg<'_> {
    pub fn color_glyph(
        &mut self,
        face: &ParsedFace,
        plan: &COLRv1Plan,
        origin: Point,
        size: f64,
        depth: usize,
    ) -> String {
        let Some(clip) = plan.clip else {
            self.fail("native SVG color glyph has no bounded paint plan");
            return String::new();
        };
        let scale = size / face.outline().units_per_em() as f64;
        let transform = Matrix::translate(origin.x, origin.y).mul(Matrix::scale(scale, -scale));
        let b = Box {
            x: clip.x_min,
            y: clip.y_min,
            width: clip.x_max - clip.x_min,
            height: clip.y_max - clip.y_min,
        };
        let id = self.id("color-clip");
        self.def(format!(
            "<clipPath id=\"{id}\"><rect{}/></clipPath>",
            box_attrs(b)
        ));
        let body = self.color_paint(face, &plan.root, b, Matrix::identity(), depth + 1);
        format!(
            "<g transform=\"{}\"><g clip-path=\"url(#{id})\">{body}</g></g>",
            matrix(transform)
        )
    }
    fn color_paint(
        &mut self,
        face: &ParsedFace,
        p: &C,
        b: Box,
        current: Matrix,
        depth: usize,
    ) -> String {
        if !self.admit(depth) {
            return String::new();
        }
        let stops = |line: &COLRv1ColorLine| {
            line.stops
                .iter()
                .map(|s| {
                    let c = s.color;
                    GradientStop {
                        offset: s.offset,
                        color: Nrgba::new(c.r, c.g, c.b, c.a),
                    }
                })
                .collect()
        };
        let fill = match p {
            C::Layers(p) => {
                let mut body = String::new();
                for child in &p.paints {
                    let s = self.color_paint(face, child, b, current, depth + 1);
                    self.append(&mut body, s)
                }
                return body;
            }
            C::Transform(p) => {
                let m = p.matrix;
                let m = Matrix {
                    a: m.xx,
                    b: m.yx,
                    c: m.xy,
                    d: m.yy,
                    e: m.dx,
                    f: m.dy,
                };
                let body = self.color_paint(face, &p.paint, b, current.mul(m), depth + 1);
                return format!("<g transform=\"{}\">{body}</g>", matrix(m));
            }
            C::Glyph(p) => {
                let name = match self.glyph_definition(
                    face,
                    p.glyph_id,
                    face.outline().units_per_em() << 6,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        self.fail(e);
                        return String::new();
                    }
                };
                let id = self.id("glyph-clip");
                self.def(format!("<clipPath id=\"{id}\"><use transform=\"scale(1 -1)\" href=\"#{name}\"/></clipPath>"));
                let body = self.color_paint(face, &p.paint, b, current, depth + 1);
                return format!("<g clip-path=\"url(#{id})\">{body}</g>");
            }
            C::Composite(p) => {
                let backdrop = self.color_paint(face, &p.backdrop, b, current, depth + 1);
                let source = self.color_paint(face, &p.source, b, current, depth + 1);
                match p.mode {
                    COLRv1CompositeMode::SrcIn => {
                        let id = self.id("color-mask");
                        let inv = match current.inverse() {
                            Ok(m) => m,
                            Err(e) => {
                                self.fail(e);
                                return String::new();
                            }
                        };
                        let bb = b.bounds().transform(inv).to_box();
                        self.def(format!("<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\"{} mask-type=\"alpha\">{backdrop}</mask>",box_attrs(bb)));
                        return format!("<g mask=\"url(#{id})\">{source}</g>");
                    }
                    COLRv1CompositeMode::SoftLight => {
                        return format!(
                            "<g style=\"isolation:isolate\">{backdrop}<g style=\"mix-blend-mode:soft-light\">{source}</g></g>"
                        );
                    }
                }
            }
            C::Solid(p) => {
                let c = p.color;
                Paint::solid(Nrgba::new(c.r, c.g, c.b, c.a))
            }
            C::LinearGradient(p) => {
                let (vx, vy, rx, ry) = (p.x1 - p.x0, p.y1 - p.y0, p.x2 - p.x0, p.y2 - p.y0);
                // ARM64 colorPaint:82,87 fuses the first product.
                let den = go_fmadd(rx, rx, ry * ry);
                if den <= 0.0 {
                    self.fail("native SVG invalid color gradient");
                    return String::new();
                }
                let k = go_fmadd(vx, rx, vy * ry) / den;
                Paint::LinearGradient(LinearGradient {
                    start: Point::new(p.x0, p.y0),
                    end: Point::new(go_fmsub(k, rx, p.x0 + vx), go_fmsub(k, ry, p.y0 + vy)),
                    stops: stops(&p.color_line),
                    units: PaintUnits::USER_SPACE_ON_USE,
                    transform: Matrix::identity(),
                    ..Default::default()
                })
            }
            C::RadialGradient(p) => Paint::RadialGradient(RadialGradient {
                center: Point::new(p.x1, p.y1),
                radius: p.radius1,
                focal: Point::new(p.x0, p.y0),
                focal_radius: p.radius0,
                stops: stops(&p.color_line),
                units: PaintUnits::USER_SPACE_ON_USE,
                transform: Matrix::identity(),
                ..Default::default()
            }),
        };
        let inv = match current.inverse() {
            Ok(m) => m,
            Err(e) => {
                self.fail(e);
                return String::new();
            }
        };
        let bb = b.bounds().transform(inv).to_box();
        let previous = self.linear_gradients;
        self.linear_gradients = true;
        let s = self.primitive(
            Some(&Primitive::Rect(Rect {
                r#box: bb,
                fill: Some(fill),
                ..Default::default()
            })),
            depth + 1,
        );
        self.linear_gradients = previous;
        s
    }
}

#[cfg(test)]
impl SurfaceSvg<'_> {
    pub(super) fn original_test_color_paint(
        &mut self,
        face: &ParsedFace,
        p: &C,
        b: Box,
        current: Matrix,
        depth: usize,
    ) -> String {
        self.color_paint(face, p, b, current, depth)
    }
}
