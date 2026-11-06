// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/array_rules.go and environments.go applyColumnSpec.
use super::*;

pub(super) type Shared = std::rc::Rc<std::cell::RefCell<Rules>>;
#[derive(Clone, Debug, Default)]
pub(super) struct Rules {
    pub rows: usize,
    frame: Vec<&'static str>,
    lines: Vec<&'static str>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Finished {
    pub frame: Vec<&'static str>,
    pub lines: Vec<&'static str>,
    pub columns: Vec<&'static str>,
    pub full: bool,
    pub dashed: bool,
    pub zero_padding: bool,
}
impl Rules {
    pub fn add(&mut self, name: &str) {
        if self.rows == 0 {
            self.frame.push("top");
        } else {
            self.lines.resize(self.rows, "none");
            self.lines[self.rows - 1] = if name == "hdashline" {
                "dashed"
            } else {
                "solid"
            };
        }
    }
    pub fn finish(&self, count: usize, spec: &str) -> Finished {
        let mut result = Finished {
            frame: self.frame.clone(),
            lines: self.lines.clone(),
            ..Default::default()
        };
        // Each alignment collapses all following rules to the final rule.
        let syntax: Vec<_> = std::iter::once('c')
            .chain(spec.chars())
            .filter(|c| "lcr|:".contains(*c))
            .collect();
        let mut positions = Vec::new();
        let mut i = 0;
        while i < syntax.len() {
            let mut c = syntax[i];
            if c != '|' && c != ':' {
                while i + 1 < syntax.len() && matches!(syntax[i + 1], '|' | ':') {
                    i += 1;
                    c = syntax[i];
                }
            }
            positions.push(c);
            i += 1;
        }
        if positions.iter().any(|c| matches!(c, '|' | ':')) {
            let mut frame = Vec::new();
            if matches!(positions[0], '|' | ':') {
                frame.push("left");
            }
            if matches!(positions[positions.len() - 1], '|' | ':') {
                frame.push("right");
            }
            frame.append(&mut result.frame);
            result.frame = frame;
            result.dashed = positions[0] == ':';
            result.columns = positions
                .iter()
                .skip(1)
                .take(positions.len().saturating_sub(2))
                .map(|c| match c {
                    '|' => "solid",
                    ':' => "dashed",
                    _ => "none",
                })
                .collect();
        }
        if !result.lines.is_empty() {
            if result.lines.len() == count {
                result.frame.push("bottom");
                result.lines.pop();
            } else if result.lines.len() < count.saturating_sub(1) {
                result.lines.push("none");
            }
        }
        result.full = result.frame.len() == 4;
        let mut rows = result.lines.clone();
        while rows.len() > 1 && rows[rows.len() - 2..] == ["none", "none"] {
            rows.pop();
        }
        result.zero_padding = (!result.columns.is_empty() && result.columns != ["none"])
            || (!rows.is_empty() && rows != ["none"]);
        result
    }
}

// svg/menclose.go: partial Array frames retain an anonymous child block;
// the table's own frame spacing is independent of enclosure padding.
pub(super) fn enclose(
    mut table: Layout,
    mut width: f64,
    mut h_em: f64,
    mut d_em: f64,
    scale: f64,
    rules: &Finished,
) -> (Layout, f64, f64, f64, &'static str) {
    use table::{blank, group};
    let mut table_kind = "mtable";
    if !rules.frame.is_empty() && !rules.full {
        let ext = if rules.zero_padding { 0.067 } else { 0.267 };
        let sides = ["top", "right", "bottom", "left"].map(|side| {
            if rules.frame.contains(&side) {
                ext
            } else {
                0.0
            }
        });
        let [top, right, bottom, left] = sides;
        table.width = width * scale * 1000.0;
        table.ascent = h_em * scale * 1000.0;
        table.depth = d_em * scale * 1000.0;
        let mut outer = blank();
        let mut block = blank();
        block.groups.push(group(table, 0.0, 0.0, scale, "mtable"));
        outer
            .groups
            .push(group(block, left * scale * 1000.0, 0.0, scale, ""));
        width += left + right;
        h_em = h_em.max(0.0) + top;
        d_em = d_em.max(0.0) + bottom;
        // menclose suppresses duplicate notations while preserving source order.
        let mut seen = Vec::new();
        let mut enclosure = group(blank(), 0.0, 0.0, scale, "#array-frame-lines");
        for side in &rules.frame {
            if seen.contains(side) {
                continue;
            }
            seen.push(side);
            let p = match *side {
                "top" => [0.0, h_em - 0.0335, width, h_em - 0.0335],
                "right" => [width - 0.0335, -d_em, width - 0.0335, h_em],
                "bottom" => [0.0, 0.0335 - d_em, width, 0.0335 - d_em],
                _ => [0.0335, -d_em, 0.0335, h_em],
            };
            enclosure.lines.push((
                p[0] * 1000.0 * scale,
                p[1] * 1000.0 * scale,
                p[2] * 1000.0 * scale,
                p[3] * 1000.0 * scale,
                67.0 * scale,
            ));
        }
        // Lines belong to menclose itself, after its mtable child.
        outer.groups.push(enclosure);
        table = outer;
        table_kind = if rules.zero_padding {
            "#array-menclose-zero"
        } else {
            "menclose"
        };
    }
    (table, width, h_em, d_em, table_kind)
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn rule_items_belong_to_the_current_cell_and_respect_pending_scopes() {
        // HLine is a no-node event: earlier completed cells do not block it,
        // and MathFont does not push a pending style item.
        assert_eq!(
            render_exact(r"\begin{matrix}a&\hline b\end{matrix}"),
            render_exact(r"\begin{matrix}\hline a&b\end{matrix}"),
        );
        assert_eq!(
            render_exact(r"\begin{matrix}\bf\hline a\end{matrix}"),
            render_exact(r"\begin{matrix}\hline\bf a\end{matrix}"),
        );
        for expression in [
            r"\hline",
            r"\begin{matrix}a\hline\end{matrix}",
            r"\begin{matrix}{\hline}a\end{matrix}",
            r"\begin{matrix}a\bf\hline b\end{matrix}",
            r"\begin{matrix}\scriptstyle\hline a\end{matrix}",
            r"\begin{matrix}\color{red}\hline a\end{matrix}",
        ] {
            assert!(
                render_exact(expression)
                    .unwrap()
                    .contains(r#"data-mjx-error="Misplaced \hline""#),
                "{expression}"
            );
        }
    }

    #[test]
    fn array_rule_styles_frames_and_enclosure_widths_follow_the_public_contract() {
        let svg = render_exact(r"\begin{split}a&=b\\\hdashline c&=d\end{split}").unwrap();
        assert!(svg.contains(r#"<line data-line="h" class="mjx-dashed" x1="0" y1="250""#));
        // Rules before the first row select a top enclosure, even hdashline.
        assert_eq!(
            render_exact(r"\begin{matrix}\hline a\end{matrix}"),
            render_exact(r"\begin{matrix}\hdashline a\end{matrix}"),
        );
        let svg =
            render_exact(r"\begin{array}{:c:c:}\hline a&b\\\hline c&d\\\hline\end{array}").unwrap();
        assert!(svg.contains(r#"<rect data-frame="true" class="mjx-dashed""#));
        assert!(svg.contains(r#"data-line="v" class="mjx-dashed""#));
        let svg = render_exact(r"\begin{multline}\hline a\\\hline b\end{multline}").unwrap();
        let doc = roxmltree::Document::parse(&svg).unwrap();
        assert_eq!(doc.root_element().attribute("width"), Some("5.722ex"));
        assert!(svg.contains(r#"data-mml-node="mtable" data-width-includes-label="true""#));
        let svg = render_exact(r"\begin{CD}a\\\hdashline b\end{CD}").unwrap();
        assert!(svg.contains(r#"data-line="h" class="mjx-dashed""#));
    }
}
