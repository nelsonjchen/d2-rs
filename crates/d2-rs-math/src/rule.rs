// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 internal/tex/rule.go and commands.go.
use super::*;

impl Parser {
    // tex/commands.go rule accepts an unvalidated optional voffset, then
    // reads two dimensions. Rule/Space use the separate three-dimension form.
    pub(super) fn rule(&mut self) -> Option<Atom> {
        let offset = match self.padded_brackets() {
            Ok(raw) => raw,
            Err(message) => {
                return Some(Atom::ErrorDisplay(
                    if message == "Could not find closing ']' for argument" {
                        format!("{message} to \\rule")
                    } else {
                        message
                    },
                ));
            }
        };
        let mut dimensions = [Dimension::relative(0.); 3];
        for dimension in &mut dimensions[..2] {
            let raw = match self.read_dimension("rule") {
                Ok(raw) => raw,
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            *dimension = length::dimension(&raw);
        }
        let space = Atom::RuleSpace {
            dimensions,
            background: Some(self.active_color.clone().unwrap_or_else(|| "black".into())),
            mathsize: 1.,
        };
        if offset.is_empty() {
            return Some(space);
        }
        let adjustments = if let Some(positive) = offset.strip_prefix('-') {
            [offset.clone(), format!("+{positive}")]
        } else {
            [format!("+{offset}"), "+0em".into()]
        };
        Some(Atom::Padded {
            body: Box::new(space),
            padding: padded::Padding {
                position: Some(adjustments),
                voffset: Some(offset),
                ..Default::default()
            },
        })
    }

    pub(super) fn rule_space(&mut self, name: &str) -> Option<Atom> {
        let mut dimensions = [Dimension::relative(0.); 3];
        for dimension in &mut dimensions {
            let raw = match self.read_dimension(name) {
                Ok(raw) => raw,
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            *dimension = length::dimension(&raw);
        }
        Some(Atom::RuleSpace {
            dimensions,
            background: (name != "Space")
                .then(|| self.active_color.clone().unwrap_or_else(|| "black".into())),
            mathsize: 1.,
        })
    }
}

#[cfg(test)]
mod contracts {
    use super::*;
    #[test]
    fn rule_spaces_keep_all_dimensions_signed_bounds_and_lexical_backgrounds() {
        let rule = render_exact(r"\Rule{1em}{.4em}{.2em}").unwrap();
        let space = render_exact(r"\Space{1em}{.4em}{.2em}").unwrap();
        for svg in [&rule, &space] {
            let document = roxmltree::Document::parse(svg).unwrap();
            assert_eq!(
                document.root_element().attribute("viewBox"),
                Some("0 -400 1000 600")
            );
        }
        assert!(rule.contains(
            "fill=\"black\" x=\"0\" y=\"-200\" width=\"1000\" height=\"600\" data-bgcolor=\"true\""
        ));
        assert!(!space.contains("data-bgcolor"));
        let colored =
            render_exact(r"\textcolor{red}{\Rule{1em}{.4em}{.2em}}\Rule{1em}{.4em}{.2em}").unwrap();
        assert!(colored.contains("<rect fill=\"red\""));
        assert!(colored.contains("<rect fill=\"black\""));
        let negative = render_exact(r"\Space{-1em}{-.4em}{-.2em}").unwrap();
        let document = roxmltree::Document::parse(&negative).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some("0 400 16 16")
        );
        assert_eq!(
            render(r"\Rule{1em}{.4em}").unwrap(),
            cd::render_error(r"Missing dimension or its units for \Rule")
        );

        // Pinned tex/commands.go rule and svg/mpadded.go keep a rule's
        // offset separate from its signed height/depth adjustments.
        for (expression, bounds, shift) in [
            (r"\rule{1em}{.4em}", "0 -400 1000 400", None),
            (r"\rule[.2em]{1em}{.4em}", "0 -600 1000 600", Some("200")),
            (r"\rule[-.2em]{1em}{.4em}", "0 -200 1000 400", Some("-200")),
            (r"\rule[+1em]{1em}{.4em}", "0 -800 1000 800", Some("1000")),
            (
                r"\rule[2height]{1em}{.4em}",
                "0 -1200 1000 1200",
                Some("800"),
            ),
            (r"\rule[50%]{1em}{.4em}", "0 -600 1000 600", None),
            (r"\rule[-2height]{1em}{.4em}", "0 0 1000 1200", Some("-800")),
            (
                r"\rule[.5width]{1em}{.4em}",
                "0 -1500 1000 1500",
                Some("500"),
            ),
            (r"\rule[1depth]{1em}{.4em}", "0 0 1000 16", None),
        ] {
            let svg = render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(
                document.root_element().attribute("viewBox"),
                Some(bounds),
                "{expression}"
            );
            let space = document
                .descendants()
                .find(|node| node.attribute("data-mml-node") == Some("mspace"))
                .unwrap();
            assert_eq!(
                space.parent_element().unwrap().attribute("transform"),
                shift.map(|y| format!("translate(0,{y})")).as_deref(),
                "{expression}"
            );
        }
        let colored = render_exact(r"\textcolor{red}{\rule{1em}{.4em}}\rule{1em}{.4em}").unwrap();
        assert!(colored.contains("<rect fill=\"red\""));
        assert!(colored.contains("<rect fill=\"black\""));
        for (expression, bounds) in [
            (r"\color{red}a = \\ \rule{1em}{.4em}", "0 -583 2584.8 665"),
            (
                r"\color{red}a = \\ \rule[.2em]{1em}{.4em}",
                "0 -600 2584.8 682",
            ),
        ] {
            let svg = render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(document.root_element().attribute("viewBox"), Some(bounds));
        }
        for (expression, message) in [
            (
                r"\rule[1em{1em}{.4em}",
                r"Could not find closing ']' for argument to \rule",
            ),
            (
                r"\rule[}]{1em}{.4em}",
                "Extra close brace while looking for ']'",
            ),
            (r"\rule{1em}", r"Missing dimension or its units for \rule"),
        ] {
            assert_eq!(render(expression).unwrap(), cd::render_error(message));
        }

        // Pinned svg/wrapper.go computeBBox and mpadded.go paddedDimension
        // resolve absolute lengths against both scriptlevel and mathsize.
        for (expression, bounds) in [
            (r"\tiny \rule[2px]{20px}{4px}", "0 -331.5 1105 331.5"),
            (r"\tiny \rule[-2px]{20px}{4px}", "0 -110.5 1105 221"),
            (r"\tiny\colorbox{red}{x}", "0 -491.8 816.5 768"),
        ] {
            let svg = render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(
                document.root_element().attribute("viewBox"),
                Some(bounds),
                "{expression}"
            );
        }
    }
}
