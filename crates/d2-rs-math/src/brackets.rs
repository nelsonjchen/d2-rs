// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: tex/mathtools_handlers_commands.go; svg/mpadded.go,style.go.
use super::*;
impl Parser {
    pub(super) fn bracket_command(&mut self, name: &str) -> Option<Atom> {
        let mut option = |default: &str| -> Result<String, String> {
            while self
                .chars
                .get(self.index)
                .is_some_and(|c| c.is_whitespace())
            {
                self.index += 1;
            }
            let present = self.chars.get(self.index) == Some(&'[');
            let value = self.padded_brackets()?;
            Ok(if present { value } else { default.into() })
        };
        let thickness = match option(".1em") {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let height = match option(".2em") {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let raw = match self.padded_raw_argument(name) {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let base = self.padded_parse_string(&raw)?;
        // Original parseString returns row(children, true): a singleton is
        // the child itself, so it must not acquire a synthetic script mrow.
        let base = match base {
            Atom::Sequence(mut children) if children.len() == 1 => children.remove(0),
            other => other,
        };
        if cd::first_error(&base).is_some() {
            return Some(base);
        }
        // The source parses twice; macro/tag state effects belong to both parses.
        let copy = self.padded_parse_string(&raw)?;
        if cd::first_error(&copy).is_some() {
            return Some(copy);
        }
        // mathtools resolves the authored dimension at parser time with a
        // .1em relative size and 16px/em, then stores layout.Em's rounded CSS.
        let thickness = length::dimension_with_em(&thickness, 16.).em(0.1, 1.);
        let thickness = if thickness.abs() < 0.001 {
            0.
        } else {
            format!("{thickness:.3}").parse().unwrap()
        };
        let under = name == "underbracket";
        let mark = Box::new(Atom::BracketMark {
            body: Box::new(copy),
            thickness,
            height,
            under,
            mathsize: 1.,
        });
        let stack = Atom::Scripts {
            base: Box::new(base),
            sub: if under { Some(mark.clone()) } else { None },
            sup: if under { None } else { Some(mark) },
            limits: true,
            eager: false,
            leading_space: 0.,
            accent: Some(accents::Spec {
                enabled: true,
                under,
                mathsize: 1.,
            }),
        };
        Some(Atom::ScriptPolicy {
            body: Box::new(Atom::Class {
                body: Box::new(stack),
                class: 1,
            }),
            moves: true,
            movable: None,
            movable_property: false,
        })
    }
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.,
        ascent: 0.,
        depth: 0.,
        glyphs: vec![],
        rules: vec![],
        groups: vec![],
    }
}
pub(super) fn layout(
    body: &Atom,
    thickness: f64,
    height: &str,
    under: bool,
    mathsize: f64,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    prime: bool,
) -> Option<f64> {
    let mut content = blank();
    content.width = layout_at_with_style(body, 0., 0., scale, &mut content, false, prime)?;
    retain_negative_glyph_depth(&mut content);
    let (h, d) = styles::em_bounds(&content);
    let h = h / scale;
    let d = d / scale;
    let w = content.width_em.unwrap_or(content.width / 1000.) / scale;
    let size = if height.contains("width") {
        w
    } else if height.contains("depth") {
        d
    } else {
        h
    };
    let mut padded_h = if height.is_empty() {
        h
    } else {
        length::dimension(height).em(size, scale * mathsize)
    };
    if height.starts_with(['+', '-']) {
        padded_h += size;
    }
    padded_h = h + (padded_h.max(0.) - h);
    let t = thickness.max(0.);
    let outer_h = padded_h + if under { 0. } else { t };
    let outer_d = if under { t } else { 0. };
    let outer_w = w + t + t;
    let mut local = blank();
    // The source mpadded owns an mphantom child. Its copied argument supplies
    // the dimensions above, while phantomToSVG emits only the empty wrapper.
    layout_at_with_style(
        &Atom::EmptyNode("mphantom"),
        0.,
        0.,
        scale,
        &mut local,
        false,
        prime,
    )?;
    local.width = outer_w * 1000. * scale;
    local.width_em = Some(outer_w * scale);
    local.ascent = outer_h * 1000. * scale;
    local.depth = outer_d * 1000. * scale;
    local.bounds_em = Some((outer_h * scale, outer_d * scale));
    out.width_em = local.width_em;
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    out.groups.push(LayoutGroup {
        data_cramped: None,
        own_translate: None,
        accent_offset: None,
        box_border: Some(thickness * 1000. * scale),
        reference: None,
        tagged: None,
        percentage: None,
        table_line: None,
        mathsize: None,
        tex_class: 0,
        before_glyph: out.glyphs.len(),
        scale,
        x,
        baseline,
        color: None,
        background: None,
        kind: if under {
            "#bracket-under"
        } else {
            "#bracket-over"
        },
        delimiter: None,
        arrow: None,
        body: Box::new(local),
        lines: vec![],
    });
    Some(outer_w * 1000. * scale)
}
pub(super) fn render(svg: &mut String, local: &Layout, t: f64, under: bool) {
    let (h, d) = styles::em_bounds(local);
    let h = h + 0.005;
    let d = d + 0.005;
    let width = local.width / 1000. + 0.005;
    let t = (t / 1000.).max(0.);
    let widths = if under { [0., t, t, t] } else { [t, t, 0., t] };
    let [top, right, bottom, left] = widths;
    let outer_rt = [width, h];
    let outer_lt = [-0.005, h];
    let outer_rb = [width, -d];
    let outer_lb = [-0.005, -d];
    let inner_rt = [width - right, h - top];
    let inner_lt = [-0.005 + left, h - top];
    let inner_rb = [width - right, -d + bottom];
    let inner_lb = [-0.005 + left, -d + bottom];
    for (i, points) in [
        [outer_lt, outer_rt, inner_rt, inner_lt],
        [outer_rb, outer_rt, inner_rt, inner_rb],
        [outer_lb, outer_rb, inner_rb, inner_lb],
        [outer_lb, outer_lt, inner_lt, inner_lb],
    ]
    .into_iter()
    .enumerate()
    {
        if widths[i] == 0. {
            continue;
        }
        let points = points
            .map(|p| {
                format!(
                    "{},{}",
                    root_number((p[0] - t) * 1000.),
                    root_number(p[1] * 1000.)
                )
            })
            .join(" ");
        svg.push_str(&format!(
            "<polygon points=\"{points}\" stroke=\"none\" fill=\"currentColor\"></polygon>"
        ));
    }
}

pub(super) fn border_style(raw: f64, under: bool) -> String {
    let width = raw / 1000.;
    // mathtoolsUnderOverBracket authors "<thickness> solid". The pinned
    // style.go splitBorder/cssValue preserves that shorthand verbatim,
    // including unitless zero and negative widths.
    let border = if width == 0. {
        "0 solid".to_owned()
    } else {
        format!("{}em solid", number(width))
    };
    let sides = [
        if under { "none" } else { &border },
        &border,
        if under { &border } else { "none" },
        &border,
    ];
    format!(
        " style=\"border-top: {}; border-right: {}; border-bottom: {}; border-left: {};\"",
        sides[0], sides[1], sides[2], sides[3]
    )
}

#[cfg(test)]
mod contracts {
    use super::*;
    #[test]
    fn bracket_viewport_retains_original_unrounded_root_bounds() {
        // Keep the original .2em rounding witness explicit: a unitless .2
        // now correctly denotes .02em in the pinned mathtools handler.
        let expression = r"\scriptscriptstyle \underbracket[.2em][-50%]{\frac{a}{b}}";
        let native = super::super::layout(&parse(expression).unwrap()).unwrap();
        assert_eq!(
            styles::em_bounds(&native),
            (0.4175000000000001, 0.6837500000000001)
        );
    }
}
