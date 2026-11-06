// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2svg LaTeX labels: the `Language == "latex"` branches of
// d2renderers/d2svg/d2svg.go drawConnection (1235-1249) and drawShape
// (2079-2109), which call d2renderers/d2latex.Render (mathjax.Render from
// github.com/d2lang/mathjax-go v0.1.0). The MathJax SVG itself comes from
// d2_rs_math::render_exact, which refuses rather than approximates any
// output it cannot reproduce byte-for-byte.

use super::element::{ThemableElement, Theme};
use super::geo::Point;
use super::label::{self, Position};
use super::shape::Shape;
use crate::target::{TargetConnection, TargetShape};

// d2svg.go strips these from the LaTeX SVG output before embedding it.
const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;
const DOCTYPE: &str = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.0//EN" "http://www.w3.org/TR/2001/REC-SVG-20010904/DTD/svg10.dtd">"#;

/// d2latex.Render: mathjax.Render wrapped with xdefer.Errorf("latex failed to parse").
fn d2latex_render(tex: &str) -> Result<String, String> {
    // d2latex.Render validates before installing the parse-error wrapper.
    d2_rs_math::validate_input(tex).map_err(|error| error.to_string())?;
    let svg =
        d2_rs_math::render_exact(tex).map_err(|error| format!("latex failed to parse: {error}"))?;
    Ok(svg.replace(XML_DECL, "").replace(DOCTYPE, ""))
}

/// drawShape's LaTeX label.
pub(crate) fn shape_label(
    w: &mut String,
    s: &TargetShape,
    sh: &dyn Shape,
    theme: Option<&Theme>,
) -> Result<(), String> {
    let render = d2latex_render(&s.label)?;
    let mut g = ThemableElement::new("g", theme);
    let mut position = Position::from_string(s.label_position_str());
    if position == Position::Unset {
        position = Position::InsideMiddleCenter;
    }
    let b = if position.is_outside() {
        sh.get_box()
    } else {
        sh.get_inner_box()
    };
    let label_tl = position.get_point_on_box(
        &b,
        label::PADDING,
        s.label_width as f64,
        s.label_height as f64,
    );
    g.set_translate(label_tl.x, label_tl.y);
    g.color.clone_from(&s.stroke);
    g.content = render;
    w.push_str(&g.render());
    Ok(())
}

/// drawConnection's LaTeX label.
pub(crate) fn connection_label(
    w: &mut String,
    c: &TargetConnection,
    label_tl: Point,
    theme: Option<&Theme>,
) -> Result<(), String> {
    let render = d2latex_render(&c.label)?;
    let mut g = ThemableElement::new("g", theme);
    g.set_translate(label_tl.x, label_tl.y);
    g.color.clone_from(&c.color);
    g.content = render;
    w.push_str(&g.render());
    Ok(())
}
