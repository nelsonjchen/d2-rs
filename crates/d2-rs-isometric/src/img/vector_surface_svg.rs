// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg/vector_surface_svg.go and vector_surface_animation.go.
//! Retained scene surfaces serialize to self-contained SVG in UV coordinates.
use super::vector_surface::VectorSurface;
use base64::Engine as _;
use d2_rs_fontface::ParsedFace;
use d2_rs_scene::*;
use std::fmt::Write;
use std::{collections::HashMap, rc::Rc};

pub(super) struct SurfaceSvg<'a> {
    pub doc: &'a Document,
    context: super::context::RenderContext,
    prefix: &'a str,
    defs: String,
    next: usize,
    nodes: usize,
    commands: usize,
    pub err: Option<String>,
    pub fonts: HashMap<String, Rc<ParsedFace>>,
    pub glyphs: HashMap<(usize, u32, i32), String>,
    pub transform: Matrix,
    pub linear_gradients: bool,
}

pub fn number(n: f64) -> String {
    if n == 0.0 { "0".into() } else { format_g(n) }
}
pub fn matrix(m: Matrix) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        number(m.a),
        number(m.b),
        number(m.c),
        number(m.d),
        number(m.e),
        number(m.f)
    )
}
pub fn attr(name: &str, value: &str) -> String {
    let s = value
        .replace('&', "&amp;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;");
    format!(" {name}=\"{s}\"")
}
pub fn float(name: &str, value: f64) -> String {
    attr(name, &number(value))
}
pub fn color(c: Nrgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}
pub fn box_attrs(b: Box) -> String {
    float("x", b.x) + &float("y", b.y) + &float("width", b.width) + &float("height", b.height)
}
fn fill_rule(r: FillRule) -> &'static str {
    if r == FillRule::EVEN_ODD {
        "evenodd"
    } else {
        "nonzero"
    }
}

pub fn viewport(doc: &Document) -> Result<Matrix, String> {
    let (width, height) = (doc.logical_width, doc.logical_height);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("native SVG surface has invalid dimensions".into());
    }
    let align = if doc.viewport_fit == ViewportFit::MEET {
        if doc.viewport_align == ViewportAlign::X_MID_Y_MID {
            AspectAlign::X_MID_Y_MID
        } else {
            AspectAlign::X_MIN_Y_MIN
        }
    } else {
        AspectAlign::NONE
    };
    let m = aspect_ratio_matrix(
        doc.view_box,
        Box {
            width,
            height,
            ..Default::default()
        },
        AspectRatio {
            align,
            ..Default::default()
        },
    )
    .map_err(|e| format!("native SVG surface has invalid viewport: {e}"))?;
    Ok(Matrix::scale(1.0 / width, 1.0 / height).mul(m))
}

pub fn native_surface_svg(surface: &VectorSurface, prefix: &str) -> Result<String, String> {
    native_surface_svg_with_context(surface, prefix, &Default::default())
}

/// Preserve the export's cancellation cause while traversing retained artwork.
pub fn native_surface_svg_with_context(
    surface: &VectorSurface,
    prefix: &str,
    context: &super::context::RenderContext,
) -> Result<String, String> {
    if prefix.is_empty() {
        return Err("native SVG requires a vector surface and prefix".into());
    }
    if !prefix
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err("native SVG has an invalid resource prefix".into());
    }
    let document = surface.document.borrow();
    let doc = &*document;
    let m = viewport(doc)?;
    let mut w = SurfaceSvg {
        doc,
        context: context.clone(),
        prefix,
        defs: String::new(),
        next: 0,
        nodes: 0,
        commands: 0,
        err: None,
        fonts: HashMap::new(),
        glyphs: HashMap::new(),
        transform: Matrix::scale(doc.logical_width, doc.logical_height).mul(m),
        linear_gradients: false,
    };
    let body = w.node(doc.root.as_deref(), 0);
    if let Some(e) = w.err.take() {
        return Err(e);
    }
    let mut content = format!("<g transform=\"{}\">{body}</g>", matrix(m));
    if let Some(c) = surface.cap_background {
        content = format!(
            "<rect width=\"1\" height=\"1\" fill=\"{}\"/>{content}",
            color(c)
        );
    }
    if let Some(p) = &surface.cap_paint {
        content = w.primitive(
            Some(&Primitive::Rect(Rect {
                r#box: Box {
                    width: 1.0,
                    height: 1.0,
                    ..Default::default()
                },
                fill: Some(p.clone()),
                ..Default::default()
            })),
            0,
        ) + &content;
    }
    if let Some(ink) = &*surface.ink_coverage.borrow() {
        let ink = native_surface_svg_with_context(ink, &format!("{prefix}-coverage"), context)?;
        let filter = w.id("coverage-filter");
        let mask = w.id("coverage-mask");
        let values = (0..256)
            .map(|i| {
                let a = i as f64 / 255.0;
                number((1.0 - a) / (1.0 - a * surface.coverage_opacity.get()))
            })
            .collect::<Vec<_>>()
            .join(" ");
        w.def(format!("<filter id=\"{filter}\" filterUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"1\" height=\"1\" color-interpolation-filters=\"sRGB\"><feComponentTransfer in=\"SourceAlpha\"><feFuncR type=\"linear\" slope=\"0\" intercept=\"1\"/><feFuncG type=\"linear\" slope=\"0\" intercept=\"1\"/><feFuncB type=\"linear\" slope=\"0\" intercept=\"1\"/><feFuncA type=\"table\" tableValues=\"{values}\"/></feComponentTransfer></filter>"));
        w.def(format!("<mask id=\"{mask}\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"1\" height=\"1\" mask-type=\"alpha\"><g filter=\"url(#{filter})\">{ink}</g></mask>"));
        content = format!("<g mask=\"url(#{mask})\">{content}</g>");
    }
    w.within(content.len() + 13);
    if let Some(e) = w.err {
        return Err(e);
    }
    Ok(format!("<defs>{}</defs>{content}", w.defs))
}

impl SurfaceSvg<'_> {
    pub fn fail(&mut self, e: impl Into<String>) {
        if self.err.is_none() {
            self.err = Some(e.into());
        }
    }
    pub fn id(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{}-{kind}-{}", self.prefix, self.next)
    }
    pub fn admit(&mut self, depth: usize) -> bool {
        if self.err.is_some() {
            return false;
        }
        if let Err(error) = self.context.check_string() {
            self.fail(error);
            return false;
        }
        self.nodes += 1;
        if depth > 256 || self.nodes > 200000 || self.commands > 2000000 {
            self.fail("native SVG surface exceeds node, depth, path or byte budget");
            false
        } else {
            true
        }
    }
    fn within(&mut self, size: usize) -> bool {
        if self.err.is_some() {
            return false;
        }
        if size > (32usize << 20).saturating_sub(self.defs.len()) {
            self.fail("native SVG surface exceeds 32 MiB byte budget");
            false
        } else {
            true
        }
    }
    fn keep(&mut self, s: String) -> String {
        if self.within(s.len()) {
            s
        } else {
            String::new()
        }
    }
    pub fn append(&mut self, out: &mut String, s: String) {
        if self.within(out.len() + s.len()) {
            out.push_str(&s);
        }
    }
    pub fn def(&mut self, s: String) {
        if self.within(s.len()) {
            self.defs.push_str(&s);
        }
    }
    pub fn node(&mut self, n: Option<&Node>, depth: usize) -> String {
        let Some(n) = n else { return String::new() };
        if !self.admit(depth) {
            return String::new();
        }
        let initial;
        let n = if n.animations.as_ref().is_some_and(|a| !a.is_empty()) {
            match initial_node(n) {
                Ok(v) => {
                    initial = v;
                    &initial
                }
                Err(e) => {
                    self.fail(e);
                    return String::new();
                }
            }
        } else {
            n
        };
        if n.opacity <= 0.0 {
            return String::new();
        }
        if !n.transform.is_finite() || !n.opacity.is_finite() {
            self.fail("native SVG has invalid node transform or opacity");
            return String::new();
        }
        let previous = self.transform;
        self.transform = previous.mul(n.transform);
        let mut attrs = attr("transform", &matrix(n.transform));
        if n.opacity < 1.0 {
            attrs += &float("opacity", n.opacity)
        }
        if n.blend != BlendMode::NORMAL {
            let mode = match n.blend {
                BlendMode::MULTIPLY => "multiply",
                BlendMode::DARKEN => "darken",
                BlendMode::COLOR_BURN => "color-burn",
                BlendMode::OVERLAY => "overlay",
                BlendMode::LIGHTEN => "lighten",
                _ => {
                    self.fail(format!("native SVG unsupported blend mode {}", n.blend.0));
                    ""
                }
            };
            attrs += &attr("style", &format!("mix-blend-mode:{mode};isolation:isolate"));
        }
        if let Some(c) = &n.clip {
            let id = self.id("clip");
            let path = self.path(&c.path);
            self.def(format!("<clipPath id=\"{id}\" clipPathUnits=\"userSpaceOnUse\"><path transform=\"{}\" d=\"{path}\" clip-rule=\"{}\"/></clipPath>",matrix(c.transform),fill_rule(c.path.fill_rule)));
            attrs += &attr("clip-path", &format!("url(#{id})"));
        }
        if let Some(m) = &n.mask {
            let id = self.id("mask");
            let kind = if m.r#type == MaskType::LUMINANCE {
                "luminance"
            } else {
                "alpha"
            };
            let body = self.node(m.root.as_deref(), depth + 1);
            let b = self.surface_bounds();
            self.def(format!("<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\"{} mask-type=\"{kind}\"><g transform=\"{}\">{body}</g></mask>",box_attrs(b),matrix(m.transform)));
            attrs += &attr("mask", &format!("url(#{id})"));
        }
        if let Some(filters) = n.filters.as_ref().filter(|f| !f.is_empty()) {
            let id = self.id("filter");
            let mut effects = String::new();
            for f in filters {
                effects += &match f {
                    Filter::GaussianBlur { sigma_x, sigma_y } => format!(
                        "<feGaussianBlur stdDeviation=\"{} {}\"/>",
                        number(*sigma_x),
                        number(*sigma_y)
                    ),
                    Filter::DropShadow(f) => format!(
                        "<feDropShadow{}{}{}{}{}/>",
                        float("dx", f.offset_x),
                        float("dy", f.offset_y),
                        attr(
                            "stdDeviation",
                            &format!("{} {}", number(f.sigma_x), number(f.sigma_y))
                        ),
                        attr("flood-color", &color(f.color)),
                        float("flood-opacity", f.color.a as f64 / 255.0)
                    ),
                };
            }
            self.def(format!("<filter id=\"{id}\" x=\"-100%\" y=\"-100%\" width=\"300%\" height=\"300%\" color-interpolation-filters=\"sRGB\">{effects}</filter>"));
            attrs += &attr("filter", &format!("url(#{id})"));
        }
        let mut body = String::new();
        let s = self.primitive(n.primitive.as_ref(), depth + 1);
        self.append(&mut body, s);
        for child in n.children() {
            let s = self.node(Some(child), depth + 1);
            self.append(&mut body, s)
        }
        self.transform = previous;
        self.keep(format!("<g{attrs}>{body}</g>"))
    }
    pub fn path(&mut self, path: &Path) -> String {
        self.commands += path.commands.len();
        if self.commands > 2000000 {
            self.fail("native SVG surface exceeds path command budget");
            return String::new();
        }
        let mut s = String::new();
        let point = |p: Point| format!("{} {}", number(p.x), number(p.y));
        for c in &path.commands {
            use PathCommandKind as K;
            s += &match c.kind {
                K::MOVE => format!("M{}", point(c.p1)),
                K::LINE => format!("L{}", point(c.p1)),
                K::QUADRATIC => format!("Q{} {}", point(c.p1), point(c.p2)),
                K::CUBIC => format!("C{} {} {}", point(c.p1), point(c.p2), point(c.p3)),
                K::ARC => format!(
                    "A{} {} {} {} {} {}",
                    number(c.radius_x),
                    number(c.radius_y),
                    number(c.rotation * 180.0 / std::f64::consts::PI),
                    u8::from(c.large_arc),
                    u8::from(c.sweep),
                    point(c.p1)
                ),
                K::CLOSE => "Z".into(),
                _ => {
                    self.fail(format!("native SVG unsupported path command {}", c.kind.0));
                    String::new()
                }
            };
            if !self.within(s.len()) {
                return String::new();
            }
        }
        s
    }
    pub fn paint(&mut self, p: Option<&Paint>, role: &str, depth: usize) -> String {
        let Some(p) = p else {
            return attr(role, "none");
        };
        if let Paint::SolidPaint { color: c } = p {
            return attr(role, &color(*c)) + &float(&format!("{role}-opacity"), c.a as f64 / 255.0);
        }
        let id = self.id("paint");
        let units = |u| {
            if u == PaintUnits::USER_SPACE_ON_USE {
                "userSpaceOnUse"
            } else {
                "objectBoundingBox"
            }
        };
        let spread = |s| {
            if s == SpreadMethod::REFLECT {
                "reflect"
            } else if s == SpreadMethod::REPEAT {
                "repeat"
            } else {
                "pad"
            }
        };
        let stops = |ss: &[GradientStop]| {
            ss.iter()
                .map(|s| {
                    format!(
                        "<stop{}{}{}/>",
                        float("offset", s.offset),
                        attr("stop-color", &color(s.color)),
                        float("stop-opacity", s.color.a as f64 / 255.0)
                    )
                })
                .collect::<String>()
        };
        let interpolation = attr(
            "color-interpolation",
            if self.linear_gradients {
                "linearRGB"
            } else {
                "sRGB"
            },
        );
        let def = match p {
            Paint::LinearGradient(p) => format!(
                "<linearGradient id=\"{id}\"{}{}{}{}{}{}{}{}>{}</linearGradient>",
                attr("gradientUnits", units(p.units)),
                interpolation,
                attr("gradientTransform", &matrix(p.transform)),
                attr("spreadMethod", spread(p.spread)),
                float("x1", p.start.x),
                float("y1", p.start.y),
                float("x2", p.end.x),
                float("y2", p.end.y),
                stops(&p.stops)
            ),
            Paint::RadialGradient(p) => format!(
                "<radialGradient id=\"{id}\"{}{}{}{}{}{}{}{}{}{}>{}</radialGradient>",
                attr("gradientUnits", units(p.units)),
                interpolation,
                attr("gradientTransform", &matrix(p.transform)),
                attr("spreadMethod", spread(p.spread)),
                float("cx", p.center.x),
                float("cy", p.center.y),
                float("r", p.radius),
                float("fx", p.focal.x),
                float("fy", p.focal.y),
                float("fr", p.focal_radius),
                stops(&p.stops)
            ),
            Paint::PatternPaint(p) => {
                let body = self.node(p.root.as_deref(), depth + 1);
                format!(
                    "<pattern id=\"{id}\"{}{}{}{}{}{}{} preserveAspectRatio=\"none\">{body}</pattern>",
                    attr("patternUnits", units(p.units)),
                    attr("patternTransform", &matrix(p.transform)),
                    box_attrs(p.tile),
                    "",
                    "",
                    "",
                    attr(
                        "viewBox",
                        &format!("0 0 {} {}", number(p.tile.width), number(p.tile.height))
                    )
                )
            }
            Paint::SolidPaint { .. } => unreachable!(),
        };
        self.def(def);
        attr(role, &format!("url(#{id})"))
    }
    pub fn style(&mut self, fill: Option<&Paint>, stroke: Option<&Stroke>, depth: usize) -> String {
        let mut s = self.paint(fill, "fill", depth);
        let Some(stroke) = stroke.filter(|s| s.paint.is_some() && s.width > 0.0) else {
            return s + " stroke=\"none\"";
        };
        s += &self.paint(stroke.paint.as_ref(), "stroke", depth);
        s += &float("stroke-width", stroke.width);
        let (Some(cap), Some(join)) = (
            ["butt", "round", "square"].get(stroke.cap.0 as usize),
            ["miter", "round", "bevel"].get(stroke.join.0 as usize),
        ) else {
            self.fail("native SVG has invalid stroke joins");
            return String::new();
        };
        s += &attr("stroke-linecap", cap);
        s += &attr("stroke-linejoin", join);
        if stroke.miter_limit > 0.0 {
            s += &float("stroke-miterlimit", stroke.miter_limit)
        }
        if let Some(ds) = stroke.dashes.as_ref().filter(|ds| !ds.is_empty()) {
            s += &attr(
                "stroke-dasharray",
                &ds.iter().map(|v| number(*v)).collect::<Vec<_>>().join(" "),
            );
            s += &float("stroke-dashoffset", stroke.dash_offset)
        }
        s
    }
    pub fn primitive(&mut self, p: Option<&Primitive>, depth: usize) -> String {
        let Some(p) = p else { return String::new() };
        match p {
            Primitive::Path(p) => {
                let d = self.path(p);
                let style = self.style(p.fill.as_ref(), p.stroke.as_ref(), depth);
                format!(
                    "<path{}{}{style}/>",
                    attr("d", &d),
                    attr("fill-rule", fill_rule(p.fill_rule))
                )
            }
            Primitive::Rect(p) => {
                let style = self.style(p.fill.as_ref(), p.stroke.as_ref(), depth);
                format!(
                    "<rect{}{}{}{style}/>",
                    box_attrs(p.r#box),
                    float("rx", p.radius_x),
                    float("ry", p.radius_y)
                )
            }
            Primitive::Ellipse(p) => {
                let style = self.style(p.fill.as_ref(), p.stroke.as_ref(), depth);
                format!(
                    "<ellipse{}{}{}{}{style}/>",
                    float("cx", p.center.x),
                    float("cy", p.center.y),
                    float("rx", p.radius_x),
                    float("ry", p.radius_y)
                )
            }
            Primitive::TextRun(p) => self.text(p, depth),
            Primitive::Image(p) => match self.doc.assets.get(&p.asset).cloned() {
                None => {
                    self.fail(format!("native SVG missing image asset {:?}", p.asset));
                    String::new()
                }
                Some(Asset::VectorAsset { view_box, root }) => {
                    let m = match aspect_ratio_matrix(view_box, p.r#box, p.aspect) {
                        Ok(m) => m,
                        Err(e) => {
                            self.fail(e);
                            return String::new();
                        }
                    };
                    let id = self.id("image-clip");
                    self.def(format!(
                        "<clipPath id=\"{id}\"><rect{}/></clipPath>",
                        box_attrs(p.r#box)
                    ));
                    let previous = self.transform;
                    self.transform = previous.mul(m);
                    let body = self.node(root.as_deref(), depth + 1);
                    self.transform = previous;
                    format!(
                        "<g clip-path=\"url(#{id})\"><g transform=\"{}\">{body}</g></g>",
                        matrix(m)
                    )
                }
                Some(Asset::RasterAsset {
                    mime_type, data, ..
                }) => {
                    if data.len() > 32 << 20 {
                        self.fail("native SVG raster asset exceeds byte budget");
                        return String::new();
                    }
                    let encoded = match super::backends::normalized_raster_png(&data, &mime_type) {
                        Ok(b) => b,
                        Err(e) => {
                            self.fail(e);
                            return String::new();
                        }
                    };
                    let source = "data:image/png;base64,".to_owned()
                        + &base64::engine::general_purpose::STANDARD.encode(encoded);
                    let aligns = [
                        "none", "xMinYMin", "xMidYMin", "xMaxYMin", "xMinYMid", "xMidYMid",
                        "xMaxYMid", "xMinYMax", "xMidYMax", "xMaxYMax",
                    ];
                    let align = aligns
                        .get(p.aspect.align.0 as usize)
                        .copied()
                        .unwrap_or("none");
                    let aspect = if align == "none" {
                        "none".into()
                    } else {
                        format!(
                            "{align} {}",
                            if p.aspect.fit == AspectFit::SLICE {
                                "slice"
                            } else {
                                "meet"
                            }
                        )
                    };
                    format!(
                        "<image{}{}{}/>",
                        box_attrs(p.r#box),
                        attr("preserveAspectRatio", &aspect),
                        attr("href", &source)
                    )
                }
                Some(Asset::FontAsset { .. }) => {
                    self.fail("native SVG unsupported image asset d2scene.FontAsset");
                    String::new()
                }
            },
        }
    }
    pub fn surface_bounds(&mut self) -> Box {
        let inv = match self.transform.inverse() {
            Ok(m) => m,
            Err(e) => {
                self.fail(e);
                return Box::default();
            }
        };
        let b = Box {
            width: self.doc.logical_width,
            height: self.doc.logical_height,
            ..Default::default()
        }
        .bounds()
        .transform(inv);
        Box {
            x: b.min.x,
            y: b.min.y,
            width: b.max.x - b.min.x,
            height: b.max.y - b.min.y,
        }
    }
}

fn initial_node(n: &Node) -> Result<Node, String> {
    let mut n = n.clone();
    for t in n.animations.as_deref().unwrap_or_default() {
        let v = t
            .keyframes
            .as_deref()
            .and_then(|ks| ks.first())
            .ok_or("native SVG animation has no keyframes")?
            .value;
        match t.property {
            AnimationProperty::OPACITY => n.opacity = v.number,
            AnimationProperty::TRANSFORM => n.transform = v.transform,
            AnimationProperty::DROP_SHADOW => {
                let f = n
                    .filters
                    .as_mut()
                    .and_then(|fs| {
                        usize::try_from(t.target_index)
                            .ok()
                            .and_then(|i| fs.get_mut(i))
                    })
                    .ok_or("native SVG shadow animation target is invalid")?;
                *f = Filter::DropShadow(v.shadow);
            }
            prop => {
                let (fill, stroke) = match n.primitive.as_mut() {
                    Some(Primitive::Path(p)) => (&mut p.fill, &mut p.stroke),
                    Some(Primitive::Rect(p)) => (&mut p.fill, &mut p.stroke),
                    Some(Primitive::Ellipse(p)) => (&mut p.fill, &mut p.stroke),
                    Some(Primitive::TextRun(p)) => (&mut p.fill, &mut p.stroke),
                    _ => return Err("native SVG animated paint has no painted primitive".into()),
                };
                if prop == AnimationProperty::FILL_COLOR {
                    *fill = Some(Paint::solid(v.color));
                    continue;
                }
                let stroke = stroke
                    .as_mut()
                    .ok_or("native SVG animated stroke is missing")?;
                match prop {
                    AnimationProperty::STROKE_COLOR => stroke.paint = Some(Paint::solid(v.color)),
                    AnimationProperty::STROKE_DASH_OFFSET => stroke.dash_offset = v.number,
                    _ => {
                        return Err(format!(
                            "native SVG unsupported animation property {}",
                            prop.0
                        ));
                    }
                }
            }
        }
    }
    Ok(n)
}

pub(crate) fn format_g(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_owned();
    }
    if v.is_infinite() {
        return if v > 0.0 { "+Inf" } else { "-Inf" }.to_owned();
    }
    // Rust's `{:e}` yields the shortest round-tripping digits, as strconv's
    // shortest mode does.
    let shortest = format!("{v:e}");
    let digit_count = shortest
        .split_once('e')
        .unwrap()
        .0
        .bytes()
        .filter(u8::is_ascii_digit)
        .count();
    let exact = format!("{v:.*e}", digit_count.saturating_sub(1));
    let sci = if exact.parse::<f64>().ok() == Some(v) {
        exact
    } else {
        shortest
    };
    let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("{:e} exponent is an integer");
    let (neg, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if digits == "0" {
        out.push('0');
        return out;
    }
    if !(-4..6).contains(&exp) {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        let _ = write!(out, "{:02}", exp.abs());
    } else if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            out.push_str(&digits);
            out.extend(std::iter::repeat_n('0', int_len - digits.len()));
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    } else {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
        out.push_str(&digits);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface(root: Node) -> VectorSurface {
        VectorSurface {
            document: std::rc::Rc::new(std::cell::RefCell::new(Document::new(
                Box {
                    width: 10.0,
                    height: 20.0,
                    ..Default::default()
                },
                root,
            ))),
            ..Default::default()
        }
    }
    #[test]
    fn shortest_numbers_use_go_ties_and_exponents() {
        assert_eq!(number(65.791_778_564_453_13), "65.79177856445312");
        assert_eq!(number(1e6), "1e+06");
        assert_eq!(number(1e-5), "1e-05");
        assert_eq!(number(-0.0), "0");
    }
    #[test]
    fn invalid_resources_and_viewports_fail() {
        let surface = surface(Node::new(None));
        assert_eq!(
            native_surface_svg(&surface, "bad prefix").unwrap_err(),
            "native SVG has an invalid resource prefix"
        );
        assert_eq!(
            native_surface_svg(&VectorSurface::default(), "good").unwrap_err(),
            "native SVG surface has invalid dimensions"
        );
    }
    #[test]
    fn animation_uses_first_keyframe_without_mutating_source() {
        let mut root = Node::new(Some(Primitive::Rect(Rect {
            r#box: Box {
                width: 10.0,
                height: 20.0,
                ..Default::default()
            },
            fill: Some(Paint::solid(Nrgba::new(0, 0, 0, 255))),
            ..Default::default()
        })));
        root.animations = Some(vec![Track {
            property: AnimationProperty::FILL_COLOR,
            keyframes: Some(vec![Keyframe {
                value: AnimationValue {
                    color: Nrgba::new(255, 0, 0, 255),
                    ..Default::default()
                },
                ..Default::default()
            }]),
            ..Default::default()
        }]);
        let surface = surface(root);
        let before = surface.document.borrow().clone();
        let svg = native_surface_svg(&surface, "s").unwrap();
        assert!(svg.contains("fill=\"#ff0000\""));
        assert_eq!(*surface.document.borrow(), before);
    }
    #[test]
    fn excessive_depth_is_an_error() {
        let mut root = Node::new(None);
        for _ in 0..258 {
            root = Node {
                children: Some(vec![root]),
                ..Node::new(None)
            }
        }
        assert_eq!(
            native_surface_svg(&surface(root), "s").unwrap_err(),
            "native SVG surface exceeds node, depth, path or byte budget"
        );
    }
}

#[cfg(test)]
#[path = "fidelity_api_tests.rs"]
mod fidelity_api_tests;
