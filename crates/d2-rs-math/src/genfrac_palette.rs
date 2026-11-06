// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/genfrac_palette.go and base_ams_handlers.go:amsGenfrac.
// parseRootIndex from the same source file lives in root_index.rs.
use super::*;

fn trim_spaces(raw: &str) -> String {
    let mut key = raw.trim_matches(internal_text::space).to_owned();
    if key.ends_with('\\') && raw.ends_with(' ') {
        key.push(' ');
    }
    key
}

// amsGenfracStyleIndex uses parseInt's signed decimal prefix, observing only
// the four palette indices. Do not use floating point or machine-sized ints.
fn style_index(style: &str) -> Option<usize> {
    let mut digits = style.chars().peekable();
    let negative = digits.peek() == Some(&'-');
    if matches!(digits.peek(), Some('+' | '-')) {
        digits.next();
    }
    let mut value = 0;
    let mut count = 0;
    for c in digits.take_while(char::is_ascii_digit) {
        value = value * 10 + c as usize - '0' as usize;
        if value > 3 {
            return None;
        }
        count += 1;
    }
    (count != 0 && (!negative || value == 0)).then_some(value)
}

impl Parser {
    // GetDelimiterArg retains the exact SymbolMap key. An absent delimiter
    // and a present dot have distinct row/withDelims ownership.
    fn genfrac_delimiter(&mut self, name: &str) -> Result<Option<String>, String> {
        let key = trim_spaces(&self.padded_raw_argument(name)?);
        if key.is_empty() {
            return Ok(None);
        }
        if braket::delimiter(&key).is_none() {
            return Err(format!("Missing or unrecognized delimiter for \\{name}"));
        }
        Ok(Some(key))
    }

    pub(super) fn genfrac_palette(&self, fence: &str, side: &str) -> Option<Atom> {
        let fence = if matches!(fence, "{" | "}") {
            format!("\\{fence}")
        } else {
            fence.to_owned()
        };
        let display = format!("{{\\bigg{side} {fence}}}");
        let text = format!("{{\\big{side} {fence}}}");
        let source = format!("\\mathchoice{display}{text}{text}{text}");
        let count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let mut child = self.padded_child(&source);
        child.in_root = false;
        child.active_font = None;
        child.active_color = None;
        child.normal_font = false;
        child.calligraphic_font = false;
        child.multi_letter_font = false;
        child.operator_letters = false;
        child.operator_pattern = false;
        child.no_auto_operator = false;
        child.vector_alias = false;
        let result = child.sequence(None);
        self.ams.borrow_mut().paired_macro_count = count;
        // Tokens have already resolved in the creating parser's lexical font
        // scope; the caller must not apply its ambient font a second time.
        result.map(|result| match result {
            Atom::Sequence(mut children) if children.len() == 1 => children.remove(0),
            other => other,
        })
    }

    pub(super) fn general_fraction_command(&mut self, name: &str) -> Option<Atom> {
        let read = |parser: &mut Self| -> Result<(Option<String>, Option<String>, String, String), String> {
            let left = parser.genfrac_delimiter(name)?;
            let right = parser.genfrac_delimiter(name)?;
            let thickness = parser.padded_raw_argument(name)?;
            let style = trim_spaces(&parser.padded_raw_argument(name)?);
            Ok((left, right, thickness, style))
        };
        let (left, right, thickness, style) = match read(self) {
            Ok(arguments) => arguments,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let numerator = self.padded_argument(name)?;
        if cd::first_error(&numerator).is_some() {
            return Some(numerator);
        }
        let denominator = self.padded_argument(name)?;
        if cd::first_error(&denominator).is_some() {
            return Some(denominator);
        }
        let with_delims = left.is_some() || right.is_some();
        let fraction = Atom::GeneralFraction(
            Box::new(numerator),
            Box::new(denominator),
            general_fraction::Spec::new(
                if thickness.is_empty() {
                    "medium"
                } else {
                    &thickness
                },
                with_delims,
            ),
        );
        let mut content = if with_delims {
            let mut children = Vec::new();
            if let Some(left) = left {
                let palette = self.genfrac_palette(&left, "l")?;
                if cd::first_error(&palette).is_some() {
                    return Some(palette);
                }
                match palette {
                    Atom::Sequence(items) => children.extend(items),
                    item => children.push(item),
                }
            }
            children.push(fraction);
            if let Some(right) = right {
                let palette = self.genfrac_palette(&right, "r")?;
                if cd::first_error(&palette).is_some() {
                    return Some(palette);
                }
                match palette {
                    Atom::Sequence(items) => children.extend(items),
                    item => children.push(item),
                }
            }
            Atom::Fenced {
                close_color: None,
                open: 0,
                body: children,
                close: 0,
                stretchy: false,
                inner: false,
            }
        } else {
            fraction
        };
        // Palette execution precedes style validation, after both arguments.
        if !style.is_empty() {
            let Some(index) = style_index(&style) else {
                return Some(Atom::ErrorDisplay(format!("Bad math style for \\{name}")));
            };
            content = Atom::DeclaredStyle {
                body: Box::new(content),
                display: index == 0,
                level: index.saturating_sub(1),
            };
        }
        Some(content)
    }
}

#[cfg(test)]
mod contract {
    use super::*;

    #[test]
    fn genfrac_retains_exact_delimiter_keys_and_present_dot_slots() {
        for key in ["lt", "vert", "\\ "] {
            let tex = format!("\\genfrac{{{key}}}{{}}{{0}}{{}}{{x}}{{y}}");
            assert_eq!(
                render_exact(&tex).unwrap(),
                cd::render_error(r"Missing or unrecognized delimiter for \genfrac")
            );
        }
        let absent = render_exact(r"\genfrac{}{}{0}{}{x}{y}").unwrap();
        let dotted = render_exact(r"\genfrac{.}{.}{0}{}{x}{y}").unwrap();
        assert!(!absent.contains("data-mjx-texclass=\"OPEN\""));
        assert!(dotted.contains("data-mjx-texclass=\"OPEN\""));
        assert!(dotted.contains("data-mjx-texclass=\"CLOSE\""));
        assert_eq!(
            dotted
                .matches("<g data-mml-node=\"mo\" transform=\"translate(0 250)\"></g>")
                .count(),
            2
        );
        assert_ne!(
            measure(r"\genfrac{}{}{0}{}{x}{y}").unwrap().width,
            measure(r"\genfrac{.}{.}{0}{}{x}{y}").unwrap().width
        );
        for (alias, index) in [
            ("+2tail", "2"),
            ("-0xyz", "0"),
            ("00003.5", "3"),
            ("0x2", "0"),
            ("\u{a0}2\u{2003}", "2"),
        ] {
            assert_eq!(
                render_exact(&format!("\\genfrac{{(}}{{)}}{{0}}{{{alias}}}{{x}}{{y}}")).unwrap(),
                render_exact(&format!("\\genfrac{{(}}{{)}}{{0}}{{{index}}}{{x}}{{y}}")).unwrap()
            );
        }
    }

    #[test]
    fn genfrac_validates_delimiters_arguments_palettes_then_style() {
        for (tex, message) in [
            (
                r"\genfrac{bad}",
                r"Missing or unrecognized delimiter for \genfrac",
            ),
            (
                r"\genfrac{(}{bad}",
                r"Missing or unrecognized delimiter for \genfrac",
            ),
            (
                r"\genfrac{(}{)}{0}{bad}{\unknown}{x}",
                r"Undefined control sequence \unknown",
            ),
            (
                r"\genfrac{(}{)}{0}{bad}{x}",
                r"Missing argument for \genfrac",
            ),
            (
                r"\DeclarePairedDelimiterX{\bigl}[1]{(}{)}{\unknown}\genfrac{(}{)}{0}{bad}{x}{y}",
                r"Undefined control sequence \unknown",
            ),
            (
                r"\genfrac{(}{)}{0}{-1}{x}{y}",
                r"Bad math style for \genfrac",
            ),
            (
                r"\genfrac{(}{)}{0}{4}{x}{y}",
                r"Bad math style for \genfrac",
            ),
        ] {
            assert_eq!(
                render_exact(tex).unwrap(),
                cd::render_error(message),
                "{tex}"
            );
        }
    }

    #[test]
    fn genfrac_palettes_execute_shared_registrations_with_fresh_counters() {
        let registered = r"\DeclareMathOperator{\bigl}{Q}\textstyle\genfrac{(}{)}{0}{}{a}{b}";
        let svg = render_exact(registered).unwrap();
        assert_eq!(svg.matches("data-c=\"51\"").count(), 1);
        let prefix = r"\DeclarePairedDelimiter{\p}{(}{)}\DeclarePairedDelimiter{\biggl}{[}{]}\DeclarePairedDelimiter{\bigl}{[}{]}";
        let tex = format!(
            "{prefix}{}\\genfrac{{(}}{{)}}{{0}}{{}}{{a}}{{b}}\\p{{x}}",
            r"\p0".repeat(998)
        );
        assert!(!render_exact(&tex).unwrap().contains("data-mjx-error"));
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\bigl}{Q}\binom{a}{b}").unwrap(),
            render_exact(r"\binom{a}{b}").unwrap()
        );
        // A palette registration is local to this compilation.
        assert!(
            render_exact(r"\genfrac{(}{)}{0}{}{a}{b}")
                .unwrap()
                .contains("data-mjx-texclass=\"OPEN\"")
        );
    }
}
