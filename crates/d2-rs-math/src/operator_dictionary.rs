// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: mathjax-go v0.1.0 tex/inheritance.go,nodes.go and svg/mo.go.
use super::*;
#[path = "operator_dictionary_data.rs"]
mod data;
#[derive(Clone, Copy, Debug)]
pub(super) enum Property {
    Bool(bool),
    Text(&'static str),
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Definition {
    pub class: i8,
    pub left: u8,
    pub right: u8,
    pub properties: &'static [(&'static str, Property)],
}
pub(super) struct Entry {
    pub text: &'static str,
    pub definition: Definition,
}
pub(super) struct Range {
    pub first: u32,
    pub last: u32,
    pub class: i8,
    pub kind: &'static str,
    pub variant: Option<&'static str>,
}
impl Definition {
    pub fn flag(self, name: &str) -> bool {
        self.properties
            .iter()
            .find(|p| p.0 == name)
            .is_some_and(|p| match p.1 {
                Property::Bool(v) => v,
                Property::Text(v) => !v.is_empty() && v != "false" && v != "0",
            })
    }
}
#[derive(Clone, Copy)]
pub(super) enum Form {
    Prefix,
    Infix,
    Postfix,
}
pub(super) fn range(code: u32) -> Option<&'static Range> {
    data::RANGES
        .iter()
        .find(|r| code >= r.first && code <= r.last)
}
pub(super) fn lookup(code: u32, form: Form) -> Definition {
    if let Some(definition) = lookup_registered(code, form) {
        return definition;
    }
    let class = data::RANGES
        .iter()
        .find(|r| code >= r.first && code <= r.last)
        .map_or(0, |r| r.class);
    Definition {
        class,
        left: 0,
        right: 0,
        properties: &[],
    }
}
// The pre-inheritance mo getter defaults to REL when no dictionary entry
// exists; it must not use lookup's later Unicode-range fallback.
pub(super) fn lookup_registered(code: u32, form: Form) -> Option<Definition> {
    let text = char::from_u32(code).unwrap().to_string();
    let forms = match form {
        Form::Prefix => [data::PREFIX, data::INFIX, data::POSTFIX],
        Form::Infix => [data::INFIX, data::PREFIX, data::POSTFIX],
        Form::Postfix => [data::POSTFIX, data::INFIX, data::PREFIX],
    };
    for dictionary in forms {
        if let Ok(i) = dictionary.binary_search_by_key(&text.as_str(), |e| e.text) {
            return Some(dictionary[i].definition);
        }
    }
    None
}
pub(super) fn spacelike(a: &Atom) -> bool {
    match a {
        Atom::FractionStrut(_)
        | Atom::ApplyFunction
        | Atom::Space(_)
        | Atom::NonscriptSpace { .. }
        | Atom::Kern(_)
        | Atom::EmptyNode("mspace")
        | Atom::RuleSpace { .. }
        | Atom::Strut(..) => true,
        Atom::Sequence(v) | Atom::ExplicitRow(v) => v.iter().all(spacelike),
        Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a) => spacelike(a),
        // An empty mstyle (e.g. a trailing \color{red}) returns the previous
        // node from setTeXClass, taking no TeX spacing (svg/texclass.go).
        Atom::Color { body, .. } => matches!(body.as_ref(), Atom::Sequence(v) if v.is_empty()),
        _ => false,
    }
}
// The MML node isSpacelike flag (tex/mml_definitions.go, nodes.go
// refreshDynamicFlags) used by operatorForms and embellished-row detection.
// Unlike TeX-class spacing, it also counts mtext as spacelike.
pub(super) fn flag_spacelike(a: &Atom) -> bool {
    match a {
        Atom::Text(_) | Atom::StyledText(_) => true,
        Atom::Sequence(v) | Atom::ExplicitRow(v) => v.iter().all(flag_spacelike),
        _ => spacelike(a),
    }
}
pub(super) fn embellished(a: &Atom) -> bool {
    match a {
        Atom::Glyph(code) | Atom::FontGlyph { original: code, .. }
            if is_large_operator(*code) || (xarrow::recipe(*code).is_some() && *code != 0x3d) =>
        {
            true
        }
        Atom::RawOperator { .. } | Atom::MathMLSpace { .. } => true,
        Atom::Sequence(v) | Atom::ExplicitRow(v) => {
            v.iter().filter(|a| !flag_spacelike(a)).count() == 1
                && v.iter().filter(|a| !flag_spacelike(a)).all(embellished)
        }
        Atom::Scripts { base: a, .. }
        | Atom::Multiscripts { base: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a) => embellished(a),
        _ => false,
    }
}
// operatorForms follows embellished cores through semantic parents and skips
// spacelike siblings. Single-core rows borrow their enclosing positional form.
pub(super) fn inherit(a: &mut Atom, form: Form) {
    match a {
        Atom::RawOperator {
            code,
            definition,
            texclass,
            ..
        } => {
            *definition = lookup(*code, form);
            // applyOperatorInheritance keeps an explicit texClass property.
            if let Some(class) = *texclass {
                definition.class = class;
            }
        }
        Atom::InferredRow(v) | Atom::ExplicitRow(v) | Atom::Sequence(v) => {
            let positions = v
                .iter()
                .enumerate()
                .filter(|(_, a)| !flag_spacelike(a))
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            let single = positions.len() == 1 && embellished(&v[positions[0]]);
            for (i, child) in v.iter_mut().enumerate() {
                let own = if single {
                    form
                } else if positions.len() > 1 && positions.first() == Some(&i) {
                    Form::Prefix
                } else if positions.len() > 1 && positions.last() == Some(&i) {
                    Form::Postfix
                } else {
                    Form::Infix
                };
                inherit(child, own);
            }
            combine_relations(v);
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            inherit(base, form);
            inherit(sub, Form::Infix);
            inherit(sup, Form::Infix);
            if let Some(v) = post {
                for a in v {
                    inherit(a, Form::Infix);
                }
            }
        }
        Atom::Scripts { base, sub, sup, .. } => {
            inherit(base, form);
            if let Some(a) = sub {
                inherit(a, Form::Infix);
            }
            if let Some(a) = sup {
                inherit(a, Form::Infix);
            }
        }
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => {
            inherit(a, Form::Infix);
            inherit(b, Form::Infix);
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a) => inherit(a, form),
        Atom::Fenced { body, .. } => {
            for a in body.iter_mut() {
                inherit(a, Form::Infix);
            }
            combine_relations(body);
        }
        Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => inherit(a, Form::Infix),
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            for a in rows.iter_mut().flatten() {
                inherit(a, Form::Infix);
            }
        }
        Atom::XArrow { over, under, .. } => {
            inherit(over, Form::Infix);
            if let Some(a) = under {
                inherit(a, Form::Infix);
            }
        }
        Atom::CdArrow { over, under, .. } => {
            if let Some(a) = over {
                inherit(a, Form::Infix);
            }
            if let Some(a) = under {
                inherit(a, Form::Infix);
            }
        }
        _ => {}
    }
}

// tex/combine_relations.go: only adjacent mo siblings are candidates; rows,
// scripts and TeXAtoms keep their own ownership boundaries. The native token
// representation stores the explicit class on RawOperator and font/variant
// attributes on its text child. False stretchy is excluded by the source's
// explicit-attribute comparison.
fn relation(atom: &Atom) -> Option<(Vec<Atom>, Option<i8>)> {
    if atom_tex_class(atom) != Some(3) {
        return None;
    }
    match atom {
        Atom::RawOperator { body, texclass, .. } => match body.as_ref() {
            Atom::Sequence(children) => Some((children.clone(), *texclass)),
            Atom::Glyph(_)
            | Atom::LiteralGlyph(_)
            | Atom::VariantGlyph(_)
            | Atom::FontGlyph { .. }
            | Atom::ItalicGlyph(_)
            | Atom::CalligraphicGlyph(_) => Some((vec![(**body).clone()], *texclass)),
            _ => None,
        },
        Atom::Glyph(_)
        | Atom::VariantGlyph(_)
        | Atom::FontGlyph { .. }
        | Atom::ItalicGlyph(_)
        | Atom::CalligraphicGlyph(_)
            if matches!(atom, Atom::Glyph(c) | Atom::VariantGlyph(c) if mathml_node_kind(*c) == "mo") =>
        {
            Some((vec![atom.clone()], None))
        }
        _ => None,
    }
}

fn relation_font(atom: &Atom) -> (&'static str, bool) {
    match atom {
        Atom::VariantGlyph(_) => ("normal", true),
        Atom::FontGlyph { font_variant, .. } => (font_variant, false),
        Atom::ItalicGlyph(_) => ("italic", false),
        Atom::CalligraphicGlyph(_) => ("script", false),
        _ => ("normal", false),
    }
}

fn relation_attributes(atom: &Atom) -> Vec<(&'static str, &'static str)> {
    if let Atom::RawOperator { attributes, .. } = atom {
        attributes
            .iter()
            .copied()
            .filter(|(name, value)| {
                !matches!(*name, "texClass" | "movesupsub" | "variantForm")
                    && !(*name == "stretchy" && *value == "false")
            })
            .collect()
    } else {
        Vec::new()
    }
}

fn combine_relations(children: &mut Vec<Atom>) {
    let mut index = 0;
    while index + 1 < children.len() {
        let Some((mut left, left_class)) = relation(&children[index]) else {
            index += 1;
            continue;
        };
        let Some((right, right_class)) = relation(&children[index + 1]) else {
            index += 1;
            continue;
        };
        if relation_attributes(&children[index]) != relation_attributes(&children[index + 1])
            || relation_font(&left[0]) != relation_font(&right[0])
        {
            index += 1;
            continue;
        }
        let code = match &children[index] {
            Atom::RawOperator { code, .. } => *code,
            Atom::Glyph(code) | Atom::VariantGlyph(code) => *code,
            _ => unreachable!(),
        };
        left.extend(right);
        let attributes = if let Atom::RawOperator { attributes, .. } = &children[index] {
            *attributes
        } else {
            &[]
        };
        children[index] = Atom::RawOperator {
            attributes,
            cleanup: false,
            code,
            texclass: right_class.or(left_class),
            body: Box::new(Atom::Sequence(left)),
            // A multi-character relation has range spacing, not the original
            // single-character operator's stretchy/symmetric properties.
            definition: Definition {
                class: 3,
                left: 5,
                right: 5,
                properties: &[],
            },
        };
        children.remove(index + 1);
    }
    // A differing variant or explicit attribute stops coalescing but still
    // writes the touching rspace/lspace to zero. This makes each operator use
    // MathML spacing, including its untouched outer dictionary space.
    let mut touching = vec![(false, false); children.len()];
    for index in 0..children.len().saturating_sub(1) {
        if relation(&children[index]).is_some() && relation(&children[index + 1]).is_some() {
            touching[index].1 = true;
            touching[index + 1].0 = true;
        }
    }
    for (child, (left, right)) in children.iter_mut().zip(touching) {
        if left || right {
            *child = Atom::MathMLSpace {
                body: Box::new(child.clone()),
                left: left.then_some(Dimension::relative(0.)),
                right: right.then_some(Dimension::relative(0.)),
            };
        }
    }
}
// The script core keeps the actual mo font variant and inherited largeop.
pub(super) fn italic_correction(code: u32, definition: Definition, display: bool) -> f64 {
    let table = if definition.flag("largeop") && display {
        large_glyphs::ITALIC_CORRECTIONS
    } else if definition.flag("largeop")
        && smallop_glyphs::SMALL_OP_GLYPHS
            .binary_search_by_key(&code, |g| g.codepoint)
            .is_ok()
    {
        smallop_glyphs::ITALIC_CORRECTIONS
    } else {
        glyphs::ITALIC_CORRECTIONS
    };
    table
        .binary_search_by_key(&code, |g| g.0)
        .ok()
        .map_or(0.0, |i| table[i].1)
}
pub(super) fn body_correction(body: &Atom, definition: Definition, display: bool) -> f64 {
    if !definition.flag("largeop")
        && matches!(
            body,
            Atom::FontGlyph { .. } | Atom::ItalicGlyph(_) | Atom::CalligraphicGlyph(_)
        )
    {
        return classes::italic_correction(body, display);
    }
    // A variantForm mo's metrics, including its italic correction, come from
    // the -tex-variant font.
    if let Atom::VariantGlyph(code) = body
        && !definition.flag("largeop")
    {
        return tex_variant_glyphs::ITALIC_CORRECTIONS
            .binary_search_by_key(code, |g| g.0)
            .ok()
            .map_or(0.0, |i| tex_variant_glyphs::ITALIC_CORRECTIONS[i].1);
    }
    let code = match body {
        Atom::Glyph(c)
        | Atom::LiteralGlyph(c)
        | Atom::ItalicGlyph(c)
        | Atom::CalligraphicGlyph(c) => *c,
        Atom::FontGlyph { paint, .. } => *paint,
        _ => return 0.,
    };
    italic_correction(code, definition, display)
}
pub(super) fn layout(
    body: &Atom,
    definition: Definition,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
) -> Option<f64> {
    if let Atom::Sequence(children) = body {
        return pending_not::layout_children(children, "mo", x, baseline, scale, out);
    }
    let code = match body {
        Atom::Glyph(c)
        | Atom::LiteralGlyph(c)
        | Atom::ItalicGlyph(c)
        | Atom::CalligraphicGlyph(c)
        | Atom::VariantGlyph(c) => *c,
        Atom::FontGlyph { paint, .. } => *paint,
        _ => return None,
    };
    let large = definition.flag("largeop") && display;
    let small = definition.flag("largeop")
        && !display
        && smallop_glyphs::SMALL_OP_GLYPHS
            .binary_search_by_key(&code, |g| g.codepoint)
            .is_ok();
    let item = PositionedGlyph {
        font_variant: if definition.flag("largeop") {
            "normal"
        } else if let Atom::FontGlyph { font_variant, .. } = body {
            font_variant
        } else {
            "normal"
        },
        font_original: if definition.flag("largeop") {
            None
        } else if let Atom::FontGlyph { original, .. } = body {
            Some(*original)
        } else {
            None
        },
        bold: matches!(body, Atom::FontGlyph { bold: true, .. }) && !definition.flag("largeop"),
        codepoint: code,
        x: 0.0,
        baseline: 0.0,
        scale,
        large,
        small_operator: small,
        // A variantForm mo selects the -tex-variant font.
        variant: matches!(body, Atom::VariantGlyph(_)) && !definition.flag("largeop"),
        unit: false,
        calligraphic: matches!(body, Atom::CalligraphicGlyph(_)) && !definition.flag("largeop"),
        italic: matches!(body, Atom::ItalicGlyph(_)) && !definition.flag("largeop"),
        stretch: false,
        radical_variant: None,
        assembly_height: None,
    };
    let glyph = glyph_for(item)?;
    let shift = if definition.flag("symmetric") || definition.flag("largeop") {
        ((glyph.height + glyph.depth) / 2.0 + 0.25 - glyph.height) * 1000.0 * scale
    } else {
        0.0
    };
    let bbox_shift = if definition.flag("symmetric") {
        shift
    } else {
        0.0
    };
    let correction = body_correction(body, definition, display);
    // svg/mo.go keeps BBox.W in em; Append scales that advance before
    // SVG's final conversion to font units.
    let width_em = (glyph.width + correction) * scale;
    let width = width_em * 1000.0;
    out.width_em = Some(width_em);
    let paint = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width,
        ascent: glyph.height * 1000.0 * scale,
        depth: glyph.depth * 1000.0 * scale,
        glyphs: vec![item],
        rules: Vec::new(),
        groups: Vec::new(),
    };
    let mut local = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width,
        ascent: glyph.height * 1000.0 * scale + bbox_shift,
        depth: glyph.depth * 1000.0 * scale - bbox_shift,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    };
    // Original svg/mo.go moToSVG: the centering offset is the mo's own
    // "translate(0 u)" attribute, not a nested frame.
    local.glyphs = paint.glyphs;
    let own_translate = (shift != 0.0).then_some(shift / scale);
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    out.groups.push(LayoutGroup {
        x_frame: None,
        data_cramped: None,
        own_translate,
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
        kind: "mo",
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: Vec::new(),
    });
    Some(width)
}
