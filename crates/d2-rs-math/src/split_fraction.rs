// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go tex/mathtools_handlers_commands.go and svg/mfrac.go.
use super::*;
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.,
        ascent: 0.,
        depth: 0.,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
fn probe(atom: &Atom, scale: f64, prime: bool) -> Option<Layout> {
    let mut b = blank();
    b.width = layout_at_with_style(atom, 0., 0., scale, &mut b, false, prime)?;
    retain_negative_glyph_depth(&mut b);
    Some(b)
}
pub(super) fn layout(
    num: &Atom,
    den: &Atom,
    mathsize: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let n = probe(num, scale, prime)?;
    let d = probe(den, scale, true)?;
    let unit = 1000. * scale;
    let mut u = if display { 0.676 } else { 0.444 };
    let mut v = if display { 0.686 } else { 0.345 };
    // fractionUVQ deliberately uses the child's absolute Scale rather than
    // RScale, even though painting and BBox.Combine use relative scale.
    let q = (u - n.depth / unit * mathsize) - (d.ascent / unit * mathsize - v);
    let minimum = if display { 7. * 0.06 } else { 3. * 0.06 };
    if q < minimum {
        u += (minimum - q) / 2.;
        v += (minimum - q) / 2.;
    }
    let content = n.width.max(d.width) / unit;
    let width_em = content + 2. * 0.12;
    let ascent = (n.ascent / unit + u).max(d.ascent / unit - v);
    let depth = (n.depth / unit - u).max(d.depth / unit + v);
    let mut local = blank();
    let dx = (content - d.width / unit + 0.12) * unit;
    classes::place(n, 0.12 * unit, u * unit, scale, &mut local);
    classes::place(d, dx, -v * unit, scale, &mut local);
    local.width = width_em * unit;
    local.width_em = Some(width_em);
    local.ascent = ascent * unit;
    local.depth = depth * unit;
    out.groups.push(LayoutGroup {
        x_frame: None,
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
        baseline,
        color: None,
        background: None,
        kind: "mfrac",
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: Vec::new(),
    });
    out.ascent = out.ascent.max(baseline + ascent * unit);
    out.depth = out.depth.max(depth * unit - baseline);
    Some(width_em * unit)
}
