// SPDX-License-Identifier: MPL-2.0
//! legend.go buildLegendShapeIcon: ordinary compiled target geometry at 1/5 scale.
use super::*;
use d2_rs_core::LegendShape;

pub(super) fn draw(
    svg: &mut String,
    legend: &Legend,
    row: &LegendShape,
    index: usize,
    x: f64,
    y: f64,
    palette: PaletteView<'_>,
    precision: bool,
    sketch: bool,
    streaks: &mut BTreeSet<&'static str>,
) {
    let compiled = legend.source_graph.as_deref().and_then(|source| {
        let id = row.source_node?;
        let node = source.graph.node(id)?;
        Some((source, id, node))
    });
    let (shape, mut style, font_size) = if let Some((source, id, node)) = compiled {
        let raw = source.styles.get(&id).cloned().unwrap_or_default();
        let style = themed_node_style(&source.graph, id, &raw, palette);
        let font_size = style
            .target_font_size
            .or_else(|| {
                matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable)
                    .then_some(style.font_size.unwrap_or(20))
            })
            .or(node.font_size)
            .unwrap_or(16);
        (
            target_shape_for_node(&source.graph, id).unwrap_or(node.shape),
            style,
            font_size,
        )
    } else {
        // Direct graph callers can still supply semantic rows without a compiler graph.
        (
            row.shape,
            SourceNodeStyle {
                fill: row.fill.clone(),
                stroke: row.stroke.clone(),
                ..Default::default()
            },
            16,
        )
    };
    let opacity = style.opacity.take().unwrap_or_else(|| "1".into());
    style.animated = None;
    style.language = None;
    style.border_radius.get_or_insert_with(|| "0".into());
    style.stroke_width.get_or_insert_with(|| "2".into());
    let shadow = style_flag(Some(&style), |s| s.shadow.as_ref())
        && !matches!(
            shape,
            ShapeKind::Text | ShapeKind::Code | ShapeKind::Class | ShapeKind::SqlTable
        );
    style.shadow = None;
    let icon = style.icon.clone();
    let embedded = matches!(
        shape,
        ShapeKind::Image | ShapeKind::Class | ShapeKind::SqlTable
    );
    if !embedded {
        style.icon = None;
    }
    let rect = Rect {
        origin: Point::default(),
        size: Size {
            width: LEGEND_ICON_SIZE * 5.0,
            height: LEGEND_ICON_SIZE * 5.0,
        },
    };
    let native_dash = if precision {
        stroke_dash_values(
            style.stroke_width.as_deref().unwrap_or("2"),
            style.stroke_dash.as_deref(),
        )
        .map(|(dash, gap)| {
            format!(
                " data-d2-rs-dash-owner-id=\"d2-rs-legend-native-raster-shape-{index}\"{}",
                raster_dash_attributes(dash, gap)
            )
        })
        .unwrap_or_default()
    } else {
        String::new()
    };
    let identity = format!("d2-rs-native-legend-shape-icon-{index}");
    let _ = writeln!(
        svg,
        "<g id=\"{identity}\"{native_dash} stroke-linecap=\"butt\" stroke-linejoin=\"round\" transform=\"translate({} {}) scale(0.2)\"><g opacity=\"{}\">",
        number(x),
        number(y),
        escape(&opacity)
    );
    if shadow {
        let _ = writeln!(
            svg,
            "<defs><filter id=\"d2-rs-legend-shape-shadow-filter-{index}\" x=\"-100%\" y=\"-100%\" width=\"300%\" height=\"300%\" color-interpolation-filters=\"sRGB\"><feDropShadow dx=\"3\" dy=\"5\" stdDeviation=\"1.7\" flood-color=\"#3d4574\" flood-opacity=\"0.4\"/></filter></defs><g id=\"d2-rs-native-legend-shape-shadow-{index}\" data-d2-rs-legend-shape-shadow=\"scene\" filter=\"url(#d2-rs-legend-shape-shadow-filter-{index})\">"
        );
    }
    // Shape opacity belongs to the outer group, including embedded structured text.
    // The source clone clears its main label but retains its original member rows.
    if shape == ShapeKind::Class {
        shapes::draw_class_shape_with_sketch(
            svg,
            rect,
            &style,
            "",
            font_size.saturating_add(4),
            Size::default(),
            palette,
            precision.then_some(identity.as_str()),
            sketch.then_some(&mut *streaks),
        );
    } else if shape == ShapeKind::SqlTable {
        draw_sql_table_shape_with_sketch(
            svg,
            rect,
            &style,
            "",
            font_size.saturating_add(4),
            Size::default(),
            palette,
            precision.then_some(identity.as_str()),
            sketch.then_some(&mut *streaks),
        );
    } else if shape != ShapeKind::Code {
        let rough = sketch
            && sketch_shapes::draw_rectangle(
                svg, shape, rect, &style, palette, None, streaks, precision,
            );
        let rough = rough
            || (sketch
                && sketch_shapes::draw_typed(
                    svg, shape, rect, &style, palette, None, streaks, precision,
                ));
        if !rough {
            draw_shape_with_pattern_overlay(
                svg,
                shape,
                rect,
                &style,
                palette,
                &format!("d2-rs-legend-native-raster-shape-{index}"),
                precision,
                false,
            );
        }
    }
    if shadow {
        svg.push_str("</g>\n");
    }
    if !embedded && let Some(icon) = icon.as_deref() {
        draw_icon(
            svg,
            rect,
            shape,
            icon,
            LabelPosition::InsideMiddleCenter,
            style
                .icon_border_radius
                .as_deref()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0) as f64,
            &format!("{identity}-icon"),
        );
    }
    svg.push_str("</g></g>\n");
}
