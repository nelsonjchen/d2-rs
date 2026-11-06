// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 positioned tooltips: d2target CalculateTooltipPosition and
// calculateTooltipBounds, and d2svg calculateTooltipPosition,
// renderTooltipTail and renderPositionedTooltip.

use super::element::{ThemableElement, Theme};
use super::markdown::MarkdownRenderer;
use super::svgtext::format_float;
use crate::target::TargetShape;

/// d2fonts.FONT_SIZE_M.
const FONT_SIZE_M: i64 = 16;

/// d2target.CalculateTooltipPosition.
pub(crate) fn calculate_position(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    tooltip_width: i64,
    tooltip_height: i64,
    position: &str,
) -> (f64, f64) {
    let padding = 10.0;
    let (tw, th) = (tooltip_width as f64, tooltip_height as f64);
    match position {
        "top-left" => (x, y - th - padding),
        "top-center" => (x + width / 2. - tw / 2., y - th - padding),
        "top-right" => (x + width - tw, y - th - padding),
        "center-left" => (x - tw - padding, y + height / 2. - th / 2.),
        "center-right" => (x + width + padding, y + height / 2. - th / 2.),
        "bottom-left" => (x, y + height + padding),
        "bottom-center" => (x + width / 2. - tw / 2., y + height + padding),
        "bottom-right" => (x + width - tw, y + height + padding),
        _ => (x + width / 2. - tw / 2., y - th - padding),
    }
}

/// d2target.calculateTooltipBounds. A Markdown layout failure falls back to
/// Go's character-count estimate.
pub(crate) fn calculate_bounds(
    s: &TargetShape,
    font_family: Option<&str>,
    mono_font_family: Option<&str>,
) -> (i64, i64, i64, i64) {
    if s.tooltip.is_empty() || s.tooltip_position_str().is_empty() {
        return (0, 0, 0, 0);
    }
    let mut renderer = MarkdownRenderer::new(font_family, mono_font_family, None);
    let (width, height) = match renderer.layout(&s.tooltip, &s.font_family, FONT_SIZE_M) {
        Ok(layout) => (layout.width + 20, layout.height + 20),
        Err(_) => ((s.tooltip.len() as i64 * 8 + 20).min(200), 30),
    };
    let (x, y) = calculate_position(
        s.pos.x as f64,
        s.pos.y as f64,
        s.width as f64,
        s.height as f64,
        width,
        height,
        s.tooltip_position_str(),
    );
    (
        x.floor() as i64,
        y.floor() as i64,
        x.ceil() as i64 + width,
        y.ceil() as i64 + height,
    )
}

/// d2svg.calculateTooltipPosition: position plus tail direction/offset.
fn position_and_tail(
    s: &TargetShape,
    position: &str,
    tw: i64,
    th: i64,
) -> (f64, f64, &'static str, f64, f64) {
    let (x, y) = calculate_position(
        s.pos.x as f64,
        s.pos.y as f64,
        s.width as f64,
        s.height as f64,
        tw,
        th,
        position,
    );
    let (tw, th) = (tw as f64, th as f64);
    let (direction, tail_x, tail_y) = match position {
        "top-left" => ("bottom", 20., th),
        "top-center" => ("bottom", tw / 2., th),
        "top-right" => ("bottom", tw - 20., th),
        "center-left" => ("right", tw, th / 2.),
        "center-right" => ("left", 0., th / 2.),
        "bottom-left" => ("top", 20., 0.),
        "bottom-center" => ("top", tw / 2., 0.),
        "bottom-right" => ("top", tw - 20., 0.),
        _ => ("bottom", tw / 2., th),
    };
    (x, y, direction, tail_x, tail_y)
}

/// d2svg.renderTooltipTail.
fn render_tail(direction: &str, x: f64, y: f64, theme: Option<&Theme>) -> String {
    let size = 8.0;
    let f = format_float;
    let d = match direction {
        "top" => format!(
            "M {} {} L {} {} L {} {} Z",
            f(x - size / 2.),
            f(y),
            f(x + size / 2.),
            f(y),
            f(x),
            f(y - size)
        ),
        "bottom" => format!(
            "M {} {} L {} {} L {} {} Z",
            f(x - size / 2.),
            f(y),
            f(x + size / 2.),
            f(y),
            f(x),
            f(y + size)
        ),
        "left" => format!(
            "M {} {} L {} {} L {} {} Z",
            f(x),
            f(y - size / 2.),
            f(x),
            f(y + size / 2.),
            f(x - size),
            f(y)
        ),
        "right" => format!(
            "M {} {} L {} {} L {} {} Z",
            f(x),
            f(y - size / 2.),
            f(x),
            f(y + size / 2.),
            f(x + size),
            f(y)
        ),
        _ => return String::new(),
    };
    let mut tail = ThemableElement::new("path", theme);
    tail.d = d;
    tail.fill = "N7".into();
    tail.stroke = "N5".into();
    tail.attributes = r#"stroke-width="1""#.into();
    tail.render()
}

/// d2svg.renderPositionedTooltip.
pub(crate) fn render_positioned(
    s: &TargetShape,
    markdown: &mut MarkdownRenderer<'_>,
) -> Result<String, String> {
    let position = s.tooltip_position_str();
    if s.tooltip.is_empty() || position.is_empty() {
        return Ok(String::new());
    }
    let layout = markdown.layout(&s.tooltip, &s.font_family, FONT_SIZE_M)?;
    let (tw, th) = (layout.width + 20, layout.height + 20);
    let (x, y, direction, tail_x, tail_y) = position_and_tail(s, position, tw, th);
    let content = markdown.render(
        &layout,
        x + 10.,
        y + 10.,
        layout.width,
        layout.height,
        "N1",
        "",
        false,
        false,
    );
    let theme = markdown.inline_theme();
    let mut tooltip_box = ThemableElement::new("rect", theme);
    tooltip_box.x = x;
    tooltip_box.y = y;
    tooltip_box.width = tw as f64;
    tooltip_box.height = th as f64;
    tooltip_box.rx = 4.;
    tooltip_box.ry = 4.;
    tooltip_box.fill = "N7".into();
    tooltip_box.stroke = "N5".into();
    tooltip_box.attributes = r#"stroke-width="1""#.into();
    let tail = render_tail(direction, tail_x + x, tail_y + y, theme);
    Ok(format!(
        r#"<g class="positioned-tooltip">{}{tail}{content}</g>"#,
        tooltip_box.render()
    ))
}
