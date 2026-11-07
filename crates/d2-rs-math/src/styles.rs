// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go v0.1.0 tex/parser.go,commands.go,inheritance.go.
use super::*;
pub(super) fn declaration(name: &str) -> Option<(bool, usize)> {
    Some(match name {
        "displaystyle" => (true, 0),
        "textstyle" => (false, 0),
        "scriptstyle" => (false, 1),
        "scriptscriptstyle" => (false, 2),
        _ => return None,
    })
}
// Pinned mml/definitions.go and svg/wrapper.go:getScale use this exact
// decimal, one ULP below FRAC_1_SQRT_2. Keep child-script transitions
// consistent across fractions, scripts, limits, arrows and CD labels.
pub(super) const SCRIPT_MULTIPLIER: f64 = 0.7071067811865475;
pub(super) fn scale(level: usize) -> f64 {
    (std::f64::consts::FRAC_1_SQRT_2.powi(level as i32)).max(0.5)
}
pub(super) fn resolve(atom: &mut Atom, display: bool, level: usize, prime: bool) {
    match atom {
        Atom::NonscriptSpace { body, remove } => {
            *remove = level > 0;
            resolve(body, display, level, prime);
        }
        Atom::Choice(branches) => {
            let selection = if display { 0 } else { (level + 1).min(3) };
            let mut selected = (*branches[selection]).clone();
            resolve(&mut selected, display, level, prime);
            *atom = selected;
        }
        Atom::DeclaredStyle {
            body,
            display,
            level,
        } => resolve(body, *display, *level, false),
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => {
            for child in v {
                resolve(child, display, level, prime)
            }
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            resolve(base, display, level, prime);
            resolve(sub, false, level + 1, true);
            resolve(sup, false, level + 1, prime);
            if let Some([sub, sup]) = post {
                resolve(sub, false, level + 1, true);
                resolve(sup, false, level + 1, prime);
            }
        }
        Atom::Scripts {
            base,
            sub,
            sup,
            limits,
            accent,
            ..
        } => {
            resolve(base, display, level, prime || (*limits && sup.is_some()));
            let force_script = *limits && !display && accents::force_script(base);
            for (a, under) in [(sub, true), (sup, false)] {
                if let Some(a) = a {
                    let explicit_accent =
                        *limits && accent.is_some_and(|spec| spec.enabled && spec.under == under);
                    resolve(
                        a,
                        false,
                        level + usize::from(force_script || !explicit_accent),
                        prime || under,
                    );
                    // inheritUnderOver first inherits the script, then derives
                    // an unset accent from its refreshed core and repeats with
                    // the parent's display/prime frame. MathChoice has already
                    // selected and removed its transient branch by this point.
                    if *limits && accent.is_none() && accents::inherited_script_accent(a) {
                        resolve(a, display, level + usize::from(force_script), prime);
                    }
                }
            }
        }
        Atom::Fraction(a, b) | Atom::SplitFraction(a, b, _) | Atom::GeneralFraction(a, b, _) => {
            let child_level = level + usize::from(!display || level > 0);
            resolve(a, false, child_level, prime);
            resolve(b, false, child_level, true);
        }
        Atom::Sqrt(a) | Atom::SizedSqrt { body: a, .. } => resolve(a, display, level, true),
        // MmlMroot.setChildInheritedAttributes: the index is two script levels
        // deeper, never display style, and keeps the inherited prime style.
        Atom::Root { body, index, .. } => {
            resolve(body, display, level, true);
            resolve(index, false, level + 2, prime);
        }
        Atom::DeclaredSize { body: a, .. }
        | Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Padded { body: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::Color { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Cancel { body: a, .. } => resolve(a, display, level, prime),
        Atom::Matrix {
            rows, environment, ..
        }
        | Atom::TaggedTable {
            rows, environment, ..
        } => {
            for a in rows.iter_mut().flatten() {
                resolve(
                    a,
                    table::display(environment),
                    if environment.contains("smallmatrix") || environment.contains("subarray") {
                        1
                    } else {
                        level
                    },
                    environment == "crampedsubarray",
                )
            }
        }
        Atom::XArrow { over, under, .. } => {
            resolve(over, false, level + 1, prime);
            if let Some(a) = under {
                resolve(a, false, level + 1, true);
            }
        }
        Atom::CdArrow { over, under, .. } => {
            if let Some(a) = over {
                resolve(a, false, level + 1, prime)
            }
            if let Some(a) = under {
                resolve(a, false, level + 1, true)
            }
        }
        _ => {}
    }
}
pub(super) fn layout_decl(
    body: &Atom,
    display: bool,
    level: usize,
    x: f64,
    baseline: f64,
    out: &mut Layout,
) -> Option<f64> {
    let scale = scale(level);
    let mut local = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    };
    local.width = layout_at_with_style(body, 0.0, 0.0, scale, &mut local, display, false)?;
    retain_negative_glyph_depth(&mut local);
    // BBox.Append combines the intrinsic advance with this wrapper's
    // getScale result before the final conversion to SVG units.
    let own_scale = SCRIPT_MULTIPLIER.powi(level.min(2) as i32);
    let width_em = (local.width_em.unwrap_or(local.width / 1000.) / scale) * own_scale;
    let width = width_em * 1000.;
    local.width = width;
    local.width_em = Some(width_em);
    out.width_em = Some(width_em);
    // Keep intrinsic em bounds independent of the font-unit paint frame.
    // BBox.Append applies the declaration scale before SVG unit conversion.
    if scale != own_scale {
        local.bounds_em = Some(styles::em_bounds(&local));
        local.ascent = (local.ascent / scale / 1000.) * own_scale * 1000.;
        local.depth = (local.depth / scale / 1000.) * own_scale * 1000.;
    }
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
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
        kind: "mstyle",
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: Vec::new(),
    });
    Some(width)
}

// svg/msubsup.go scriptBaseInfo records the first under/over flags even
// when that stack is not a character math accent and traversal stops there.
pub(super) fn base_accent(mut atom: &Atom, under: bool) -> bool {
    loop {
        match atom {
            Atom::Scripts {
                limits: true,
                accent,
                sub,
                sup,
                ..
            } => {
                return accents::script_accent(
                    if under {
                        sub.as_deref()
                    } else {
                        sup.as_deref()
                    },
                    *accent,
                    under,
                );
            }
            Atom::Class { class: 8, .. } => return false,
            Atom::Sequence(v) | Atom::Identifier(v) if v.len() == 1 => atom = &v[0],
            Atom::Ord(a)
            | Atom::Class { body: a, .. }
            | Atom::Operator(a)
            | Atom::AutoOperator(a)
            | Atom::MathMLSpace { body: a, .. }
            | Atom::NamedFunction(a)
            | Atom::MovesScripts(a)
            | Atom::ScriptPolicy { body: a, .. }
            | Atom::Phantom(a)
            | Atom::CrampedStyle(a, _)
            | Atom::DeclaredStyle { body: a, .. }
            | Atom::DeclaredSize { body: a, .. }
            | Atom::Padded { body: a, .. } => atom = a,
            _ => return false,
        }
    }
}

pub(super) fn script_core(
    mut atom: &Atom,
    mut scale: f64,
    mut display: bool,
    mut prime: bool,
) -> (&Atom, f64, bool, bool) {
    loop {
        match atom {
            Atom::Class { class: 8, .. } => return (atom, scale, display, prime),
            Atom::Scripts {
                base,
                sub,
                sup,
                accent: Some(_),
                ..
            } if [sub, sup].into_iter().flatten().any(|a| {
                matches!(
                    a.as_ref(),
                    Atom::HorizontalMark {
                        mathaccent: true,
                        ..
                    }
                )
            }) =>
            {
                // svg/msubsup.go initializeMathAccent accepts one-character
                // mi/mn and unstretched mo, including large operators. Its
                // later baseIsChar check separately excludes largeop when
                // computing script drops; do not use that restriction here.
                let info = script_core(base, scale, display, prime);
                let character = matches!(
                    info.0,
                    Atom::Glyph(_)
                        | Atom::LiteralGlyph(_)
                        | Atom::FontGlyph { .. }
                        | Atom::ItalicGlyph(_)
                        | Atom::CalligraphicGlyph(_)
                        | Atom::RawOperator { .. }
                );
                if !character || info.1 != scale {
                    return (atom, scale, display, prime);
                }
                (atom, scale, display, prime) = info;
            }
            Atom::Sequence(v) | Atom::Identifier(v) if v.len() == 1 => atom = &v[0],
            Atom::DeclaredStyle {
                body,
                display: own_display,
                level,
            } => {
                scale = self::scale(*level);
                display = *own_display;
                prime = false;
                atom = body;
            }
            Atom::DeclaredSize { body, factor } => {
                scale *= factor;
                atom = body;
            }
            Atom::BracketMark { body: a, .. }
            | Atom::HorizontalMark { body: a, .. }
            | Atom::NamedFunction(a)
            | Atom::Boxed(a)
            | Atom::Mrow(a)
            | Atom::Ord(a)
            | Atom::Class { body: a, .. }
            | Atom::Operator(a)
            | Atom::AutoOperator(a)
            | Atom::MathMLSpace { body: a, .. }
            | Atom::MovesScripts(a)
            | Atom::ScriptPolicy { body: a, .. }
            | Atom::Phantom(a)
            | Atom::CrampedStyle(a, _)
            | Atom::Padded { body: a, .. } => atom = a,
            _ => return (atom, scale, display, prime),
        }
    }
}

// Original parser.go SetSize scopes a mathsize declaration over the remainder
// of its row. Nested declarations replace inherited mathsize (not multiply it).
pub(super) fn size_declaration(name: &str) -> Option<f64> {
    Some(match name {
        "tiny" => 0.5,
        "Tiny" => 0.6,
        "scriptsize" => 0.7,
        "small" => 0.85,
        "normalsize" => 1.0,
        "large" => 1.2,
        "Large" => 1.44,
        "LARGE" => 1.73,
        "huge" => 2.07,
        "Huge" => 2.49,
        _ => return None,
    })
}
pub(super) fn resolve_sizes(atom: &mut Atom, inherited: f64) {
    match atom {
        Atom::NonscriptSpace { body, .. } => resolve_sizes(body, inherited),
        Atom::BracketMark { body, mathsize, .. } => {
            *mathsize = inherited;
            resolve_sizes(body, inherited);
        }
        Atom::Sqrt(body) => {
            resolve_sizes(body, inherited);
            if inherited != 1.0 {
                *atom = Atom::SizedSqrt {
                    body: body.clone(),
                    mathsize: inherited,
                };
            }
        }
        Atom::DeclaredSize { body, factor } => {
            let absolute = *factor;
            *factor = absolute / inherited;
            resolve_sizes(body, absolute);
        }
        Atom::Sequence(v)
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v)
        | Atom::Fenced { body: v, .. } => {
            for a in v {
                resolve_sizes(a, inherited)
            }
        }
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            for a in rows.iter_mut().flatten() {
                resolve_sizes(a, inherited)
            }
        }
        Atom::Choice(v) => {
            for a in v {
                resolve_sizes(a, inherited)
            }
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            mathsize,
            ..
        } => {
            *mathsize = inherited;
            for a in [base, sub, sup] {
                resolve_sizes(a, inherited);
            }
            if let Some(v) = post {
                for a in v {
                    resolve_sizes(a, inherited);
                }
            }
        }
        Atom::Scripts {
            base,
            sub,
            sup,
            accent,
            ..
        } => {
            if let Some(s) = accent {
                s.mathsize = inherited;
            }
            resolve_sizes(base, inherited);
            for a in [sub, sup].into_iter().flatten() {
                resolve_sizes(a, inherited)
            }
        }
        Atom::Fraction(a, b) => {
            resolve_sizes(a, inherited);
            resolve_sizes(b, inherited)
        }
        Atom::Root {
            body,
            index,
            mathsize,
        } => {
            *mathsize = inherited;
            resolve_sizes(body, inherited);
            resolve_sizes(index, inherited)
        }
        Atom::FractionStrut(mathsize) => *mathsize = inherited,
        Atom::RuleSpace { mathsize, .. } => *mathsize = inherited,
        Atom::ColorBox { body, mathsize, .. } => {
            *mathsize = inherited;
            resolve_sizes(body, inherited);
        }
        Atom::Padded { body, padding } => {
            // wrapper.getScale combines inherited mathsize with scriptlevel
            // before mpadded converts any absolute dimensions to local em.
            padding.mathsize = Some(inherited);
            if let Some((_, _, mathsize)) = &mut padding.aligned_width {
                *mathsize = inherited;
            }
            resolve_sizes(body, inherited);
        }
        Atom::GeneralFraction(a, b, spec) => {
            spec.mathsize = inherited;
            resolve_sizes(a, inherited);
            resolve_sizes(b, inherited)
        }
        Atom::SplitFraction(a, b, mathsize) => {
            *mathsize = inherited;
            resolve_sizes(a, inherited);
            resolve_sizes(b, inherited)
        }
        Atom::XArrow { over, under, .. } => {
            resolve_sizes(over, inherited);
            if let Some(a) = under {
                resolve_sizes(a, inherited)
            }
        }
        Atom::CdArrow { over, under, .. } => {
            for a in [over, under].into_iter().flatten() {
                resolve_sizes(a, inherited)
            }
        }
        Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Reference { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => resolve_sizes(a, inherited),
        _ => {}
    }
}
// Script-size floors and display-style decisions are evaluated in the inherited
// TeX frame. wrapper.getScale applies mathsize independently of scriptlevel.
pub(super) fn layout_size(
    body: &Atom,
    factor: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let mut local = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    };
    local.width = layout_at_with_style(body, 0.0, 0.0, scale, &mut local, display, prime)?;
    Some(place_size(local, factor, x, baseline, scale, out))
}
pub(super) fn place_size(
    mut local: Layout,
    factor: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> f64 {
    retain_negative_glyph_depth(&mut local);
    let width_em = local.width_em.unwrap_or(local.width / 1000.0) * factor;
    let width = width_em * 1000.0;
    out.width_em = Some(width_em);
    let (h, d) = em_bounds(&local);
    let ascent = h * factor * 1000.0;
    let depth = d * factor * 1000.0;
    out.ascent = out.ascent.max(baseline + ascent);
    out.depth = out.depth.max(depth - baseline);
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
        mathsize: Some(factor),
        tex_class: 0,
        before_glyph: out.glyphs.len(),
        scale: scale * factor,
        x,
        baseline,
        color: None,
        background: None,
        kind: "mstyle",
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: Vec::new(),
    });
    width
}

pub(super) fn em_bounds(layout: &Layout) -> (f64, f64) {
    em_bounds_in_frame(layout, false)
}

// The paint plan retains declaration coordinates in its local font frame.
// The root viewport instead receives BBox.Append's literal wrapper scale.
pub(super) fn viewport_bounds(layout: &Layout) -> (f64, f64) {
    em_bounds_in_frame(layout, true)
}
fn em_bounds_in_frame(layout: &Layout, viewport: bool) -> (f64, f64) {
    if let Some(bounds) = layout.bounds_em {
        return bounds;
    }
    let mut height = layout.ascent / 1000.0;
    let mut depth = layout.depth / 1000.0;
    for group in &layout.groups {
        let (mut h, mut d, raw_h, raw_d) = if let Some((h, d, raw_h, raw_d)) =
            group.delimiter.and_then(|d| {
                group
                    .body
                    .bounds_em
                    .map(|(h, d)| (h, d, group.body.ascent, group.body.depth))
                    .or_else(|| table::fixed_fence_bounds(d))
            }) {
            (
                h * group.scale,
                d * group.scale,
                raw_h * group.scale,
                raw_d * group.scale,
            )
        } else if group.kind == "TeXAtom" && group.tex_class == 8 {
            let (h, d) = em_bounds_in_frame(&group.body, viewport);
            let delta = (h + d) / 2. + 0.25 * group.scale - h;
            (
                h + delta,
                d - delta,
                (h + delta) * 1000.,
                (d - delta) * 1000.,
            )
        } else {
            let (h, d) = em_bounds_in_frame(&group.body, viewport);
            let factor = group.mathsize.unwrap_or(1.);
            let (raw_h, raw_d) = if group.mathsize.is_some() {
                (h * factor * 1000., d * factor * 1000.)
            } else {
                (group.body.ascent, group.body.depth)
            };
            (h * factor, d * factor, raw_h, raw_d)
        };
        if viewport && group.kind == "mstyle" && group.mathsize.is_none() {
            let level = if group.scale >= 1. {
                0
            } else if group.scale >= SCRIPT_MULTIPLIER {
                1
            } else {
                2
            };
            let wrapper_scale = SCRIPT_MULTIPLIER.powi(level);
            if group.scale != wrapper_scale {
                h = (h / group.scale) * wrapper_scale;
                d = (d / group.scale) * wrapper_scale;
            }
        }
        // Retain which child set the existing logical max, replacing only its
        // unit conversion with the original BBox.Combine operation order.
        if raw_h + group.baseline == layout.ascent {
            height = h + group.baseline / 1000.0;
        }
        if raw_d - group.baseline == layout.depth {
            depth = d - group.baseline / 1000.0;
        }
    }
    (height, depth)
}

pub(super) fn group_natural_bounds(group: &LayoutGroup) -> (f64, f64) {
    if group.kind == "TeXAtom" && group.tex_class == 8 {
        let (h, d) = em_bounds(&group.body);
        let delta = (h + d) / 2. + 0.25 * group.scale - h;
        ((h + delta) * 1000., (d - delta) * 1000.)
    } else {
        (
            group.body.ascent * group.mathsize.unwrap_or(1.),
            group.body.depth * group.mathsize.unwrap_or(1.),
        )
    }
}

// Intrinsic bounds are authoritative where retained. Older layout paths still
// expose font-unit bounds; do not infer new scalar precision for those paths.
pub(super) fn has_em_bounds(layout: &Layout) -> bool {
    layout.bounds_em.is_some()
        || layout
            .groups
            .iter()
            .any(|g| g.mathsize.is_some() || has_em_bounds(&g.body))
}
