// Copyright 2017-2022 The MathJax Consortium; Apache-2.0.
// Source: tex/parser.go fontDeclarations/applyMathVariant and font/{font,smp}.go.
use super::*;
pub(super) fn declaration(name: &str) -> bool {
    matches!(name, "rm" | "bf" | "it" | "cal" | "tt" | "sf")
}
pub(super) fn apply(atom: &mut Atom, name: &str) {
    match name {
        "rm" => apply_math_font(atom, false),
        "bf" => double_struck::apply_bold(atom),
        "it" => physics::apply_italic(atom),
        "cal" => apply_math_font(atom, true),
        "tt" => double_struck::apply_named(atom, "monospace"),
        "sf" => double_struck::apply_named(atom, "sans-serif"),
        _ => unreachable!(),
    }
}
pub(super) fn exact(code: u32, variant: &str) -> Option<(GlyphData, f64, &'static str, f64)> {
    let extra = match variant {
        "fraktur" => Some(wrapper_font_glyphs::FRAKTUR),
        "bold-italic" => Some(wrapper_font_glyphs::BOLD_ITALIC),
        _ => None,
    };
    if let Some(table) = extra {
        return table.binary_search_by_key(&code, |r| r.0).ok().map(|i| {
            let r = table[i];
            (r.1, r.2, r.3, r.4)
        });
    }
    let table = match variant {
        "monospace" => declaration_font_glyphs::MONO,
        "sans-serif" => declaration_font_glyphs::SANS,
        _ => return None,
    };
    table
        .binary_search_by_key(&code, |row| row.0)
        .ok()
        .map(|i| {
            let row = table[i];
            (row.1, row.2, row.3, 0.)
        })
}
pub(super) fn extended(variant: &str) -> bool {
    matches!(
        variant,
        "monospace" | "sans-serif" | "script" | "fraktur" | "bold-italic"
    )
}
pub(super) fn mapped(code: u32, variant: &str) -> u32 {
    if exact(code, variant).is_some() {
        return code;
    }
    let (upper, lower, greek_upper, greek_lower, digit) = match variant {
        "monospace" => (0x1D670, 0x1D68A, 0, 0, 0x1D7F6),
        "sans-serif" => (0x1D5A0, 0x1D5BA, 0, 0, 0x1D7E2),
        "script" => (0x1D49C, 0x1D4B6, 0, 0, 0),
        "fraktur" => (0x1D504, 0x1D51E, 0, 0, 0),
        "bold-italic" => (0x1D468, 0x1D482, 0x1D71C, 0x1D736, 0),
        _ => return code,
    };
    let smp = match code {
        0x41..=0x5A => Some(code - 0x41 + upper),
        0x61..=0x7A => Some(code - 0x61 + lower),
        0x30..=0x39 if digit != 0 => Some(code - 0x30 + digit),
        0x391..=0x3A9 if greek_upper != 0 && code != 0x3A2 => Some(code - 0x391 + greek_upper),
        0x3B1..=0x3C9 if greek_lower != 0 => Some(code - 0x3B1 + greek_lower),
        0x2207 if greek_upper != 0 => Some(greek_upper + 0x19),
        0x3F4 if greek_upper != 0 => Some(greek_upper + 0x11),
        0x2202 if greek_lower != 0 => Some(greek_lower + 0x19),
        0x3F5 if greek_lower != 0 => Some(greek_lower + 0x1A),
        0x3D1 if greek_lower != 0 => Some(greek_lower + 0x1B),
        0x3F0 if greek_lower != 0 => Some(greek_lower + 0x1C),
        0x3D5 if greek_lower != 0 => Some(greek_lower + 0x1D),
        0x3F1 if greek_lower != 0 => Some(greek_lower + 0x1E),
        0x3D6 if greek_lower != 0 => Some(greek_lower + 0x1F),
        _ => None,
    };
    if let Some(smp) = smp {
        return match smp {
            0x1D49D => 0x212C,
            0x1D4A0 => 0x2130,
            0x1D4A1 => 0x2131,
            0x1D4A3 => 0x210B,
            0x1D4A4 => 0x2110,
            0x1D4A7 => 0x2112,
            0x1D4A8 => 0x2133,
            0x1D4AD => 0x211B,
            0x1D4BA => 0x212F,
            0x1D4BC => 0x210A,
            0x1D4C4 => 0x2134,
            0x1D506 => 0x212D,
            0x1D50B => 0x210C,
            0x1D50C => 0x2111,
            0x1D515 => 0x211C,
            0x1D51D => 0x2128,
            _ => smp,
        };
    }
    if variant == "bold-italic" {
        let (paint, own) = double_struck::bold_child(code);
        if own || paint != code {
            return paint;
        }
    }
    if matches!(variant, "script" | "bold-italic")
        && italic_glyphs::ITALIC_GLYPHS
            .binary_search_by_key(&code, |r| r.codepoint)
            .is_err()
    {
        return italic_font_code(code);
    }
    code
}
pub(super) fn lookup(code: u32, variant: &str) -> Option<GlyphData> {
    if let Some(row) = exact(code, variant) {
        return Some(row.0);
    }
    if variant == "bold-italic" {
        if let Some(row) = double_struck::bold(code) {
            return Some(row.0);
        }
    }
    if matches!(variant, "script" | "bold-italic") {
        return italic_glyph(code);
    }
    glyph(code)
}
pub(super) fn correction(code: u32, variant: &str) -> f64 {
    if let Some(row) = exact(code, variant) {
        return row.1;
    }
    if variant == "bold-italic" {
        if let Some(row) = double_struck::bold(code) {
            return row.1;
        }
    }
    if matches!(variant, "script" | "bold-italic") {
        return italic_correction(code);
    }
    double_struck::correction(code, false)
}
pub(super) fn content(code: u32, variant: &str) -> Option<&'static str> {
    exact(code, variant).and_then(|r| (!r.2.is_empty()).then_some(r.2))
}

pub(super) fn wrapper(name: &str) -> bool {
    matches!(
        name,
        "mathrm"
            | "mathbf"
            | "mathit"
            | "mathsf"
            | "mathtt"
            | "mathbb"
            | "mathcal"
            | "mathscr"
            | "mathfrak"
            | "boldsymbol"
    )
}
pub(super) fn apply_wrapper(atom: &mut Atom, name: &str) {
    match name {
        "mathrm" => apply(atom, "rm"),
        "mathbf" => apply(atom, "bf"),
        "mathit" => apply(atom, "it"),
        "mathcal" => apply(atom, "cal"),
        "mathsf" => apply(atom, "sf"),
        "mathtt" => apply(atom, "tt"),
        "mathbb" => double_struck::apply(atom),
        "mathscr" => double_struck::apply_named(atom, "script"),
        "mathfrak" => double_struck::apply_named(atom, "fraktur"),
        "boldsymbol" => double_struck::apply_named(atom, "bold-italic"),
        _ => unreachable!(),
    }
}
