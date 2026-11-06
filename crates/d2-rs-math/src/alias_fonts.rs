// Copyright (c) 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned internal/font/{font,smp}.go: own maps, links, inheritance and SMP.
// Additional source registrations use these variants; qualified older paths
// continue using their existing tables and handlers.
use super::*;
type Entry = (GlyphData, f64, &'static str, f64);

pub(super) fn supports(variant: &str) -> bool {
    matches!(
        variant,
        "bold-fraktur"
            | "bold-script"
            | "bold-sans-serif"
            | "sans-serif-italic"
            | "sans-serif-bold-italic"
            | "-tex-bold-calligraphic"
            | "-tex-oldstyle"
    )
}
pub(super) fn name(variant: &str) -> &'static str {
    match variant {
        "bold-fraktur" => "bold-fraktur",
        "bold-script" => "bold-script",
        "bold-sans-serif" => "bold-sans-serif",
        "sans-serif-italic" => "sans-serif-italic",
        "sans-serif-bold-italic" => "sans-serif-bold-italic",
        "-tex-bold-calligraphic" => "-tex-bold-calligraphic",
        "-tex-oldstyle" => "-tex-oldstyle",
        _ => unreachable!(),
    }
}
fn table(variant: &str) -> Option<&'static [(u32, GlyphData, f64, &'static str, f64)]> {
    use alias_font_glyphs::*;
    Some(match variant {
        "bold-fraktur" => FRAKTUR_BOLD,
        "bold-script" => SCRIPT_BOLD,
        "bold-sans-serif" => SANS_SERIF_BOLD,
        "sans-serif-italic" => SANS_SERIF_ITALIC,
        "sans-serif-bold-italic" => SANS_SERIF_BOLD_ITALIC,
        "-tex-bold-calligraphic" => TEX_CALLIGRAPHIC_BOLD,
        "-tex-oldstyle" => TEX_OLDSTYLE,
        _ => return None,
    })
}
fn exact(code: u32, variant: &str) -> Option<Entry> {
    if let Some(table) = table(variant) {
        return table.binary_search_by_key(&code, |r| r.0).ok().map(|i| {
            let r = table[i];
            (r.1, r.2, r.3, r.4)
        });
    }
    match variant {
        "script" => None, // The source Script own map is empty.
        "normal" => {
            if let Some(g) = compound(code) {
                return Some((
                    glyph(code)?,
                    double_struck::correction(code, false),
                    g.content,
                    0.,
                ));
            }
            let i = glyphs::GLYPHS
                .binary_search_by_key(&code, |g| g.codepoint)
                .ok()?;
            let g = glyphs::GLYPHS[i];
            Some((
                GlyphData {
                    height: g.height,
                    depth: g.depth,
                    width: g.width,
                    path: g.path,
                },
                double_struck::correction(code, false),
                "",
                0.,
            ))
        }
        "italic" => {
            let i = italic_glyphs::ITALIC_GLYPHS
                .binary_search_by_key(&code, |g| g.codepoint)
                .ok()?;
            let g = italic_glyphs::ITALIC_GLYPHS[i];
            Some((
                GlyphData {
                    height: g.height,
                    depth: g.depth,
                    width: g.width,
                    path: g.path,
                },
                italic_correction(code),
                "",
                0.,
            ))
        }
        "bold" => double_struck::bold(code).map(|r| (r.0, r.1, r.2, 0.)),
        _ => declaration_fonts::exact(code, variant),
    }
}
fn rule(variant: &str) -> (&str, &str) {
    match variant {
        "bold-fraktur" => ("bold", "fraktur"),
        "bold-script" => ("bold-italic", "script"),
        "bold-sans-serif" => ("bold", "sans-serif"),
        "sans-serif-italic" => ("italic", "sans-serif"),
        "sans-serif-bold-italic" => ("bold-italic", "bold-sans-serif"),
        "bold-italic" => ("italic", "bold"),
        "script" | "-tex-bold-calligraphic" => (
            if variant == "script" {
                "italic"
            } else {
                "bold-italic"
            },
            "",
        ),
        "bold" | "italic" | "fraktur" | "sans-serif" | "-tex-oldstyle" => ("normal", ""),
        _ => ("", ""),
    }
}
fn smp(code: u32, variant: &str) -> Option<u32> {
    let [upper, lower, greek_upper, greek_lower, digit] = match variant {
        "bold" => [0x1D400, 0x1D41A, 0x1D6A8, 0x1D6C2, 0x1D7CE],
        "italic" => [0x1D434, 0x1D44E, 0x1D6E2, 0x1D6FC, 0],
        "bold-italic" => [0x1D468, 0x1D482, 0x1D71C, 0x1D736, 0],
        "script" => [0x1D49C, 0x1D4B6, 0, 0, 0],
        "bold-script" => [0x1D4D0, 0x1D4EA, 0, 0, 0],
        "fraktur" => [0x1D504, 0x1D51E, 0, 0, 0],
        "bold-fraktur" => [0x1D56C, 0x1D586, 0, 0, 0],
        "sans-serif" => [0x1D5A0, 0x1D5BA, 0, 0, 0x1D7E2],
        "bold-sans-serif" => [0x1D5D4, 0x1D5EE, 0x1D756, 0x1D770, 0x1D7EC],
        "sans-serif-italic" => [0x1D608, 0x1D622, 0, 0, 0],
        "sans-serif-bold-italic" => [0x1D63C, 0x1D656, 0x1D790, 0x1D7AA, 0],
        _ => return None,
    };
    let code = match code {
        0x41..=0x5A => upper + code - 0x41,
        0x61..=0x7A => lower + code - 0x61,
        0x30..=0x39 if digit != 0 => digit + code - 0x30,
        0x391..=0x3A9 if greek_upper != 0 && code != 0x3A2 => greek_upper + code - 0x391,
        0x3B1..=0x3C9 if greek_lower != 0 => greek_lower + code - 0x3B1,
        0x2207 if greek_upper != 0 => greek_upper + 0x19,
        0x3F4 if greek_upper != 0 => greek_upper + 0x11,
        0x2202 if greek_lower != 0 => greek_lower + 0x19,
        0x3F5 if greek_lower != 0 => greek_lower + 0x1A,
        0x3D1 if greek_lower != 0 => greek_lower + 0x1B,
        0x3F0 if greek_lower != 0 => greek_lower + 0x1C,
        0x3D5 if greek_lower != 0 => greek_lower + 0x1D,
        0x3F1 if greek_lower != 0 => greek_lower + 0x1E,
        0x3D6 if greek_lower != 0 => greek_lower + 0x1F,
        0x3DC..=0x3DD if variant == "bold" => 0x1D7CA + code - 0x3DC,
        _ => return None,
    };
    Some(match code {
        0x1D455 => 0x210E,
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
        other => other,
    })
}
enum Own {
    Glyph(Entry),
    Remap(u32),
}
fn own(code: u32, variant: &str) -> Option<Own> {
    exact(code, variant)
        .map(Own::Glyph)
        .or_else(|| smp(code, variant).map(Own::Remap))
}
fn effective(code: u32, variant: &str) -> Option<Own> {
    if let Some(value) = own(code, variant) {
        return Some(value);
    }
    let (inherit, link) = rule(variant);
    if !link.is_empty() {
        if let Some(value) = own(code, link) {
            return Some(value);
        }
    }
    if inherit.is_empty() {
        None
    } else {
        effective(code, inherit)
    }
}
pub(super) fn remap(code: u32, variant: &str) -> u32 {
    match effective(code, variant) {
        Some(Own::Remap(mapped)) => mapped,
        _ => code,
    }
}
pub(super) fn lookup(code: u32, variant: &str) -> Option<Entry> {
    match effective(code, variant)? {
        Own::Glyph(entry) => Some(entry),
        Own::Remap(mapped) => match effective(mapped, variant)? {
            Own::Glyph(entry) => Some(entry),
            Own::Remap(_) => None,
        },
    }
}
