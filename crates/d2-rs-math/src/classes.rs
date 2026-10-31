// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go v0.1.0 svg/texclass.go,msubsup.go,munderover.go.
use super::*;
pub(super) fn demote_group(groups: &mut [LayoutGroup]) {
    for group in groups {
        if group.kind == "TeXAtom" {
            if group.tex_class == 2 {
                group.tex_class = 0;
            }
            break;
        }
        if matches!(
            group.kind,
            "mstyle" | "mpadded" | "mphantom" | "msub" | "msup" | "msubsup"
        ) {
            demote_group(&mut group.body.groups);
            break;
        }
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
fn probe(atom: &Atom, scale: f64, display: bool, prime: bool) -> Option<Layout> {
    let mut bounds = blank();
    bounds.width = layout_at_with_style(atom, 0.0, 0.0, scale, &mut bounds, display, prime)?;
    retain_negative_glyph_depth(&mut bounds);
    Some(bounds)
}
fn core(mut atom: &Atom) -> &Atom {
    loop {
        match atom {
            Atom::Sequence(v) | Atom::Identifier(v) if v.len() == 1 => atom = &v[0],
            Atom::Ord(a)
            | Atom::Operator(a)
            | Atom::MovesScripts(a)
            | Atom::ScriptPolicy { body: a, .. }
            | Atom::Class { body: a, .. }
            | Atom::Phantom(a)
            | Atom::Style(a) | Atom::CrampedStyle(a, _)
            | Atom::Padded { body: a, .. } => atom = a,
            _ => return atom,
        }
    }
}
pub(super) fn movable_limits(base: &Atom, display: bool) -> bool {
    if display {
        return false;
    }
    let mut node = base;
    loop {
        match node {
            Atom::MovesScripts(_) => return true,
            Atom::Class { class: 8, .. } | Atom::Boxed(_) => return false,
            Atom::ScriptPolicy {
                movable: Some(value),
                ..
            } => return *value,
            Atom::Sequence(v) | Atom::Identifier(v) if v.len() == 1 => node = &v[0],
            Atom::ScriptPolicy { body, .. }
            | Atom::Ord(body)
            | Atom::Class { body, .. }
            | Atom::Operator(body)
            | Atom::NamedFunction(body)
            | Atom::Phantom(body)
            | Atom::Style(body) | Atom::CrampedStyle(body, _)
            | Atom::Padded { body, .. }
            | Atom::DeclaredStyle { body, .. }
            | Atom::DeclaredSize { body, .. } => node = body,
            Atom::RawOperator { definition, .. } => return definition.flag("movablelimits"),
            Atom::Glyph(0x220f | 0x2211) => return true,
            _ => return false,
        }
    }
}
pub(super) fn italic_correction(atom: &Atom, display: bool) -> f64 {
    let (code, table) = match core(atom) {
        Atom::RawOperator {
            body, definition, ..
        } => return operator_dictionary::body_correction(body, *definition, display),
        Atom::FontGlyph {
            paint,
            font_variant,
            ..
        } if declaration_fonts::extended(font_variant) && !is_large_operator(*paint) => {
            return declaration_fonts::correction(*paint, font_variant);
        }
        Atom::FontGlyph { paint, bold, .. } => return font_correction(*paint, *bold, display),
        Atom::VariantGlyph(c) => (*c, tex_variant_glyphs::ITALIC_CORRECTIONS),
        Atom::CalligraphicGlyph(c) if !is_large_operator(*c) => return calligraphic_correction(*c),
        Atom::ItalicGlyph(c) if !is_large_operator(*c) => return super::italic_correction(*c),
        Atom::Glyph(c)
        | Atom::CalligraphicGlyph(c)
        | Atom::ItalicGlyph(c)
        | Atom::LiteralGlyph(c)
        | Atom::Unit(c) => (
            *c,
            if !display && matches!(*c, 0x220F | 0x2211 | 0x222B) {
                smallop_glyphs::ITALIC_CORRECTIONS
            } else if matches!(*c, 0x220F | 0x2211 | 0x222B) {
                large_glyphs::ITALIC_CORRECTIONS
            } else {
                glyphs::ITALIC_CORRECTIONS
            },
        ),
        _ => return 0.0,
    };
    table
        .binary_search_by_key(&code, |g| g.0)
        .ok()
        .map_or(0.0, |i| table[i].1)
}
pub(super) fn place(mut child: Layout, x: f64, y: f64, scale: f64, out: &mut Layout) {
    if child.glyphs.is_empty()
        && child.rules.is_empty()
        && child.groups.len() == 1
        && child.groups[0].x == 0.0
        && child.groups[0].baseline == 0.0
        && (child.groups[0].scale == scale || child.groups[0].kind == "mstyle")
    {
        let mut group = child.groups.pop().unwrap();
        group.x = x;
        group.baseline = y;
        group.before_glyph = out.glyphs.len();
        out.groups.push(group);
    } else if child.groups.is_empty() && child.rules.is_empty() && child.glyphs.len() == 1 {
        let mut glyph = child.glyphs.pop().unwrap();
        glyph.x += x;
        glyph.baseline += y;
        out.glyphs.push(glyph);
    } else {
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
            kind: "mrow",
            delimiter: None,
            arrow: None,
            body: Box::new(child),
            lines: Vec::new(),
        });
    }
}
// setBaseTeXClass visits the base TeXAtom before adopting its deeper mo core.
// Its left space belongs to that base BBox: Combine includes it, but stack
// offsets and paint placement deliberately use W without L.
pub(super) fn wrapped_core_class(mut base: &Atom) -> Option<usize> {
    loop {
        match base {
            Atom::Sequence(v) if v.len() == 1 => base = &v[0],
            Atom::ScriptPolicy { body, .. } | Atom::MovesScripts(body) | Atom::Operator(body) => {
                base = body
            }
            Atom::Class { body, .. } | Atom::Ord(body)
                if operator_dictionary::embellished(body) =>
            {
                return atom_tex_class(base);
            }
            _ => return None,
        }
    }
}
pub(super) fn adjustment_class(atom: &Atom, prepared: Option<usize>) -> Option<usize> {
    if let Atom::Scripts { base, .. } = atom {
        if let Some(c) = wrapped_core_class(base) {
            return Some(if c == 2 && prepared == Some(0) { 0 } else { c });
        }
    }
    prepared
}
pub(super) fn with_script_space(atom: &Atom, space: f64) -> Option<Atom> {
    match atom {
        Atom::Scripts {
            base,
            sub,
            sup,
            limits,
            accent,
            ..
        } if wrapped_core_class(base).is_some() => Some(Atom::Scripts {
            base: base.clone(),
            sub: sub.clone(),
            sup: sup.clone(),
            limits: *limits,
            accent: *accent,
            leading_space: space,
        }),
        _ => None,
    }
}

pub(super) fn layout_limits(
    base: &Atom,
    under: Option<&Atom>,
    over: Option<&Atom>,
    leading_space: f64,
    accent: Option<accents::Spec>,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let base_box = probe(base, scale, display, prime || over.is_some())?;
    let script = (scale * std::f64::consts::FRAC_1_SQRT_2).max(0.5);
    let actual_base_scale = accents::own_scale(base, scale);
    let force_script = !display && accents::force_script(base);
    let requested_em = base_box.width_em.unwrap_or(base_box.width / 1000.0);
    let under_scale = if accent.is_some_and(|s| s.under) && !force_script {
        scale
    } else {
        script
    };
    let over_scale = if accent.is_some_and(|s| !s.under) && !force_script {
        scale
    } else {
        script
    };
    let mut under_box = match under {
        Some(a) => Some(accents::probe_mark(
            a,
            requested_em,
            under_scale,
            true,
            accent.is_some_and(|s| s.under),
        )?),
        None => None,
    };
    let mut over_box = match over {
        Some(a) => Some(accents::probe_mark(
            a,
            requested_em,
            over_scale,
            prime,
            accent.is_some_and(|s| !s.under),
        )?),
        None => None,
    };
    let is_mark = |a: &Atom| matches!(a, Atom::HorizontalMark { stretchy: true, .. });
    let requested_em = requested_em
        .max(if under.is_some_and(is_mark) {
            0.
        } else {
            under_box
                .as_ref()
                .map_or(0., |b| b.width_em.unwrap_or(b.width / 1000.))
        })
        .max(if over.is_some_and(is_mark) {
            0.
        } else {
            over_box
                .as_ref()
                .map_or(0., |b| b.width_em.unwrap_or(b.width / 1000.))
        });
    if let Some(a) = under.filter(|a| is_mark(a)) {
        under_box = Some(accents::probe_mark(
            a,
            requested_em,
            under_scale,
            true,
            accent.is_some_and(|s| s.under),
        )?);
    }
    if let Some(a) = over.filter(|a| is_mark(a)) {
        over_box = Some(accents::probe_mark(
            a,
            requested_em,
            over_scale,
            prime,
            accent.is_some_and(|s| !s.under),
        )?);
    }
    let (base_core, core_scale, core_display, _) = styles::script_core(base, scale, display, prime);
    let ic = italic_correction(base_core, core_display) * 1000.0 * core_scale;
    let large = matches!(base_core, Atom::Glyph(0x220F | 0x2211 | 0x222B))
        || matches!(base_core,Atom::FontGlyph{original,..} if is_large_operator(*original))
        || matches!(base_core,Atom::RawOperator{definition,..} if definition.flag("largeop"));
    let delta = 0.75 * ic;
    let over_delta = delta
        + if accent.is_some_and(|s| !s.under) {
            accents::skew(base_core) * 1000.0 * core_scale
        } else {
            0.
        };
    let unit = 1000.0 * scale;
    let em = |b: &Layout| b.width_em.unwrap_or(b.width / 1000.0) / scale;
    let base_em = em(&base_box);
    let mut widths = vec![
        base_em
            - if under.is_some() && !large {
                ic / unit
            } else {
                0.0
            },
    ];
    let mut deltas = vec![0.0];
    if let Some(bounds) = &under_box {
        widths.push(em(bounds));
        deltas.push(-delta / unit);
    }
    if let Some(bounds) = &over_box {
        widths.push(em(bounds));
        deltas.push(over_delta / unit);
    }
    let maximum = widths.iter().copied().fold(0.0, f64::max);
    let mut offsets: Vec<_> = widths
        .iter()
        .zip(&deltas)
        .map(|(w, d)| (maximum - w) / 2.0 + d)
        .collect();
    // Source stackOffsets tests offset < minimum (not offset < -minimum).
    let mut minimum = 0.0;
    for offset in &offsets {
        if *offset < minimum {
            minimum = -*offset;
        }
    }
    if minimum != 0.0 {
        for offset in &mut offsets {
            *offset += minimum;
        }
    }
    let (base_h, base_d) = styles::em_bounds(&base_box);
    let (base_h, base_d) = (base_h / scale, base_d / scale);
    let (mut height_em, mut depth_em) = (base_h, base_d);
    let mut local = blank();
    let mut width_em = offsets[0] + base_em + leading_space / unit;
    local.width = width_em * unit;
    place(base_box, offsets[0] * unit, 0.0, scale, &mut local);
    let mut index = 1;
    if let Some(bounds) = under_box {
        let (h, d) = styles::em_bounds(&bounds);
        let (h, d) = (h / scale, d / scale);
        let gap = if accent.is_some_and(|s| s.under) {
            0.06 * 1.75
        } else {
            0.167_f64.max(0.6 - h)
        };
        let rscale = actual_base_scale / scale;
        let v = -((base_d / rscale).mul_add(rscale, gap) + h);
        width_em = width_em.max(offsets[index] + em(&bounds));
        height_em = height_em.max(v + h);
        depth_em = depth_em.max(d - v);
        place(
            bounds,
            offsets[index] * unit,
            v * unit,
            under_scale,
            &mut local,
        );
        index += 1;
    }
    if let Some(bounds) = over_box {
        let (h, d) = styles::em_bounds(&bounds);
        let (h, d) = (h / scale, d / scale);
        let gap = if accent.is_some_and(|s| !s.under) {
            0.06 * 1.75
        } else {
            0.111_f64.max(0.2 - d.max(0.))
        };
        let rscale = actual_base_scale / scale;
        // Go ARM64 emits FMADD for base.H*RScale+separation.
        let u = (base_h / rscale).mul_add(rscale, gap) + d;
        let bbox_u = if let Some(spec) = accent.filter(|s| !s.under) {
            (base_h / rscale)
                .max(0.442 * actual_base_scale * spec.mathsize)
                .mul_add(rscale, gap)
                + d
        } else {
            u
        };
        width_em = width_em.max(offsets[index] + em(&bounds));
        height_em = height_em.max(bbox_u + h);
        depth_em = depth_em.max(d - bbox_u);
        place(
            bounds,
            offsets[index] * unit,
            u * unit,
            over_scale,
            &mut local,
        );
    }
    if under.is_some() {
        depth_em += 0.1;
    }
    if over.is_some() {
        height_em += 0.1;
    }
    local.width = width_em * unit;
    local.bounds_em = Some((height_em * scale, depth_em * scale));
    local.ascent = height_em * unit;
    local.depth = depth_em * unit;
    local.width_em = Some(width_em * scale);
    out.width_em = Some(width_em * scale);
    let width = local.width;
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
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
        baseline,
        color: None,
        background: None,
        kind: if under.is_some() {
            if over.is_some() {
                "munderover"
            } else {
                "munder"
            }
        } else {
            "mover"
        },
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: Vec::new(),
    });
    Some(width)
}

pub(super) fn font_correction(code: u32, is_bold: bool, display: bool) -> f64 {
    if is_large_operator(code) {
        italic_correction(&Atom::Glyph(code), display)
    } else {
        double_struck::correction(code, is_bold)
    }
}
