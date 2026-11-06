// Copyright (c) 2019-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/physics_matrix_quantity.go and
// buildrel.go readUpTo (TexParser.GetUpTo).
use super::*;

impl Parser {
    fn matrix_up_to(&mut self, name: &str, token: char) -> Result<String, String> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| internal_text::space(*c))
        {
            self.index += 1;
        }
        let start = self.index;
        let mut depth = 0;
        while self.index < self.chars.len() {
            let end = self.index;
            while self
                .chars
                .get(self.index)
                .is_some_and(|c| internal_text::space(*c))
            {
                self.index += 1;
            }
            let Some(c) = self.chars.get(self.index).copied() else {
                break;
            };
            self.index += 1;
            match c {
                '\\' => {
                    self.capture_control_sequence();
                    continue;
                }
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        return Err(format!("Extra close brace while looking for {token}"));
                    }
                    depth -= 1;
                }
                _ => {}
            }
            if depth == 0 && c == token {
                return Ok(self.chars[start..end].iter().collect());
            }
        }
        Err(format!("Could not find {token} for \\{name}"))
    }

    pub(super) fn matrix_quantity(&mut self, name: &str) -> Option<Atom> {
        // The small S-style table's metrics are not translated yet. Keep the
        // established explicit refusal rather than claim exact layout from the
        // ordinary smallmatrix environment (whose empty first group differs).
        if matches!(name, "smqty" | "smallmatrixquantity") {
            return None;
        }
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| internal_text::space(*c))
        {
            self.index += 1;
        }
        let star = self.chars.get(self.index) == Some(&'*');
        if star {
            self.index += 1;
        }
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| internal_text::space(*c))
        {
            self.index += 1;
        }
        let array = if matches!(name, "smqty" | "smallmatrixquantity") {
            "smallmatrix"
        } else {
            "array"
        };
        let (mut open, mut close) = ("", "");
        let raw = match self.chars.get(self.index).copied() {
            Some('{') => self.padded_raw_argument(name),
            Some(next @ ('(' | '[' | '|')) => {
                self.index += 1;
                (open, close) = match next {
                    '(' if star => ("\\lgroup", "\\rgroup"),
                    '(' => ("(", ")"),
                    '[' => ("[", "]"),
                    _ => ("|", "|"),
                };
                self.matrix_up_to(
                    name,
                    match next {
                        '(' => ')',
                        '[' => ']',
                        _ => '|',
                    },
                )
            }
            _ => {
                (open, close) = ("(", ")");
                Ok(String::new())
            }
        };
        let raw = match raw {
            Ok(raw) => raw,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let mut expansion = format!("\\begin{{{array}}}{{}} {raw}\\end{{{array}}}");
        if !open.is_empty() {
            expansion = format!("\\left{open}{expansion}\\right{close}");
        }
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let parsed = self.padded_parse_string(&expansion);
        self.ams.borrow_mut().paired_macro_count = count;
        self.splice_inferred = true;
        parsed
    }
}
