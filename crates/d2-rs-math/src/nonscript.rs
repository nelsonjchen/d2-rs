// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/nonscript.go.
use super::*;

pub(super) fn space(atom: Atom) -> Atom {
    let eligible = match &atom {
        Atom::Space(_) | Atom::Kern(_) | Atom::RuleSpace { .. } => true,
        // spacer()'s NotParent mstyle is distinct from an authored StyleItem.
        Atom::DeclaredStyle {
            body,
            level: 0,
            display: false,
        } => {
            matches!(body.as_ref(), Atom::Space(_))
        }
        _ => false,
    };
    if eligible {
        Atom::NonscriptSpace {
            body: Box::new(atom),
            remove: false,
        }
    } else {
        atom
    }
}

// Run after inheritance: under/over inheritance can revisit a candidate with
// a different level. Remove only registered nodes, preserving authored rows.
// Sequence is the parser's inferred row and collapses a sole child in the
// source. Carry its real parent through that collapse for the arity guard.
pub(super) fn filter(atom: &mut Atom, required: Option<&str>) -> Result<bool, String> {
    if let Atom::NonscriptSpace { body, remove } = atom {
        if *remove {
            if let Some(parent) = required {
                return Err(format!(
                    "nonscript removal leaves {parent} without a required child"
                ));
            }
            *atom = Atom::Sequence(Vec::new());
            return Ok(true);
        }
        *atom = *body.clone();
    }
    match atom {
        Atom::Sequence(nodes) if nodes.len() == 1 => {
            if filter(&mut nodes[0], required)? {
                nodes.clear();
            }
        }
        Atom::Sequence(nodes) if required.is_some() => {
            let before = nodes.len();
            let mut index = 0;
            while index < nodes.len() {
                if filter(&mut nodes[index], None)? {
                    nodes.remove(index);
                } else {
                    index += 1;
                }
            }
            // A multi-child ParseArg mrow already exists before the filter;
            // removing a space must not collapse that row to its last token.
            if before > 1 && nodes.len() < 2 {
                *atom = Atom::ExplicitRow(std::mem::take(nodes));
            }
        }
        Atom::Sequence(nodes)
        | Atom::InferredRow(nodes)
        | Atom::ExplicitRow(nodes)
        | Atom::Fenced { body: nodes, .. }
        | Atom::Identifier(nodes)
        | Atom::OperatorToken(nodes)
        | Atom::Number(nodes)
        | Atom::StyledText(nodes) => {
            let mut index = 0;
            while index < nodes.len() {
                if filter(&mut nodes[index], None)? {
                    nodes.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        Atom::Fraction(a, b) | Atom::GeneralFraction(a, b, _) | Atom::SplitFraction(a, b, _) => {
            filter(a, Some("mfrac"))?;
            filter(b, Some("mfrac"))?;
        }
        Atom::Root { body, index, .. } => {
            filter(body, Some("mroot"))?;
            filter(index, Some("mroot"))?;
        }
        Atom::Scripts {
            base,
            sub,
            sup,
            limits,
            ..
        } => {
            let parent = match (*limits, sub.is_some(), sup.is_some()) {
                (true, true, true) => "munderover",
                (true, true, false) => "munder",
                (true, false, _) => "mover",
                (false, true, true) => "msubsup",
                (false, true, false) => "msub",
                _ => "msup",
            };
            filter(base, Some(parent))?;
            for child in [sub, sup].into_iter().flatten() {
                filter(child, Some(parent))?;
            }
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            // mmultiscripts has minimum arity one, not fixed arity: its
            // remaining script children prevent the required-child error.
            filter(base, None)?;
            filter(sub, None)?;
            filter(sup, None)?;
            if let Some(post) = post {
                for child in post {
                    filter(child, None)?;
                }
            }
        }
        Atom::XArrow { over, under, .. } => {
            filter(
                over,
                Some(if under.is_some() {
                    "munderover"
                } else {
                    "mover"
                }),
            )?;
            if let Some(under) = under {
                filter(under, Some("munderover"))?;
            }
        }
        Atom::CdArrow { over, under, .. } => {
            for child in [over, under].into_iter().flatten() {
                filter(child, None)?;
            }
        }
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            for child in rows.iter_mut().flatten() {
                filter(child, None)?;
            }
        }
        Atom::Choice(branches) => {
            for child in branches {
                filter(child, None)?;
            }
        }
        Atom::HorizontalMark { body, .. } => {
            filter(body, None)?;
        }
        Atom::BracketMark { body, .. }
        | Atom::Sqrt(body)
        | Atom::SizedSqrt { body, .. }
        | Atom::Reference { body, .. }
        | Atom::NotToken { first: body, .. }
        | Atom::Boxed(body)
        | Atom::Mrow(body)
        | Atom::NamedFunction(body)
        | Atom::AutoOperator(body)
        | Atom::MathMLSpace { body, .. }
        | Atom::RawOperator { body, .. }
        | Atom::Operator(body)
        | Atom::MovesScripts(body)
        | Atom::ScriptPolicy { body, .. }
        | Atom::Ord(body)
        | Atom::Class { body, .. }
        | Atom::Phantom(body)
        | Atom::Padded { body, .. }
        | Atom::CrampedStyle(body, _)
        | Atom::DeclaredStyle { body, .. }
        | Atom::DeclaredSize { body, .. }
        | Atom::ColorBox { body, .. }
        | Atom::Color { body, .. }
        | Atom::Cancel { body, .. } => {
            filter(body, None)?;
        }
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn nonscript_removes_only_the_next_space_after_inherited_style() {
        for (source, equivalent) in [
            (r"a\nonscript\,b", r"a\,b"),
            (r"x^{a\nonscript\,b}", "x^{ab}"),
            (r"x^{a\nonscript\quad b}", "x^{ab}"),
            (r"\scriptstyle a\nonscript\kern1em b", r"\scriptstyle ab"),
            (r"\frac{a\nonscript\,b}{c}", r"\frac{a\,b}{c}"),
            (r"\tfrac{a\nonscript\,b}{c}", r"\tfrac{ab}{c}"),
            (r"x^{a\nonscript\Rule{1em}{1em}{1em}b}", "x^{ab}"),
            (r"x^{a\nonscript\,\,b}", r"x^{a\,b}"),
        ] {
            assert_eq!(render_exact(source), render_exact(equivalent), "{source}");
        }
        // A pre-existing multi-child argument row survives removal, even when
        // only one token remains. It must not be rebuilt as a bare mi.
        let svg = render_exact(r"\tfrac{\nonscript\,a}{x}").unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let fraction = document
            .descendants()
            .find(|n| n.attribute("data-mml-node") == Some("mfrac"))
            .unwrap();
        assert_eq!(
            fraction
                .children()
                .find(|n| n.is_element())
                .unwrap()
                .attribute("data-mml-node"),
            Some("mrow")
        );
    }

    #[test]
    fn nonscript_observes_stack_boundaries_empty_results_and_registration() {
        for (source, equivalent) in [
            (r"x^{a\nonscript\label{a}\,b}", r"x^{ab}"),
            (r"x^{a\nonscript\bf\,b}", r"x^{a\bf b}"),
            (r"x^{a\nonscript{\,}\,b}", r"x^{a{\,}\,b}"),
            (r"x^{a\nonscript\color{red}\,b}", r"x^{a\color{red}\,b}"),
            (r"x^{a\nonscript\dots\,b}", r"x^{a\dots\,b}"),
            (
                r"\DeclareMathOperator{\nonscript}{f}\nonscript x",
                r"\DeclareMathOperator{\probe}{f}\probe x",
            ),
        ] {
            assert_eq!(render_exact(source), render_exact(equivalent), "{source}");
        }
        for (source, message) in [
            (r"x^\nonscript", "Missing open brace for superscript"),
            (r"x_\nonscript", "Missing open brace for subscript"),
            (
                r"\sum\nonscript\limits",
                r"\limits is allowed only on operators",
            ),
            (
                r"\begin{array}{c}\nonscript\hfill a\end{array}",
                r"Unsupported use of \hfill",
            ),
            (
                r"\begin{array}{c}\nonscript\hline a\end{array}",
                r"Misplaced \hline",
            ),
            (
                r"\begin{multline}\nonscript\shoveleft a\end{multline}",
                r"\shoveleft only allowed in multline environment",
            ),
        ] {
            assert_eq!(
                render(source).unwrap(),
                cd::render_error(message),
                "{source}"
            );
        }
    }

    #[test]
    fn nonscript_required_child_failures_are_conversion_errors_after_parse_errors() {
        for (source, parent) in [
            (r"\tfrac{\nonscript\,}{x}", "mfrac"),
            (r"\tfrac{\nonscript\quad}{x}", "mfrac"),
            (r"\sqrt[\nonscript\,]{x}", "mroot"),
            (r"\overset{\nonscript\,}{x}", "mover"),
            (r"\underset{\nonscript\,}{x}", "munder"),
            (r"\tfrac{\nonscript\,^2}{x}", "msup"),
        ] {
            let expected = format!("nonscript removal leaves {parent} without a required child");
            assert!(
                matches!(render_exact(source), Err(MathError::SourceConversion(message)) if message == expected),
                "{source}"
            );
            assert!(
                matches!(measure(source), Err(MathError::SourceConversion(message)) if message == expected),
                "{source}"
            );
            assert_eq!(
                render(&format!(r"{source}\UndefinedProbe")).unwrap(),
                cd::render_error(r"Undefined control sequence \UndefinedProbe")
            );
        }
        // An authored group survives removal of its contained space.
        assert!(render_exact(r"\tfrac{{\nonscript\,}}{x}").is_ok());
    }
}
