// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/hfill.go: ArrayItem.Size, finish and arrayCellNode.
use super::*;

pub(super) fn is_fill(name: &str) -> bool {
    matches!(name, "hfil" | "hfill" | "hfilll")
}
pub(super) fn unsupported(name: &str) -> Atom {
    Atom::ErrorDisplay(format!("Unsupported use of \\{name}"))
}
#[derive(Debug, Default)]
pub(super) struct Fills {
    positions: Vec<usize>,
    pub offset: usize,
    pub blocked: bool,
}
impl Fills {
    pub fn push(&mut self, size: usize) {
        self.positions.push(self.offset + size);
    }
    pub fn alignment(&self, size: usize) -> Option<&'static str> {
        let first = *self.positions.first()? == 0;
        let last = *self.positions.last()? == size;
        match (first, last) {
            (true, true) => Some("center"),
            (true, false) => Some("right"),
            (false, true) => Some("left"),
            _ => None,
        }
    }
}
pub(super) fn size(atom: &Atom) -> usize {
    match atom {
        Atom::Sequence(nodes) => nodes.len(),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_align_array_cells_without_emitting_nodes() {
        for fill in ["hfil", "hfill", "hfilll"] {
            for (left, right, column) in [(true, false, 'r'), (false, true, 'l'), (true, true, 'c')]
            {
                let cell = |text: &str| {
                    format!(
                        "{}{text}{}",
                        if left {
                            format!("\\{fill} ")
                        } else {
                            String::new()
                        },
                        if right {
                            format!("\\{fill}")
                        } else {
                            String::new()
                        }
                    )
                };
                assert_eq!(
                    render_exact(&format!(
                        r"\begin{{array}}{{c}}{}\\{}\end{{array}}",
                        cell("x"),
                        cell("longer")
                    )),
                    render_exact(&format!(
                        r"\begin{{array}}{{{column}}}x\\longer\end{{array}}"
                    )),
                    "{fill}: {column}",
                );
            }
        }
        // EqnArray accepts HFill but its EndEntry deliberately uses plain mtd.
        assert_eq!(
            render_exact(r"\begin{aligned}x\hfill&z\\longer&zz\end{aligned}"),
            render_exact(r"\begin{aligned}x&z\\longer&zz\end{aligned}"),
        );
        // CD preserves the same cell metadata through its raw-arrow scanner.
        assert_eq!(
            render_exact(r"\begin{CD}\hfill x\hfill&z\\\hfill longer\hfill&zz\end{CD}"),
            render_exact(r"\begin{CD}x&z\\longer&zz\end{CD}"),
        );
    }

    #[test]
    fn fills_count_items_across_font_continuations_and_clear_each_cell() {
        assert_eq!(
            render_exact(r"\begin{array}{r}x\bf y\hfill\\\bf longer\hfill\end{array}"),
            render_exact(r"\begin{array}{l}x\bf y\\\bf longer\end{array}"),
        );
        assert_eq!(
            render_exact(r"\begin{array}{l}x\bf\hfill y\\longer\end{array}"),
            render_exact(r"\begin{array}{l}x\bf y\\longer\end{array}"),
        );
        assert_eq!(
            render_exact(r"\begin{array}{l}\hfill x\\longer\end{array}"),
            render_exact(r"\begin{array}{r}\hfill x\\longer\hfill\end{array}"),
        );
        assert_eq!(
            render_exact(r"\DeclareMathOperator{\hfill}{z}\hfill+x"),
            render_exact(r"\operatorname{z}+x"),
        );
    }

    #[test]
    fn fills_require_the_actual_array_stack_item() {
        for fill in ["hfil", "hfill", "hfilll"] {
            for prefix in [
                r"\sin",
                r"\dots",
                r"\color{red}",
                r"\scriptstyle",
                "x'",
                r"\raise1em",
            ] {
                let direct = render_exact(&format!("\\{fill}")).unwrap();
                assert!(
                    direct.contains(&format!("data-mjx-error=\"Unsupported use of \\{fill}\""))
                );
                assert_eq!(
                    render_exact(&format!(r"\begin{{matrix}}{prefix}\{fill}\end{{matrix}}")),
                    Ok(direct),
                    "{prefix}: {fill}",
                );
            }
            for source in [
                format!(r"\begin{{matrix}}{{\{fill}}}\end{{matrix}}"),
                format!(r"\begin{{matrix}}\frac{{x}}{{\{fill}}}\end{{matrix}}"),
                format!(r"\begin{{matrix}}x\over\{fill}\end{{matrix}}"),
            ] {
                assert_eq!(
                    render_exact(&source),
                    render_exact(&format!("\\{fill}")),
                    "{source}"
                );
            }
        }
    }
}
