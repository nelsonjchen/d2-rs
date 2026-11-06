// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2svg/d2svg.go RenderLegend,
// renderLegendShapeIcon, renderLegendConnectionIcon and the legend parts of
// dimensions/Render.

use std::fmt::Write as _;

use super::classtable::html_escape;
use super::element::{ThemableElement, Theme};
use super::markdown::MarkdownRenderer;
use super::render::{draw_connection, draw_shape, hash};
use super::svgtext::format_float;
use crate::target::{TargetConnection, TargetDiagram};

pub(crate) const LEGEND_PADDING: i64 = 20;
pub(crate) const LEGEND_ITEM_SPACING: i64 = 15;
pub(crate) const LEGEND_ICON_SIZE: i64 = 24;
pub(crate) const LEGEND_FONT_SIZE: i64 = 14;
pub(crate) const LEGEND_CORNER_PADDING: i64 = 10;

/// d2graph.GetTextDimensions for an unstyled MText (Ruler.Measure, ints).
/// fontToFamily only maps "default"/"mono"; anything else is SourceSansPro.
fn text_dimensions(text: &str, font_family: &str) -> Result<(i64, i64), String> {
    use d2_rs_text::measure;
    let error = |e: measure::FontError| format!("{e:?}");
    let mono = measure::bundled_source_code_pro_regular().map_err(error)?;
    let face = if font_family == "mono" {
        mono.clone()
    } else {
        measure::bundled_source_sans_pro_regular().map_err(error)?
    };
    let metrics = face.measure_with_fallback(text, LEGEND_FONT_SIZE as f64, &mono);
    Ok((metrics.width.ceil() as i64, metrics.height.ceil() as i64))
}

pub(crate) fn has_items(diagram: &TargetDiagram) -> bool {
    diagram.legend.as_ref().is_some_and(|legend| {
        !legend.shapes.as_deref().unwrap_or(&[]).is_empty()
            || !legend.connections.as_deref().unwrap_or(&[]).is_empty()
    })
}

/// The shared legend size pass. `dimensions` adds the trailing
/// LEGEND_PADDING*1.5/1.2 term; Render's second pass adds plain padding.
fn measure_items(diagram: &TargetDiagram) -> Result<(i64, i64, usize), String> {
    let legend = diagram.legend.as_ref().expect("legend");
    let mut total_height = LEGEND_PADDING + LEGEND_FONT_SIZE + LEGEND_ITEM_SPACING;
    let (mut max_label_width, mut item_count) = (0, 0);
    let labels = legend
        .shapes
        .iter()
        .flatten()
        .map(|s| (&s.label, &s.font_family))
        .chain(
            legend
                .connections
                .iter()
                .flatten()
                .map(|c| (&c.label, &c.font_family)),
        );
    for (label, family) in labels {
        if label.is_empty() {
            continue;
        }
        let (w, h) = text_dimensions(label, family)?;
        max_label_width = max_label_width.max(w);
        total_height += h.max(LEGEND_ICON_SIZE) + LEGEND_ITEM_SPACING;
        item_count += 1;
    }
    if item_count > 0 {
        total_height -= LEGEND_ITEM_SPACING / 2;
    }
    Ok((total_height, max_label_width, item_count))
}

/// d2svg.dimensions' legend expansion.
pub(crate) fn expand_dimensions_for_legend(
    diagram: &TargetDiagram,
    pad: i64,
    tl: (i64, i64),
    br: (i64, i64),
    dims: &mut (i64, i64, i64, i64),
    render_pass: bool,
) -> Result<(), String> {
    if !has_items(diagram) {
        return Ok(());
    }
    let (mut total_height, max_label_width, item_count) = measure_items(diagram)?;
    let has_connections = !diagram
        .legend
        .as_ref()
        .unwrap()
        .connections
        .as_deref()
        .unwrap_or(&[])
        .is_empty();
    if render_pass {
        total_height += LEGEND_PADDING;
    } else if item_count > 0 && has_connections {
        total_height += LEGEND_PADDING * 3 / 2;
    } else {
        total_height += LEGEND_PADDING * 6 / 5;
    }
    if total_height > 0 && max_label_width > 0 {
        let (left, top, width, height) = dims;
        let legend_width = LEGEND_PADDING * 2 + LEGEND_ICON_SIZE + LEGEND_PADDING + max_label_width;
        let legend_y = (br.1 - total_height).max(tl.1);
        let legend_right = br.0 + LEGEND_CORNER_PADDING + legend_width;
        if *left + *width < legend_right {
            *width = legend_right - *left + pad / 2;
        }
        if legend_y < *top {
            let diff = *top - legend_y;
            *top -= diff;
            *height += diff;
        }
        let legend_bottom = legend_y + total_height;
        if *top + *height < legend_bottom {
            *height = legend_bottom - *top + pad / 2;
        }
    }
    Ok(())
}

/// d2svg.RenderLegend.
pub(crate) fn render_legend(
    buf: &mut String,
    diagram: &TargetDiagram,
    diagram_hash: &str,
    theme: Option<&Theme>,
) -> Result<(), String> {
    if !has_items(diagram) {
        return Ok(());
    }
    let legend = diagram.legend.as_ref().expect("legend");
    for shape in legend.shapes.iter().flatten() {
        super::paint_validation::link(
            &shape.link,
            &format!(
                "legend shape {}",
                d2_rs_core::quote_go_bytes(shape.id.as_bytes())
            ),
        )?;
    }
    let (tl, br) = diagram.go_bounding_box()?;
    let (mut total_height, max_label_width, item_count) = measure_items(diagram)?;
    let connections: &[TargetConnection] = legend.connections.as_deref().unwrap_or(&[]);
    if item_count > 0 && !connections.is_empty() {
        total_height += LEGEND_PADDING * 3 / 2;
    } else {
        total_height += LEGEND_PADDING * 6 / 5;
    }
    let legend_width = LEGEND_PADDING * 2 + LEGEND_ICON_SIZE + LEGEND_PADDING + max_label_width;
    let legend_x = br.0 + LEGEND_CORNER_PADDING;
    let legend_y = (br.1 - total_height).max(tl.1);
    for (fill, style) in [
        (
            "#F7F7FA",
            "stroke-width: 1px; filter: drop-shadow(0px 2px 3px rgba(0, 0, 0, 0.1))",
        ),
        ("#ffffff", "stroke-width: 1px"),
    ] {
        let mut el = ThemableElement::new("rect", theme);
        el.fill = fill.into();
        el.stroke = "#DEE1EB".into();
        el.style = style.into();
        el.x = legend_x as f64;
        el.y = legend_y as f64;
        el.width = legend_width as f64;
        el.height = total_height as f64;
        el.rx = 4.0;
        buf.push_str(&el.render());
    }
    let label = if legend.label.is_empty() {
        "Legend"
    } else {
        legend.label.as_str()
    };
    write!(
        buf,
        r#"<text class="text-bold" x="{}" y="{}" style="font-size: {}px;">{}</text>"#,
        legend_x + LEGEND_PADDING,
        legend_y + LEGEND_PADDING + LEGEND_FONT_SIZE,
        LEGEND_FONT_SIZE + 2,
        html_escape(label)
    )
    .unwrap();
    let mut current_y = legend_y + LEGEND_PADDING * 2 + LEGEND_FONT_SIZE;
    let mut shape_count = 0;
    for s in legend
        .shapes
        .iter()
        .flatten()
        .filter(|s| !s.label.is_empty())
    {
        let icon_x = legend_x + LEGEND_PADDING;
        // renderLegendShapeIcon.
        let mut icon = s.clone();
        icon.pos.x = 0;
        icon.pos.y = 0;
        icon.width = LEGEND_ICON_SIZE * 5;
        icon.height = LEGEND_ICON_SIZE * 5;
        icon.label.clear();
        let (mut shape_buf, mut appendix) = (String::new(), String::new());
        draw_shape(
            &mut shape_buf,
            &mut appendix,
            diagram_hash,
            &icon,
            false,
            theme,
            &mut MarkdownRenderer::new(None, None, theme),
        )?;
        write!(
            buf,
            r#"<g transform="translate({icon_x}, {current_y}) scale({})">{shape_buf}</g>"#,
            format_float(1.0 / 5.0)
        )
        .unwrap();
        let (_, h) = text_dimensions(&s.label, &s.font_family)?;
        let row_height = h.max(LEGEND_ICON_SIZE);
        let text_y = current_y + row_height / 2 + (h as f64 * 0.3) as i64;
        write!(
            buf,
            r#"<text class="text" x="{}" y="{text_y}" style="font-size: {LEGEND_FONT_SIZE}px;">{}</text>"#,
            icon_x + LEGEND_ICON_SIZE + LEGEND_PADDING,
            html_escape(&s.label)
        )
        .unwrap();
        current_y += row_height + LEGEND_ITEM_SPACING;
        shape_count += 1;
    }
    if shape_count > 0 && !connections.is_empty() {
        current_y += LEGEND_ITEM_SPACING / 2;
        let mut sep = ThemableElement::new("line", theme);
        sep.x1 = (legend_x + LEGEND_PADDING) as f64;
        sep.y1 = current_y as f64;
        sep.x2 = (legend_x + legend_width - LEGEND_PADDING) as f64;
        sep.y2 = current_y as f64;
        sep.stroke = "#DEE1EB".into();
        sep.stroke_dash_array = "2,2".into();
        buf.push_str(&sep.render());
        current_y += LEGEND_ITEM_SPACING;
    }
    for c in connections.iter().filter(|c| !c.label.is_empty()) {
        let icon_x = legend_x + LEGEND_PADDING;
        let icon_y = current_y + LEGEND_ICON_SIZE / 2;
        buf.push_str(&legend_connection_icon(c, icon_x, icon_y, theme)?);
        let (_, h) = text_dimensions(&c.label, &c.font_family)?;
        let row_height = h.max(LEGEND_ICON_SIZE);
        let text_y = current_y + row_height / 2 + (h as f64 * 0.2) as i64;
        write!(
            buf,
            r#"<text class="text" x="{}" y="{text_y}" style="font-size: {LEGEND_FONT_SIZE}px;">{}</text>"#,
            icon_x + LEGEND_ICON_SIZE + LEGEND_PADDING,
            html_escape(&c.label)
        )
        .unwrap();
        current_y += row_height + LEGEND_ITEM_SPACING;
    }
    Ok(())
}

/// d2svg.renderLegendConnectionIcon.
fn legend_connection_icon(
    c: &TargetConnection,
    x: i64,
    y: i64,
    theme: Option<&Theme>,
) -> Result<String, String> {
    let route = vec![
        Some(crate::target::HashPoint {
            x: crate::target::GoFloat(0.0),
            y: crate::target::GoFloat(0.0),
        }),
        Some(crate::target::HashPoint {
            x: crate::target::GoFloat((LEGEND_ICON_SIZE * 2) as f64),
            y: crate::target::GoFloat(0.0),
        }),
    ];
    let legend_conn = TargetConnection {
        id: c.id.clone(),
        src_arrow: c.src_arrow.clone(),
        dst_arrow: c.dst_arrow.clone(),
        stroke_dash: c.stroke_dash,
        stroke_width: c.stroke_width,
        stroke: c.stroke.clone(),
        fill: c.fill.clone(),
        border_radius: c.border_radius,
        opacity: c.opacity,
        animated: c.animated,
        route: Some(route.into()),
        italic: true,
        font_family: "DEFAULT".into(),
        ..Default::default()
    };
    let legend_hash = format!("legend-{}", hash(&format!("{}-{x}-{y}", c.id)));
    let mut out = String::new();
    let mut markers = std::collections::BTreeSet::new();
    let id_to_shape = std::collections::HashMap::new();
    draw_connection(
        &mut out,
        &legend_hash,
        &legend_conn,
        &mut markers,
        &id_to_shape,
        false,
        theme,
        &mut MarkdownRenderer::new(None, None, theme),
    )?;
    Ok(format!(
        r#"<g transform="translate({x}, {y}) scale({})">{out}</g>"#,
        format_float(1.0 / 2.0)
    ))
}
