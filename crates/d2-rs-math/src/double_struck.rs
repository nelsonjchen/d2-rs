// Copyright 2017-2022 The MathJax Consortium; Apache-2.0.
// Source mathjax-go font/{font,smp}.go and tex/commands.go mathFont.
use super::*;
pub(super) fn bold(code: u32) -> Option<(GlyphData, f64, &'static str)> {
    bold_glyphs::BOLD
        .binary_search_by_key(&code, |r| r.0)
        .ok()
        .map(|i| {
            let r = bold_glyphs::BOLD[i];
            (r.1, r.2, r.3)
        })
}
fn smp(code: u32, bb: bool) -> Option<u32> {
    let result = match code {
        0x41..=0x5a => code - 0x41 + if bb { 0x1d538 } else { 0x1d400 },
        0x61..=0x7a => code - 0x61 + if bb { 0x1d552 } else { 0x1d41a },
        0x30..=0x39 => code - 0x30 + if bb { 0x1d7d8 } else { 0x1d7ce },
        0x391..=0x3a9 if !bb && code != 0x3a2 => code - 0x391 + 0x1d6a8,
        0x3b1..=0x3c9 if !bb => code - 0x3b1 + 0x1d6c2,
        0x2207 if !bb => 0x1d6c1,
        0x2202 if !bb => 0x1d6db,
        0x3f4 if !bb => 0x1d6b9,
        0x3d1 if !bb => 0x1d6dd,
        0x3d5 if !bb => 0x1d6df,
        0x3d6 if !bb => 0x1d6e1,
        0x3f0 if !bb => 0x1d6de,
        0x3f1 if !bb => 0x1d6e0,
        0x3f5 if !bb => 0x1d6dc,
        0x3dc..=0x3dd if !bb => code - 0x3dc + 0x1d7ca,
        _ => return None,
    };
    Some(match result {
        0x1d53a => 0x2102,
        0x1d53f => 0x210d,
        0x1d545 => 0x2115,
        0x1d547 => 0x2119,
        0x1d548 => 0x211a,
        0x1d549 => 0x211d,
        0x1d551 => 0x2124,
        _ => result,
    })
}
fn mapped(original: u32, font_variant: &'static str) -> Atom {
    if declaration_fonts::extended(font_variant) {
        return Atom::FontGlyph {
            original,
            paint: declaration_fonts::mapped(original, font_variant),
            bold: false,
            font_variant,
        };
    }
    let bb = font_variant == "double-struck";
    if let Some(paint) = smp(original, bb) {
        return Atom::FontGlyph {
            original,
            paint,
            bold: false,
            font_variant,
        };
    }
    if bold(original).is_some() {
        return Atom::FontGlyph {
            original,
            paint: original,
            bold: true,
            font_variant,
        };
    }
    Atom::FontGlyph {
        original,
        paint: smp(original, false).unwrap_or(original),
        bold: false,
        font_variant,
    }
}
pub(super) fn apply(atom: &mut Atom) {
    apply_variant(atom, "double-struck");
}
pub(super) fn apply_bold(atom: &mut Atom) {
    apply_variant(atom, "bold");
}
pub(super) fn apply_named(atom: &mut Atom, variant: &'static str) {
    apply_variant(atom, variant);
}
fn apply_variant(atom: &mut Atom, font_variant: &'static str) {
    match atom {
        Atom::Glyph(c) | Atom::CalligraphicGlyph(c) | Atom::ItalicGlyph(c) => {
            *atom = mapped(original_font_code(*c), font_variant)
        }
        Atom::LiteralGlyph(c) => *atom = mapped(*c, font_variant),
        Atom::FontGlyph { original, .. } => *atom = mapped(*original, font_variant),
        Atom::Choice(v) => {
            for a in v {
                apply_variant(a, font_variant)
            }
        }
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => {
            for a in v {
                apply_variant(a, font_variant)
            }
        }
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root { body: a, index: b, .. } => {
            apply_variant(a, font_variant);
            apply_variant(b, font_variant)
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Operator(a)
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Reference { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Padded { body: a, .. }
        | Atom::Style(a) | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => apply_variant(a, font_variant),
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            for a in rows.iter_mut().flatten() {
                apply_variant(a, font_variant)
            }
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            for a in [base, sub, sup] {
                apply_variant(a, font_variant);
            }
            if let Some(v) = post {
                for a in v {
                    apply_variant(a, font_variant);
                }
            }
        }
        Atom::Scripts { base, sub, sup, .. } => {
            apply_variant(base, font_variant);
            for a in [sub, sup].into_iter().flatten() {
                apply_variant(a, font_variant)
            }
        }
        Atom::XArrow { over, under, .. } => {
            apply_variant(over, font_variant);
            if let Some(a) = under {
                apply_variant(a, font_variant)
            }
        }
        Atom::CdArrow { over, under, .. } => {
            for a in [over, under].into_iter().flatten() {
                apply_variant(a, font_variant)
            }
        }
        _ => {}
    }
}
pub(super) fn correction(paint: u32, is_bold: bool) -> f64 {
    if is_bold {
        bold(paint).map_or(0.0, |r| r.1)
    } else {
        glyphs::ITALIC_CORRECTIONS
            .binary_search_by_key(&paint, |r| r.0)
            .ok()
            .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1)
    }
}
pub(super) fn layout(
    original: u32,
    paint: u32,
    is_bold: bool,
    font_variant: &'static str,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
) -> Option<f64> {
    if is_large_operator(original) {
        return layout_at_with_style(
            &Atom::Glyph(original),
            x,
            baseline,
            scale,
            out,
            display,
            false,
        );
    }
    let data = if declaration_fonts::extended(font_variant) {
        declaration_fonts::lookup(paint, font_variant)?
    } else if is_bold {
        bold(paint)?.0
    } else {
        glyph(paint)?
    };
    out.glyphs.push(PositionedGlyph {
        font_original: Some(original),
        font_variant,
        bold: is_bold,
        codepoint: paint,
        x,
        baseline,
        scale,
        small_operator: false,
        variant: false,
        unit: false,
        calligraphic: false,
        italic: false,
        large: false,
        stretch: false,
        radical_variant: None,
        assembly_height: None,
    });
    out.ascent = out.ascent.max(baseline + data.height * 1000.0 * scale);
    out.depth = out.depth.max(data.depth * 1000.0 * scale - baseline);
    let _ = original;
    let width = data.width
        + if declaration_fonts::extended(font_variant) {
            declaration_fonts::correction(paint, font_variant)
        } else {
            correction(paint, is_bold)
        };
    out.width_em = Some(width * scale);
    Some(width * 1000.0 * scale)
}

pub(super) fn bold_child(code: u32) -> (u32, bool) {
    if bold(code).is_some() {
        (code, true)
    } else {
        (smp(code, false).unwrap_or(code), false)
    }
}
