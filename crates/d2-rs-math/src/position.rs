// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/position.go and commands.go.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct Position {
    name: String,
    amounts: [String; 2],
    horizontal: bool,
}

impl Position {
    pub(super) fn missing_box(&self) -> Atom {
        Atom::ErrorDisplay(format!("Missing box for \\{}", self.name))
    }
    fn apply(self, child: Atom) -> Vec<Atom> {
        if self.horizontal {
            vec![
                Atom::Kern(length::dimension(&self.amounts[0])),
                child,
                Atom::Kern(length::dimension(&self.amounts[1])),
            ]
        } else {
            vec![Atom::Padded {
                body: Box::new(child),
                padding: padded::Padding {
                    position: Some(self.amounts),
                    ..Default::default()
                },
            }]
        }
    }
}

// Each horizontal position emits three final items; a waiting outer position
// consumes only the first spacer. Vertical positions replace the incoming item.
pub(super) fn deliver(positions: &mut Vec<Position>, atom: Atom) -> Vec<Atom> {
    if positions.is_empty() {
        return vec![atom];
    }
    let named_function = physics::named_function(&atom);
    let mut items = std::collections::VecDeque::from([atom]);
    let mut delivered = Vec::new();
    while let Some(item) = items.pop_front() {
        if let Some(position) = positions.pop() {
            for output in position.apply(item).into_iter().rev() {
                items.push_front(output);
            }
        } else {
            delivered.push(item);
        }
    }
    if named_function {
        vec![Atom::NamedFunction(Box::new(Atom::Sequence(delivered)))]
    } else {
        delivered
    }
}

impl Parser {
    // CrLaTeX validates its optional dimension before the close item reaches
    // PositionItem. Macro dispatch still precedes the built-in close handlers.
    pub(super) fn position_linebreak(&self) -> bool {
        if self.chars.get(self.index) != Some(&'\\') {
            return false;
        }
        let name = if self.chars.get(self.index + 1) == Some(&'\\') {
            "\\".into()
        } else {
            self.chars[self.index + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect::<String>()
        };
        matches!(name.as_str(), "\\" | "cr" | "newline")
            && self.paired_definition(&name).is_none()
            && !self.ams.borrow().macros.contains_key(&name)
    }

    pub(super) fn position_command(&mut self, name: &str) -> Option<Atom> {
        let raw = match self.read_dimension(name) {
            Ok(raw) => raw,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        let horizontal = matches!(name, "moveleft" | "moveright");
        let mut amounts = if horizontal {
            let opposite = raw
                .strip_prefix('-')
                .map(str::to_owned)
                .unwrap_or_else(|| format!("-{raw}"));
            [raw, opposite]
        } else {
            let negative = raw.starts_with('-');
            let amount = raw.strip_prefix('-').unwrap_or(&raw);
            let mut values = [format!("+{amount}"), format!("-{amount}")];
            if (name == "lower") != negative {
                values.swap(0, 1);
            }
            values
        };
        if name == "moveleft" {
            amounts.swap(0, 1);
        }
        self.command_position = Some(Position {
            name: name.into(),
            amounts,
            horizontal,
        });
        Some(Atom::Sequence(Vec::new()))
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn positions_consume_final_items_and_keep_horizontal_replacements_separate() {
        assert_eq!(
            render_exact(r"\raise1em{x}"),
            render_exact(r"\lower-1em{x}")
        );
        assert_eq!(
            render_exact(r"\moveleft2px{x}"),
            render_exact(r"\moveright-2px{x}")
        );
        let svg = render_exact(r"\raise1em\moveright2em{x}").unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let math = document
            .descendants()
            .find(|n| n.attribute("data-mml-node") == Some("math"))
            .unwrap();
        let children: Vec<_> = math.children().filter(|n| n.is_element()).collect();
        assert_eq!(
            children
                .iter()
                .map(|n| n.attribute("data-mml-node").unwrap())
                .collect::<Vec<_>>(),
            ["mpadded", "TeXAtom", "mspace"]
        );
        assert!(
            children[0]
                .descendants()
                .any(|n| n.attribute("data-mml-node") == Some("mspace"))
        );
        assert!(
            children[1]
                .descendants()
                .any(|n| n.attribute("data-c") == Some("1D465"))
        );
        let function = render_exact(r"\moveright1em\sin_i x").unwrap();
        assert!(function.contains("data-c=\"2061\""));
        assert!(!render_exact(r"\LaTeX").unwrap().contains("<text"));
        let raised = render_exact(r"\raise1em\operatorname*{foo}_i x").unwrap();
        let document = roxmltree::Document::parse(&raised).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some("0 -1705 2205 1862.8")
        );
    }

    #[test]
    fn pending_positions_preserve_dimension_close_and_script_error_precedence() {
        for name in ["raise", "lower", "moveleft", "moveright"] {
            for source in [
                format!("\\{name}1em"),
                format!("\\{name}1em&x"),
                format!("\\{name}1em\\\\[1em]"),
            ] {
                assert_eq!(
                    render(&source).unwrap(),
                    cd::render_error(&format!("Missing box for \\{name}")),
                    "{source}"
                );
            }
            assert_eq!(
                render(&format!("x^\\{name}1em y")).unwrap(),
                cd::render_error("Missing open brace for superscript")
            );
            assert_eq!(
                render(&format!("\\{name}1em\\\\[bad]")).unwrap(),
                cd::render_error("Bracket argument to \\\\ must be a dimension")
            );
        }
    }
}
