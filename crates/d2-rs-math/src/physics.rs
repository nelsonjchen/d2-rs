// Copyright 2018–2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/commands.go quantity, derivative, physicsDifferential,
// readUpToByte and parser.go readArgument/readBrackets.
use super::*;
fn children(a: Atom) -> Vec<Atom> {
    let a = collapsed(a);
    if let Atom::Sequence(v) | Atom::InferredRow(v) | Atom::ExplicitRow(v) = a {
        v
    } else {
        vec![a]
    }
}
// An mo with an explicit TeX class and mathvariant bold.
fn bold_operator(code: u32, class: i8) -> Option<Atom> {
    let mut body = Atom::Glyph(code);
    declaration_fonts::apply_wrapper(&mut body, "mathbf");
    if !matches!(body, Atom::Glyph(_) | Atom::FontGlyph { .. }) {
        return None;
    }
    let mut definition = operator_dictionary::lookup(code, operator_dictionary::Form::Infix);
    definition.class = class;
    Some(Atom::RawOperator {
        attributes: &[],
        cleanup: false,
        texclass: Some(class),
        code,
        body: Box::new(body),
        definition,
    })
}
pub(super) enum After {
    Derivative { ignore: bool },
    Eval { open: char, smash: bool },
    Application { name: String, fences: &'static str },
}
// physics_autoopen.go: a same-parser stack recipient. Raw AutoClose marks
// belong to delivered items; identical command-produced operators are unmarked.
pub(super) struct AutoOpen {
    open: char,
    close: char,
    count: i32,
    pub closed: bool,
    pub trailing: Vec<Atom>,
}
impl AutoOpen {
    pub fn stop_error() -> Atom {
        Atom::ErrorDisplay("Extra open or missing close delimiter".into())
    }
    pub fn accept(&mut self, atom: &Atom, marker: Option<char>) -> bool {
        let code = match atom {
            Atom::RawOperator { code, .. } => Some(*code),
            // clean_stretchy wraps raw ORD operators after parser delivery.
            Atom::Ord(body) => match body.as_ref() {
                Atom::RawOperator { code, .. } => Some(*code),
                _ => None,
            },
            _ => None,
        };
        if code == Some(self.open as u32) {
            self.count += 1;
        }
        if marker == Some(self.close) && code == Some(self.close as u32) {
            let before = self.count;
            self.count -= 1;
            self.closed = before == 0;
        }
        self.closed
    }
}
impl Parser {
    // vector_font.go applyVectorFactory runs at token creation, before an
    // enclosing font wrapper. Only one UTF-16 code unit is eligible.
    pub(super) fn vector_font_token(&self, mut atom: Atom) -> Atom {
        let Some(star) = self.vector_font else {
            return atom;
        };
        if self.active_font.is_some_and(|font| !font.is_empty()) {
            return atom;
        }
        fn code(atom: &Atom) -> Option<u32> {
            match atom {
                Atom::Glyph(c) | Atom::ItalicGlyph(c) | Atom::LiteralGlyph(c) => {
                    Some(original_font_code(*c))
                }
                Atom::FontGlyph { original, .. } => Some(*original),
                Atom::Identifier(v) | Atom::Number(v) | Atom::OperatorToken(v) if v.len() == 1 => {
                    code(&v[0])
                }
                Atom::RawOperator { body, .. } => code(body),
                _ => None,
            }
        }
        if code(&atom).is_some_and(|c| {
            matches!(c, 0x41..=0x5a | 0x61..=0x7a | 0x391..=0x3a9 | 0x30..=0x39)
                || star && (0x3b1..=0x3c9).contains(&c)
        }) {
            declaration_fonts::apply_wrapper(&mut atom, if star { "boldsymbol" } else { "mathbf" });
        }
        atom
    }
    pub(super) fn physics_vector_bold(&mut self, name: &str) -> Option<Atom> {
        let star = self.physics_star();
        let raw = match self.padded_raw_argument(name) {
            Ok(raw) => raw,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let mut parser = self.padded_child(&raw);
        parser.active_font = None;
        parser.normal_font = false;
        parser.calligraphic_font = false;
        parser.vector_font = Some(star);
        parser.multi_letter_font = self.multi_letter_font;
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let body = parser.sequence(None);
        self.ams.borrow_mut().paired_macro_count = count;
        // The pinned handler deletes the calling vector environment.
        self.vector_font = None;
        self.splice_inferred = true;
        body
    }
    pub(super) fn physics_complete_after(&mut self, after: After) -> Option<Atom> {
        while self.chars.get(self.index).is_some_and(Self::prime_space) {
            self.index += 1;
        }
        if let After::Eval { open, smash } = after {
            let (mut body, trailing) = self.physics_auto_body(open, '|')?;
            if cd::first_error(&body).is_some() {
                return Some(body);
            }
            if smash {
                body = Atom::Ord(Box::new(Atom::Padded {
                    body: Box::new(body),
                    padding: padded::Padding {
                        height_zero: true,
                        depth_zero: true,
                        ..Default::default()
                    },
                }));
            }
            let mut nodes = children(body);
            // AutoOpen appends the child's one inferred MML result.
            let phantom = self.padded_parse_string(r"\vphantom{\int}")?;
            nodes.push(match phantom {
                Atom::Sequence(v) => Atom::InferredRow(v),
                atom => atom,
            });
            let mut result =
                vec![self.physics_fence(open as u32, Atom::Sequence(nodes), '|' as u32, true)];
            result.extend(trailing);
            return Some(Atom::Sequence(result));
        }
        let (open, close, ignore) = match after {
            After::Eval { .. } => unreachable!(),
            After::Derivative { ignore } => {
                if self.chars.get(self.index) != Some(&'(') {
                    return Some(Atom::Sequence(Vec::new()));
                }
                ('(', ')', ignore)
            }
            After::Application { name, fences } => {
                let Some(&left) = self.chars.get(self.index) else {
                    return Some(Atom::Sequence(Vec::new()));
                };
                let enlarge = fences.contains(left);
                if left == '{' {
                    let raw = match self.padded_raw_argument(&name) {
                        Ok(raw) => raw,
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    };
                    let replacement = if enlarge {
                        format!("\\left\\{{ {raw} \\right\\}}")
                    } else {
                        format!(" {raw} ")
                    };
                    self.chars
                        .splice(self.index..self.index, replacement.chars());
                    return Some(Atom::Sequence(Vec::new()));
                }
                if !enlarge {
                    return Some(Atom::Sequence(Vec::new()));
                }
                (left, if left == '[' { ']' } else { ')' }, false)
            }
        };
        self.index += 1;
        let (body, trailing) = self.physics_auto_body(open, close)?;
        if cd::first_error(&body).is_some() {
            return Some(body);
        }
        let mut nodes = Vec::new();
        if !ignore {
            nodes.push(self.physics_fence(open as u32, body, close as u32, true));
        }
        nodes.extend(trailing);
        Some(Atom::Sequence(nodes))
    }
    fn physics_auto_body(&mut self, open: char, close: char) -> Option<(Atom, Vec<Atom>)> {
        let outer = self.physics_auto.replace(AutoOpen {
            open,
            close,
            count: 0,
            closed: false,
            trailing: Vec::new(),
        });
        let body = self.sequence(None);
        let auto = self.physics_auto.take()?;
        self.physics_auto = outer;
        let body = body?;
        if cd::first_error(&body).is_some() {
            return Some((body, Vec::new()));
        }
        if !auto.closed {
            return Some((AutoOpen::stop_error(), Vec::new()));
        }
        Some((body, auto.trailing))
    }
    fn physics_space(&mut self) {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
    }
    fn physics_star(&mut self) -> bool {
        self.physics_space();
        if self.chars.get(self.index) == Some(&'*') {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn physics_argument(&mut self, name: &str, optional: bool) -> Result<(String, bool), String> {
        self.physics_space();
        if optional && (self.index == self.chars.len() || self.chars.get(self.index) == Some(&'}'))
        {
            return Ok((String::new(), false));
        }
        let braced = self.chars.get(self.index) == Some(&'{');
        self.padded_raw_argument(name).map(|s| (s, braced))
    }
    fn physics_brackets(&mut self) -> Result<(String, bool), String> {
        self.physics_space();
        let present = self.chars.get(self.index) == Some(&'[');
        self.padded_brackets().map(|s| (s, present))
    }
    fn physics_up_to(&mut self, close: char) -> Result<String, String> {
        let open = match close {
            ')' => '(',
            ']' => '[',
            _ => close,
        };
        let start = self.index;
        let mut braces = 0;
        let mut fences = 0;
        while self.index < self.chars.len() {
            let c = self.chars[self.index];
            if c == '\\' {
                self.index += 1;
                self.capture_control_sequence();
                continue;
            }
            if c == '{' {
                braces += 1;
            } else if c == '}' && braces > 0 {
                braces -= 1;
            } else if braces == 0 {
                if c == open && open != close {
                    fences += 1;
                } else if c == close {
                    if fences == 0 {
                        let raw = self.chars[start..self.index].iter().collect();
                        self.index += 1;
                        return Ok(raw);
                    }
                    fences -= 1;
                }
            }
            self.index += 1;
        }
        Err("Could not find closing delimiter".into())
    }
    fn physics_fence(&self, open: u32, body: Atom, close: u32, stretchy: bool) -> Atom {
        Atom::Fenced {
            close_color: None,
            open,
            body: children(body),
            close,
            stretchy,
            inner: false,
        }
    }
    /// tex/commands.go physicsNabla: TeXAtom(ORD, TeXAtom(ORD, mo ∇ bold)).
    pub(super) fn physics_nabla(&self) -> Option<Atom> {
        Some(Atom::Ord(Box::new(Atom::Ord(Box::new(bold_operator(
            0x2207, 0,
        )?)))))
    }
    /// tex/physics_operator_application.go: deliver the operator first, then
    /// prepare the argument in the caller. Only raw parentheses/brackets grow.
    pub(super) fn physics_operator_application(&mut self, name: &str) -> Option<Atom> {
        // physics_operator_application.go derives both policies from the maps.
        let (source, fences, vector) = match name {
            "div" | "divergence" => (r"\vnabla\vdot", "([", true),
            "curl" => (r"\vnabla\crossproduct", "([", true),
            "grad" | "gradient" => (r"\vnabla", "([", false),
            "Res" => (r"\Residue", "([{", false),
            "pv" | "principalvalue" => (r"{\cal P}", "", false),
            "PV" => (r"{\rm P.V.}", "", false),
            "Re" => (r"\mathrm{Re}", "{", false),
            "Im" => (r"\mathrm{Im}", "{", false),
            _ => return None,
        };
        let operator = self.padded_parse_string(source)?;
        if cd::first_error(&operator).is_some() {
            return Some(operator);
        }
        self.physics_after = Some(After::Application {
            name: name.into(),
            fences,
        });
        if vector {
            self.splice_inferred = true;
            Some(operator)
        } else {
            Some(Atom::NamedFunction(Box::new(parsed_row(operator))))
        }
    }
    // tex/commands.go parseExpansion of a source macro: equivalent to
    // splicing the self-contained expansion in front of the unparsed input.
    fn physics_splice(&mut self, expansion: &str) -> Option<Atom> {
        // parseExpansion parses the macro text on its own (so a trailing
        // control word cannot absorb following letters) and returns its
        // inferred row's children to the current row.
        let parsed = self.padded_parse_string(expansion)?;
        if matches!(parsed, Atom::Sequence(_)) {
            self.splice_inferred = true;
        }
        Some(parsed)
    }
    // parseChild gives generated braket/commutator programs one fresh
    // macro budget, shared by both operands and restored on every exit.
    fn physics_child_splice(&mut self, expansion: &str) -> Option<Atom> {
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let parsed = self.physics_splice(expansion);
        self.ams.borrow_mut().paired_macro_count = count;
        parsed
    }
    /// tex/physics_handlers.go physicsCommand: eval, outer products and
    /// matrix elements; tex/commands.go physicsBraket expval and commutator.
    pub(super) fn physics_macro(&mut self, name: &str) -> Option<Atom> {
        match name {
            "evaluated" | "eval" => {
                let star = self.physics_star();
                self.physics_space();
                match self.chars.get(self.index).copied() {
                    Some('{') => {
                        let raw = match self.padded_raw_argument(name) {
                            Ok(raw) => raw,
                            Err(message) => return Some(Atom::ErrorDisplay(message)),
                        };
                        let raw = if star {
                            format!("\\smash{{{raw}}}")
                        } else {
                            raw
                        };
                        // Braced Eval continues on the caller's input.
                        let expansion = format!("\\left. {raw} \\vphantom{{\\int}}\\right|");
                        self.chars.splice(self.index..self.index, expansion.chars());
                    }
                    Some(open @ ('(' | '[')) => {
                        self.index += 1;
                        self.physics_after = Some(After::Eval { open, smash: star });
                    }
                    _ => return Some(Atom::ErrorDisplay(format!("Missing argument for \\{name}"))),
                }
                self.splice_inferred = true;
                Some(Atom::Sequence(Vec::new()))
            }
            "outerproduct" | "dyad" | "ketbra" | "op" => {
                let star = self.physics_star();
                let ket = match self.physics_argument(name, false) {
                    Ok((raw, _)) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                let bra = match self.physics_argument(name, true) {
                    Ok((raw, true)) => raw,
                    Ok(_) => ket.clone(),
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                let expansion = if star {
                    format!("\\vert{{{ket}}}\\rangle\\!\\langle{{{bra}}}\\vert")
                } else {
                    format!(
                        "\\left\\vert{{{ket}}}\\middle\\rangle\\!\\middle\\langle{{{bra}}}\\right\\vert"
                    )
                };
                self.physics_child_splice(&expansion)
            }
            "matrixelement" | "matrixel" | "mel" => {
                let star1 = self.physics_star();
                let star2 = star1 && self.physics_star();
                let mut args = Vec::new();
                for _ in 0..3 {
                    match self.physics_argument(name, false) {
                        Ok((raw, _)) => args.push(raw),
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    }
                }
                let (a, b, c) = (&args[0], &args[1], &args[2]);
                let expansion = if star1 && star2 {
                    format!(
                        "\\left\\langle{{{a}}}\\middle\\vert{{{b}}}\\middle\\vert{{{c}}}\\right\\rangle"
                    )
                } else if star1 {
                    format!("\\langle{{{a}}}\\vert{{{b}}}\\vert{{{c}}}\\rangle")
                } else {
                    format!(
                        "\\left\\langle{{{a}}}\\right\\vert{{{b}}}\\left\\vert{{{c}}}\\right\\rangle"
                    )
                };
                self.physics_child_splice(&expansion)
            }
            "expectationvalue" | "expval" | "ev" => {
                // tex/physics_expectation.go: read both raw operands before
                // selecting one complete child-parser program.
                let star1 = self.physics_star();
                let star2 = star1 && self.physics_star();
                let first = match self.padded_raw_argument(name) {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                self.physics_space();
                let second = if self.chars.get(self.index) == Some(&'{') {
                    match self.padded_raw_argument(name) {
                        Ok(raw) => raw,
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    }
                } else {
                    String::new()
                };
                let program = if !first.is_empty() && !second.is_empty() {
                    if star2 {
                        format!(
                            "\\left\\langle{{{second}}}\\middle\\vert{{{first}}}\\middle\\vert{{{second}}}\\right\\rangle"
                        )
                    } else if star1 {
                        format!("\\langle{{{second}}}\\vert{{{first}}}\\vert{{{second}}}\\rangle")
                    } else {
                        format!(
                            "\\left\\langle{{{second}}}\\right\\vert{{{first}}}\\left\\vert{{{second}}}\\right\\rangle"
                        )
                    }
                } else if star1 {
                    format!("\\langle {{{first}}} \\rangle")
                } else {
                    format!("\\left\\langle {{{first}}} \\right\\rangle")
                };
                self.physics_child_splice(&program)
            }
            "commutator" | "comm" | "anticommutator" | "acomm" | "poissonbracket" | "pb" => {
                // commands.go commutator: one lexical child owns the comma
                // and both operands, including declarations in the left one.
                let star = self.physics_star();
                self.physics_space();
                let mut big = String::new();
                if self.chars.get(self.index) == Some(&'\\') {
                    self.index += 1;
                    big = self.capture_control_sequence();
                    if !matches!(big.as_str(), "big" | "Big" | "bigg" | "Bigg") {
                        return Some(Atom::ErrorDisplay(format!("Missing argument for \\{name}")));
                    }
                    self.physics_space();
                }
                if self.chars.get(self.index) != Some(&'{') {
                    return Some(Atom::ErrorDisplay(format!("Missing argument for \\{name}")));
                }
                let mut parts = Vec::new();
                for _ in 0..2 {
                    match self.padded_raw_argument(name) {
                        Ok(raw) => parts.push(raw),
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    }
                }
                let (open, close) =
                    if matches!(name, "anticommutator" | "acomm" | "poissonbracket" | "pb") {
                        (r"\{", r"\}")
                    } else {
                        ("[", "]")
                    };
                let argument = format!("{},{}", parts[0], parts[1]);
                let program = if star {
                    format!("{open} {argument} {close}")
                } else if !big.is_empty() {
                    format!("\\{big}l{open} {argument} \\{big}r{close}")
                } else {
                    format!("\\left{open} {argument} \\right{close}")
                };
                self.physics_child_splice(&program)
            }
            "mqty" | "matrixquantity" | "smqty" | "smallmatrixquantity" => {
                self.matrix_quantity(name)
            }
            _ => None,
        }
    }
    pub(super) fn physics_quantity(&mut self, name: &str) -> Option<Atom> {
        // physics_quantity.go reads the size command before validating the
        // physical opener, and restores it when Quantity emits empty fences.
        let argument = !matches!(name, "qty" | "quantity");
        let star = argument && self.physics_star();
        self.physics_space();
        let saved = self.index;
        let mut big = String::new();
        if self.chars.get(self.index) == Some(&'\\') {
            self.index += 1;
            big = self.capture_control_sequence();
            if !matches!(big.as_str(), "big" | "Big" | "bigg" | "Bigg") {
                self.index = saved;
                return Some(Atom::Fenced {
                    close_color: None,
                    open: 0x28,
                    close: 0x29,
                    body: vec![Atom::EmptyNode("mrow")],
                    stretchy: true,
                    inner: true,
                });
            }
            self.physics_space();
        }
        let next = self.chars.get(self.index).copied();
        if argument && next != Some('{') {
            return Some(Atom::ErrorDisplay(format!("Missing argument for \\{name}")));
        }
        if !matches!(next, Some('(' | '[' | '{' | '|')) {
            self.index = saved;
            return Some(Atom::Fenced {
                close_color: None,
                open: 0x28,
                close: 0x29,
                body: vec![Atom::EmptyNode("mrow")],
                stretchy: true,
                inner: true,
            });
        }
        if next == Some('{') {
            let raw = match self.padded_raw_argument(name) {
                Ok(raw) => raw,
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            let (open, close) = if !argument {
                (r"\{", r"\}")
            } else {
                match name {
                    "bqty" => ("[", "]"),
                    "Bqty" => (r"\{", r"\}"),
                    "vqty" | "abs" | "absolutevalue" => ("|", "|"),
                    "norm" => (r"\|", r"\|"),
                    _ => ("(", ")"),
                }
            };
            let source = if star {
                format!("{open} {raw} {close}")
            } else if big.is_empty() {
                format!("\\left{open} {raw} \\right{close}")
            } else {
                format!("\\{big}l{open} {raw} \\{big}r{close}")
            };
            let fenced = self.padded_parse_string(&source)?;
            if cd::first_error(&fenced).is_some() {
                return Some(fenced);
            }
            let mut nodes = children(fenced);
            if name == "order" {
                nodes.insert(0, Atom::ApplyFunction);
                nodes.insert(
                    0,
                    Atom::NamedFunction(Box::new(Atom::Operator(Box::new(
                        Atom::CalligraphicGlyph(0x4f),
                    )))),
                );
            }
            self.splice_inferred = true;
            return Some(Atom::Sequence(nodes));
        }
        let open = next?;
        let close = if open == '(' {
            ')'
        } else if open == '[' {
            ']'
        } else {
            '|'
        };
        self.index += 1;
        let (body, trailing) = self.physics_auto_body(open, close)?;
        if cd::first_error(&body).is_some() {
            return Some(body);
        }
        let mut node = self.physics_fence(open as u32, body, close as u32, true);
        if !big.is_empty() {
            let Atom::Fenced { body, .. } = node else {
                unreachable!()
            };
            let mut nodes = children(self.padded_parse_string(&format!("\\{big}l{open}"))?);
            nodes.extend(body);
            nodes.extend(children(
                self.padded_parse_string(&format!("\\{big}r{close}"))?,
            ));
            node = Atom::Mrow(Box::new(Atom::Sequence(nodes)));
        }
        let mut nodes = vec![node];
        nodes.extend(trailing);
        // The closed recipient is delivered before scripts on its result.
        // Ordinary Quantity emits only this one completed row.
        if nodes.len() == 1 {
            nodes.pop()
        } else {
            self.splice_inferred = true;
            Some(Atom::Sequence(nodes))
        }
    }
    pub(super) fn physics_derivative(&mut self, name: &str) -> Option<Atom> {
        let differential = matches!(name, "dd" | "differential" | "var" | "variation");
        let star = !differential && self.physics_star();
        let (order, present) = match self.physics_brackets() {
            Ok(r) => r,
            Err(e) => {
                return Some(Atom::ErrorDisplay(
                    if e == "Could not find closing ']' for argument" {
                        format!("Could not find closing ']' for argument to \\{name}")
                    } else {
                        e
                    },
                ));
            }
        };
        if differential {
            // physics_derivative.go Differential reparses the complete literal
            // operator/power/operand in one fresh child, including empty braces.
            let mut source = if matches!(name, "var" | "variation") {
                r"\delta".to_owned()
            } else {
                r"\diffd".to_owned()
            };
            if present {
                source.push_str(&format!("^{{{order}}}"));
            } else {
                source.push(' ');
            }
            self.physics_space();
            let parens = self.chars.get(self.index) == Some(&'(');
            let braced = self.chars.get(self.index) == Some(&'{');
            if !parens {
                let raw = match self.physics_argument(name, !braced) {
                    Ok((raw, _)) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                source.push_str(&raw);
            }
            let parsed = self.padded_parse_string(&source)?;
            if cd::first_error(&parsed).is_some() {
                return Some(parsed);
            }
            if braced {
                return Some(Atom::ScriptPolicy {
                    body: Box::new(Atom::Class {
                        class: 1,
                        body: Box::new(parsed_row(parsed)),
                    }),
                    moves: false,
                    movable: Some(false),
                    movable_property: false,
                });
            }
            let nodes = children(parsed);
            if parens {
                self.physics_after = Some(After::Derivative { ignore: false });
            }
            self.splice_inferred = true;
            return Some(Atom::Sequence(nodes));
        }
        let (first, _) = match self.physics_argument(name, false) {
            Ok(r) => r,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let mut args = vec![first];
        let partial =
            name.contains("partial") || name.starts_with("pdv") || name.starts_with("pderivative");
        let code = if partial {
            0x2202
        } else if name.starts_with('f') || name.contains("functional") {
            0x3b4
        } else {
            0x64
        };
        while args.len() < if partial { 3 } else { 2 } {
            self.physics_space();
            if self.chars.get(self.index) != Some(&'{') {
                break;
            }
            let (raw, _) = match self.physics_argument(name, false) {
                Ok(r) => r,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            args.push(raw);
        }
        // commands.go derivative reparses literal TeX: the denominator power
        // follows the raw variable, and mixed partials supply the total order.
        let (power1, power2) = if partial && args.len() > 2 {
            (format!("^{{{}}}", args.len() - 1), " ".to_owned())
        } else if present {
            let power = format!("^{{{order}}}");
            (power.clone(), power)
        } else {
            (" ".to_owned(), " ".to_owned())
        };
        let op = match code {
            0x2202 => "\\partial",
            0x3b4 => "\\delta",
            _ => "\\diffd",
        };
        let (function, variable) = if args.len() > 1 {
            (args[0].as_str(), args[1].as_str())
        } else {
            ("", args[0].as_str())
        };
        let mut rest = String::new();
        for raw in args.iter().skip(2).take_while(|raw| !raw.is_empty()) {
            rest.push_str(&format!("{op} {raw}"));
        }
        let numerator = format!("{op}{power1}{function}");
        let denominator = format!("{op} {variable}{power2} {rest}");
        let fraction = if star { "flatfrac" } else { "frac" };
        let expansion = format!("\\{fraction}{{{numerator}}}{{{denominator}}}");
        let parsed = self.physics_splice(&expansion)?;
        if cd::first_error(&parsed).is_none() {
            self.physics_after = Some(After::Derivative {
                ignore: partial && (args.len() > 2 || present && args.len() > 1),
            });
        }
        Some(parsed)
    }
}

fn collapsed(a: Atom) -> Atom {
    if let Atom::Sequence(mut v) = a {
        if v.len() == 1 {
            v.pop().unwrap()
        } else {
            Atom::Sequence(v)
        }
    } else {
        a
    }
}
fn parsed_row(a: Atom) -> Atom {
    match collapsed(a) {
        Atom::Sequence(v) if v.len() > 1 => Atom::InferredRow(v),
        a => a,
    }
}
pub(super) fn named_function(a: &Atom) -> bool {
    match a {
        Atom::NamedFunction(_) => true,
        Atom::Scripts { base, .. } | Atom::ScriptPolicy { body: base, .. } => named_function(base),
        _ => false,
    }
}
pub(super) fn accepts_application(a: &Atom) -> bool {
    !operator_dictionary::spacelike(a) && !matches!(atom_tex_class(a), Some(2 | 3 | 5 | 6))
}
// tex/symbols.go functionNames extends Base with every NamedFn entry in
// source_tables_physics_maps.go. These aliases use FnItem delivery only;
// unlike Expression they never consume optional powers or enlarge fences.
pub(super) fn function_name(name: &str) -> Option<&str> {
    Some(match name {
        "sine" => "sin",
        "hypsine" => "sinh",
        "arcsine" => "arcsin",
        "asine" => "asin",
        "cosine" => "cos",
        "hypcosine" => "cosh",
        "arccosine" => "arccos",
        "acosine" => "acos",
        "tangent" => "tan",
        "hyptangent" => "tanh",
        "arctangent" => "arctan",
        "atangent" => "atan",
        "cosecant" => "csc",
        "hypcosecant" => "csch",
        "arccosecant" => "arccsc",
        "acosecant" => "acsc",
        "secant" => "sec",
        "hypsecant" => "sech",
        "arcsecant" => "arcsec",
        "asecant" => "asec",
        "cotangent" => "cot",
        "hypcotangent" => "coth",
        "arccotangent" => "arccot",
        "acotangent" => "acot",
        "exponential" => "exp",
        "logarithm" => "log",
        "naturallogarithm" => "ln",
        "determinant" => "det",
        "Probability" => "Pr",
        "liminf" => "lim\u{2006}inf",
        "limsup" => "lim\u{2006}sup",
        "arccos" | "arcsin" | "arctan" | "arg" | "cos" | "cosh" | "cot" | "coth" | "csc"
        | "deg" | "det" | "dim" | "exp" | "gcd" | "hom" | "inf" | "ker" | "lg" | "lim" | "ln"
        | "log" | "max" | "min" | "Pr" | "sec" | "sin" | "sinh" | "sup" | "tan" | "tanh"
        | "rank" => name,
        _ => return None,
    })
}
pub(super) fn is_function(name: &str) -> bool {
    matches!(
        name,
        "sin"
            | "sinh"
            | "arcsin"
            | "asin"
            | "cos"
            | "cosh"
            | "arccos"
            | "acos"
            | "tan"
            | "tanh"
            | "arctan"
            | "atan"
            | "csc"
            | "csch"
            | "arccsc"
            | "acsc"
            | "sec"
            | "sech"
            | "arcsec"
            | "asec"
            | "cot"
            | "coth"
            | "arccot"
            | "acot"
            | "exp"
            | "log"
            | "ln"
            | "det"
            | "Pr"
            | "tr"
            | "trace"
            | "Tr"
            | "Trace"
            | "erf"
    )
}
impl Parser {
    pub(super) fn physics_expression(&mut self, name: &str) -> Option<Atom> {
        let id = match name {
            "trace" => "tr",
            "Trace" => "Tr",
            _ => name,
        };
        let mut base = Atom::Operator(Box::new(self.nonambient_font(Atom::Identifier(
            id.chars().map(|c| Atom::Glyph(c as u32)).collect(),
        ))));
        if !matches!(
            name,
            "exp" | "det" | "Pr" | "tr" | "trace" | "Tr" | "Trace" | "erf"
        ) {
            let exponent = match self.physics_brackets() {
                Ok((s, _)) => s,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            if !exponent.is_empty() {
                base = Atom::Scripts {
                    limits: false,
                    accent: None,
                    eager: true,
                    leading_space: 0.,
                    base: Box::new(base),
                    sup: Some(Box::new(collapsed(self.padded_parse_string(&exponent)?))),
                    sub: None,
                };
            }
        }
        self.physics_space();
        if self.chars.get(self.index) != Some(&'(') {
            return Some(Atom::NamedFunction(Box::new(base)));
        }
        self.index += 1;
        let raw = match self.physics_up_to(')') {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let fence = self.physics_fence(0x28, self.padded_parse_string(&raw)?, 0x29, true);
        Some(Atom::Sequence(vec![
            base,
            Atom::ApplyFunction,
            self.attach_scripts(fence)?,
        ]))
    }
}
pub(super) fn layout_inferred(
    children: &[Atom],
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let width = layout_at_with_style(
        &Atom::Sequence(children.to_vec()),
        0.0,
        0.0,
        scale,
        out,
        display,
        prime,
    )?;
    if x != 0.0 || baseline != 0.0 {
        out.groups.push(LayoutGroup {
            data_cramped: None,
            own_translate: None,
            accent_offset: None,
            box_border: None,
            reference: None,
            tagged: None,
            percentage: None,
            table_line: None,
            mathsize: None,
            tex_class: 0,
            before_glyph: out.glyphs.len(),
            x,
            baseline,
            scale,
            color: None,
            background: None,
            kind: "#inferred-place",
            delimiter: None,
            arrow: None,
            body: Box::new(Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: vec![],
                rules: vec![],
                groups: vec![],
            }),
            lines: vec![],
        });
    }
    Some(width)
}

// Original setBaseTeXClass preserves an embellished/mi core; other script
// bases produce ORD even when the child wrapper has a different class.
pub(super) fn script_class(base: &Atom) -> Option<usize> {
    match base {
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::ScriptPolicy { body: a, .. } => script_class(a),
        Atom::Sequence(v) => {
            if v.len() == 1 {
                script_class(&v[0]).or(Some(0))
            } else if operator_dictionary::embellished(base) {
                embellished_class(base)
            } else {
                Some(0)
            }
        }
        Atom::Ord(_) | Atom::Class { .. } | Atom::Padded { .. } => {
            if operator_dictionary::embellished(base) {
                embellished_class(base)
            } else {
                Some(0)
            }
        }
        // setBaseTeXClass: an operator whose node is a (non-embellished)
        // TeXAtom leaves the script node ORD; only an mi base passes OP up.
        Atom::Operator(a) if matches!(**a, Atom::Class { .. } | Atom::Ord(_)) => script_class(a),
        Atom::Fenced { .. }
        | Atom::Fraction(..)
        | Atom::SplitFraction(..)
        | Atom::GeneralFraction(..)
        | Atom::Sqrt(..)
        | Atom::Root { .. }
        | Atom::Matrix { .. }
        | Atom::InferredRow(_)
        | Atom::ExplicitRow(_) => Some(0),
        _ => atom_tex_class(base),
    }
}

// Source svg/texclass.go coreNode follows embellished wrappers to mo,
// independently of the enclosing TeXAtom's explicitly requested class.
fn embellished_class(atom: &Atom) -> Option<usize> {
    match atom {
        Atom::Sequence(v) => v
            .iter()
            .find(|a| !operator_dictionary::spacelike(a))
            .and_then(embellished_class),
        Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Padded { body: a, .. } => embellished_class(a),
        Atom::Scripts { base, .. } | Atom::Multiscripts { base, .. } => embellished_class(base),
        _ => atom_tex_class(atom),
    }
}

// Original parser.go applyMathVariant walks mi/mn/mo descendants, including
// explicit differential tokens and named functions inside quantity frames.
pub(super) fn apply_italic(a: &mut Atom) {
    match a {
        Atom::FontGlyph { original, .. } => {
            if let Some(c) = char::from_u32(*original) {
                *a = automatic_italic_atom(c);
            }
        }
        Atom::Glyph(c) | Atom::ItalicGlyph(c) | Atom::CalligraphicGlyph(c) => {
            let c = original_font_code(*c);
            if let Some(character) = char::from_u32(c) {
                *a = automatic_italic_atom(character);
            }
        }
        Atom::Sequence(v)
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v)
        | Atom::Fenced { body: v, .. } => {
            for a in v {
                apply_italic(a);
            }
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Boxed(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Cancel { body: a, .. }
        | Atom::RawOperator { body: a, .. } => apply_italic(a),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => {
            apply_italic(a);
            apply_italic(b);
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            for a in [base, sub, sup] {
                apply_italic(a);
            }
            if let Some(v) = post {
                for a in v {
                    apply_italic(a);
                }
            }
        }
        Atom::Scripts { base, sub, sup, .. } => {
            apply_italic(base);
            for a in [sub, sup].into_iter().flatten() {
                apply_italic(a);
            }
        }
        Atom::Choice(v) => {
            for a in v {
                apply_italic(a);
            }
        }
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            for a in rows.iter_mut().flatten() {
                apply_italic(a);
            }
        }
        Atom::XArrow { over, under, .. } => {
            apply_italic(over);
            if let Some(a) = under {
                apply_italic(a);
            }
        }
        Atom::CdArrow { over, under, .. } => {
            for a in [over, under].into_iter().flatten() {
                apply_italic(a);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn named_functions_keep_literal_brackets_and_mathchoice_keeps_its_row() {
        assert_eq!(render_exact(r"\sine x"), render_exact(r"\sin x"));
        assert_eq!(render_exact(r"\Probability"), render_exact(r"\Pr"));
        let alias = render_exact(r"\sine[2](x)").unwrap();
        let expression = render_exact(r"\sin[2](x)").unwrap();
        assert!(!alias.contains("data-mml-node=\"msup\""));
        assert!(expression.contains("data-mml-node=\"msup\""));
        for prefix in ["", r"\scriptstyle "] {
            let svg = render_exact(&format!(
                r"{prefix}\dd{{\mathchoice{{a+b}}{{c+d}}{{e+f}}{{g+h}}}}"
            ))
            .unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let selected = doc
                .descendants()
                .find(|n| {
                    n.attribute("data-mml-node") == Some("mrow")
                        && n.children().filter(|c| c.is_element()).count() == 3
                })
                .unwrap();
            assert_eq!(selected.attribute("transform"), Some("translate(556,0)"));
        }
    }

    #[test]
    fn autoopen_uses_delivered_markers_and_preserves_script_recipients() {
        for expression in [r"\qty|x|", r"\qty(x\rparen", r"\dd(x"] {
            let svg = render_exact(expression).unwrap();
            assert!(
                svg.contains("data-mjx-error=\"Extra open or missing close delimiter\""),
                "{expression}"
            );
        }
        // A vertical Position replaces the marked closer; its later raw
        // sibling closes AutoOpen. Horizontal Position replays that same item.
        for expression in [r"\qty(x\raise1em)y)", r"\qty(x\moveleft1em)y"] {
            assert!(
                !render_exact(expression).unwrap().contains("data-mjx-error"),
                "{expression}"
            );
        }
        let svg = render_exact(r"x^\dv{f}{x}(g)+z").unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let math = document
            .descendants()
            .find(|n| n.attribute("data-mml-node") == Some("math"))
            .unwrap();
        let kinds: Vec<_> = math
            .children()
            .filter(|n| n.is_element())
            .map(|n| n.attribute("data-mml-node").unwrap())
            .collect();
        assert_eq!(kinds, ["msup", "mrow", "mo", "mi"]);
        assert_eq!(
            render_exact(r"\pdv[2]{f}{x}(\rm g)+z"),
            render_exact(r"\pdv[2]{f}{x}+z")
        );
    }

    #[test]
    fn starred_quantity_scripts_and_differential_operands_keep_source_owners() {
        let svg = render_exact(r"\pqty*{\frac{a}{b}}_i^2").unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let script = document
            .descendants()
            .find(|n| n.attribute("data-mml-node") == Some("msubsup"))
            .unwrap();
        assert_eq!(
            script
                .children()
                .find(|n| n.is_element())
                .unwrap()
                .attribute("data-mml-node"),
            Some("mo")
        );
        assert!(
            !script
                .descendants()
                .any(|n| n.attribute("data-mml-node") == Some("mfrac"))
        );
        for (command, operator) in [("dd", r"\diffd"), ("var", r"\delta")] {
            let actual = render_exact(&format!("\\{command}[2]^3")).unwrap();
            let expected = render_exact(&format!("{operator}^{{2}}^3")).unwrap();
            assert_eq!(actual, expected);
        }
        let svg = render_exact(r"\qty(\rm x)y").unwrap();
        assert!(svg.contains("data-c=\"78\""));
        assert!(svg.contains("data-c=\"1D466\""));
    }
}
