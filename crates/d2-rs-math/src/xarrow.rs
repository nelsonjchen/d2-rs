// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go internal/tex/base_ams_handlers.go and svg/munderover.go,
// mo.go,stretch.go; font/data_delimiters_gen.go.
use super::*;
pub(super) fn parameters(name: &str) -> Option<(u32, f64, f64)> {
    Some(match name {
        "xrightarrow" => (0x2192, 5.0, 10.0),
        "xleftarrow" => (0x2190, 10.0, 5.0),
        "xleftrightarrow" => (0x2194, 10.0, 10.0),
        "xLeftarrow" => (0x21D0, 12.0, 7.0),
        "xRightarrow" => (0x21D2, 7.0, 12.0),
        "xLeftrightarrow" => (0x21D4, 12.0, 12.0),
        "xhookleftarrow" => (0x21A9, 10.0, 5.0),
        "xhookrightarrow" => (0x21AA, 5.0, 10.0),
        "xmapsto" => (0x21A6, 10.0, 10.0),
        "xrightharpoondown" => (0x21C1, 5.0, 10.0),
        "xleftharpoondown" => (0x21BD, 10.0, 5.0),
        "xrightleftharpoons" => (0x21CC, 10.0, 10.0),
        "xrightharpoonup" => (0x21C0, 5.0, 10.0),
        "xleftharpoonup" => (0x21BC, 10.0, 5.0),
        "xleftrightharpoons" => (0x21CB, 10.0, 10.0),
        _ => return None,
    })
}
pub(super) fn recipe(code: u32) -> Option<[u32; 3]> {
    Some(match code {
        0x3D => [0, 0x3D, 0],
        0xAF => [0, 0xAF, 0],
        0x20D7 => [0, 0x2212, 0x2192],
        0x5F | 0x2013 => [0, 0x2013, 0],
        0x2190 => [0x2190, 0x2212, 0],
        0x2192 => [0, 0x2212, 0x2192],
        0x2194 => [0x2190, 0x2212, 0x2192],
        0x21D0 => [0x21D0, 0x3D, 0],
        0x21D2 => [0, 0x3D, 0x21D2],
        0x21D4 => [0x21D0, 0x3D, 0x21D2],
        0x21A6 => [0x2223, 0x2212, 0x2192],
        0x21BC => [0x21BC, 0x2212, 0],
        0x21BD => [0x21BD, 0x2212, 0],
        0x21C0 => [0, 0x2212, 0x21C0],
        0x21C1 => [0, 0x2212, 0x21C1],
        _ => return None,
    })
}
fn em_length(value: f64) -> f64 {
    if value.abs() < 0.001 {
        0.0
    } else {
        (value * 1000.0).round() / 1000.0
    }
}
fn padded_label(atom: Atom, left: f64, right: f64, top: bool) -> Atom {
    let mut children = match atom {
        Atom::Sequence(v) => v,
        a => vec![a],
    };
    children.push(if top {
        Atom::Strut(0.0, 250.0, "mspace")
    } else {
        Atom::Strut(750.0, 0.0, "mspace")
    });
    Atom::Padded {
        body: Box::new(Atom::Sequence(children)),
        padding: padded::Padding {
            width_add: em_length((left + right) / 18.0),
            lspace_em: em_length(left / 18.0),
            voffset_em: if top { -0.2 } else { 0.15 },
            height_add: if top { -0.2 } else { 0.0 },
            depth_add: if top { 0.0 } else { -0.15 },
            ..Default::default()
        },
    }
}
impl Parser {
    pub(super) fn xarrow_argument(
        &mut self,
        name: &str,
        (code, left, right): (u32, f64, f64),
    ) -> Option<Atom> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        let has_below = self.chars.get(self.index) == Some(&'[');
        let below = match self.padded_brackets() {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let above = self.padded_argument(name)?;
        if cd::first_error(&above).is_some() {
            return Some(above);
        }
        let under = if has_below {
            let a = self.padded_parse_string(&below)?;
            if cd::first_error(&a).is_some() {
                return Some(a);
            }
            Some(Box::new(padded_label(a, left, right, false)))
        } else {
            None
        };
        Some(Atom::XArrow {
            code,
            over: Box::new(padded_label(above, left, right, true)),
            under,
            left_space: 0.0,
        })
    }
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
fn group(body: Layout, kind: &'static str, x: f64, baseline: f64, scale: f64) -> LayoutGroup {
    LayoutGroup {
        data_cramped: None,
        own_translate: None,
        accent_offset: None,
        box_border: None,
        reference: None,
        tagged: None,
        percentage: None,
        table_line: None,
        mathsize: None,
        tex_class: 0,
        before_glyph: 0,
        scale,
        x,
        baseline,
        color: None,
        background: None,
        kind,
        delimiter: None,
        arrow: None,
        body: Box::new(body),
        lines: Vec::new(),
    }
}
fn probe(a: &Atom, scale: f64, prime: bool) -> Option<Layout> {
    let mut b = blank();
    b.width = layout_at_with_style(a, 0.0, 0.0, scale, &mut b, false, prime)?;
    retain_negative_glyph_depth(&mut b);
    Some(b)
}
pub(super) fn layout(
    code: u32,
    over: &Atom,
    under: Option<&Atom>,
    left_space: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    prime: bool,
) -> Option<f64> {
    let script = (scale * std::f64::consts::FRAC_1_SQRT_2).max(0.5);
    let over_box = probe(over, script, prime)?;
    let under_box = under.map(|a| probe(a, script, true)).transpose_option()?;
    let requested = over_box
        .width
        .max(under_box.as_ref().map_or(0.0, |b| b.width));
    // Arrow mstyle has absolute scriptlevel0, regardless of the stack's level.
    let fixed = recipe(code).is_none() || (requested * 0.901).max(requested - 300.0) <= 1000.0;
    let data = glyph(code)?;
    let (base_width, h, d) = if fixed {
        (
            data.width * 1000.0,
            data.height * 1000.0,
            data.depth * 1000.0,
        )
    } else {
        (requested, 583.0, 82.0)
    };
    let width = base_width.max(requested);
    let mut arrow = group(blank(), "mo", 0.0, 0.0, 1.0);
    arrow.delimiter = Some(cd::Delimiter {
        font_variant: None,
        code,
        length: base_width,
        height: h,
        depth: d,
        fixed,
    });
    arrow.body.width = base_width;
    arrow.body.ascent = h;
    arrow.body.depth = d;
    let mut base = blank();
    base.width = base_width;
    base.ascent = h;
    base.depth = d;
    base.groups.push(arrow);
    let mut local = blank();
    local
        .groups
        .push(group(base, "mstyle", (width - base_width) / 2.0, 0.0, 1.0));
    local.width = (width - base_width) / 2.0 + base_width + left_space;
    local.ascent = h;
    local.depth = d;
    if let Some(mut b) = under_box {
        let v = -(d + (167.0 * scale).max(600.0 * scale - b.ascent) + b.ascent);
        local.width = local.width.max((width - b.width) / 2.0 + b.width);
        local.ascent = local.ascent.max(v + b.ascent);
        local.depth = local.depth.max(b.depth - v) + 100.0 * scale;
        let label_width = b.width;
        let mut frame = if b.glyphs.is_empty()
            && b.rules.is_empty()
            && b.groups.len() == 1
            && b.groups[0].scale == script
        {
            b.groups.pop()?
        } else {
            group(b, "mrow", 0.0, 0.0, script)
        };
        frame.x = (width - label_width) / 2.0;
        frame.baseline = v;
        local.groups.push(frame);
    }
    let u = h + (111.0 * scale).max(200.0 * scale - over_box.depth.max(0.0)) + over_box.depth;
    local.width = local
        .width
        .max((width - over_box.width) / 2.0 + over_box.width);
    local.ascent = local.ascent.max(u + over_box.ascent) + 100.0 * scale;
    local.depth = local.depth.max(over_box.depth - u);
    let mut over_box = over_box;
    let mut frame = over_box.groups.pop()?;
    frame.x = (width - over_box.width) / 2.0;
    frame.baseline = u;
    local.groups.push(frame);
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    let measured_width = local.width;
    let mut frame = group(
        local,
        if under.is_some() {
            "munderover"
        } else {
            "mover"
        },
        x,
        baseline,
        scale,
    );
    frame.before_glyph = out.glyphs.len();
    out.groups.push(frame);
    Some(measured_width)
}
trait Transpose<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> Transpose<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(Some(v)) => Some(Some(v)),
            Some(None) => None,
        }
    }
}

// texclass.go moves previous class from the core mo to mstyle before the
// script wrapper asks that core for its own previous class. The space remains
// on the base mstyle: BBox.Combine includes it, while stackOffsets uses W only.
pub(super) fn accepts_space(atom: &Atom) -> bool {
    match atom {
        Atom::XArrow { .. } => true,
        Atom::Scripts { base, .. } => accepts_space(base),
        Atom::Sequence(v) if v.len() == 1 => accepts_space(&v[0]),
        Atom::DeclaredStyle { body, .. } => accepts_space(body),
        _ => false,
    }
}
pub(super) fn with_space(atom: &Atom, space: f64) -> Option<Atom> {
    if space == 0.0 || !accepts_space(atom) {
        return None;
    }
    fn set(atom: &mut Atom, space: f64) {
        match atom {
            Atom::XArrow { left_space, .. } => *left_space = space,
            Atom::Scripts { base, .. } => set(base, space),
            Atom::Sequence(v) => set(&mut v[0], space),
            Atom::DeclaredStyle { body, .. } => set(body, space),
            _ => unreachable!(),
        }
    }
    let mut prepared = atom.clone();
    set(&mut prepared, space);
    Some(prepared)
}
