// Copyright 2017-2022 The MathJax Consortium; Apache-2.0.
// Source: tex/parser.go fontDeclarations/applyMathVariant and font/{font,smp}.go.
use super::*;
// font_mappings.go: retain the source registration's variant, including the
// explicitly empty MathFont variant. Existing handlers use these internal names.
pub(super) fn font_name(name: &str) -> Option<&'static str> {
    let variant = font_bindings::wrapper(name)
        .or_else(|| font_bindings::declaration(name))
        .or_else(|| (name == "boldsymbol").then_some("bold-italic"))?;
    Some(match variant {
        "normal" => "mathrm",
        "bold" => "mathbf",
        "-tex-mathit" => "mathit",
        "-tex-calligraphic" => "mathcal",
        "monospace" => "mathtt",
        "sans-serif" => "mathsf",
        "double-struck" => "mathbb",
        "script" => "mathscr",
        "fraktur" => "mathfrak",
        "bold-italic" if name == "boldsymbol" => "boldsymbol",
        "bold-italic" => "mathbfit",
        other => other,
    })
}
pub(super) fn declaration(name: &str) -> bool {
    font_bindings::declaration(name).is_some()
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
    if alias_fonts::supports(variant) {
        return alias_fonts::lookup(code, variant);
    }
    if variant == "-tex-mathit" {
        return mathit_glyphs::MATHIT_GLYPHS
            .binary_search_by_key(&code, |g| g.codepoint)
            .ok()
            .map(|i| {
                let g = mathit_glyphs::MATHIT_GLYPHS[i];
                let correction = mathit_glyphs::ITALIC_CORRECTIONS
                    .binary_search_by_key(&code, |r| r.0)
                    .ok()
                    .map_or(0., |i| mathit_glyphs::ITALIC_CORRECTIONS[i].1);
                (
                    GlyphData {
                        height: g.height,
                        depth: g.depth,
                        width: g.width,
                        path: g.path,
                    },
                    correction,
                    "",
                    0.,
                )
            });
    }
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
    alias_fonts::supports(variant)
        || matches!(
            variant,
            "monospace" | "sans-serif" | "script" | "fraktur" | "bold-italic" | "-tex-mathit"
        )
}
pub(super) fn mapped(code: u32, variant: &str) -> u32 {
    if alias_fonts::supports(variant) {
        return alias_fonts::remap(code, variant);
    }
    if exact(code, variant).is_some() {
        return code;
    }
    if variant == "-tex-mathit" {
        return if italic_glyphs::ITALIC_GLYPHS
            .binary_search_by_key(&code, |g| g.codepoint)
            .is_ok()
        {
            code
        } else {
            italic_font_code(code)
        };
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
    if matches!(variant, "script" | "bold-italic" | "-tex-mathit")
        && italic_glyphs::ITALIC_GLYPHS
            .binary_search_by_key(&code, |r| r.codepoint)
            .is_err()
    {
        return italic_font_code(code);
    }
    code
}
pub(super) fn lookup(code: u32, variant: &str) -> Option<GlyphData> {
    if alias_fonts::supports(variant) {
        return Some(alias_fonts::lookup(code, variant).map_or_else(
            || GlyphData {
                height: 0.75,
                depth: 0.2,
                width: unknown_glyph_width(code),
                path: "",
            },
            |r| r.0,
        ));
    }
    if let Some(row) = exact(code, variant) {
        return Some(row.0);
    }
    if variant == "bold-italic" {
        if let Some(row) = double_struck::bold(code) {
            return Some(row.0);
        }
    }
    if matches!(variant, "script" | "bold-italic" | "-tex-mathit") {
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
    if matches!(variant, "script" | "bold-italic" | "-tex-mathit") {
        return italic_correction(code);
    }
    double_struck::correction(code, false)
}
pub(super) fn content(code: u32, variant: &str) -> Option<&'static str> {
    exact(code, variant).and_then(|r| (!r.2.is_empty()).then_some(r.2))
}

pub(super) fn wrapper(name: &str) -> bool {
    font_bindings::wrapper(name).is_some() || name == "boldsymbol"
}
pub(super) fn apply_wrapper(atom: &mut Atom, name: &str) {
    match name {
        "" => {}
        "italic" => physics::apply_italic(atom),
        "mathrm" => apply(atom, "rm"),
        "mathbf" => apply(atom, "bf"),
        "mathit" => double_struck::apply_named(atom, "-tex-mathit"),
        "mathcal" => apply(atom, "cal"),
        "mathsf" => apply(atom, "sf"),
        "mathtt" => apply(atom, "tt"),
        "mathbb" => double_struck::apply(atom),
        "mathscr" => double_struck::apply_named(atom, "script"),
        "mathfrak" => double_struck::apply_named(atom, "fraktur"),
        "boldsymbol" | "mathbfit" => double_struck::apply_named(atom, "bold-italic"),
        other if alias_fonts::supports(other) => {
            double_struck::apply_named(atom, alias_fonts::name(other))
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod source_registration_contracts {
    use super::*;

    #[test]
    fn source_mathfont_aliases_share_arguments_fonts_and_nested_scope() {
        for (alias, canonical) in [
            ("symrm", "mathrm"),
            ("symup", "mathup"),
            ("symnormal", "mathnormal"),
            ("symbf", "mathbf"),
            ("symbfup", "mathbfup"),
            ("symbfit", "mathbfit"),
            ("symbb", "mathbb"),
            ("symfrak", "mathfrak"),
            ("symbffrak", "mathbffrak"),
            ("symscr", "mathscr"),
            ("symbfscr", "mathbfscr"),
            ("symsf", "mathsf"),
            ("symsfup", "mathsfup"),
            ("symbfsf", "mathbfsf"),
            ("symbfsfup", "mathbfsfup"),
            ("symsfit", "mathsfit"),
            ("symbfsfit", "mathbfsfit"),
            ("symtt", "mathtt"),
            ("symcal", "mathcal"),
            ("symbfcal", "mathbfcal"),
        ] {
            for argument in ["", "Abxy09", "αΓ!?", r"a\mathrm{bc}d", r"x_i^2", "é漢𝒜"] {
                let alias = format!("\\{alias}{{{argument}}}");
                let canonical = format!("\\{canonical}{{{argument}}}");
                assert_eq!(
                    render(&alias).unwrap(),
                    render(&canonical).unwrap(),
                    "{alias}"
                );
                assert_eq!(
                    measure(&alias).unwrap(),
                    measure(&canonical).unwrap(),
                    "{alias}"
                );
            }
        }
        assert_eq!(
            render(r"\symnormal{ab}").unwrap(),
            render(r"\mathnormal{ab}").unwrap()
        );
        assert_ne!(
            render(r"\symit{ab}").unwrap(),
            render(r"\mathrm{ab}").unwrap()
        );
    }

    #[test]
    fn additional_fonts_keep_own_outlines_and_css_fallbacks() {
        for (command, variant, family, italic, bold) in [
            ("mathbffrak", "bold-fraktur", "serif", false, true),
            ("mathbfscr", "bold-script", "cursive", false, true),
            ("mathbfsf", "bold-sans-serif", "sans-serif", false, true),
            ("mathsfit", "sans-serif-italic", "sans-serif", true, false),
            (
                "mathbfsfit",
                "sans-serif-bold-italic",
                "sans-serif",
                true,
                true,
            ),
            ("mathbfcal", "-tex-bold-calligraphic", "cursive", true, true),
        ] {
            let svg = render(&format!("\\{command}{{é}}")).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let text = doc.descendants().find(|n| n.has_tag_name("text")).unwrap();
            assert_eq!(text.attribute("data-variant"), Some(variant));
            assert_eq!(text.attribute("font-family"), Some(family));
            assert_eq!(text.attribute("font-style"), italic.then_some("italic"));
            assert_eq!(text.attribute("font-weight"), bold.then_some("bold"));
        }
        // Bold-fraktur owns its exclamation outline; bold-script has an empty
        // own table and obtains this character from its bold-italic chain.
        let fraktur = render(r"\mathbffrak{!}").unwrap();
        assert!(fraktur.contains("M121 621"));
        assert_ne!(fraktur, render(r"\mathbf{!}").unwrap());
        let old = render(r"{\oldstyle 0}+0").unwrap();
        assert!(old.contains("M39 213"));
        assert!(old.contains("M96 585"));
    }

    #[test]
    fn font_registrations_keep_argument_errors_and_user_macro_priority() {
        for name in ["symrm", "symnormal", "mathbffrak", "mathbfscr", "mathbfcal"] {
            let svg = render(&format!("\\{name}")).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let error = doc
                .descendants()
                .find_map(|n| n.attribute("data-mjx-error"));
            assert_eq!(
                error,
                Some(format!("Missing argument for \\{name}").as_str())
            );
        }
        assert_eq!(render(r"\oldstyle").unwrap(), render("").unwrap());
        // Registered commands take priority over both SetFont and MathFont.
        for name in ["oldstyle", "symbf"] {
            let source = format!(r"\DeclarePairedDelimiter{{\{name}}}{{(}}{{)}}\{name}{{x}}");
            assert_eq!(
                render(&source).unwrap(),
                render(r"\DeclarePairedDelimiter{\test}{(}{)}\test{x}").unwrap()
            );
        }
    }

    #[test]
    fn mathfont_child_macro_budget_is_independent_and_restores_the_caller() {
        let definition = r"\DeclarePairedDelimiterX{\d}[0]{(}{)}{x}";
        let block = r"\d".repeat(600);
        let success = format!("{definition}{block}\\symbf{{{block}}}");
        assert!(!render(&success).unwrap().contains("data-mjx-error"));
        let outer_overflow = format!("{success}{block}");
        assert!(
            render(&outer_overflow)
                .unwrap()
                .contains("maximum macro substitution count exceeded")
        );
        let child_overflow = format!("{definition}\\mathbf{{{block}{block}}}");
        assert!(
            render(&child_overflow)
                .unwrap()
                .contains("maximum macro substitution count exceeded")
        );
        assert!(!render(r"\symbf{x}").unwrap().contains("data-mjx-error"));
    }
}
