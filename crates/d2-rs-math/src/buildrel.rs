// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/buildrel.go and physics_derivative.go:parseChild.
use super::*;

impl Parser {
    // GetCS consumes one delimiter space after a control word; an absent
    // character or a non-dotall line ending denotes control-space.
    fn relation_control_sequence(&mut self) -> String {
        let start = self.index;
        while self
            .chars
            .get(self.index)
            .is_some_and(char::is_ascii_alphabetic)
        {
            self.index += 1;
        }
        if self.index != start {
            let name = self.chars[start..self.index].iter().collect();
            if self.chars.get(self.index) == Some(&' ') {
                self.index += 1;
            }
            return name;
        }
        match self.chars.get(self.index).copied() {
            None => " ".into(),
            Some(c) => {
                self.index += 1;
                if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                    " ".into()
                } else {
                    c.to_string()
                }
            }
        }
    }

    fn relation_skip_space(&mut self) {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| internal_text::space(*c))
        {
            self.index += 1;
        }
    }

    // GetUpTo searches lexical control sequences. Braces protect the token;
    // comments and macros belong to the independent child parser afterward.
    fn relation_read_up_to(&mut self) -> Result<String, String> {
        self.relation_skip_space();
        let start = self.index;
        let mut depth = 0;
        while self.index < self.chars.len() {
            let end = self.index;
            self.relation_skip_space();
            let Some(c) = self.chars.get(self.index).copied() else {
                break;
            };
            self.index += 1;
            match c {
                '\\' => {
                    let name = self.relation_control_sequence();
                    if depth == 0 && name == "over" {
                        return Ok(self.chars[start..end].iter().collect());
                    }
                }
                '{' => depth += 1,
                '}' if depth == 0 => {
                    return Err("Extra close brace while looking for \\over".into());
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        Err("Could not find \\over for \\buildrel".into())
    }

    fn relation_child(&self, raw: &str) -> Option<Atom> {
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let mut child = self.padded_child(raw);
        child.multi_letter_font = self.multi_letter_font;
        let result = child.sequence(None);
        self.ams.borrow_mut().paired_macro_count = count;
        Some(match result? {
            Atom::Sequence(mut children) if children.len() == 1 => children.pop().unwrap(),
            atom => atom,
        })
    }

    pub(super) fn build_relation(&mut self) -> Option<Atom> {
        let raw = match self.relation_read_up_to() {
            Ok(raw) => raw,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let top = self.relation_child(&raw)?;
        if cd::first_error(&top).is_some() {
            return Some(top);
        }
        self.relation_skip_space();
        let raw = match self.chars.get(self.index).copied() {
            None => return Some(Atom::ErrorDisplay("Missing argument for \\buildrel".into())),
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
                format!("\\{}", self.relation_control_sequence())
            }
            Some(c) => {
                self.index += 1;
                c.to_string()
            }
        };
        let base = self.relation_child(&raw)?;
        if cd::first_error(&base).is_some() {
            return Some(base);
        }
        // cleanSubSup reduces munderover(base, null, top) to mover.
        // BuildRel makes no Overset accent or movable-limit adjustments.
        Some(Atom::Class {
            class: 3,
            body: Box::new(Atom::Scripts {
                base: Box::new(base),
                sub: None,
                sup: Some(Box::new(top)),
                limits: true,
                accent: None,
                eager: false,
                leading_space: 0.,
            }),
        })
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn relation_search_protects_groups_and_orders_lexical_and_child_errors() {
        for (source, message) in [
            (
                r"\buildrel a\overline b",
                r"Could not find \over for \buildrel",
            ),
            (
                r"\buildrel {a\over b}",
                r"Could not find \over for \buildrel",
            ),
            (
                r"\buildrel a}\over b",
                r"Extra close brace while looking for \over",
            ),
            (
                r"\buildrel\undefined\over",
                r"Undefined control sequence \undefined",
            ),
            (r"\buildrel a\over", r"Missing argument for \buildrel"),
        ] {
            assert_eq!(
                render(source).unwrap(),
                cd::render_error(message),
                "{source}"
            );
        }
        let svg = render(r"x\buildrel a\over =y").unwrap();
        assert!(svg.contains("data-mml-node=\"mover\""));
        assert!(svg.contains("data-mml-node=\"TeXAtom\""));
    }

    #[test]
    fn relation_children_share_definitions_and_restore_the_callers_macro_count() {
        // buildrel_test.go: each ParseUpTo/ParseArg child owns a fresh
        // substitution counter, while definitions remain shared on failure.
        for source in [
            r"\buildrel \DeclarePairedDelimiter{\foo}{(}{)}\foo{x}\over{\foo{y}}",
            r"\buildrel \DeclarePairedDelimiter{\foo}{(}{)}\foo{x}\over\undefined",
            r"\buildrel \DeclarePairedDelimiter{\foo}{(}{)}\undefined\over{\foo{y}}",
        ] {
            let shared = ams::Shared::default();
            shared.borrow_mut().paired_macro_count = 1000;
            let parsed = parse_unresolved_shared(source, shared.clone()).unwrap();
            assert_eq!(shared.borrow().paired_macro_count, 1000);
            assert_eq!(
                cd::first_error(&parsed).is_some(),
                source.contains("undefined"),
                "{source}: {:?}",
                cd::first_error(&parsed)
            );
            shared.borrow_mut().paired_macro_count = 0;
            let reused = parse_unresolved_shared(r"\foo{z}", shared.clone()).unwrap();
            assert!(cd::first_error(&reused).is_none());
        }
    }
}
