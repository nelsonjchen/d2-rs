// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/pending_not.go and not_remap data.
use super::*;

fn remap(code: u32) -> Option<u32> {
    Some(match code {
        0x2190 => 0x219A,
        0x2192 => 0x219B,
        0x2194 => 0x21AE,
        0x21D0 => 0x21CD,
        0x21D2 => 0x21CF,
        0x21D4 => 0x21CE,
        0x2208 => 0x2209,
        0x220B => 0x220C,
        0x2223 => 0x2224,
        0x2225 => 0x2226,
        0x223C => 0x2241,
        0x7E => 0x2241,
        0x2243 => 0x2244,
        0x2245 => 0x2247,
        0x2248 => 0x2249,
        0x224D => 0x226D,
        0x3D => 0x2260,
        0x2261 => 0x2262,
        0x3C => 0x226E,
        0x3E => 0x226F,
        0x2264 => 0x2270,
        0x2265 => 0x2271,
        0x2272 => 0x2274,
        0x2273 => 0x2275,
        0x2276 => 0x2278,
        0x2277 => 0x2279,
        0x227A => 0x2280,
        0x227B => 0x2281,
        0x2282 => 0x2284,
        0x2283 => 0x2285,
        0x2286 => 0x2288,
        0x2287 => 0x2289,
        0x22A2 => 0x22AC,
        0x22A8 => 0x22AD,
        0x22A9 => 0x22AE,
        0x22AB => 0x22AF,
        0x227C => 0x22E0,
        0x227D => 0x22E1,
        0x2291 => 0x22E2,
        0x2292 => 0x22E3,
        0x22B2 => 0x22EA,
        0x22B3 => 0x22EB,
        0x22B4 => 0x22EC,
        0x22B5 => 0x22ED,
        0x2203 => 0x2204,
        _ => return None,
    })
}

pub(super) fn fallback() -> Atom {
    Atom::Class {
        class: 3,
        body: Box::new(Atom::Padded {
            body: Box::new(Atom::Text("\u{29f8}".into())),
            padding: padded::Padding {
                width_zero: true,
                ..Default::default()
            },
        }),
    }
}

fn scalar(atom: &Atom) -> Option<u32> {
    match atom {
        Atom::Glyph(c)
        | Atom::ItalicGlyph(c)
        | Atom::CalligraphicGlyph(c)
        | Atom::VariantGlyph(c)
        | Atom::FontGlyph { original: c, .. } => Some(*c),
        Atom::LiteralGlyph(c) if *c <= 0xffff => Some(*c),
        _ => None,
    }
}
fn replace_glyph(atom: &mut Atom, code: u32) {
    match atom {
        Atom::Glyph(c)
        | Atom::LiteralGlyph(c)
        | Atom::ItalicGlyph(c)
        | Atom::CalligraphicGlyph(c)
        | Atom::VariantGlyph(c) => *c = code,
        Atom::FontGlyph { font_variant, .. } => {
            let variant = *font_variant;
            *atom = Atom::Glyph(code);
            double_struck::apply_named(atom, variant);
        }
        _ => unreachable!(),
    }
}

// The caller supplies the next final item, never a macro argument. An authored
// TeXAtom, mn, multi-character token or movesupsub owner is not eligible.
pub(super) fn apply(atom: &mut Atom) -> bool {
    // OperatorName and similar parser markers own token properties without
    // creating another MML node. Only a truthy movesupsub blocks Not.
    if let Atom::ScriptPolicy {
        body, moves: false, ..
    }
    | Atom::Operator(body) = atom
    {
        return apply(body);
    }
    let cleanup = matches!(atom, Atom::RawOperator { cleanup: true, .. })
        || matches!(atom, Atom::Glyph(c) | Atom::FontGlyph { original: c, .. }
            if mathml_node_kind(*c) == "mo");
    let attributes = if let Atom::RawOperator { attributes, .. } = atom {
        *attributes
    } else {
        &[]
    };
    let original_class = atom_tex_class(atom).unwrap_or(0);
    let (kind, mut first, class, raw_code) = match &*atom {
        Atom::RawOperator {
            body,
            code,
            texclass,
            ..
        } => (
            "mo",
            *body.clone(),
            texclass.map(|c| c.max(0) as usize),
            Some(*code),
        ),
        Atom::Identifier(v) | Atom::StyledText(v) | Atom::OperatorToken(v) if v.len() == 1 => (
            if matches!(atom, Atom::StyledText(_)) {
                "mtext"
            } else if matches!(atom, Atom::OperatorToken(_)) {
                "mo"
            } else {
                "mi"
            },
            match atom {
                Atom::Identifier(v) | Atom::StyledText(v) | Atom::OperatorToken(v) => v[0].clone(),
                _ => unreachable!(),
            },
            Some(0),
            None,
        ),
        Atom::Text(s) if s.encode_utf16().count() == 1 => (
            "mtext",
            Atom::LiteralGlyph(s.chars().next().unwrap() as u32),
            Some(0),
            None,
        ),
        // Command symbols can represent a token without a token-list wrapper.
        Atom::Glyph(c) | Atom::FontGlyph { original: c, .. } | Atom::VariantGlyph(c)
            if !script_moves(atom) =>
        {
            (
                mathml_node_kind(*c),
                atom.clone(),
                if is_large_operator(*c) { Some(1) } else { None },
                Some(*c),
            )
        }
        // CleanStretchy is deferred until after Not in the source. This native
        // marker is a raw ORD mo, distinct from authored Class/Ord row owners.
        Atom::Ord(body) if matches!(body.as_ref(), Atom::RawOperator { .. }) => {
            let mut body = *body.clone();
            if !apply(&mut body) {
                return false;
            }
            *atom = body;
            return true;
        }
        _ => return false,
    };
    let Some(code) = scalar(&first) else {
        return false;
    };
    if raw_code.unwrap_or(code) > 0xffff && matches!(first, Atom::LiteralGlyph(_)) {
        return false;
    }
    if let Some(replacement) = remap(raw_code.unwrap_or(code)) {
        replace_glyph(&mut first, replacement);
        *atom = match kind {
            "mo" => Atom::RawOperator {
                attributes,
                cleanup,
                code: replacement,
                body: Box::new(first),
                texclass: class.map(|c| c as i8),
                definition: operator_dictionary::lookup(
                    replacement,
                    operator_dictionary::Form::Infix,
                ),
            },
            "mtext" => Atom::StyledText(vec![first]),
            _ => Atom::Identifier(vec![first]),
        };
    } else {
        if let Some(code) = raw_code {
            replace_glyph(&mut first, code);
        }
        // Appending a second text child bypasses mo's one-character renderer.
        // Large-operator prototypes must use the ordinary token font here.
        if matches!(first, Atom::Glyph(c) if is_large_operator(c)) {
            if let Atom::Glyph(c) = first {
                first = Atom::LiteralGlyph(c);
            }
        }
        *atom = Atom::NotToken {
            kind,
            first: Box::new(first),
            class: class.unwrap_or_else(|| {
                raw_code
                    .and_then(operator_dictionary::range)
                    .map_or(original_class, |r| r.class.max(0) as usize)
            }),
        };
    }
    if cleanup && atom_tex_class(atom) == Some(0) {
        *atom = Atom::Ord(Box::new(atom.clone()));
    }
    true
}

pub(super) fn layout_token(
    first: &Atom,
    kind: &'static str,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    _display: bool,
    _prime: bool,
) -> Option<f64> {
    let mut slash = Atom::LiteralGlyph(0x338);
    match first {
        Atom::FontGlyph { font_variant, .. } => {
            double_struck::apply_named(&mut slash, font_variant)
        }
        Atom::CalligraphicGlyph(_) => slash = Atom::CalligraphicGlyph(0x338),
        Atom::ItalicGlyph(_) => slash = Atom::ItalicGlyph(0x338),
        _ => {}
    }
    layout_children(&[first.clone(), slash], kind, x, baseline, scale, out)
}

// svg/wrapper.go computeChildrenBBox and text-node rendering: a token with
// several text children retains each child frame, with no TeX spacing.
pub(super) fn layout_children(
    children: &[Atom],
    kind: &'static str,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    let mut local = general_fraction::blank();
    let mut cursor = 0.;
    for child in children {
        let mut text = general_fraction::blank();
        let code = scalar(child)?;
        let item = PositionedGlyph {
            font_original: if let Atom::FontGlyph { original, .. } = child {
                Some(*original)
            } else {
                None
            },
            font_variant: if let Atom::FontGlyph { font_variant, .. } = child {
                font_variant
            } else {
                "normal"
            },
            bold: matches!(child, Atom::FontGlyph { bold: true, .. }),
            small_operator: false,
            variant: matches!(child, Atom::VariantGlyph(_)),
            unit: false,
            calligraphic: matches!(child, Atom::CalligraphicGlyph(_)),
            italic: matches!(child, Atom::ItalicGlyph(_)),
            codepoint: if let Atom::FontGlyph { paint, .. } = child {
                *paint
            } else {
                code
            },
            x: 0.,
            baseline: 0.,
            scale,
            large: false,
            stretch: false,
            radical_variant: None,
            assembly_height: None,
        };
        let glyph = glyph_for(item)?;
        text.width = glyph.width * 1000. * scale;
        text.ascent = glyph.height * 1000. * scale;
        text.depth = glyph.depth * 1000. * scale;
        text.glyphs.push(item);
        retain_negative_glyph_depth(&mut text);
        local.ascent = local.ascent.max(text.ascent);
        local.depth = local.depth.max(text.depth);
        let advance = text.width;
        local
            .groups
            .push(table::group(text, cursor, 0., scale, "text"));
        cursor += advance;
    }
    local.width = cursor;
    retain_negative_glyph_depth(&mut local);
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    out.width_em = Some(cursor / 1000.);
    let mut group = table::group(local, x, baseline, scale, kind);
    group.before_glyph = out.glyphs.len();
    out.groups.push(group);
    Some(cursor)
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn not_remaps_the_next_token_across_empty_results_and_font_continuations() {
        for (source, equivalent) in [
            (r"\not=", r"\neq"),
            (r"\not\in", r"\notin"),
            (r"a\not\to b", r"a\nrightarrow b"),
            (r"\not\label{q}=", r"\neq"),
            (r"\not\DeclareMathOperator{\probe}{f}=", r"\neq"),
            (r"\not\rm=", r"\neq"),
        ] {
            assert_eq!(render_exact(source), render_exact(equivalent), "{source}");
        }
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\not}{f}\not x"),
            render_exact(r"\DeclareMathOperator{\probe}{f}\probe x")
        );
    }

    #[test]
    fn not_appends_text_children_with_the_token_font_and_keeps_utf16_eligibility() {
        for source in [r"\not x", r"\not+", r"\not\text{x}", r"\mathbf{\not x}"] {
            let svg = render_exact(source).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let texts: Vec<_> = doc
                .descendants()
                .filter(|n| n.attribute("data-mml-node") == Some("text"))
                .collect();
            assert_eq!(texts.len(), 2, "{source}");
            assert_eq!(
                texts[1]
                    .children()
                    .find(|n| n.is_element())
                    .unwrap()
                    .attribute("data-c"),
                Some("338")
            );
            assert!(!svg.contains("data-mjx-texclass=\"REL\""), "{source}");
        }
        for source in [
            r"\not1",
            r"\not{x}",
            r"\not\text{xy}",
            r"\not 𝑥",
            r"\not\sum",
        ] {
            let svg = render_exact(source).unwrap();
            assert!(svg.contains("data-c=\"29F8\""), "{source}");
            assert!(!svg.contains("data-mml-node=\"text\""), "{source}");
        }
        let svg = render_exact(r"\not").unwrap();
        let doc = roxmltree::Document::parse(&svg).unwrap();
        assert!(
            doc.root_element()
                .attribute("viewBox")
                .unwrap()
                .ends_with("16 931")
        );
    }

    #[test]
    fn not_is_a_row_item_and_scripts_reject_an_unbraced_pending_item() {
        for (source, message) in [
            (r"x^\not=", "Missing open brace for superscript"),
            (r"x_\not=", "Missing open brace for subscript"),
            (r"\not\limits", r"\limits is allowed only on operators"),
        ] {
            assert_eq!(
                render(source).unwrap(),
                cd::render_error(message),
                "{source}"
            );
        }
        // Another Not emits the previous fallback, and a grouped Not cannot
        // consume the following row's equality token.
        for source in [r"\not\not=", r"{\not}=", r"\not\scriptstyle="] {
            let svg = render_exact(source).unwrap();
            assert!(svg.contains("data-c=\"29F8\""), "{source}");
        }
        assert!(!render_exact(r"\not=").unwrap().contains("data-c=\"29F8\""));
    }
}
