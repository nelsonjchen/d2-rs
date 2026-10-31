// Copyright (c) 2020-2022 MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: tex/mathtools_handlers_commands.go mathtoolsAdjustLimits;
// newcommand.go readDelimitedParameter and commands.go parseExpansion.
use super::*;
impl Parser {
    fn limits_parameter(&mut self, name: &str) -> Result<String, String> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        let start = self.index;
        let mut depth = 0usize;
        while self.index < self.chars.len() {
            if depth == 0 && self.chars[self.index] == '_' {
                let raw = self.chars[start..self.index].iter().collect();
                self.index += 1;
                return Ok(raw);
            }
            match self.chars[self.index] {
                '\\' => {
                    self.index += 1;
                    self.capture_control_sequence();
                }
                '{' => {
                    depth += 1;
                    self.index += 1;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    self.index += 1;
                }
                _ => self.index += 1,
            }
        }
        Err(format!("Runaway argument for \\{name}"))
    }
    pub(super) fn adjust_limits(&mut self, name: &str) -> Option<Atom> {
        let read = |p: &mut Self| -> Result<(String, String, String, String), String> {
            let first = p.limits_parameter(name)?;
            let first_sub = p.padded_raw_argument(name)?;
            let second = p.limits_parameter(name)?;
            let second_sub = p.padded_raw_argument(name)?;
            Ok((first, first_sub, second, second_sub))
        };
        let (first, a, second, b) = match read(self) {
            Ok(v) => v,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let operator = |base: &str, peer: &str, sub: &str, sub_peer: &str| {
            [
                r"\mathop{{",
                base,
                r"}\vphantom{{",
                peer,
                r"}}}_{{",
                sub,
                r"}\vphantom{{",
                sub_peer,
                r"}}}",
            ]
            .concat()
        };
        let expansion = operator(&first, &second, &a, &b) + &operator(&second, &first, &b, &a);
        let atom = self.padded_parse_string(&expansion)?;
        if cd::first_error(&atom).is_some() {
            return Some(atom);
        }
        let mut nodes = match atom {
            Atom::Sequence(v) => v,
            a => vec![a],
        };
        if nodes.len() == 2 {
            nodes.insert(1, Atom::ApplyFunction);
        }
        self.splice_inferred = true;
        Some(Atom::Sequence(nodes))
    }
}
