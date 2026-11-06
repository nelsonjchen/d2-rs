// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/root_index.go; genfrac_palette.go:parseRootIndex;
// commands.go:Sqrt,Root. Source: MathJax 3.2.2 BaseMethods.MoveRoot.
use super::*;

#[derive(Default)]
pub(super) struct Moves {
    up: Option<String>,
    left: Option<String>,
}
pub(super) type Shared = std::rc::Rc<std::cell::RefCell<Moves>>;

// ArrayItem subclasses start a fresh lexical environment. Wrapper/equation
// handlers retain the enclosing index flag (rootIndexArrayEnvironment).
pub(super) fn array_environment(name: &str) -> bool {
    known_environment(name) && !matches!(name, "equation" | "equation*" | "spreadlines")
}

fn offset(raw: &str) -> String {
    let raw = raw.trim_start_matches(internal_text::space);
    let prefix: String = raw
        .chars()
        .enumerate()
        .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && matches!(c, '+' | '-')))
        .map(|(_, c)| c)
        .collect();
    let value = prefix.parse::<f64>().unwrap_or(f64::NAN) / 15.;
    let text = if value.is_nan() {
        "NaN".into()
    } else if value.is_infinite() {
        if value.is_sign_negative() {
            "-Infinity"
        } else {
            "Infinity"
        }
        .into()
    } else if value == 0. {
        "0".into()
    } else {
        let text = d2_rs_core::go_json_float(value);
        if let Some((mantissa, exponent)) = text.split_once('e') {
            format!("{mantissa}e{:+}", exponent.parse::<i32>().unwrap())
        } else {
            text
        }
    };
    format!("{}{text}em", if text.starts_with('-') { "" } else { "+" })
}

impl Parser {
    pub(super) fn move_root(&mut self, name: &str) -> Result<(), String> {
        if !self.in_root {
            return Err(format!("\\{name} can appear only within a root"));
        }
        let used = {
            let moves = self.root_moves.borrow();
            if name == "leftroot" {
                moves.left.is_some()
            } else {
                moves.up.is_some()
            }
        };
        if used {
            return Err(format!("Multiple use of \\{name}"));
        }
        let raw = self.padded_raw_argument(name)?;
        // The Go validity regexp is deliberately unanchored (-?[0-9]+).
        // parseInt, independently, only accepts a signed decimal prefix.
        if !raw.chars().any(|c| c.is_ascii_digit()) {
            return Err(format!("The argument to \\{name} must be an integer"));
        }
        let value = offset(&raw);
        let mut moves = self.root_moves.borrow_mut();
        if name == "leftroot" {
            moves.left = Some(value);
        } else {
            moves.up = Some(value);
        }
        Ok(())
    }

    fn root_index(&self, raw: &str) -> Option<Atom> {
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let mut child = self.padded_child(raw);
        child.in_root = true;
        let moves = child.root_moves.clone();
        let index = child.sequence(None);
        self.ams.borrow_mut().paired_macro_count = count;
        let index = index?;
        if cd::first_error(&index).is_some() {
            return Some(index);
        }
        let moves = moves.borrow();
        if moves.up.is_none() && moves.left.is_none() {
            return Some(index);
        }
        Some(Atom::Padded {
            body: Box::new(index),
            padding: padded::Padding {
                root: Some([moves.left.clone(), moves.up.clone()]),
                ..Default::default()
            },
        })
    }

    pub(super) fn root_command(&mut self, name: &str) -> Option<Atom> {
        let raw_index = if name == "sqrt" {
            match self.padded_brackets() {
                Ok(raw) => raw,
                Err(error) => return Some(Atom::ErrorDisplay(error)),
            }
        } else {
            let start = self.index;
            let mut depth = 0usize;
            loop {
                if self.index == self.chars.len() {
                    return Some(Atom::ErrorDisplay("Could not find \\of for \\root".into()));
                }
                if depth == 0 && self.chars[self.index..].starts_with(&['\\', 'o', 'f']) {
                    let raw = self.chars[start..self.index].iter().collect();
                    self.index += 3;
                    break raw;
                }
                match self.chars[self.index] {
                    '{' => depth += 1,
                    '}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                self.index += 1;
            }
        };
        let mut raw = match self.padded_raw_argument(name) {
            Ok(raw) => raw,
            Err(error) => return Some(Atom::ErrorDisplay(error)),
        };
        // Sqrt collects a following bare fraction before its child parser.
        if name == "sqrt" && raw == "\\frac" {
            for _ in 0..2 {
                match self.padded_raw_argument(name) {
                    Ok(argument) => raw.push_str(&format!("{{{argument}}}")),
                    Err(error) => return Some(Atom::ErrorDisplay(error)),
                }
            }
        }
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let body = self.padded_parse_string(&raw);
        self.ams.borrow_mut().paired_macro_count = count;
        let body = body?;
        if cd::first_error(&body).is_some() {
            return Some(body);
        }
        if name == "sqrt" && raw_index.is_empty() {
            return Some(Atom::Sqrt(Box::new(body)));
        }
        let index = self.root_index(&raw_index)?;
        if cd::first_error(&index).is_some() {
            return Some(index);
        }
        Some(Atom::Root {
            body: Box::new(body),
            index: Box::new(index),
            mathsize: 1.,
        })
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn root_arguments_and_moves_preserve_pinned_diagnostic_order() {
        for (tex, error) in [
            (r"\uproot", r"\uproot can appear only within a root"),
            (
                r"\leftroot{bad}",
                r"\leftroot can appear only within a root",
            ),
            (r"\root", r"Could not find \of for \root"),
            (r"\root n\of", r"Missing argument for \root"),
            (
                r"\sqrt[\uproot{bad}]{\bad}",
                r"Undefined control sequence \bad",
            ),
            (
                r"\sqrt[\uproot{bad}]{x}",
                r"The argument to \uproot must be an integer",
            ),
            (r"\sqrt[\uproot{1}\uproot]{x}", r"Multiple use of \uproot"),
            (
                r"\sqrt[\leftroot{1}\leftroot{bad}]{x}",
                r"Multiple use of \leftroot",
            ),
        ] {
            assert_eq!(render_exact(tex).unwrap(), cd::render_error(error), "{tex}");
        }
        for tex in [
            r"\sqrt[\uproot{x2}n]{x}",
            r"\sqrt[\uproot{.2}n]{x}",
            r"\sqrt[n^\uproot{2}a]{x}",
        ] {
            assert!(
                !render_exact(tex).unwrap().contains("data-mjx-error"),
                "{tex}"
            );
        }
        assert_eq!(
            render_exact(r"\sqrt[]{x}").unwrap(),
            render_exact(r"\sqrt{x}").unwrap()
        );
        assert_eq!(
            render_exact(r"\sqrt\frac{x}{y}").unwrap(),
            render_exact(r"\sqrt{\frac{x}{y}}").unwrap()
        );
    }

    #[test]
    fn root_index_scope_distinguishes_groups_children_arrays_and_wrappers() {
        // Braces share Stack.global, a genuine argument parser owns another.
        assert_eq!(
            render_exact(r"\sqrt[{\uproot{2}n}\uproot{3}]{x}").unwrap(),
            cd::render_error(r"Multiple use of \uproot")
        );
        assert_eq!(
            render_exact(r"\sqrt[\frac{\uproot{2}a}{b}\uproot{3}n]{x}").unwrap(),
            render_exact(r"\sqrt[\frac{a}{b}\uproot{3}n]{x}").unwrap()
        );
        assert_eq!(
            render_exact(r"\sqrt[\begin{matrix}\uproot{2}n\end{matrix}]{x}").unwrap(),
            cd::render_error(r"\uproot can appear only within a root")
        );
        for env in ["equation", "spreadlines"] {
            let option = if env == "spreadlines" { "{1pt}" } else { "" };
            let tex = format!("\\sqrt[\\begin{{{env}}}{option}\\uproot{{2}}n\\end{{{env}}}]{{x}}");
            assert!(
                !render_exact(&tex).unwrap().contains("data-mjx-error"),
                "{tex}"
            );
        }
        assert_eq!(
            render_exact(r"\sqrt[\uproot{2}n]{x}\uproot{1}").unwrap(),
            cd::render_error(r"\uproot can appear only within a root")
        );
    }

    #[test]
    fn root_index_parser_restores_counter_on_success_and_failure() {
        let mut caller = Parser {
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
            active_font: None,
            normal_font: false,
            calligraphic_font: false,
            multi_letter_font: false,
            operator_letters: false,
            operator_pattern: false,
            no_auto_operator: false,
        };
        caller.ams.borrow_mut().paired_macro_count = 1000;
        for source in [r"\sqrt[\dv{f}{x}]{x}", r"\sqrt[\uproot{bad}]{x}"] {
            caller.chars = source.chars().collect();
            caller.index = 0;
            assert!(caller.sequence(None).is_some());
            assert_eq!(caller.ams.borrow().paired_macro_count, 1000);
            assert!(!caller.in_root);
            assert!(caller.root_moves.borrow().up.is_none());
        }
    }
}
