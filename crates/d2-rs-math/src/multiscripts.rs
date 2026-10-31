// Copyright 2018-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: pinned mathjax-go svg/mmultiscripts.go and msubsup.go.
use super::*;
// amsSideScripts unwraps only the inferred argument row and takes side
// scripts from msub/msup/msubsup; unrelated argument content is ignored.
pub(super) fn side_scripts(mut atom: Atom) -> [Box<Atom>; 2] {
    if let Atom::Sequence(v) = &mut atom {
        if v.len() == 1 {
            atom = v.pop().unwrap();
        }
    }
    let empty = || Box::new(Atom::EmptyNode("none"));
    let script = |value: Option<Box<Atom>>| {
        let body = value.unwrap_or_else(empty);
        // parseScriptArgument gives a braced script an ORD TeXAtom frame.
        if matches!(body.as_ref(), Atom::Sequence(_)) {
            Box::new(Atom::Ord(body))
        } else {
            body
        }
    };
    if let Atom::Scripts {
        limits: false,
        sub,
        sup,
        ..
    } = atom
    {
        [script(sub), script(sup)]
    } else {
        [empty(), empty()]
    }
}

fn own_scale(atom: &Atom, scale: f64) -> f64 {
    match atom {
        Atom::Sequence(v) if v.len() == 1 => own_scale(&v[0], scale),
        Atom::DeclaredStyle { level, .. } => styles::scale(*level),
        Atom::DeclaredSize { factor, .. } => scale * factor,
        _ => scale,
    }
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        width: 0.,
        ascent: 0.,
        depth: 0.,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
fn probe(atom: &Atom, scale: f64, display: bool, prime: bool) -> Option<Layout> {
    let mut result = blank();
    result.width = layout_at_with_style(atom, 0., 0., scale, &mut result, display, prime)?;
    retain_negative_glyph_depth(&mut result);
    Some(result)
}
fn place_child(mut child: Layout, x: f64, scale: f64, out: &mut Layout) {
    // A one-child argument is the child itself. An explicit style wrapper keeps
    // its actual scale rather than an extra script-scale frame.
    if child.glyphs.is_empty()
        && child.rules.is_empty()
        && child.groups.len() == 1
        && child.groups[0].kind == "mstyle"
        && child.groups[0].x == 0.
        && child.groups[0].baseline == 0.
    {
        let mut g = child.groups.pop().unwrap();
        g.x = x;
        g.before_glyph = out.glyphs.len();
        out.groups.push(g);
    } else {
        classes::place(child, x, 0., scale, out);
    }
}
fn group(child: Layout, x: f64, y: f64, scale: f64, kind: &'static str, out: &mut Layout) {
    out.groups.push(LayoutGroup {
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
        before_glyph: out.glyphs.len(),
        scale,
        x,
        baseline: y,
        color: None,
        background: None,
        kind,
        delimiter: None,
        arrow: None,
        body: Box::new(child),
        lines: Vec::new(),
    });
}
pub(super) fn layout(
    base: &Atom,
    sub: &Atom,
    sup: &Atom,
    post: Option<&[Box<Atom>; 2]>,
    mathsize: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let base_box = probe(base, scale, display, prime)?;
    let child_scale = (scale * std::f64::consts::FRAC_1_SQRT_2).max(0.5);
    let sub_box = probe(sub, child_scale, false, true)?;
    let sup_box = probe(sup, child_scale, false, prime)?;
    let post_boxes = if let Some(pair) = post {
        Some((
            probe(&pair[0], child_scale, false, true)?,
            probe(&pair[1], child_scale, false, prime)?,
        ))
    } else {
        None
    };
    let (core, core_scale, core_display, core_prime) =
        styles::script_core(base, scale, display, prime);
    let core_box = probe(core, core_scale, core_display, core_prime)?;
    let character = core_scale == scale
        && match core {
            Atom::Glyph(c)
            | Atom::LiteralGlyph(c)
            | Atom::VariantGlyph(c)
            | Atom::CalligraphicGlyph(c)
            | Atom::ItalicGlyph(c)
            | Atom::Unit(c) => !is_large_operator(*c),
            Atom::FontGlyph { paint, .. } => !is_large_operator(*paint),
            Atom::RawOperator { definition, .. } => !definition.flag("largeop"),
            Atom::Identifier(v) | Atom::Number(v) => v.len() == 1,
            _ => false,
        };
    let unit = 1000. * scale;
    let (core_h, core_d) = styles::em_bounds(&core_box);
    let core_h = core_h / scale;
    let core_d = core_d / scale;
    // The inherited primary script is mprescripts, whose box is zero and
    // whose default scriptlevel remains zero (inheritMultiscripts skips it).
    let zero = |value| if character { 0. } else { value };
    let primary_rscale = post.map_or(1.0 / (scale * mathsize), |pair| {
        own_scale(&pair[0], child_scale) / scale
    });
    let (primary_h, primary_d) = post_boxes
        .as_ref()
        .map_or((0., 0.), |(sub, _)| (sub.ascent / unit, sub.depth / unit));
    let get_u = zero(core_h - 0.386 * primary_rscale)
        .max(if prime {
            0.289
        } else if display {
            0.413
        } else {
            0.363
        })
        .max(primary_d + 0.1105);
    let get_v = zero(core_d + 0.05 * primary_rscale)
        .max(0.15)
        .max(primary_h - 0.3536);
    let sh = post_boxes
        .as_ref()
        .map_or(sub_box.ascent, |p| sub_box.ascent.max(p.0.ascent))
        / unit;
    let sd = post_boxes
        .as_ref()
        .map_or(sub_box.depth, |p| sub_box.depth.max(p.0.depth))
        / unit;
    let uh = post_boxes
        .as_ref()
        .map_or(sup_box.ascent, |p| sup_box.ascent.max(p.1.ascent))
        / unit;
    let ud = post_boxes
        .as_ref()
        .map_or(sup_box.depth, |p| sup_box.depth.max(p.1.depth))
        / unit;
    let (mut u, mut v) = if sh == 0. && sd == 0. {
        (get_u, 0.)
    } else if uh == 0. && ud == 0. {
        (-get_v, 0.)
    } else {
        let mut u = get_u;
        let mut down = zero(core_d + 0.05).max(0.247);
        let q = (u - ud) - (sh - down);
        if q < 0.18 {
            down += 0.18 - q;
            let p = 0.3536 - (u - ud);
            if p > 0. {
                u += p;
                down -= p;
            }
        }
        (u, -down)
    };
    u *= unit;
    v *= unit;
    let sub_width = sub_box.width;
    let sup_width = sup_box.width;
    let column = sub_width.max(sup_width);
    let mut local = blank();
    let mut sup_row = blank();
    place_child(
        sup_box,
        if post.is_some() {
            0.
        } else {
            column - sup_width
        },
        child_scale,
        &mut sup_row,
    );
    let mut sub_row = blank();
    place_child(
        sub_box,
        if post.is_some() {
            0.
        } else {
            column - sub_width
        },
        child_scale,
        &mut sub_row,
    );
    group(sup_row, 0.05 * unit, u, scale, "", &mut local);
    group(sub_row, 0.05 * unit, v, scale, "", &mut local);
    let offset = (0.05 + column / unit) * unit;
    let base_end_em = 0.05 + column / unit + base_box.width / unit;
    let mut width_em = base_end_em;
    let ascent = base_box.ascent.max(uh * unit + u).max(sh * unit + v);
    let depth = base_box.depth.max(ud * unit - u).max(sd * unit - v);
    classes::place(base_box, offset, 0., scale, &mut local);
    if let Some((sub, sup)) = post_boxes {
        width_em += sub.width.max(sup.width) / unit;
        width_em += 0.05;
        let mut sup_row = blank();
        place_child(sup, 0., child_scale, &mut sup_row);
        let mut sub_row = blank();
        place_child(sub, 0., child_scale, &mut sub_row);
        group(sup_row, base_end_em * unit, u, scale, "", &mut local);
        group(sub_row, base_end_em * unit, v, scale, "", &mut local);
    }
    local.width_em = Some(width_em);
    local.width = width_em * unit;
    local.ascent = ascent;
    local.depth = depth;
    group(local, x, baseline, scale, "mmultiscripts", out);
    out.ascent = out.ascent.max(baseline + ascent);
    out.depth = out.depth.max(depth - baseline);
    Some(width_em * unit)
}
