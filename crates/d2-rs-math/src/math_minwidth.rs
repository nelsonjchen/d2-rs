// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: pinned mathjax-go internal/svg/math_minwidth.go and wrapper.go wrap.
use super::*;

// The public TeX entry point uses display math with center alignment and zero
// indent (mathAlignShift). The late display pass uses the last eligible table's
// outer box, rather than the root box containing adjacent references or text.
pub(super) fn minimum(layout: &Layout) -> Option<f64> {
    fn visit(layout: &Layout, top: bool, selected: &mut Option<f64>) {
        for group in &layout.groups {
            let transparent = matches!(
                group.kind,
                "" | "mrow" | "#inferred-place" | "TeXAtom" | "mstyle"
            );
            // wrap constructs children before selecting the current table.
            visit(&group.body, top && transparent, selected);
            if top && group.kind == "mtable" {
                let width = if let Some(tagged) = &group.tagged {
                    group.body.width + if tagged.align == "center" { 2.0 } else { 1.0 } * tagged.pad
                } else {
                    group.body.width
                };
                // outerBBox is in the selected wrapper's own em frame.
                *selected = Some((width / group.scale).max(0.0));
            }
        }
    }
    if !ams::responsive(layout) {
        return None;
    }
    let mut selected = None;
    visit(layout, true, &mut selected);
    selected.filter(|width| *width != 0.0 && !width.is_nan())
}

#[cfg(test)]
mod contracts {
    use super::*;

    fn minimum_style(expression: &str) -> String {
        let svg = render_exact(expression).unwrap();
        let doc = roxmltree::Document::parse(&svg).unwrap();
        doc.root_element()
            .attribute("style")
            .unwrap()
            .split("min-width: ")
            .nth(1)
            .unwrap()
            .to_owned()
    }

    #[test]
    fn display_minimum_uses_the_table_without_adjacent_reference_width() {
        let equation = r"\begin{equation}x\tag{1}\label{a}\end{equation}";
        assert_eq!(
            minimum_style(equation),
            minimum_style(&format!("{equation}\\eqref{{a}}"))
        );
        // wrapper.outerBBox reports the table's own ems in the late pass.
        assert_eq!(
            minimum_style(equation),
            minimum_style(&format!("\\scriptstyle {equation}"))
        );
    }

    #[test]
    fn equation_stack_survives_end_and_cells_but_arguments_get_fresh_stacks() {
        for expression in [
            r"\begin{equation}x\end{equation}\begin{equation}y\end{equation}",
            r"\begin{align}\begin{equation}x\end{equation}&=y\end{align}",
            r"\begin{matrix}\begin{equation}x\end{equation}&\begin{equation}y\end{equation}\end{matrix}",
        ] {
            assert!(
                render_exact(expression)
                    .unwrap()
                    .contains("Erroneous nesting of equation structures")
            );
        }
        let separate_arguments =
            r"\frac{\begin{equation}x\end{equation}}{\begin{equation}y\end{equation}}";
        assert!(
            !render_exact(separate_arguments)
                .unwrap()
                .contains("data-mjx-error")
        );
    }
}
