// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/pending_dots.go and parser.go delivery.
use super::*;

#[derive(Default)]
pub(super) struct Pending(Option<[Atom; 2]>, bool, bool);

impl Pending {
    pub fn start(parser: &Parser) -> Self {
        // Both candidates belong to the creating parser, even when a later
        // SetFont changes the environment before the successor is delivered.
        Self(
            Some([0x2026, 0x22ef].map(|code| Atom::RawOperator {
                attributes: &[],
                cleanup: false,
                texclass: None,
                code,
                body: Box::new(parser.nonambient_font(Atom::Glyph(code))),
                definition: operator_dictionary::lookup(code, operator_dictionary::Form::Infix),
            })),
            false,
            false,
        )
    }
    pub fn start_not() -> Self {
        Self(None, true, false)
    }
    pub fn start_nonscript() -> Self {
        Self(None, false, true)
    }
    pub fn active(&self) -> bool {
        self.0.is_some() || self.1 || self.2
    }
    pub fn finish(&mut self, positions: &mut Vec<position::Position>, children: &mut Vec<Atom>) {
        self.2 = false;
        if std::mem::take(&mut self.1) {
            children.extend(position::deliver(positions, pending_not::fallback()));
        }
        if let Some([lower, _]) = self.0.take() {
            children.extend(position::deliver(positions, lower));
        }
    }
    pub fn deliver(&mut self, positions: &mut Vec<position::Position>, atom: Atom) -> Vec<Atom> {
        if !self.active() {
            return position::deliver(positions, atom);
        }
        let mut nodes = Vec::new();
        let mut atom = atom;
        if std::mem::take(&mut self.2) {
            atom = nonscript::space(atom);
        }
        if std::mem::take(&mut self.1) && !pending_not::apply(&mut atom) {
            nodes.extend(position::deliver(positions, pending_not::fallback()));
        }
        if let Some([lower, upper]) = self.0.take() {
            // FnItem is an open stack item, so it settles Dots with no MML
            // successor. Its eventual payload must not select centered dots.
            let centered =
                !physics::named_function(&atom) && matches!(core_class(&atom), Some(2 | 3));
            nodes.extend(position::deliver(
                positions,
                if centered { upper } else { lower },
            ));
        }
        nodes.extend(position::deliver(positions, atom));
        nodes
    }
}

// dotsCoreMO follows only embellished cores. A TeXAtom's own assigned class
// does not change the underlying mo getter; an mi in mathbin is still an mi.
fn core_class(atom: &Atom) -> Option<i8> {
    match atom {
        Atom::RawOperator { code, texclass, .. } => Some(texclass.unwrap_or_else(|| {
            operator_dictionary::lookup_registered(*code, operator_dictionary::Form::Infix)
                .map_or(3, |d| d.class)
        })),
        Atom::Glyph(code)
        | Atom::VariantGlyph(code)
        | Atom::FontGlyph { original: code, .. }
        | Atom::LiteralGlyph(code) => Some(
            operator_dictionary::lookup_registered(*code, operator_dictionary::Form::Infix)
                .map_or(3, |d| d.class),
        ),
        Atom::Sequence(nodes) | Atom::InferredRow(nodes) | Atom::ExplicitRow(nodes) => {
            let mut cores = nodes
                .iter()
                .filter(|a| !operator_dictionary::flag_spacelike(a));
            let first = cores.next()?;
            if cores.next().is_some() {
                None
            } else {
                core_class(first)
            }
        }
        Atom::MathMLSpace { body, .. } if matches!(body.as_ref(), Atom::OperatorToken(_)) => {
            Some(3)
        }
        Atom::Ord(body)
        | Atom::Class { body, .. }
        | Atom::Mrow(body)
        | Atom::MathMLSpace { body, .. }
        | Atom::ScriptPolicy { body, .. }
        | Atom::MovesScripts(body)
        | Atom::Scripts { base: body, .. }
        | Atom::Multiscripts { base: body, .. }
        | Atom::Phantom(body)
        | Atom::Padded { body, .. }
        | Atom::CrampedStyle(body, _)
        | Atom::DeclaredStyle { body, .. }
        | Atom::DeclaredSize { body, .. }
        | Atom::Color { body, .. }
        | Atom::Operator(body)
        | Atom::NamedFunction(body)
        | Atom::AutoOperator(body) => core_class(body),
        Atom::Fraction(body, _) | Atom::GeneralFraction(body, _, _) => core_class(body),
        Atom::XArrow { .. } => Some(3),
        _ => None,
    }
}

// DotsItem ends an earlier FnItem without an ApplyFunction. Keep its MML
// payload and scripts, dropping only the parser's pending-function marker.
pub(super) fn finish_function(atom: &mut Atom) {
    match atom {
        Atom::NamedFunction(body) => *atom = *body.clone(),
        Atom::Scripts { base, .. } | Atom::ScriptPolicy { body: base, .. } => finish_function(base),
        _ => {}
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    fn first_code(source: &str) -> String {
        let svg = render_exact(source).unwrap();
        let doc = roxmltree::Document::parse(&svg).unwrap();
        doc.descendants()
            .find_map(|n| n.attribute("data-c"))
            .unwrap()
            .to_owned()
    }

    #[test]
    fn dots_select_the_final_embellished_operator_core_before_inheritance() {
        for tail in [
            "+x",
            "=x",
            "{+}",
            r"\mathord{+}",
            r"\frac{+}{x}",
            r"\overset{a}{+}",
            r"\phantom{+}",
            r"\mmlToken{mo}[lspace=1em]{foo}",
        ] {
            assert_eq!(first_code(&format!(r"\dots {tail}")), "22EF", "{tail}");
        }
        for tail in [
            "",
            "x",
            "@",
            "{}+",
            "{x+y}",
            r"\mathbin{x}",
            r"\mathrel{x}",
            r"\left(+\right)",
            r"\boxed{+}",
            r"\sin x",
            r"\text{a}+",
        ] {
            assert_eq!(first_code(&format!(r"\dots {tail}")), "2026", "{tail}");
        }
    }

    #[test]
    fn dots_finish_on_stack_boundaries_and_scripts_start_with_empty_bases() {
        for source in [r"x^\dots", r"x_\dots"] {
            assert_eq!(
                render(source).unwrap(),
                cd::render_error(if source.contains('^') {
                    "Missing open brace for superscript"
                } else {
                    "Missing open brace for subscript"
                })
            );
        }
        for name in ["limits", "nolimits"] {
            assert_eq!(
                render(&format!(r"\dots\{name}")).unwrap(),
                cd::render_error(&format!(r"\{name} is allowed only on operators"))
            );
        }
        for source in [r"\dots_i", r"\dots^2", r"\dots'", r"\raise1em\dots'"] {
            let svg = render_exact(source).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let script = doc
                .descendants()
                .find(|n| matches!(n.attribute("data-mml-node"), Some("msup" | "msub")))
                .unwrap();
            let base = script.children().find(|n| n.is_element()).unwrap();
            assert_eq!(base.attribute("data-mml-node"), Some("mi"), "{source}");
            assert!(
                !base.descendants().any(|n| n.attribute("data-c").is_some()),
                "{source}"
            );
        }
    }

    #[test]
    fn dots_keep_creation_scope_and_ignore_empty_macro_results() {
        for (source, code) in [
            (r"\dots\label{a}+", "22EF"),
            (r"\dots\bf +", "22EF"),
            (r"\dots\scriptstyle +", "2026"),
            (r"\dots\Huge +", "2026"),
            (r"\dots\color{red}+", "2026"),
            (r"\dots\dots+", "2026"),
        ] {
            assert_eq!(first_code(source), code, "{source}");
        }
        assert!(
            !render_exact(r"\sin\dots x")
                .unwrap()
                .contains("data-c=\"2061\"")
        );
        // A dynamic command-map registration precedes the built-in DotsItem.
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\dots}{f}\dots x"),
            render_exact(r"\DeclareMathOperator{\probe}{f}\probe x")
        );
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\dots}{f}x^\dots"),
            render_exact(r"\DeclareMathOperator{\probe}{f}x^\probe")
        );
    }
}
