// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/array_body.go and cell_item.go: ArrayItem recipients,
// cell closing events, fresh lexical environments, and per-row CR spacing.
use super::*;

// tex/matrix_command.go matrixDimensionEm is ParseUtil.dimen2em, not
// the SVG renderer's CSS Length2Em. Input has already passed GetDimen.
pub(super) fn dimension_em(raw: &str) -> f64 {
    let raw = raw.trim();
    let Some(split) = raw.len().checked_sub(2) else {
        return 0.0;
    };
    let Some(value) = raw.get(..split).and_then(|s| s.trim().parse::<f64>().ok()) else {
        return 0.0;
    };
    match &raw[split..] {
        "em" => value,
        "ex" => value * 0.43,
        "pt" => value / 10.0,
        "pc" => value * 1.2,
        "px" => (value * 7.2) / 72.0,
        "in" => value * 7.2,
        "cm" => (value * 7.2) / 2.54,
        "mm" => (value * 7.2) / 25.4,
        "mu" => value / 18.0,
        _ => 0.0,
    }
}

#[derive(Clone, Debug)]
pub(super) struct CellItem {
    pub cr: bool,
    pub spacing: Option<String>,
}
impl CellItem {
    pub fn entry() -> Self {
        Self {
            cr: false,
            spacing: None,
        }
    }

    pub fn spaces(self) -> Vec<Atom> {
        let mut nodes = Vec::new();
        if let Some(spacing) = self.spacing {
            nodes.push(Atom::Strut(
                0.,
                length::dimension(&spacing).em(0., 1.) * 1000.,
                "mspace",
            ));
        }
        nodes.push(Atom::EmptyNode("mspace"));
        nodes
    }
}

// cell_item.go saves lexical state once, before closing styles or fractions.
// Open owners swallow line breaks and resume with that same font/color env.
pub(super) struct PendingCell {
    pub cell: CellItem,
    pub linebreak: bool,
    pub font: Option<&'static str>,
    pub color: Option<String>,
}
#[derive(Debug)]
pub(super) struct CellState {
    pub environment: String,
    pub fills: hfill::Fills,
    pub rules: Option<array_rules::Shared>,
    pub rule_allowed: bool,
    pub equation: Option<alignment::Shared>,
    pub at_start: bool,
    pub shove_allowed: bool,
    pub stop: Option<CellItem>,
    pub shove: Option<&'static str>,
    pub cd_end: Option<bool>,
}

pub(super) struct Body {
    pub rows: Vec<Vec<Atom>>,
    pub align: Vec<Vec<&'static str>>,
    pub spacing: Vec<f64>,
    pub rules: array_rules::Rules,
}

impl Parser {
    pub(super) fn cell_command(&mut self, name: &str) -> Option<Atom> {
        let mut spacing = None;
        if name == "\\" {
            if self.chars.get(self.index) == Some(&'*') {
                self.index += 1;
            }
            if self.chars.get(self.index) == Some(&'[') {
                let raw = match self.padded_brackets() {
                    Ok(raw) => raw,
                    Err(_) => {
                        return Some(Atom::ErrorDisplay(
                            "Could not find closing ']' for argument to \\\\".into(),
                        ));
                    }
                };
                if !raw.is_empty() {
                    // Validate with the same full dimension reader as CrLaTeX.
                    let mut dimension = self.padded_child(&format!("{{{raw}}}"));
                    match dimension.read_dimension("\\") {
                        Ok(value) => spacing = Some(value),
                        Err(_) => {
                            return Some(Atom::ErrorDisplay(
                                "Bracket argument to \\\\ must be a dimension".into(),
                            ));
                        }
                    }
                }
            }
        }
        if let Some(cell) = &mut self.array_cell {
            cell.stop = Some(CellItem { cr: true, spacing });
            return Some(Atom::Sequence(Vec::new()));
        }
        self.pending_cell = Some(PendingCell {
            cell: CellItem { cr: true, spacing },
            linebreak: name != "cr",
            font: self.active_font,
            color: self.active_color.clone(),
        });
        Some(Atom::Sequence(Vec::new()))
    }

    // A genuine child parser never inherits an enclosing ArrayItem recipient.
    pub(super) fn padded_child(&self, raw: &str) -> Parser {
        Parser {
            in_root: self.in_root,
            root_moves: Default::default(),
            physics_after: None,
            physics_auto: None,
            command_position: None,
            pending_cell: None,
            pending_infix: None,
            row_delimiter: None,
            active_color: self.active_color.clone(),
            equation_seen: Default::default(),
            array_cell: None,
            vector_alias: self.vector_alias,
            vector_font: self.vector_font,
            ams: self.ams.clone(),
            splice_inferred: false,
            chars: raw.chars().collect(),
            index: 0,
            active_font: self.active_font,
            normal_font: self.normal_font,
            calligraphic_font: self.calligraphic_font,
            multi_letter_font: false,
            operator_letters: self.operator_letters,
            operator_pattern: self.operator_pattern,
            no_auto_operator: self.no_auto_operator,
        }
    }
}

// parseArrayBody resumes the *expanded* source and cursor, rather than a
// substring from a pre-split input. Explicit Entry/CR commits even empty cells;
// EOF drops only the pending empty last row. Fresh parsers implement clearEnv.
pub(super) fn parse(
    content: &str,
    environment: &str,
    ams: ams::Shared,
    equation_seen: EquationStack,
) -> Option<Body> {
    let rules = array_rules::Shared::default();
    let mut source: Vec<char> = content.chars().collect();
    let mut position = 0;
    let mut rows = Vec::new();
    let mut align = Vec::new();
    let mut entries = Vec::new();
    let mut entry_align = Vec::new();
    let mut adjustments = Vec::new();
    let equation = alignment::is_equation(environment)
        .then(|| std::rc::Rc::new(std::cell::RefCell::new(alignment::Equation::default())));
    loop {
        let mut parser = Parser {
            in_root: false,
            root_moves: Default::default(),
            physics_after: None,
            physics_auto: None,
            command_position: None,
            pending_cell: None,
            pending_infix: None,
            row_delimiter: None,
            active_color: None,
            equation_seen: equation_seen.clone(),
            array_cell: Some(CellState {
                environment: environment.into(),
                fills: Default::default(),
                rules: Some(rules.clone()),
                rule_allowed: true,
                equation: equation.clone(),
                at_start: true,
                shove_allowed: true,
                stop: None,
                shove: None,
                cd_end: None,
            }),
            vector_alias: false,
            vector_font: None,
            ams: ams.clone(),
            splice_inferred: false,
            chars: source,
            index: position,
            active_font: None,
            normal_font: false,
            calligraphic_font: false,
            multi_letter_font: false,
            operator_letters: false,
            operator_pattern: false,
            no_auto_operator: false,
        };
        let cell = parser.sequence(None)?;
        let state = parser.array_cell.take()?;
        if cd::first_error(&cell).is_some() {
            return Some(Body {
                rows: vec![vec![cell]],
                align: Vec::new(),
                spacing: Vec::new(),
                rules: Default::default(),
            });
        }
        if let Some(equation) = &equation {
            let mut e = equation.borrow_mut();
            let empty = matches!(&cell, Atom::Sequence(v) if v.is_empty());
            if state.stop.is_none() && empty && e.entries.is_empty() {
                break;
            }
            e.end_entry(cell);
            if state.stop.as_ref().is_none_or(|item| item.cr) {
                e.end_row(&ams)?;
                if let Some(value) = state.stop.as_ref().and_then(|item| item.spacing.as_ref()) {
                    e.add_spacing(dimension_em(value));
                }
            }
            if state.stop.is_none() {
                break;
            }
            rules.borrow_mut().rows = e.rows.len();
            source = parser.chars;
            position = parser.index;
            continue;
        }
        let empty = matches!(&cell, Atom::Sequence(v) if v.is_empty());
        if state.stop.is_none() && empty && entries.is_empty() {
            break;
        }
        // Multline/Flalign own their mtd alignment and discard HFill metadata.
        let multline = environment.contains("multline");
        let align_fills = !multline
            && !matches!(
                environment,
                "flalign"
                    | "flalign*"
                    | "xalignat"
                    | "xalignat*"
                    | "xxalignat"
                    | "eqnarray"
                    | "eqnarray*"
            );
        entry_align.push(
            align_fills
                .then(|| state.fills.alignment(hfill::size(&cell)))
                .flatten()
                .or(state.shove)
                .unwrap_or(if multline { "center" } else { "" }),
        );
        entries.push(cell);
        if state.stop.as_ref().is_none_or(|item| item.cr) {
            if environment.contains("multline") && entries.len() != 1 {
                return Some(Body {
                    rows: vec![vec![Atom::ErrorDisplay(
                        "The rows within the multline environment must have exactly one column"
                            .into(),
                    )]],
                    align: Vec::new(),
                    spacing: Vec::new(),
                    rules: Default::default(),
                });
            }
            rows.push(std::mem::take(&mut entries));
            rules.borrow_mut().rows = rows.len();
            align.push(std::mem::take(&mut entry_align));
            if matches!(environment, "split" | "align" | "align*" | "aligned") {
                ams::end_row(&ams)?;
            }
            if let Some(value) = state.stop.as_ref().and_then(|item| item.spacing.as_ref()) {
                adjustments.push((rows.len() - 1, dimension_em(value)));
            }
        }
        if state.stop.is_none() {
            break;
        }
        source = parser.chars;
        position = parser.index;
    }
    if let Some(equation) = equation {
        let mut e = equation.borrow_mut();
        let spacing = e.finish_spacing();
        return Some(Body {
            rows: std::mem::take(&mut e.rows),
            align: Vec::new(),
            spacing,
            rules: rules.borrow().clone(),
        });
    }
    let mut spacing = Vec::new();
    if !adjustments.is_empty() {
        let base = table::base_spacing(environment);
        spacing = vec![alignment::row_spacing_em(base); rows.len()];
        for (row, adjustment) in adjustments {
            spacing[row] = alignment::row_spacing_em((base + adjustment).max(0.));
        }
    }
    Some(Body {
        rows,
        align,
        spacing,
        rules: rules.borrow().clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_row_adjustments_use_tex_units_in_every_style() {
        // matrix_command.go matrixDimensionEm, followed by arraySpacingEm's
        // three-decimal serialization. CSS physical lengths differ here.
        for (dimension, em) in [
            ("1em", "1"),
            ("1ex", ".43"),
            ("1pt", ".1"),
            ("1pc", "1.2"),
            ("1px", ".1"),
            ("1in", "7.2"),
            ("1cm", "2.835"),
            ("1mm", ".283"),
            ("1mu", ".056"),
        ] {
            for (prefix, suffix) in [("", ""), (r"\scriptstyle ", ""), (r"\Huge{", "}")] {
                assert_eq!(
                    render_exact(&format!(
                        r"{prefix}\displaylines{{x\\[{dimension}]y}}{suffix}"
                    )),
                    render_exact(&format!(r"{prefix}\displaylines{{x\\[{em}em]y}}{suffix}")),
                    "{prefix}: {dimension}",
                );
            }
        }
    }

    #[test]
    fn array_arguments_own_tokens_before_the_cell_recipient() {
        // GetArgument consumes the ampersand as the denominator; its child
        // parser reports Misplaced rather than ending an empty fraction cell.
        assert_eq!(
            render_exact(r"\begin{matrix}\frac{a}&b\end{matrix}"),
            render_exact("&"),
        );
        // Text arguments keep literal ampersands and row-break spelling.
        assert!(render_exact(r"\begin{matrix}\text{a&b}&c\end{matrix}").is_ok());
    }

    #[test]
    fn array_linebreaks_clear_the_cell_font_environment() {
        for separator in [r"\cr", r"\newline", r"\\", r"\\*"] {
            assert_eq!(
                render_exact(&format!(
                    r"\begin{{matrix}}\bf a{separator} b\end{{matrix}}"
                )),
                render_exact(r"\begin{matrix}\bf a\\b\end{matrix}"),
                "{separator}",
            );
        }
    }

    #[test]
    fn array_cr_dimensions_are_validated_before_delivering_the_row() {
        for expression in [r"\begin{matrix}x\\[bad]y\end{matrix}", r"\\[bad]"] {
            let svg = render_exact(expression).unwrap();
            assert!(svg.contains("Bracket argument to \\\\ must be a dimension"));
        }
        assert_eq!(
            render_exact(r"\begin{matrix}a\\*[-2em]b\end{matrix}"),
            render_exact(r"\begin{matrix}a\\[-2em]b\end{matrix}"),
        );
    }
}
