// SPDX-License-Identifier: MPL-2.0
// Original D2 v0.9.0 lib/textmeasure/markdown_svg.go: paintTable,
// collectMarkdownTableRows, markdownTableCellContext, fitMarkdownTableColumns.
use super::*;
struct Unit {
    parsed: Vec<Vec<NativeMarkdownRun>>,
    paragraph: bool,
}
struct Cell {
    units: Vec<Unit>,
    align: String,
    width: f64,
    minimum: f64,
    height: f64,
}
struct Row {
    cells: Vec<Cell>,
    stripe: bool,
    height: f64,
}
fn fit(widths: &[f64], minima: &[f64], table_width: f64) -> Vec<f64> {
    let mut fitted = widths.to_vec();
    if widths.is_empty() {
        return fitted;
    }
    let budget = (table_width - (widths.len() + 1) as f64).max(0.0);
    let natural = widths.iter().sum::<f64>();
    if natural <= budget + 0.001 {
        return fitted;
    }
    let min: Vec<_> = widths
        .iter()
        .enumerate()
        .map(|(i, w)| w.min(*minima.get(i).unwrap_or(&0.0)))
        .collect();
    let minimum = min.iter().sum::<f64>();
    let capacity = natural - minimum;
    if minimum <= budget + 0.001 && capacity > 0.0 {
        for (i, w) in widths.iter().enumerate() {
            fitted[i] = w - (natural - budget) * (w - min[i]) / capacity;
        }
    } else if minimum > budget + 0.001 {
        for (i, w) in widths.iter().enumerate() {
            if let Some(m) = minima.get(i) {
                fitted[i] = w.min(*m);
            }
        }
    } else {
        let padding = 26.0 * widths.len() as f64;
        if budget <= padding {
            fitted.fill(budget / widths.len() as f64);
        } else {
            let content = widths.iter().map(|w| (w - 26.0).max(0.0)).sum::<f64>();
            for (i, w) in widths.iter().enumerate() {
                fitted[i] = 26.0
                    + (budget - padding)
                        * if content == 0.0 {
                            1.0 / widths.len() as f64
                        } else {
                            (w - 26.0).max(0.0) / content
                        };
            }
        }
    }
    fitted
}
fn unit(markup: &str, header: bool, paragraph: bool) -> Option<Unit> {
    let parsed = if markup.trim().is_empty() {
        Vec::new()
    } else {
        let runs = native_markdown_html_inline_context(&format!("<p>{markup}</p>\n"), header)?;
        split_native_markdown_lines(&runs)
    };
    Some(Unit { parsed, paragraph })
}
fn cell_units(cell: roxmltree::Node<'_, '_>, html: &str, header: bool) -> Option<Vec<Unit>> {
    let mut units = Vec::new();
    let mut inline = String::new();
    for child in cell.children() {
        if child.has_tag_name("p") {
            if !inline.trim().is_empty() {
                units.push(unit(&inline, header, false)?);
            }
            inline.clear();
            let markup = child
                .children()
                .map(|n| html[n.range()].to_owned())
                .collect::<String>();
            units.push(unit(&markup, header, true)?);
        } else {
            inline.push_str(&html[child.range()]);
        }
    }
    if !inline.trim().is_empty() {
        units.push(unit(&inline, header, false)?);
    }
    Some(units)
}
fn collect(
    node: roxmltree::Node<'_, '_>,
    html: &str,
    header: bool,
    rows: &mut Vec<Row>,
    family: MarkdownFamily,
    size: f64,
) -> Option<()> {
    let header = header || node.has_tag_name("thead");
    let mut row_index = 0;
    for child in node.children().filter(roxmltree::Node::is_element) {
        if child.has_tag_name("tr") {
            let header = header || child.children().any(|n| n.has_tag_name("th"));
            let mut cells = Vec::new();
            let fonts = native_markdown_measurer(family)?;
            let m = fonts.measurer();
            for cell in child
                .children()
                .filter(|n| n.has_tag_name("td") || n.has_tag_name("th"))
            {
                let units = cell_units(cell, html, header)?;
                let mut width: f64 = 0.0;
                let mut minimum: f64 = 0.0;
                for unit in &units {
                    let (w, min) =
                        markdown_wrap::content_widths(&unit.parsed, size, |run, text| {
                            let font = if run.code && run.code_bold {
                                m.mono_bold_font
                            } else if run.code && run.code_italic {
                                m.mono_italic_font
                            } else if run.code {
                                m.mono_font
                            } else if run.italic_after_bold {
                                m.italic_font
                            } else if run.semibold {
                                m.semibold_font
                            } else if run.bold {
                                m.bold_font
                            } else if run.italic {
                                m.italic_font
                            } else {
                                m.font
                            };
                            font.measure_markdown_css_advance(&text.replace('\u{ad}', ""), size)
                                * if run.code { 0.85 } else { 1.0 }
                        });
                    width = width.max(w);
                    minimum = minimum.max(min);
                }
                let align = cell
                    .attribute("align")
                    .unwrap_or(if header { "center" } else { "" })
                    .to_owned();
                cells.push(Cell {
                    units,
                    align,
                    width: width + 26.0,
                    minimum: minimum + 26.0,
                    height: 0.0,
                });
            }
            rows.push(Row {
                cells,
                stripe: row_index % 2 == 1,
                height: 0.0,
            });
            row_index += 1;
        } else if matches!(child.tag_name().name(), "thead" | "tbody" | "tfoot") {
            collect(child, html, header, rows, family, size)?;
        }
    }
    Some(())
}
fn flow(
    svg: &mut String,
    cell: &Cell,
    x: f64,
    y: f64,
    width: f64,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    precision: bool,
    family: MarkdownFamily,
) -> Option<f64> {
    let mut height = 0.0;
    let mut previous_margin: f64 = 0.0;
    for (index, unit) in cell.units.iter().enumerate() {
        if index > 0 {
            height += previous_margin;
        }
        let mut painted = String::new();
        let mut h = 0.0;
        if !draw_native_markdown_inline_context(
            &mut painted,
            &unit.parsed,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: x + width,
                    height: 1024.0 + y + height,
                },
            },
            font_size,
            fill,
            class,
            precision,
            family,
            Point { x, y: y + height },
            true,
            &cell.align,
            Some(&mut h),
        ) {
            return None;
        }
        // Inline paint returns a viewport for ordinary paragraphs. Cells share
        // the table's viewport; retain only the primitive group here.
        svg.push_str(&painted[painted.find('>')? + 1..painted.len() - 6]);
        if !unit.parsed.is_empty() {
            h = h.max(f64::from(font_size) * 1.25);
        }
        height += h;
        previous_margin = if unit.paragraph { 16.0 } else { 0.0 };
    }
    Some(height)
}
fn rect(svg: &mut String, x: f64, y: f64, w: f64, h: f64, class: &str, color: &str) {
    let _ = write!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"0.000\" class=\"{}\" fill=\"{}\"/>",
        native_markdown_number(x),
        native_markdown_number(y),
        native_markdown_number(w),
        native_markdown_number(h),
        class,
        color
    );
}
pub(super) fn paint(
    svg: &mut String,
    text: &str,
    box_: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    precision: bool,
    family: MarkdownFamily,
) -> bool {
    let html = format!(
        "<body>{}</body>",
        markdown_measure::legacy_inline::measurement_html(&render_html(text.as_bytes()))
    );
    let Ok(doc) = roxmltree::Document::parse(&html) else {
        return false;
    };
    let blocks = doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    if blocks.len() != 1 || !blocks[0].has_tag_name("table") {
        return false;
    }
    let mut content = String::new();
    if paint_node(
        &mut content,
        blocks[0],
        &html,
        Point::default(),
        box_.size.width,
        font_size,
        fill,
        class,
        precision,
        family,
    )
    .is_none()
    {
        return false;
    }
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">{}</g></svg>",
        number(box_.origin.x),
        number(box_.origin.y),
        number(box_.size.width.max(1.0)),
        number(box_.size.height.max(1.0)),
        number(box_.size.width.max(1.0)),
        number(box_.size.height.max(1.0)),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
        content
    );
    true
}
pub(super) fn paint_node(
    svg: &mut String,
    node: roxmltree::Node<'_, '_>,
    html: &str,
    origin: Point,
    available_width: f64,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    precision: bool,
    family: MarkdownFamily,
) -> Option<f64> {
    let mut rows = Vec::new();
    collect(node, html, false, &mut rows, family, f64::from(font_size))?;
    let mut widths: Vec<f64> = Vec::new();
    let mut minima: Vec<f64> = Vec::new();
    for row in &rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i == widths.len() {
                widths.push(0.0);
                minima.push(0.0);
            }
            widths[i] = widths[i].max(cell.width);
            minima[i] = minima[i].max(cell.minimum);
        }
    }
    let natural = widths.iter().sum::<f64>() + (widths.len() + 1) as f64;
    let table_width = if available_width > 0.0 {
        natural.min(available_width)
    } else {
        natural
    }
    .max(1.0);
    widths = fit(&widths, &minima, table_width);
    let table_width = table_width.max(widths.iter().sum::<f64>() + (widths.len() + 1) as f64);
    for row in &mut rows {
        let mut max: f64 = 0.0;
        for (i, cell) in row.cells.iter_mut().enumerate() {
            let Some(h) = flow(
                &mut String::new(),
                cell,
                0.0,
                0.0,
                (widths[i] - 26.0).max(0.0),
                font_size,
                fill,
                class,
                precision,
                family,
            ) else {
                return None;
            };
            cell.height = h;
            max = max.max(h + 12.0);
        }
        row.height = max + 1.0;
    }
    let left = native_markdown_snap(origin.x);
    let top = native_markdown_snap(origin.y);
    let right = native_markdown_snap(origin.x + table_width - 1.0);
    let bottom = native_markdown_snap(origin.y + rows.iter().map(|r| r.height).sum::<f64>());
    let w = right - left + 1.0;
    let h = bottom - top + 1.0;
    let mut content = String::new();
    rect(&mut content, left, top, w, h, "fill-N7", "#FFFFFF");
    let mut y = origin.y;
    for row in &rows {
        if row.stripe {
            rect(
                &mut content,
                left + 1.0,
                native_markdown_snap(y),
                (right - left - 1.0).max(0.0),
                native_markdown_snap(y + row.height) - native_markdown_snap(y),
                "fill-N6",
                "#EEF1F8",
            );
        }
        y += row.height;
    }
    let mut grid = String::new();
    rect(&mut grid, left, top, w, 1.0, "fill-B1", "#0D32B2");
    rect(&mut grid, left, top, 1.0, h, "fill-B1", "#0D32B2");
    rect(&mut grid, right, top, 1.0, h, "fill-B1", "#0D32B2");
    y = origin.y;
    for row in &rows {
        let mut x = origin.x + 1.0;
        for (i, cell) in row.cells.iter().enumerate() {
            let content_y = y + 7.0 + (row.height - 13.0 - cell.height).max(0.0) / 2.0;
            flow(
                &mut content,
                cell,
                x + 13.0,
                content_y,
                widths[i] - 26.0,
                font_size,
                fill,
                class,
                precision,
                family,
            )?;
            x += widths[i];
            if i + 1 < row.cells.len() {
                rect(
                    &mut grid,
                    native_markdown_snap(x),
                    native_markdown_snap(y),
                    1.0,
                    native_markdown_snap(y + row.height) - native_markdown_snap(y),
                    "fill-B1",
                    "#0D32B2",
                );
            }
            x += 1.0;
        }
        y += row.height;
        rect(
            &mut grid,
            left,
            native_markdown_snap(y),
            w,
            1.0,
            "fill-B1",
            "#0D32B2",
        );
    }
    svg.push_str(&content);
    svg.push_str(&grid);
    Some(h)
}
