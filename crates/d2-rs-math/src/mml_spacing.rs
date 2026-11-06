// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/mml_token.go, svg/wrapper.go getMathMLSpacing.
use super::*;

fn dictionary_space(atom: &Atom, right: bool, scale: f64) -> f64 {
    let definition = match atom {
        Atom::RawOperator { definition, .. } => *definition,
        Atom::Glyph(code) | Atom::VariantGlyph(code) => {
            operator_dictionary::lookup(*code, operator_dictionary::Form::Infix)
        }
        Atom::MathMLSpace { body, .. } => return dictionary_space(body, right, scale),
        _ => return 0.,
    };
    let value = (if right {
        definition.right
    } else {
        definition.left
    } + 1) as f64
        / 18.;
    if scale < 1. {
        value.min(2. / 18.)
    } else {
        value
    }
}

fn space(atom: &Atom, dimension: Option<Dimension>, right: bool, scale: f64) -> f64 {
    dimension.map_or_else(
        || dictionary_space(atom, right, scale),
        |d| d.em(1., scale).max(0.),
    )
}

pub(super) fn core(atom: &Atom) -> Option<(Option<Dimension>, Option<Dimension>)> {
    match atom {
        Atom::MathMLSpace { left, right, .. } => Some((*left, *right)),
        Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Scripts { base: a, .. }
        | Atom::Multiscripts { base: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Padded { body: a, .. } => core(a),
        Atom::Sequence(v) if operator_dictionary::embellished(atom) => v
            .iter()
            .find(|a| !operator_dictionary::flag_spacelike(a))
            .and_then(core),
        _ => None,
    }
}

pub(super) fn row_spaces(children: &[Atom], index: usize, scale: f64) -> (f64, f64) {
    if children.len() <= 1 {
        return (0.0, 0.0);
    }
    let Some((left, right)) = core(&children[index]) else {
        return (0.0, 0.0);
    };
    // Multi-character mo tokens have no dictionary entry, so their fallback
    // operator spaces are zero. Explicit dimensions are clamped at zero.
    let right = space(&children[index], right, true, scale);
    let previous_right = if index > 0 && operator_dictionary::embellished(&children[index - 1]) {
        core(&children[index - 1]).map_or(0.0, |(_, r)| space(&children[index - 1], r, true, scale))
    } else {
        0.0
    };
    let left = (space(&children[index], left, false, scale) - previous_right).max(0.0);
    (left, right)
}

// TeXAtoms assign their own class even when their operator core uses
// MathML spacing. Transparent wrappers propagate the core's nil return.
pub(super) fn resets_tex_class(atom: &Atom) -> bool {
    match atom {
        Atom::Ord(_) | Atom::Class { .. } => false,
        Atom::MathMLSpace { .. } => true,
        Atom::Scripts { base: a, .. }
        | Atom::Multiscripts { base: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Padded { body: a, .. } => resets_tex_class(a),
        Atom::Sequence(v) if operator_dictionary::embellished(atom) => v
            .iter()
            .find(|a| !operator_dictionary::flag_spacelike(a))
            .is_some_and(resets_tex_class),
        _ => false,
    }
}
