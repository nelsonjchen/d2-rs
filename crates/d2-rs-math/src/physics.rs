// Copyright 2018–2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/commands.go quantity, derivative, physicsDifferential,
// readUpToByte and parser.go readArgument/readBrackets.
use super::*;
fn children(a: Atom) -> Vec<Atom> {
    let a = collapsed(a);
    if let Atom::Sequence(v) | Atom::InferredRow(v) = a {
        v
    } else {
        vec![a]
    }
}
fn row(mut v: Vec<Atom>) -> Atom {
    if v.len() == 1 {
        v.pop().unwrap()
    } else {
        Atom::Sequence(v)
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
        texclass: Some(class),
        code,
        body: Box::new(body),
        definition,
    })
}
impl Parser {
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
            open,
            body: children(body),
            close,
            stretchy,
            inner: false,
        }
    }
    /// tex/commands.go physicsNabla: TeXAtom(ORD, TeXAtom(ORD, mo ∇ bold)).
    pub(super) fn physics_nabla(&self) -> Option<Atom> {
        Some(Atom::Ord(Box::new(Atom::Ord(Box::new(bold_operator(0x2207, 0)?)))))
    }
    /// tex/commands.go operatorApplication (\grad, \divergence, \curl).
    pub(super) fn physics_operator_application(&mut self, name: &str) -> Option<Atom> {
        let vector = matches!(name, "divergence" | "curl");
        let mut prefix = vec![self.physics_nabla()?];
        if vector {
            // \vdot is a bold U+22C5; \crossproduct an unmodified U+00D7.
            prefix.push(if name == "curl" {
                let mut definition = operator_dictionary::lookup(0xD7, operator_dictionary::Form::Infix);
                definition.class = 2;
                Atom::RawOperator {
                    texclass: Some(2),
                    code: 0xD7,
                    body: Box::new(Atom::Glyph(0xD7)),
                    definition,
                }
            } else {
                bold_operator(0x22C5, 2)?
            });
        }
        self.physics_space();
        if self.index < self.chars.len() {
            let raw = match self.chars[self.index] {
                '(' => {
                    self.index += 1;
                    self.physics_up_to(')')
                }
                '[' => {
                    self.index += 1;
                    self.physics_up_to(']')
                }
                _ => self.physics_argument(name, false).map(|(raw, _)| raw),
            };
            let raw = match raw {
                Ok(raw) => raw,
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            let argument = self.padded_parse_string(&raw)?;
            if cd::first_error(&argument).is_some() {
                return Some(argument);
            }
            if !vector {
                prefix.push(Atom::ApplyFunction);
            }
            prefix.push(self.physics_fence(0x28, argument, 0x29, true));
        }
        self.splice_inferred = true;
        Some(Atom::Sequence(prefix))
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
    /// tex/physics_handlers.go physicsCommand: eval, outer products and
    /// matrix elements; tex/commands.go physicsBraket expval and commutator.
    pub(super) fn physics_macro(&mut self, name: &str) -> Option<Atom> {
        match name {
            "evaluated" | "eval" => {
                let star = self.physics_star();
                self.physics_space();
                let raw = match self.chars.get(self.index) {
                    Some('{') => self.physics_argument(name, false).map(|(raw, _)| raw),
                    Some('(') => {
                        self.index += 1;
                        self.physics_up_to(')')
                    }
                    Some('[') => {
                        self.index += 1;
                        self.physics_up_to(']')
                    }
                    _ => Err(format!("Missing argument for \\{name}")),
                };
                let raw = match raw {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                let raw = if star { format!("\\smash{{{raw}}}") } else { raw };
                self.physics_splice(&format!("\\left. {raw} \\vphantom{{\\int}}\\right|"))
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
                    format!("\\left\\vert{{{ket}}}\\middle\\rangle\\!\\middle\\langle{{{bra}}}\\right\\vert")
                };
                self.physics_splice(&expansion)
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
                    format!("\\left\\langle{{{a}}}\\middle\\vert{{{b}}}\\middle\\vert{{{c}}}\\right\\rangle")
                } else if star1 {
                    format!("\\langle{{{a}}}\\vert{{{b}}}\\vert{{{c}}}\\rangle")
                } else {
                    format!("\\left\\langle{{{a}}}\\right\\vert{{{b}}}\\left\\vert{{{c}}}\\right\\rangle")
                };
                self.physics_splice(&expansion)
            }
            "expectationvalue" | "expval" | "ev" => {
                let raw = match self.padded_raw_argument(name) {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                let argument = self.padded_parse_string(&raw)?;
                if cd::first_error(&argument).is_some() {
                    return Some(argument);
                }
                Some(self.physics_fence(
                    0x27E8,
                    Atom::Ord(Box::new(argument)),
                    0x27E9,
                    true,
                ))
            }
            "commutator" | "comm" | "anticommutator" | "acomm" | "poissonbracket" | "pb" => {
                let mut parts = Vec::new();
                for _ in 0..2 {
                    let raw = match self.padded_raw_argument(name) {
                        Ok(raw) => raw,
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    };
                    parts.push(raw);
                }
                let left = self.padded_parse_string(&parts[0])?;
                let right = self.padded_parse_string(&parts[1])?;
                for part in [&left, &right] {
                    if cd::first_error(part).is_some() {
                        return Some(part.clone());
                    }
                }
                let mut comma_definition =
                    operator_dictionary::lookup(0x2C, operator_dictionary::Form::Infix);
                comma_definition.class = 6;
                let comma = Atom::RawOperator {
                    texclass: Some(6),
                    code: 0x2C,
                    body: Box::new(Atom::Glyph(0x2C)),
                    definition: comma_definition,
                };
                let (open, close) = if matches!(name, "poissonbracket" | "pb") {
                    (0x7B, 0x7D)
                } else {
                    (0x5B, 0x5D)
                };
                let mut body = children(left);
                body.push(comma);
                body.extend(children(right));
                Some(Atom::Fenced {
                    open,
                    body,
                    close,
                    stretchy: true,
                    inner: false,
                })
            }
            // tex/commands.go matrixQuantity.
            "mqty" | "matrixquantity" | "pmqty" | "Pmqty" | "bmqty" | "vmqty" | "smqty"
            | "smallmatrixquantity" | "spmqty" | "sPmqty" | "sbmqty" | "svmqty" => {
                let star = self.physics_star();
                self.physics_space();
                let (raw, auto) = if self.chars.get(self.index) == Some(&'(') {
                    self.index += 1;
                    (self.physics_up_to(')'), true)
                } else {
                    (self.physics_argument(name, false).map(|(raw, _)| raw), false)
                };
                let raw = match raw {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                // The small ("S" table style) quantities differ from the
                // smallmatrix environment and are not translated.
                if name.starts_with('s') || name.contains("small") {
                    return None;
                }
                let environment = "matrix";
                let rows = table::parse_rows(&raw, environment, self.ams.clone())?;
                let table = Atom::Matrix {
                    spacing: None,
                    options: None,
                    rows,
                    environment: environment.to_owned(),
                    column_spec: String::new(),
                };
                let (open, close) = if name.contains("pmqty") || name == "Pmqty" || name == "sPmqty" {
                    (0x28, 0x29)
                } else if name.contains("bmqty") {
                    (0x5B, 0x5D)
                } else if name.contains("vmqty") {
                    (0x7C, 0x7C)
                } else if star {
                    (0x2016, 0x2016)
                } else if auto {
                    (0x28, 0x29)
                } else {
                    return Some(table);
                };
                Some(Atom::Fenced {
                    open,
                    body: vec![table],
                    close,
                    stretchy: true,
                    inner: false,
                })
            }
            _ => None,
        }
    }
    pub(super) fn physics_quantity(&mut self, name: &str) -> Option<Atom> {
        if name == "order" {
            let a = self.padded_argument(name)?;
            return Some(Atom::Sequence(vec![
                self.padded_parse_string("O")?,
                self.physics_fence(0x28, a, 0x29, true),
            ]));
        }
        let star = self.physics_star();
        self.physics_space();
        let raw = if self.index == self.chars.len() {
            Ok(String::new())
        } else {
            let c = self.chars[self.index];
            let close = match c {
                '(' => Some(')'),
                '[' => Some(']'),
                '|' => Some('|'),
                _ => None,
            };
            if let Some(close) = close {
                self.index += 1;
                self.physics_up_to(close)
            } else {
                self.physics_argument(name, false).map(|(raw, _)| raw)
            }
        };
        let raw = match raw {
            Ok(r) => r,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let a = self.padded_parse_string(&raw)?;
        let (open, close) = match name {
            "bqty" => (0x5b, 0x5d),
            "vqty" | "abs" | "absolutevalue" => (0x7c, 0x7c),
            "norm" => (0x2016, 0x2016),
            _ => (0x28, 0x29),
        };
        Some(self.physics_fence(open, a, close, !star))
    }
    fn physics_diff(&self, code: u32, standalone: bool) -> Option<Atom> {
        let glyph = if code == 0x64 {
            Atom::Identifier(vec![Atom::Glyph(0x64)])
        } else {
            self.padded_parse_string(if code == 0x2202 {
                "\\partial"
            } else {
                "\\delta"
            })?
        };
        Some(if code == 0x64 || !standalone {
            Atom::Ord(Box::new(glyph))
        } else {
            glyph
        })
    }
    pub(super) fn physics_derivative(&mut self, name: &str) -> Option<Atom> {
        let (order, present) = match self.physics_brackets() {
            Ok(r) => r,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        if matches!(name, "dd" | "differential" | "var" | "variation") {
            let code = if matches!(name, "var" | "variation") {
                0x3b4
            } else {
                0x64
            };
            let mut diff = self.physics_diff(code, true)?;
            if present {
                diff = Atom::Scripts {
                    limits: false,
                    accent: None,
                    leading_space: 0.,
                    base: Box::new(diff),
                    sup: Some(Box::new(collapsed(self.padded_parse_string(&order)?))),
                    sub: None,
                };
            }
            self.physics_space();
            if self.chars.get(self.index) == Some(&'(') {
                self.index += 1;
                let raw = match self.physics_up_to(')') {
                    Ok(s) => s,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                let a = self.padded_parse_string(&raw)?;
                return Some(Atom::Sequence(vec![
                    diff,
                    self.physics_fence(0x28, a, 0x29, true),
                ]));
            }
            let (raw, braced) = match self.physics_argument(name, true) {
                Ok(r) => r,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            if raw.is_empty() {
                return Some(diff);
            }
            let mut combined = vec![diff];
            combined.extend(children(self.padded_parse_string(&raw)?));
            let combined = row(combined);
            return Some(if braced {
                Atom::ScriptPolicy {
                    body: Box::new(Atom::Class {
                        class: 1,
                        body: Box::new(combined),
                    }),
                    moves: false,
                    movable: Some(false),
                }
            } else {
                parsed_row(combined)
            });
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
        let (function, variable) = if args.len() > 1 {
            (args[0].as_str(), args[1].as_str())
        } else {
            ("", args[0].as_str())
        };
        let mut numerator = vec![self.physics_diff(code, false)?];
        let mut denominator = vec![self.physics_diff(code, false)?];
        if !function.is_empty() {
            numerator.push(parsed_row(self.padded_parse_string(function)?));
        }
        denominator.push(parsed_row(self.padded_parse_string(variable)?));
        if present {
            let exponent = collapsed(self.padded_parse_string(&order)?);
            numerator[0] = Atom::Scripts {
                limits: false,
                accent: None,
                leading_space: 0.,
                base: Box::new(numerator[0].clone()),
                sup: Some(Box::new(exponent.clone())),
                sub: None,
            };
            denominator[0] = Atom::Scripts {
                limits: false,
                accent: None,
                leading_space: 0.,
                base: Box::new(denominator[0].clone()),
                sup: Some(Box::new(exponent)),
                sub: None,
            };
        }
        for raw in args.iter().skip(2) {
            denominator.extend([
                self.physics_diff(code, false)?,
                parsed_row(self.padded_parse_string(raw)?),
            ]);
        }
        Some(Atom::Fraction(
            Box::new(row(numerator)),
            Box::new(row(denominator)),
        ))
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
        let mut base = Atom::Operator(Box::new(Atom::Identifier(
            id.chars().map(|c| Atom::Glyph(c as u32)).collect(),
        )));
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
        Atom::Ord(_) | Atom::Class { .. } => {
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
        | Atom::InferredRow(_) => Some(0),
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
        | Atom::MovesScripts(a)
        | Atom::Style(a) | Atom::CrampedStyle(a, _)
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
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Style(a) | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Cancel { body: a, .. }
        | Atom::RawOperator { body: a, .. } => apply_italic(a),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root { body: a, index: b, .. } => {
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
