// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 internal/tex/framebox.go.
use super::*;

impl Parser {
    pub(super) fn frame_box(&mut self) -> Option<Atom> {
        let mut options = [String::new(), String::new()];
        for option in &mut options {
            // GetBrackets uses ECMAScript whitespace (BOM, but not NEL).
            while self
                .chars
                .get(self.index)
                .copied()
                .is_some_and(internal_text::space)
            {
                self.index += 1;
            }
            if self.chars.get(self.index) != Some(&'[') {
                continue;
            }
            *option = match self.padded_brackets() {
                Ok(value) => value,
                Err(message) => {
                    return Some(Atom::ErrorDisplay(
                        if message == "Could not find closing ']' for argument" {
                            format!("{message} to \\framebox")
                        } else {
                            message
                        },
                    ));
                }
            };
        }
        while self
            .chars
            .get(self.index)
            .copied()
            .is_some_and(internal_text::space)
        {
            self.index += 1;
        }
        // GetArgumentAtCursor must not apply Rust/Go Unicode whitespace here.
        let raw = match self.chars.get(self.index).copied() {
            None => return Some(Atom::ErrorDisplay("Missing argument for \\framebox".into())),
            Some('}') => {
                return Some(Atom::ErrorDisplay(
                    "Extra close brace or missing open brace".into(),
                ));
            }
            Some('{') => match self.braced_text() {
                Some(raw) => raw,
                None => return Some(Atom::ErrorDisplay("Missing close brace".into())),
            },
            Some('\\') => {
                self.index += 1;
                format!("\\{}", self.capture_control_sequence())
            }
            Some(c) => {
                self.index += 1;
                c.to_string()
            }
        };
        let mut content = self.internal_math(&raw, None, false)?;
        if !options[0].is_empty() {
            content = Atom::Padded {
                body: Box::new(content),
                padding: padded::Padding {
                    aligned_width: Some((
                        options[0].clone(),
                        match options[1].as_str() {
                            "l" => "left",
                            "r" => "right",
                            _ => "center",
                        },
                        1.,
                    )),
                    ..Default::default()
                },
            };
        }
        Some(Atom::Ord(Box::new(Atom::Boxed(Box::new(content)))))
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn framebox_text_and_empty_width_keep_ord_enclosure_ownership() {
        for text in ["abc", "", "a $x^2$ b", r"\TeX", "a~b"] {
            assert_eq!(
                render_exact(&format!(r"\framebox{{{text}}}")),
                render_exact(&format!(r"\mathord{{\fbox{{{text}}}}}")),
                "{text}"
            );
            assert_eq!(
                render_exact(&format!(r"\framebox[][r]{{{text}}}")),
                render_exact(&format!(r"\framebox{{{text}}}"))
            );
        }
    }

    #[test]
    fn framebox_width_alignment_preserves_wrapper_and_negative_delta_policy() {
        let mut viewports = Vec::new();
        for (option, align, translation) in [
            ("", "center", Some("translate(1236,0)")),
            ("[l]", "left", None),
            ("[r]", "right", Some("translate(2472,0)")),
            ("[other]", "center", Some("translate(1236,0)")),
        ] {
            let svg = render_exact(&format!(r"\framebox[3em]{option}{{x}}")).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            viewports.push(doc.root_element().attribute("viewBox").unwrap().to_owned());
            let padded = doc
                .descendants()
                .find(|n| n.attribute("data-align").is_some())
                .unwrap();
            assert_eq!(padded.attribute("data-align"), Some(align));
            assert_eq!(
                padded
                    .children()
                    .find(|n| n.is_element())
                    .unwrap()
                    .attribute("transform"),
                translation
            );
            let svg = render_exact(&format!(r"\framebox[.1em]{option}{{x}}")).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let padded = doc
                .descendants()
                .find(|n| n.attribute("data-align").is_some())
                .unwrap();
            assert_eq!(
                padded
                    .children()
                    .find(|n| n.is_element())
                    .unwrap()
                    .attribute("data-mml-node"),
                Some("mtext")
            );
        }
        assert!(viewports.windows(2).all(|p| p[0] == p[1]));
    }

    #[test]
    fn framebox_owns_option_diagnostics_whitespace_and_registration_priority() {
        for (source, message) in [
            (r"\framebox", r"Missing argument for \framebox"),
            (
                r"\framebox[",
                r"Could not find closing ']' for argument to \framebox",
            ),
            (
                r"\framebox[1em][",
                r"Could not find closing ']' for argument to \framebox",
            ),
            (r"\framebox[}]", "Extra close brace while looking for ']'"),
            (r"\framebox{", "Missing close brace"),
        ] {
            assert_eq!(render(source).unwrap(), cd::render_error(message));
        }
        assert_eq!(
            render_exact("\\framebox\u{feff}{a}"),
            render_exact(r"\framebox{a}")
        );
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\framebox}{f}\framebox x"),
            render_exact(r"\DeclareMathOperator{\probe}{f}\probe x")
        );
    }
}
