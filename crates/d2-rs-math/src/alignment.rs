// Copyright (c) 2020-2022 MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/mathtools_arrow.go and parser.go's equation-array dispatch.
use super::*;
use std::{cell::RefCell, rc::Rc};
pub(super) type Shared = Rc<RefCell<Equation>>;
#[derive(Debug, Default)]
pub(super) struct Equation {
    pub entries: Vec<Atom>,
    pub rows: Vec<Vec<Atom>>,
    spacing: Option<Vec<f64>>,
    flush_above: Option<usize>,
}
pub(super) fn is_equation(environment: &str) -> bool {
    matches!(
        environment,
        "align"
            | "align*"
            | "aligned"
            | "alignedat"
            | "alignat"
            | "alignat*"
            | "split"
            | "gather"
            | "gather*"
            | "gathered"
            | "lgathered"
            | "rgathered"
    )
}
pub(super) fn is_command(name: &str) -> bool {
    matches!(
        name,
        "ArrowBetweenLines"
            | "Aboxed"
            | "vdotswithin"
            | "shortvdotswithin"
            | "MTFlushSpaceAbove"
            | "MTFlushSpaceBelow"
    )
}
impl Equation {
    pub fn end_entry(&mut self, cell: Atom) {
        let cell = if self.entries.is_empty() {
            cell
        } else {
            fix_initial_operator(cell)
        };
        self.entries.push(cell);
    }
    pub fn end_row(&mut self, ams: &ams::Shared) -> Option<()> {
        self.rows.push(std::mem::take(&mut self.entries));
        ams::end_row(ams)
    }
    pub fn add_spacing(&mut self, adjustment: f64) {
        let rows = self.rows.len();
        let values = self
            .spacing
            .get_or_insert_with(|| vec![length::dimension("3pt").em(0., 1.)]);
        values.resize(values.len().max(rows), 0.3);
        if rows != 0 {
            values[rows - 1] = row_spacing_em((0.3 + adjustment).max(0.));
        }
    }
    pub fn finish_spacing(&mut self) -> Vec<f64> {
        let Some(mut values) = self.spacing.take() else {
            return Vec::new();
        };
        values.resize(values.len().max(self.rows.len()), 0.3);
        values
    }
}
// ParseUtil.fixInitialMO runs before labels or layout padding add columns.
fn fix_initial_operator(cell: Atom) -> Atom {
    let mut nodes = match cell {
        Atom::Sequence(v) => v,
        a => vec![a],
    };
    let repair = nodes.iter().find(|a| !matches!(a,
        Atom::Space(_) | Atom::Kern(_) | Atom::EmptyNode("mspace")) &&
        !matches!(a, Atom::Ord(body) if matches!(body.as_ref(), Atom::Sequence(v) if v.is_empty())))
        .is_some_and(|a| operator_dictionary::embellished(a) ||
            matches!(a, Atom::Class { class: 3, .. }) ||
            matches!(a, Atom::Glyph(..)) && atom_tex_class(a) == Some(3));
    if repair {
        nodes.insert(0, Atom::Identifier(vec![]));
    }
    Atom::Sequence(nodes)
}
impl Parser {
    // parseChild gives a genuine TexParser its own macro counter, while
    // retaining shared declarations and restoring the caller even on failure.
    fn alignment_child(&self, source: &str) -> Option<Atom> {
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let result = self.padded_parse_string(source);
        self.ams.borrow_mut().paired_macro_count = count;
        result
    }
    // Mathtools GetNext uses ECMAScript whitespace, including BOM.
    fn mathtools_space(&mut self) {
        while self.chars.get(self.index).is_some_and(Self::prime_space) {
            self.index += 1;
        }
    }
    fn mathtools_star(&mut self) -> bool {
        self.mathtools_space();
        let star = self.chars.get(self.index) == Some(&'*');
        if star {
            self.index += 1;
        }
        star
    }
    // TexParser.GetUpTo: protect braces and control words, retaining source
    // bytes through the point before whitespace preceding the delimiter.
    fn alignment_up_to(&mut self, token: &str) -> Result<String, String> {
        self.mathtools_space();
        let start = self.index;
        let mut depth = 0usize;
        while self.index < self.chars.len() {
            let end = self.index;
            self.mathtools_space();
            let Some(&c) = self.chars.get(self.index) else {
                break;
            };
            self.index += 1;
            let current = match c {
                '\\' => format!("\\{}", self.capture_control_sequence()),
                '{' => {
                    depth += 1;
                    c.to_string()
                }
                '}' => {
                    if depth == 0 {
                        return Err(format!("Extra close brace while looking for {token}"));
                    }
                    depth -= 1;
                    c.to_string()
                }
                _ => c.to_string(),
            };
            if depth == 0 && current == token {
                return Ok(self.chars[start..end].iter().collect());
            }
        }
        Err(format!("Could not find {token} for \\Aboxed"))
    }
    pub(super) fn equation_command(
        &mut self,
        name: &str,
        children: &mut Vec<Atom>,
        actual: bool,
    ) -> Result<bool, String> {
        let star = name == "shortvdotswithin" && self.mathtools_star();
        let equation = self
            .array_cell
            .as_ref()
            .and_then(|cell| cell.equation.clone());
        if !actual
            || self
                .array_cell
                .as_ref()
                .is_none_or(|cell| !cell.shove_allowed)
            || children.last().is_some_and(physics::named_function)
            || (name != "vdotswithin" && equation.is_none())
        {
            let guard = if name == "shortvdotswithin" {
                "MTFlushSpaceAbove"
            } else {
                name
            };
            return Err(format!(
                "\\{guard} can only be used in aligment environments"
            ));
        }
        match name {
            "ArrowBetweenLines" => {
                let equation = equation.unwrap();
                if !children.is_empty()
                    || !self.array_cell.as_ref().unwrap().at_start
                    || !equation.borrow().entries.is_empty()
                {
                    return Err("\\ArrowBetweenLines must be on a row by itself".into());
                }
                let star = self.mathtools_star();
                let present =
                    self.chars[self.index..].iter().find(|c| !c.is_whitespace()) == Some(&'[');
                let symbol = self.padded_brackets().map_err(|e| {
                    if e == "Could not find closing ']' for argument" {
                        format!("{e} to \\{name}")
                    } else {
                        e
                    }
                })?;
                let symbol = if present {
                    symbol
                } else {
                    "\\Updownarrow".into()
                };
                if star {
                    equation.borrow_mut().end_entry(Atom::Sequence(vec![]));
                    equation.borrow_mut().end_entry(Atom::Sequence(vec![]));
                }
                let expansion = if star {
                    format!("\\quad{symbol}")
                } else {
                    format!("{symbol}\\quad")
                };
                let Some(atom) = self.alignment_child(&expansion) else {
                    return Ok(false);
                };
                if let Some(e) = cd::first_error(&atom) {
                    return Err(e.to_owned());
                }
                equation.borrow_mut().end_entry(atom);
                if equation.borrow_mut().end_row(&self.ams).is_none() {
                    return Ok(false);
                }
            }
            "Aboxed" => {
                let equation = equation.unwrap();
                if equation.borrow().entries.len() % 2 == 1 {
                    equation.borrow_mut().end_entry(Atom::Sequence(vec![]));
                }
                self.mathtools_space();
                let argument = self.padded_raw_argument(name)?;
                let rest: String = self.chars[self.index..].iter().collect();
                self.chars = format!("{argument}&&\\endAboxed").chars().collect();
                self.index = 0;
                let left = self.alignment_up_to("&")?;
                let right = self.alignment_up_to("&")?;
                self.alignment_up_to("\\endAboxed")?;
                self.chars = format!("\\rlap{{\\boxed{{{left}{{}}{right}}}}}\\kern.267em\\phantom{{{left}}}&\\phantom{{{{}}{right}}}\\kern.267em{rest}").chars().collect();
                self.index = 0;
            }
            "MTFlushSpaceAbove" => {
                let equation = equation.unwrap();
                let mut e = equation.borrow_mut();
                e.flush_above = Some(e.rows.len());
                let adjust = self.ams.borrow().option("shortvdotsadjustabove");
                e.add_spacing(length::dimension(&format!("-{adjust}")).em(0., 1.));
            }
            "MTFlushSpaceBelow" => {
                let equation = equation.unwrap();
                let mut e = equation.borrow_mut();
                if !children.is_empty() {
                    e.end_entry(Atom::Sequence(std::mem::take(children)));
                }
                if e.end_row(&self.ams).is_none() {
                    return Ok(false);
                }
                let adjust = self.ams.borrow().option("shortvdotsadjustbelow");
                e.add_spacing(length::dimension(&format!("-{adjust}")).em(0., 1.));
            }
            "vdotswithin" | "shortvdotswithin" => {
                if name == "shortvdotswithin" {
                    let mut e = equation.as_ref().unwrap().borrow_mut();
                    e.flush_above = Some(e.rows.len());
                    let adjust = self.ams.borrow().option("shortvdotsadjustabove");
                    e.add_spacing(length::dimension(&format!("-{adjust}")).em(0., 1.));
                    if !star {
                        e.end_entry(Atom::Sequence(std::mem::take(children)));
                    }
                }
                let flush = equation.as_ref().is_some_and(|e| {
                    let e = e.borrow();
                    e.flush_above == Some(e.rows.len())
                });
                self.mathtools_space();
                let argument = self.padded_raw_argument("vdotswithin")?;
                // Empty mi tokens bound argument spacing in the same child parser.
                let Some(base) = self.alignment_child(&format!(
                    "\\mmlToken{{mi}}{{}}{argument}\\mmlToken{{mi}}{{}}"
                )) else {
                    return Ok(false);
                };
                if let Some(e) = cd::first_error(&base) {
                    return Err(e.to_owned());
                }
                let nodes = match base {
                    Atom::Sequence(v) => v,
                    a => vec![a],
                };
                let inner = Atom::Padded {
                    body: Box::new(Atom::OperatorToken(vec![Atom::Glyph(0x22ee)])),
                    padding: padded::Padding {
                        width_zero: true,
                        lspace_width: -0.5,
                        height_add: if flush { -0.6 } else { 0. },
                        voffset_em: if flush { -0.18 } else { 0. },
                        ..Default::default()
                    },
                };
                children.push(Atom::Padded {
                    body: Box::new(Atom::Sequence(vec![
                        inner,
                        Atom::Phantom(Box::new(Atom::InferredRow(nodes))),
                    ])),
                    padding: padded::Padding {
                        lspace_width: 0.5,
                        ..Default::default()
                    },
                });
                if name == "shortvdotswithin" {
                    let mut e = equation.as_ref().unwrap().borrow_mut();
                    if !children.is_empty() {
                        e.end_entry(Atom::Sequence(std::mem::take(children)));
                    }
                    if e.end_row(&self.ams).is_none() {
                        return Ok(false);
                    }
                    let adjust = self.ams.borrow().option("shortvdotsadjustbelow");
                    e.add_spacing(length::dimension(&format!("-{adjust}")).em(0., 1.));
                }
            }
            _ => unreachable!(),
        }
        Ok(true)
    }
}

// layout.Em serializes row-spacing updates through exact-binary JS ToFixed(3).
pub(super) fn row_spacing_em(value: f64) -> f64 {
    if value < 0.001 {
        return 0.;
    }
    round_thousandth(value)
}
pub(super) fn round_thousandth(value: f64) -> f64 {
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let numerator = (((bits & ((1_u64 << 52) - 1)) | (1_u64 << 52)) as u128) * 1000;
    let rounded = if exponent >= 0 {
        numerator.checked_shl(exponent as u32)
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            Some(0)
        } else {
            let q = numerator >> shift;
            let r = numerator & ((1_u128 << shift) - 1);
            Some(q + u128::from(r >= 1_u128 << (shift - 1)))
        }
    };
    rounded.map_or(value, |v| v as f64 / 1000.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_arrow_child_has_an_independent_macro_budget() {
        // Reach the caller's limit through public zero-argument declarations.
        // The child can still expand one macro; an outer expansion cannot.
        let prefix = r"\DeclarePairedDelimiterX{\p}[0]{}{}{}\begin{align}";
        let full = format!(
            "{prefix}{}\\ArrowBetweenLines[\\p]\\end{{align}}",
            r"\p".repeat(1000)
        );
        assert_eq!(
            render_exact(&full),
            render_exact(r"\begin{align}\ArrowBetweenLines[]\end{align}")
        );
        let exceeded = full.replace(r"\end{align}", r"\p\end{align}");
        let svg = render_exact(&exceeded).unwrap();
        assert!(svg.contains("maximum macro substitution count exceeded"));
    }

    #[test]
    fn direct_end_row_keeps_the_font_but_cr_starts_a_fresh_cell() {
        let direct = render_exact(r"\begin{align}\bf\ArrowBetweenLines[x]y\end{align}").unwrap();
        let cr = render_exact(r"\begin{align}\bf\ArrowBetweenLines[x]\\y\end{align}").unwrap();
        // EndRow leaves the SetFont recipient alive; parser-delivered CR
        // starts the next cell in ArrayItem's cleared lexical environment.
        assert!(direct.contains("data-c=\"1D432\""));
        assert!(cr.contains("data-c=\"1D466\""));
    }
}
