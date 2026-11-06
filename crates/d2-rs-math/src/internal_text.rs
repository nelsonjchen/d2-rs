// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/internal_text.go: ParseUtil.internalMath/internalText and HBox.
use super::*;

pub(super) fn space(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
        '\u{205f}' | '\u{3000}' | '\u{feff}')
}
impl Parser {
    // tex/internal_text.go internalMath resolves an omitted text font from
    // the owning parser environment, independently of MathFont eligibility.
    pub(super) fn internal_text(&self, text: String) -> Atom {
        let trimmed = text.trim_start_matches(space);
        let text = if trimmed != text {
            format!("\u{a0}{trimmed}")
        } else {
            text
        };
        let trimmed = text.trim_end_matches(space);
        let text = if trimmed != text {
            format!("{trimmed}\u{a0}")
        } else {
            text
        };
        if self.active_font.is_none() {
            return Atom::Text(text);
        }
        let mut children = Atom::Sequence(
            text.chars()
                .map(|c| {
                    if self.active_font == Some("italic") {
                        Atom::Glyph(c as u32)
                    } else {
                        Atom::LiteralGlyph(c as u32)
                    }
                })
                .collect(),
        );
        children = self.ambient_font(children);
        let Atom::Sequence(children) = children else {
            unreachable!()
        };
        Atom::StyledText(children)
    }

    // Configuration is shared, but lexical fonts, stack and expansion count
    // belong to the fresh TexParser. Tokens resolve before returning to caller.
    pub(super) fn parse_internal_math(&self, source: &str) -> Option<Atom> {
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let mut child = self.padded_child(source);
        child.active_font = None;
        child.normal_font = false;
        child.calligraphic_font = false;
        child.multi_letter_font = false;
        let result = child.sequence(None);
        self.ams.borrow_mut().paired_macro_count = count;
        result
    }

    pub(super) fn internal_math(
        &self,
        text: &str,
        font: Option<&'static str>,
        level_zero: bool,
    ) -> Option<Atom> {
        let font = font.or(self.active_font);
        let mut literal = self.padded_child("");
        literal.active_font = font;
        let mut text: Vec<char> = text.chars().collect();
        let (mut i, mut start, mut braces) = (0, 0, 0usize);
        let mut matching = None;
        let mut nodes = Vec::new();
        while i < text.len() {
            let c = text[i];
            i += 1;
            let mut end = None;
            let mut reference = false;
            match c {
                '$' if matching == Some('$') && braces == 0 => end = Some(i - 1),
                '$' if matching.is_none() => {
                    if start < i - 1 {
                        nodes.push(literal.internal_text(text[start..i - 1].iter().collect()));
                    }
                    matching = Some('$');
                    start = i;
                }
                '{' if matching.is_some() => braces += 1,
                '}' if matching == Some('}') && braces == 0 => {
                    end = Some(i);
                    reference = true;
                }
                '}' if matching.is_some() && braces > 0 => braces -= 1,
                '\\' => {
                    if matching.is_none() {
                        let rest: String = text[i..].iter().collect();
                        let prefix = if rest.starts_with("eqref") {
                            5
                        } else if rest.starts_with("ref") {
                            3
                        } else {
                            0
                        };
                        if prefix > 0 {
                            let mut next = i + prefix;
                            while text.get(next).is_some_and(|c| space(*c)) {
                                next += 1;
                            }
                            if text.get(next) == Some(&'{') {
                                if start < i - 1 {
                                    nodes.push(
                                        literal.internal_text(text[start..i - 1].iter().collect()),
                                    );
                                }
                                matching = Some('}');
                                start = i - 1;
                                i = next + 1;
                                continue;
                            }
                        }
                    }
                    if i == text.len() {
                        continue;
                    }
                    let escaped = text[i];
                    i += 1;
                    if escaped == '(' && matching.is_none() {
                        if start < i - 2 {
                            nodes.push(literal.internal_text(text[start..i - 2].iter().collect()));
                        }
                        matching = Some(')');
                        start = i;
                    } else if escaped == ')' && matching == Some(')') && braces == 0 {
                        end = Some(i - 2);
                    } else if matching.is_none() && matches!(escaped, '$' | '{' | '}' | '\\') {
                        i -= 1;
                        text.remove(i - 1);
                    }
                }
                _ => {}
            }
            if let Some(end) = end {
                let source: String = text[start..end].iter().collect();
                let mut child = self.parse_internal_math(&source)?;
                if cd::first_error(&child).is_some() {
                    return Some(child);
                }
                if reference && let Some(font) = font {
                    declaration_fonts::apply_wrapper(&mut child, font);
                }
                nodes.push(Atom::Ord(Box::new(child)));
                matching = None;
                start = i;
            }
        }
        if matching.is_some() {
            return Some(Atom::ErrorDisplay("Math not terminated in text box".into()));
        }
        if start < text.len() {
            nodes.push(literal.internal_text(text[start..].iter().collect()));
        }
        let body = match nodes.len() {
            1 => nodes.pop().unwrap(),
            _ if nodes.is_empty() || level_zero => Atom::Sequence(nodes),
            _ => Atom::Mrow(Box::new(Atom::Sequence(nodes))),
        };
        Some(if level_zero {
            Atom::DeclaredStyle {
                body: Box::new(body),
                display: false,
                level: 0,
            }
        } else {
            body
        })
    }
    pub(super) fn hbox_command(&mut self, name: &str) -> Option<Atom> {
        let raw = match self.padded_raw_argument(name) {
            Ok(raw) => raw,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let font = match name {
            "textrm" | "textup" => Some("mathrm"),
            "textbf" => Some("mathbf"),
            "textit" => Some("italic"),
            "texttt" => Some("mathtt"),
            "textsf" => Some("mathsf"),
            _ => None,
        };
        self.internal_math(&raw, font, matches!(name, "hbox" | "mbox"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_math_delimiters_and_literal_escapes_keep_their_owners() {
        for (left, right) in [
            (r"\text{a $x$ b}", r"\text{a \(x\) b}"),
            (r"\text{\$\{\}\\}", r"\textrm{\$\{\}\\}"),
            (r"\mathbf{\text{$x$}}", r"\mathbf{\text{\(x\)}}"),
        ] {
            assert_eq!(render_exact(left).unwrap(), render_exact(right).unwrap());
        }
        assert!(render_exact(r"\text{a~b}").is_ok());
        assert!(
            render_exact(r"\text{a $x$ b}")
                .unwrap()
                .contains("data-mml-node=\"mrow\"")
        );
        assert!(
            render_exact(r"\mathbf{\text{$x$}}")
                .unwrap()
                .contains("data-c=\"1D465\"")
        );
        for source in [
            r"\text{abc $x}",
            r"\text{abc \(x}",
            r"\colorbox{red}{abc $x}",
            r"x\tag{abc $x}",
        ] {
            assert_eq!(
                render_exact(source).unwrap(),
                cd::render_error("Math not terminated in text box")
            );
        }
        // A tag's fresh parser errors discard the complete outer result.
        assert_eq!(
            render_exact(r"x\tag{a $\bad$ b}").unwrap(),
            cd::render_error(r"Undefined control sequence \bad")
        );
    }
    #[test]
    fn text_children_share_configuration_and_restore_the_expansion_counter() {
        let parser = Parser {
            in_root: false,
            root_moves: Default::default(),
            physics_after: None,
            physics_auto: None,
            command_position: None,
            pending_cell: None,
            pending_infix: None,
            row_delimiter: None,
            active_color: None,
            equation_seen: Default::default(),
            array_cell: None,
            vector_alias: false,
            vector_font: None,
            ams: Default::default(),
            splice_inferred: false,
            chars: Vec::new(),
            index: 0,
            active_font: Some("mathbf"),
            normal_font: false,
            calligraphic_font: false,
            multi_letter_font: false,
            operator_letters: false,
            operator_pattern: false,
            no_auto_operator: false,
        };
        parser.ams.borrow_mut().paired_macro_count = 1000;
        let child = parser
            .internal_math(r"$\DeclarePairedDelimiter{\foo}{(}{)}\foo{x}$", None, false)
            .unwrap();
        assert!(cd::first_error(&child).is_none());
        assert_eq!(parser.ams.borrow().paired_macro_count, 1000);
        assert!(parser.paired_definition("foo").is_some());
        let bad = parser.internal_math(r"$\bad$", None, false).unwrap();
        assert_eq!(
            cd::first_error(&bad),
            Some(r"Undefined control sequence \bad")
        );
        assert_eq!(parser.ams.borrow().paired_macro_count, 1000);
    }
}
