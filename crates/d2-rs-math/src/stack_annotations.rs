// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/commands.go: normalizeStackBase, overSet,
// underSet, and overUnderSet. These commands construct authored stacks;
// unlike stackrel/stackbin, they do not expand through a mathop macro.
use super::*;

// parseArgument returns its single child directly; a multi-child argument
// retains the inferred row. The Rust child parser initially returns Sequence.
fn argument_node(atom: Atom) -> Atom {
    match atom {
        Atom::Sequence(mut children) if children.len() == 1 => children.pop().unwrap(),
        atom => atom,
    }
}

impl Parser {
    pub(super) fn stack_annotation(&mut self, name: &str) -> Option<Atom> {
        let first = argument_node(self.padded_argument(name)?);
        if cd::first_error(&first).is_some() {
            return Some(first);
        }
        let second = if name == "overunderset" {
            let child = argument_node(self.padded_argument(name)?);
            if cd::first_error(&child).is_some() {
                return Some(child);
            }
            Some(Box::new(child))
        } else {
            None
        };
        let mut base = argument_node(self.padded_argument(name)?);
        if cd::first_error(&base).is_some() {
            return Some(base);
        }
        check_movable_limits(&mut base);
        // Rust retains the primary under/over family for movable scripts,
        // so normalizeStackBase needs no side-to-stack representation change.
        // UnderSet/OverUnderSet explicitly disable stack accents. OverSet
        // leaves the stack unset but disables a direct mo annotation's accent;
        // an embellished wrapper still inherits its deeper operator's flag.
        let direct_mo = match &first {
            Atom::RawOperator { .. } | Atom::OperatorToken(_) | Atom::HorizontalMark { .. } => true,
            Atom::Glyph(code) | Atom::FontGlyph { original: code, .. } => {
                mathml_node_kind(*code) == "mo"
            }
            _ => false,
        };
        let accent = (name != "overset" || direct_mo).then_some(accents::Spec {
            under: false,
            enabled: false,
            mathsize: 1.,
        });
        let (sub, sup) = if name == "underset" {
            (Some(Box::new(first)), None)
        } else {
            (second, Some(Box::new(first)))
        };
        Some(Atom::Scripts {
            base: Box::new(base),
            sub,
            sup,
            limits: true,
            accent,
            eager: false,
            leading_space: 0.,
        })
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn annotations_keep_authored_slots_and_follow_nested_movable_cores() {
        // commands.go + script_attachment.go: an authored annotation is the
        // base of a new side script, while occupied lower slots still fault.
        for (source, annotation) in [
            (r"\overset{a}{x}^b", "mover"),
            (r"\underset{a}{x}^b", "munder"),
        ] {
            let svg = render(source).unwrap();
            assert!(svg.contains("data-mml-node=\"msup\""), "{source}");
            assert!(
                svg.contains(&format!("data-mml-node=\"{annotation}\"")),
                "{source}"
            );
        }
        for source in [r"\underset{a}{x}_b", r"\overunderset{a}{c}{x}_b"] {
            assert_eq!(
                render(source).unwrap(),
                cd::render_error("Double subscripts: use braces to clarify")
            );
        }
        {
            let source = r"\overunderset{a}{c}{x}^b";
            assert_eq!(
                render(source).unwrap(),
                cd::render_error("Double exponent: use braces to clarify")
            );
        }
        // svg/munderover.go movableLimitCore follows the nested script's
        // operator attribute. The outer authored mover retains its kind but
        // places its annotation to the right in inline style.
        let svg = render(r"\textstyle \overset{a}{\sum_i^n}").unwrap();
        assert!(svg.contains("viewBox=\"0 -828.5 2020.3 1121.6\""));
        assert!(svg.contains("data-mml-node=\"mover\""));
        assert!(svg.contains("data-mml-node=\"munderover\""));
    }
}
