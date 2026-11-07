// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go tex/commands.go underOver; svg/mo.go,munderover.go;
// font/data_*_gen.go skew metrics and horizontal delimiter recipes.
use super::*;
#[derive(Clone, Copy, Debug)]
pub(super) struct Spec {
    pub under: bool,
    pub enabled: bool,
    pub mathsize: f64,
}

// inheritance.go inheritUnderOver/setInheritedAccent: an unset stack accent
// takes the embellished script's core operator attribute. This is distinct
// from the internal mathaccent marker used for zero-width accent glyphs.
pub(super) fn inherited_script_accent(atom: &Atom) -> bool {
    match atom {
        Atom::RawOperator { definition, .. } => definition.flag("accent"),
        Atom::Glyph(code) | Atom::FontGlyph { original: code, .. }
            if mathml_node_kind(*code) == "mo" =>
        {
            operator_dictionary::lookup(*code, operator_dictionary::Form::Infix).flag("accent")
        }
        Atom::Sequence(v) | Atom::InferredRow(v) | Atom::ExplicitRow(v) => {
            let mut cores = v.iter().filter(|a| !operator_dictionary::flag_spacelike(a));
            cores
                .next()
                .is_some_and(|a| cores.next().is_none() && inherited_script_accent(a))
        }
        Atom::Scripts { base: a, .. }
        | Atom::Multiscripts { base: a, .. }
        | Atom::Fraction(a, _)
        | Atom::GeneralFraction(a, _, _)
        | Atom::NamedFunction(a)
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a) => inherited_script_accent(a),
        _ => false,
    }
}

pub(super) fn script_accent(script: Option<&Atom>, explicit: Option<Spec>, under: bool) -> bool {
    explicit.map_or_else(
        || script.is_some_and(inherited_script_accent),
        |spec| spec.enabled && spec.under == under,
    )
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.,
        ascent: 0.,
        depth: 0.,
        glyphs: vec![],
        rules: vec![],
        groups: vec![],
    }
}
fn mark_group(body: Layout, kind: &'static str, scale: f64, x: f64, baseline: f64) -> LayoutGroup {
    LayoutGroup {
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
        lines: vec![],
    }
}
pub(super) fn probe_mark(
    atom: &Atom,
    requested: f64,
    scale: f64,
    prime: bool,
    display: bool,
) -> Option<Layout> {
    let mut out = blank();
    if let Atom::HorizontalMark {
        body,
        code,
        stretchy,
        mathaccent: true,
    } = atom
        && *code != 0x2192
    {
        let mut paint = blank();
        let mut assembled = false;
        let mut fixed_correction = 0.;
        if *stretchy {
            let requested = requested / scale;
            let target = (requested / 0.901).min(requested + 0.3);
            let (alias, variant) = if *code == 0x20d7 {
                assembled = target > 1.;
                (0x20d7, 0)
            } else {
                let sizes = [0.5, 0.556, 1., 1.444, 1.889];
                if let Some(i) = sizes.iter().position(|size| *size >= target) {
                    (
                        if *code == 0x5e { 0x2c6 } else { 0x2dc },
                        if i > 0 { i - 1 } else { 0 },
                    )
                } else {
                    // Pinned svg/mo.go retains delim.c when no fixed size
                    // reaches the target, just as it does for a selected size.
                    (if *code == 0x5e { 0x2c6 } else { 0x2dc }, 4)
                }
            };
            let data = table::font_glyph(alias, variant)?;
            let corrections = match variant {
                0 => glyphs::ITALIC_CORRECTIONS,
                1 => smallop_glyphs::ITALIC_CORRECTIONS,
                2 => large_glyphs::ITALIC_CORRECTIONS,
                3 => size3_glyphs::ITALIC_CORRECTIONS,
                _ => size4_glyphs::ITALIC_CORRECTIONS,
            };
            fixed_correction = corrections
                .binary_search_by_key(&alias, |g| g.0)
                .ok()
                .map_or(0., |i| corrections[i].1)
                * 1000.
                * scale;
            paint.width = if assembled {
                requested * 1000. * scale
            } else {
                data.width * 1000. * scale
            };
            paint.ascent = if assembled {
                583. * scale
            } else {
                data.height * 1000. * scale
            };
            paint.depth = if assembled {
                82. * scale
            } else {
                data.depth * 1000. * scale
            };
            let mut frame = mark_group(blank(), "", scale, 0., 0.);
            frame.delimiter = Some(cd::Delimiter {
                font_variant: if assembled { None } else { Some(variant) },
                code: if assembled { 0x20d7 } else { alias },
                length: requested * 1000.,
                height: 0.,
                depth: 0.,
                fixed: !assembled,
            });
            paint.groups.push(frame);
        } else {
            paint.width = layout_at_with_style(body, 0., 0., scale, &mut paint, false, prime)?;
            retain_negative_glyph_depth(&mut paint);
        }
        let width = if assembled { paint.width } else { 0. };
        let h = paint.ascent;
        let d = paint.depth;
        // Pinned moToSVG calls copySkewIC on the accent prototype before
        // centering it. Ordinary layout already includes that correction.
        let prototype_width = paint.width + fixed_correction;
        let shift = if assembled {
            None
        } else {
            Some(-prototype_width / scale / 2.)
        };
        out.width = width;
        out.width_em = Some(width / 1000.);
        out.ascent = h;
        out.depth = d;
        let mut frame = mark_group(paint, "mo", scale, 0., 0.);
        frame.accent_offset = shift;
        out.groups.push(frame);
        return Some(out);
    }

    if let Atom::HorizontalMark {
        body: _,
        code,
        stretchy: true,
        mathaccent,
    } = atom
    {
        let alias = if matches!(*code, 0x5f | 0x2015) {
            0x2013
        } else {
            *code
        };
        let brace = matches!(alias, 0x23de | 0x23df);
        let arrow = matches!(alias, 0x2190 | 0x2192);
        let dimension = requested / scale;
        let target = if *mathaccent {
            (dimension / 0.901).min(dimension + 0.3)
        } else {
            (dimension * 0.901).max(dimension - 0.3)
        };
        let fixed = !brace && target <= if arrow { 1. } else { 0.5 };
        // UnderOver sets accent explicitly: CommonMo.isAccentMO therefore
        // does not apply defaultAccentRemap. Delimiter aliases stay intact.
        let g = if brace { None } else { Some(glyph(alias)?) };
        let (w, h, d) = if fixed {
            {
                let g = g?;
                (g.width, g.height, g.depth)
            }
        } else {
            (
                if brace {
                    (requested / scale).max(1.8)
                } else {
                    requested / scale
                },
                if brace {
                    0.32
                } else if arrow {
                    0.583
                } else if alias == 0xaf {
                    0.59
                } else {
                    0.285
                },
                if brace {
                    0.2
                } else if arrow {
                    0.082
                } else if alias == 0xaf {
                    -0.544
                } else {
                    -0.248
                },
            )
        };
        out.width = w * 1000. * scale;
        out.width_em = Some(w * scale);
        out.ascent = h * 1000. * scale;
        out.depth = d * 1000. * scale;
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
            before_glyph: 0,
            scale,
            x: 0.,
            baseline: 0.,
            color: None,
            background: None,
            kind: "mo",
            delimiter: Some(cd::Delimiter {
                font_variant: None,
                code: alias,
                length: w * 1000.,
                height: h * 1000.,
                depth: d * 1000.,
                fixed,
            }),
            arrow: None,
            body: Box::new(blank()),
            lines: vec![],
        });
        if *mathaccent && fixed {
            let width = out.width;
            let body = std::mem::replace(&mut out, blank());
            out.width_em = Some(0.);
            out.ascent = body.ascent;
            out.depth = body.depth;
            let mut frame = mark_group(body, "mo", scale, 0., 0.);
            frame.accent_offset = Some(-width / scale / 2.);
            // The inner frame supplies only the fixed character, while the
            // outer mo owns CommonMo's zero-width accent positioning.
            frame.body.groups[0].kind = "";
            out.groups.push(frame);
        }
    } else {
        out.width = layout_at_with_style(atom, 0., 0., scale, &mut out, display, prime)?;
        retain_negative_glyph_depth(&mut out);
    }
    Some(out)
}
fn lookup(rows: &[(u32, f64)], code: u32) -> f64 {
    rows.binary_search_by_key(&code, |r| r.0)
        .ok()
        .map_or(0., |i| rows[i].1)
}
pub(super) fn skew(atom: &Atom) -> f64 {
    match atom {
        Atom::Glyph(c) | Atom::LiteralGlyph(c) | Atom::Unit(c) => lookup(NORMAL, *c),
        Atom::ItalicGlyph(c) => {
            if italic_glyphs::ITALIC_GLYPHS
                .binary_search_by_key(c, |g| g.codepoint)
                .is_ok()
            {
                lookup(ITALIC, *c)
            } else {
                lookup(NORMAL, italic_font_code(*c))
            }
        }
        Atom::CalligraphicGlyph(c) => {
            if calligraphic_glyphs::CALLIGRAPHIC_GLYPHS
                .binary_search_by_key(c, |g| g.codepoint)
                .is_ok()
            {
                lookup(CALLIGRAPHIC, *c)
            } else if italic_glyphs::ITALIC_GLYPHS
                .binary_search_by_key(c, |g| g.codepoint)
                .is_ok()
            {
                lookup(ITALIC, *c)
            } else {
                lookup(NORMAL, italic_font_code(*c))
            }
        }
        Atom::FontGlyph {
            paint,
            font_variant,
            ..
        } if declaration_fonts::extended(font_variant) => {
            if let Some(row) = declaration_fonts::exact(*paint, font_variant) {
                row.3
            } else if *font_variant == "bold-italic" && double_struck::bold(*paint).is_some() {
                lookup(BOLD, *paint)
            } else if matches!(*font_variant, "script" | "bold-italic")
                && italic_glyphs::ITALIC_GLYPHS
                    .binary_search_by_key(paint, |r| r.codepoint)
                    .is_ok()
            {
                lookup(ITALIC, *paint)
            } else {
                lookup(NORMAL, *paint)
            }
        }
        Atom::FontGlyph { paint, bold, .. } => lookup(if *bold { BOLD } else { NORMAL }, *paint),
        Atom::RawOperator { body, .. } => skew(body),
        Atom::Identifier(v) if v.len() == 1 => skew(&v[0]),
        _ => 0.,
    }
}
static NORMAL: &[(u32, f64)] = &[
    (0x131, 0.0278),
    (0x237, 0.0833),
    (0x210B, 0.333),
    (0x210E, -0.0278),
    (0x2110, 0.333),
    (0x2112, 0.306),
    (0x2113, 0.111),
    (0x2118, 0.111),
    (0x211B, 0.194),
    (0x212C, 0.194),
    (0x2130, 0.139),
    (0x2131, 0.222),
    (0x2133, 0.444),
    (0x2202, 0.0833),
    (0x1D434, 0.139),
    (0x1D435, 0.0833),
    (0x1D436, 0.0833),
    (0x1D437, 0.0556),
    (0x1D438, 0.0833),
    (0x1D439, 0.0833),
    (0x1D43A, 0.0833),
    (0x1D43B, 0.0556),
    (0x1D43C, 0.111),
    (0x1D43D, 0.167),
    (0x1D43E, 0.0556),
    (0x1D43F, 0.0278),
    (0x1D440, 0.0833),
    (0x1D441, 0.0833),
    (0x1D442, 0.0833),
    (0x1D443, 0.0833),
    (0x1D444, 0.0833),
    (0x1D445, 0.0833),
    (0x1D446, 0.0833),
    (0x1D447, 0.0833),
    (0x1D448, 0.0278),
    (0x1D44B, 0.0833),
    (0x1D44D, 0.0833),
    (0x1D450, 0.0556),
    (0x1D451, 0.167),
    (0x1D452, 0.0556),
    (0x1D453, 0.167),
    (0x1D454, 0.0278),
    (0x1D459, 0.0833),
    (0x1D45C, 0.0556),
    (0x1D45D, 0.0833),
    (0x1D45E, 0.0833),
    (0x1D45F, 0.0556),
    (0x1D460, 0.0556),
    (0x1D461, 0.0833),
    (0x1D462, 0.0278),
    (0x1D463, 0.0278),
    (0x1D464, 0.0833),
    (0x1D465, 0.0278),
    (0x1D466, 0.0556),
    (0x1D467, 0.0556),
    (0x1D468, 0.16),
    (0x1D469, 0.0958),
    (0x1D46A, 0.0958),
    (0x1D46B, 0.0639),
    (0x1D46C, 0.0958),
    (0x1D46D, 0.0958),
    (0x1D46E, 0.0958),
    (0x1D46F, 0.0639),
    (0x1D470, 0.128),
    (0x1D471, 0.192),
    (0x1D472, 0.0639),
    (0x1D473, 0.0319),
    (0x1D474, 0.0958),
    (0x1D475, 0.0958),
    (0x1D476, 0.0958),
    (0x1D477, 0.0958),
    (0x1D478, 0.0958),
    (0x1D479, 0.0958),
    (0x1D47A, 0.0958),
    (0x1D47B, 0.0958),
    (0x1D47C, 0.0319),
    (0x1D47F, 0.0958),
    (0x1D481, 0.0958),
    (0x1D484, 0.0639),
    (0x1D485, 0.192),
    (0x1D486, 0.0639),
    (0x1D487, 0.192),
    (0x1D488, 0.0319),
    (0x1D489, -0.0319),
    (0x1D48D, 0.0958),
    (0x1D490, 0.0639),
    (0x1D491, 0.0958),
    (0x1D492, 0.0958),
    (0x1D493, 0.0639),
    (0x1D494, 0.0639),
    (0x1D495, 0.0958),
    (0x1D496, 0.0319),
    (0x1D497, 0.0319),
    (0x1D498, 0.0958),
    (0x1D499, 0.0319),
    (0x1D49A, 0.0639),
    (0x1D49B, 0.0639),
    (0x1D49C, 0.389),
    (0x1D49E, 0.278),
    (0x1D49F, 0.111),
    (0x1D4A2, 0.25),
    (0x1D4A5, 0.417),
    (0x1D4A6, 0.361),
    (0x1D4A9, 0.389),
    (0x1D4AA, 0.167),
    (0x1D4AB, 0.222),
    (0x1D4AC, 0.278),
    (0x1D4AE, 0.333),
    (0x1D4AF, 0.222),
    (0x1D4B0, 0.25),
    (0x1D4B1, 0.222),
    (0x1D4B2, 0.25),
    (0x1D4B3, 0.278),
    (0x1D4B4, 0.194),
    (0x1D4B5, 0.306),
    (0x1D6E2, 0.139),
    (0x1D6E3, 0.0833),
    (0x1D6E4, 0.0833),
    (0x1D6E5, 0.167),
    (0x1D6E6, 0.0833),
    (0x1D6E7, 0.0833),
    (0x1D6E8, 0.0556),
    (0x1D6E9, 0.0833),
    (0x1D6EA, 0.111),
    (0x1D6EB, 0.0556),
    (0x1D6EC, 0.167),
    (0x1D6ED, 0.0833),
    (0x1D6EE, 0.0833),
    (0x1D6EF, 0.0833),
    (0x1D6F0, 0.0833),
    (0x1D6F1, 0.0556),
    (0x1D6F2, 0.0833),
    (0x1D6F4, 0.0833),
    (0x1D6F5, 0.0833),
    (0x1D6F6, 0.0556),
    (0x1D6F7, 0.0833),
    (0x1D6F8, 0.0833),
    (0x1D6F9, 0.0556),
    (0x1D6FA, 0.0833),
    (0x1D6FC, 0.0278),
    (0x1D6FD, 0.0833),
    (0x1D6FF, 0.0556),
    (0x1D700, 0.0833),
    (0x1D701, 0.0833),
    (0x1D702, 0.0556),
    (0x1D703, 0.0833),
    (0x1D704, 0.0556),
    (0x1D707, 0.0278),
    (0x1D708, 0.0278),
    (0x1D709, 0.111),
    (0x1D70A, 0.0556),
    (0x1D70C, 0.0833),
    (0x1D70D, 0.0833),
    (0x1D70F, 0.0278),
    (0x1D710, 0.0278),
    (0x1D711, 0.0833),
    (0x1D712, 0.0556),
    (0x1D713, 0.111),
    (0x1D715, 0.0833),
    (0x1D716, 0.0556),
    (0x1D717, 0.0833),
    (0x1D719, 0.0833),
    (0x1D71A, 0.0833),
    (0x1D71C, 0.16),
    (0x1D71D, 0.0958),
    (0x1D71E, 0.0958),
    (0x1D71F, 0.192),
    (0x1D720, 0.0958),
    (0x1D721, 0.0958),
    (0x1D722, 0.0639),
    (0x1D723, 0.0958),
    (0x1D724, 0.128),
    (0x1D725, 0.0639),
    (0x1D726, 0.192),
    (0x1D727, 0.0958),
    (0x1D728, 0.0958),
    (0x1D729, 0.0958),
    (0x1D72A, 0.0958),
    (0x1D72B, 0.0639),
    (0x1D72C, 0.0958),
    (0x1D72E, 0.0958),
    (0x1D72F, 0.0958),
    (0x1D730, 0.0639),
    (0x1D731, 0.0958),
    (0x1D732, 0.0958),
    (0x1D733, 0.0639),
    (0x1D734, 0.0958),
    (0x1D736, 0.0319),
    (0x1D737, 0.0958),
    (0x1D739, 0.0639),
    (0x1D73A, 0.0958),
    (0x1D73B, 0.0958),
    (0x1D73C, 0.0639),
    (0x1D73D, 0.0958),
    (0x1D73E, 0.0639),
    (0x1D741, 0.0319),
    (0x1D742, 0.0319),
    (0x1D743, 0.128),
    (0x1D744, 0.0639),
    (0x1D746, 0.0958),
    (0x1D747, 0.0958),
    (0x1D749, 0.0319),
    (0x1D74A, 0.0319),
    (0x1D74B, 0.0958),
    (0x1D74C, 0.0639),
    (0x1D74D, 0.128),
    (0x1D74F, 0.0958),
    (0x1D750, 0.0639),
    (0x1D751, 0.0958),
    (0x1D753, 0.0958),
    (0x1D754, 0.0958),
];
static ITALIC: &[(u32, f64)] = &[(0x2206, 0.167)];
static CALLIGRAPHIC: &[(u32, f64)] = &[
    (0x41, 0.194),
    (0x42, 0.139),
    (0x43, 0.139),
    (0x44, 0.0833),
    (0x45, 0.111),
    (0x46, 0.111),
    (0x47, 0.111),
    (0x48, 0.111),
    (0x49, 0.0278),
    (0x4A, 0.167),
    (0x4B, 0.0556),
    (0x4C, 0.139),
    (0x4D, 0.139),
    (0x4E, 0.0833),
    (0x4F, 0.111),
    (0x50, 0.0833),
    (0x51, 0.111),
    (0x52, 0.0833),
    (0x53, 0.139),
    (0x54, 0.0278),
    (0x55, 0.0833),
    (0x56, 0.0278),
    (0x57, 0.0833),
    (0x58, 0.139),
    (0x59, 0.0833),
    (0x5A, 0.139),
];
static BOLD: &[(u32, f64)] = &[
    (0x131, 0.0278),
    (0x237, 0.0833),
    (0x210F, -0.0319),
    (0x2113, 0.128),
];

// inheritance.go effectiveCoreBool descends first children without requiring
// embellished flags, independently of SVG's scriptBaseCore traversal.
pub(super) fn force_script(mut atom: &Atom) -> bool {
    loop {
        match atom {
            Atom::RawOperator { definition, .. } => return definition.flag("movablelimits"),
            Atom::Glyph(c) | Atom::FontGlyph { original: c, .. } => {
                return matches!(*c, 0x2211 | 0x220f);
            }
            // movableLimitCore stops at a non-embellished mrow. Its first
            // descendant's movablelimits does not belong to the whole row.
            Atom::Sequence(v) | Atom::InferredRow(v) | Atom::ExplicitRow(v) => {
                if v.len() != 1 && !operator_dictionary::embellished(atom) {
                    return false;
                }
                if let Some(a) = v.iter().find(|a| !operator_dictionary::spacelike(a)) {
                    atom = a
                } else {
                    return false;
                }
            }
            Atom::Identifier(v) | Atom::OperatorToken(v) | Atom::Number(v) => {
                if let Some(a) = v.first() {
                    atom = a
                } else {
                    return false;
                }
            }
            Atom::Scripts { base, .. } | Atom::Multiscripts { base, .. } => atom = base,
            Atom::MovesScripts(_) => return true,
            Atom::ScriptPolicy {
                movable: Some(value),
                ..
            } => return *value,
            Atom::Ord(a)
            | Atom::Class { body: a, .. }
            | Atom::Operator(a)
            | Atom::AutoOperator(a)
            | Atom::MathMLSpace { body: a, .. }
            | Atom::NamedFunction(a)
            | Atom::ScriptPolicy { body: a, .. }
            | Atom::CrampedStyle(a, _)
            | Atom::DeclaredStyle { body: a, .. }
            | Atom::DeclaredSize { body: a, .. }
            | Atom::Sqrt(a)
            | Atom::SizedSqrt { body: a, .. }
            | Atom::Padded { body: a, .. }
            | Atom::Phantom(a) => atom = a,
            Atom::Fraction(a, _)
            | Atom::Root { body: a, .. }
            | Atom::GeneralFraction(a, _, _)
            | Atom::SplitFraction(a, _, _) => atom = a,
            _ => return false,
        }
    }
}

pub(super) fn own_scale(atom: &Atom, scale: f64) -> f64 {
    match atom {
        Atom::Sequence(v) if v.len() == 1 => own_scale(&v[0], scale),
        Atom::DeclaredStyle { level, .. } => styles::scale(*level),
        Atom::DeclaredSize { factor, .. } => scale * factor,
        _ => scale,
    }
}

#[cfg(test)]
mod source_bound_contract {
    use super::*;
    #[test]
    fn empty_dotted_accents_retain_original_unrounded_small_height() {
        for expression in [r"\small\dot{}", r"\small\dddot{}", r"\small\ddddot{}"] {
            let native = layout(&parse(expression).unwrap()).unwrap();
            assert_eq!(styles::em_bounds(&native), (0.595595, 0.), "{expression}");
        }
    }
}

#[cfg(test)]
mod inheritance_contract {
    use super::*;

    #[test]
    fn inherited_accents_keep_natural_annotation_size_after_choice_selection() {
        // inheritance.go inheritUnderOver reruns an unset accent with the
        // parent's level. MathChoice is removed by the first (script) pass.
        for expression in [
            r"\stackrel{\longrightarrow}{x}",
            r"\stackrel{\mathchoice{x}{y}{\longrightarrow}{z}}{x}",
        ] {
            let svg = render(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let view_box = document.root_element().attribute("viewBox").unwrap();
            assert_eq!(
                view_box.split_whitespace().nth(2),
                Some("1665.8"),
                "{expression}"
            );
            assert!(
                document
                    .descendants()
                    .any(|n| n.attribute("data-c") == Some("27F6")),
                "{expression}"
            );
        }
    }
}
