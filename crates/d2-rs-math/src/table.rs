// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Translated from pinned mathjax-go v0.1.0 tex/environments.go,
// svg/{mtable,mtr,mo,stretch}.go and font/delimiter.go.
use super::*;

pub(super) fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
pub(super) fn group(body: Layout, x: f64, y: f64, scale: f64, kind: &'static str) -> LayoutGroup {
    LayoutGroup {
        x_frame: None,
        data_cramped: None,
        own_translate: None,
        accent_offset: None,
        box_border: None,
        reference: None,
        tagged: None,
        percentage: None,
        table_line: None,
        mathsize: None,
        tex_class: 0,
        before_glyph: 0,
        body: Box::new(body),
        x,
        baseline: y,
        scale,
        kind,
        color: None,
        background: None,
        lines: Vec::new(),
        delimiter: None,
        arrow: None,
    }
}

// ArrayItem receives closing events from the parser after argument readers.
// EndTable drops a pending empty final row; explicit Entry/CR delivers empties.
pub(super) fn parse_rows_count_options(
    content: &str,
    environment: &str,
    ams: ams::Shared,
    equation_seen: EquationStack,
    pair_count: Option<&str>,
    options: &mut Option<MultlineOptions>,
) -> Option<Vec<Vec<Atom>>> {
    let body = array_body::parse(content, environment, ams.clone(), equation_seen)?;
    options.get_or_insert_with(Default::default).rules = body.rules;
    if body.align.iter().flatten().any(|a| !a.is_empty()) {
        options.get_or_insert_with(Default::default).align = body.align;
    }
    if !body.spacing.is_empty() {
        options.get_or_insert_with(Default::default).row_spacing = body.spacing;
    }
    let mut parsed = Vec::new();
    for atoms in body.rows {
        let mut row = Vec::new();
        for (column, mut cell) in atoms.into_iter().enumerate() {
            if cd::first_error(&cell).is_some() {
                return Some(vec![vec![cell]]);
            }
            if !alignment::is_equation(environment)
                && (environment.contains("align")
                    || environment.starts_with("eqnarray")
                    || environment == "split")
                && column > 0
            {
                let mut children = match cell {
                    Atom::Sequence(v) => v,
                    a => vec![a],
                };
                if children.first().is_some_and(|a| {
                    matches!(a, Atom::Glyph(..) | Atom::RawOperator { .. })
                        && atom_tex_class(a) == Some(3)
                }) {
                    children.insert(0, Atom::Identifier(Vec::new()));
                }
                cell = Atom::Sequence(children);
            }
            row.push(cell);
        }
        if matches!(
            environment,
            "flalign" | "flalign*" | "xalignat" | "xalignat*" | "xxalignat"
        ) {
            // FlalignItem.EndRow pads authored entries to the declared pair
            // count, then inserts fit cells between pairs (ams_flalign.go).
            let entries = pair_count
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0)
                * 2.0;
            if entries != 0.0 && row.len() as f64 > entries {
                return Some(vec![vec![Atom::ErrorDisplay(format!(
                    "Extra & in row of {environment}"
                ))]]);
            }
            // Keep this native subset bounded; enormous declared counts
            // remain explicitly unsupported rather than allocating in
            // proportion to a user-supplied decimal count.
            if entries > 5120.0 {
                return None;
            }
            while (row.len() as f64) < entries {
                row.push(Atom::InferredRow(Vec::new()));
            }
            let has_padding = matches!(environment, "xalignat" | "xalignat*");
            let n = row.len();
            let mut padded = Vec::new();
            if has_padding {
                padded.push(Atom::InferredRow(Vec::new()));
            }
            for (i, cell) in row.into_iter().enumerate() {
                padded.push(cell);
                if (i % 2 == 1 || i + 1 == n) && (i + 1 < n || has_padding) {
                    padded.push(Atom::InferredRow(Vec::new()));
                }
            }
            row = padded;
        }
        parsed.push(row);
    }
    Some(parsed)
}

// A "." fence of \\left/\\right is a symmetric empty mo; moToSVG centers its
// EmptyBBox (H = D = -BigDimen) to u = axis height 0.25em.
fn empty_fence(x: f64, scale: f64) -> LayoutGroup {
    let mut g = group(blank(), x, 0.0, scale, "mo");
    g.own_translate = Some(250.0);
    g
}
pub(super) fn fences(environment: &str) -> Option<(u32, u32)> {
    Some(match environment.trim_end_matches('*') {
        "pmatrix" | "psmallmatrix" => (0x28, 0x29),
        "bmatrix" | "bsmallmatrix" => (0x5B, 0x5D),
        "Bmatrix" | "Bsmallmatrix" => (0x7B, 0x7D),
        "vmatrix" | "vsmallmatrix" => (0x7C, 0x7C),
        "Vmatrix" | "Vsmallmatrix" => (0x2016, 0x2016),
        "cases" | "dcases" => (0x7B, 0),
        "rcases" | "drcases" => (0, 0x7D),
        _ => return None,
    })
}
// computeTableBBox replaces CommonWrapper's leading TeX space with labelLR.
// A bare table still returns ORD to the subsequent class traversal.
pub(super) fn replaces_leading_space(atom: &Atom) -> bool {
    matches!(atom, Atom::TaggedTable { .. })
        || matches!(atom,Atom::Matrix{environment,..} if fences(environment).is_none())
}
pub(super) fn display(environment: &str) -> bool {
    environment == "CD"
        || environment == "displaylines"
        || environment == "split"
        || environment == "equation-tag"
        || environment.starts_with("eqnarray")
        || environment.contains("align")
        || environment.contains("gather")
        || environment.contains("multline")
        || environment.starts_with('d') && environment.contains("cases")
}

pub(super) fn layout(
    rows: &[Vec<Atom>],
    environment: &str,
    column_spec: &str,
    options: Option<&MultlineOptions>,
    spacing: Option<f64>,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    _prime: bool,
) -> Option<f64> {
    if environment == "CD" {
        return cd::layout_table(rows, spacing, options, x, baseline, scale, out);
    }
    let subarray = environment.contains("subarray");
    let small = environment.contains("smallmatrix") || subarray;
    let aligns: Vec<char> = if matches!(environment, "aligned" | "alignedat") {
        Vec::new()
    } else if matches!(environment, "eqnarray" | "eqnarray*") {
        (0..rows.iter().map(Vec::len).max().unwrap_or(0))
            .map(|i| ['r', 'c', 'l'][i % 3])
            .collect()
    } else {
        column_spec
            .chars()
            .filter(|c| matches!(c, 'l' | 'c' | 'r'))
            .collect()
    };
    // wrapper.getScale uses the pinned script multiplier for the row frame.
    let cell_scale = if small {
        styles::SCRIPT_MULTIPLIER
    } else {
        scale
    };
    let aligned = environment.contains("align") || environment == "split";
    let eqnarray = matches!(environment, "eqnarray" | "eqnarray*");
    let cases = environment.contains("cases");
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let multline = matches!(environment, "multline" | "multline*");
    let multlined = environment == "multlined";
    let padding = if multline { 1.0 } else { 0.0 };
    let percent_value = if multlined {
        options
            .filter(|o| o.width.ends_with('%'))
            .and_then(|o| o.width.trim_end_matches('%').parse::<f64>().ok())
            .map(|v| v / 100.0)
    } else {
        None
    };
    let flalign = matches!(environment, "flalign" | "flalign*");
    let percentage = matches!(environment, "xalignat" | "xalignat*" | "xxalignat")
        || flalign && columns > 2
        || multline && !rows.is_empty()
        || percent_value.is_some();
    let padded = environment != "xxalignat";
    let column_gap = if let Some(gap) = options.and_then(|o| o.column_gap) {
        gap
    } else if subarray {
        0.0
    } else if small {
        0.333
    } else if aligned {
        0.0
    } else {
        1.0
    };
    let column_gaps: Vec<f64> = (0..columns.saturating_sub(1))
        .map(|i| {
            if eqnarray {
                if i % 2 == 0 { 0.0 } else { 0.278 }
            } else if matches!(environment, "align" | "align*" | "aligned") && i % 2 == 1 {
                2.0
            } else {
                column_gap
            }
        })
        .collect();
    let row_gap = if environment == "displaylines" || environment.contains("multline") {
        0.5
    } else if subarray {
        0.1
    } else if small || cases {
        0.2
    } else if aligned || eqnarray || environment.contains("gather") {
        length::dimension("3pt").em(1.0, scale)
    } else {
        length::dimension("4pt").em(1.0, scale)
    };
    let row_gap = spacing.unwrap_or(row_gap);
    let row_gaps: Vec<f64> = (0..rows.len().saturating_sub(1))
        .map(|i| {
            options
                .and_then(|o| o.row_spacing.get(i))
                .copied()
                .unwrap_or(row_gap)
        })
        .collect();
    let mut widths = vec![0.0_f64; columns];
    let mut measured = Vec::new();
    let mut hd = Vec::new();
    for row in rows {
        let (mut h, mut d) = (0.0_f64, 0.0_f64);
        let mut cells = Vec::new();
        for (column, atom) in row.iter().enumerate() {
            let mut body = blank();
            body.width = layout_at_with_style(
                atom,
                0.0,
                0.0,
                cell_scale,
                &mut body,
                display(environment),
                // inheritance.go mtable resets prime to its own data-cramped,
                // independently of the enclosing prime style.
                environment == "crampedsubarray",
            )?;
            retain_negative_glyph_depth(&mut body);
            widths[column] = widths[column].max(
                body.intrinsic_width_em
                    .map(|w| w * (cell_scale / scale))
                    .unwrap_or_else(|| body.width_em.unwrap_or(body.width / 1000.0) / scale),
            );
            let (cell_h, cell_d) = styles::em_bounds(&body);
            h = h.max(cell_h / scale);
            d = d.max(cell_d / scale);
            if !small {
                h = h.max(0.75);
                d = d.max(0.25);
            }
            cells.push(body);
        }
        hd.push((h, d));
        measured.push(cells);
    }
    if multlined && options.is_some_and(|o| !o.width.is_empty() && !o.width.ends_with('%')) {
        let width = length::dimension(&options.unwrap().width).em(1.0, scale);
        widths.fill(width);
    }
    let rules = options
        .map(|o| &o.rules)
        .cloned()
        .unwrap_or_default()
        .finish(rows.len(), column_spec);
    let column_lines: Vec<Option<char>> = (0..columns.saturating_sub(1))
        .map(|i| {
            match rules
                .columns
                .get(i)
                .or_else(|| rules.columns.last())
                .copied()
                .unwrap_or("none")
            {
                "solid" => Some('|'),
                "dashed" => Some(':'),
                _ => None,
            }
        })
        .collect();
    let row_lines: Vec<Option<&str>> = (0..rows.len().saturating_sub(1))
        .map(|i| {
            rules
                .lines
                .get(i)
                .or_else(|| rules.lines.last())
                .copied()
                .filter(|s| *s != "none")
        })
        .collect();
    let frame_line = if rules.full { 0.07 } else { 0.0 };
    let has_frame = !rules.frame.is_empty();
    let frame_x = if has_frame && !multline { 0.4 } else { 0.0 };
    let frame_y = if has_frame && !multline {
        length::dimension("0.5ex").em(0.0, scale)
    } else {
        0.0
    };
    let width = 2.0 * (padding + frame_x + frame_line)
        + widths.iter().sum::<f64>()
        + column_gaps.iter().sum::<f64>()
        + column_lines.iter().filter(|c| c.is_some()).count() as f64 * 0.07;
    let height = hd
        .iter()
        .map(|(h, _)| *h)
        .chain(hd.iter().map(|(_, d)| *d))
        .chain(row_gaps.iter().copied())
        .fold(0., |total, value| total + value)
        + 2.0 * (frame_y + frame_line)
        + row_lines.iter().filter(|v| v.is_some()).count() as f64 * 0.07;
    // Original svg/mtable.go alignmentRow/verticalPosition/bboxHD.
    let h = if multlined && options.is_some_and(|o| o.position == "t" || o.position == "b") {
        if options.unwrap().position == "t" {
            hd.first().map_or(0.0, |v| v.0)
        } else {
            height - hd.last().map_or(0.0, |v| v.1)
        }
    } else if matches!(environment, "aligned" | "alignedat") && !column_spec.is_empty() {
        let fields: Vec<_> = column_spec.split_whitespace().collect();
        let align = fields.first().copied().unwrap_or("axis");
        let row = fields
            .get(1)
            .and_then(|v| v.parse::<i64>().ok())
            .map(|v| if v < 0 { v + rows.len() as i64 + 1 } else { v })
            .filter(|v| *v >= 1 && *v <= rows.len() as i64);
        if let Some(row) = row {
            let row = row as usize - 1;
            let before = hd[..row].iter().map(|(h, d)| h + d + row_gap).sum::<f64>()
                - if row > 0 { row_gap / 2.0 } else { 0.0 };
            let half_space = if row > 0 { row_gap / 2.0 } else { 0.0 };
            before
                + match align {
                    "center" => half_space + (hd[row].0 + hd[row].1) / 2.0,
                    "bottom" => {
                        half_space
                            + hd[row].0
                            + hd[row].1
                            + if row + 1 < rows.len() {
                                row_gap / 2.0
                            } else {
                                0.0
                            }
                    }
                    "baseline" => half_space + hd[row].0,
                    "axis" => half_space + hd[row].0 - 0.25,
                    _ => 0.0,
                }
        } else {
            match align {
                "top" => 0.0,
                "bottom" => height,
                "axis" => height / 2.0 + 0.25,
                _ => height / 2.0,
            }
        }
    } else {
        height / 2.0 + 0.25
    };
    let d = height - h;
    let mut table = blank();
    let mut y = h - frame_line - frame_y;
    for (index, cells) in measured.into_iter().enumerate() {
        let top_space = if index == 0 {
            0.0
        } else {
            row_gaps[index - 1] / 2.0
        };
        let bottom_space = if index + 1 == rows.len() {
            0.0
        } else {
            row_gaps[index] / 2.0
        };
        let row_baseline = y - top_space - hd[index].0;
        let mut row = blank();
        // mtr.renderTableRow scales column widths by 1 / row.RScale before
        // tableAlignX subtracts the intrinsic cell BBox.W. Keep that order: a
        // tiny nonzero result must survive until wrapper.place tests it.
        let row_scale = cell_scale / scale;
        let inverse_row_scale = 1.0 / row_scale;
        let mut cursor = padding + frame_line + frame_x;
        for (column, mut cell) in cells.into_iter().enumerate() {
            ams::set_wrap_width(&mut cell, widths[column] * scale * 1000.0);
            let alignment = aligns.get(column).or_else(|| aligns.last()).copied();
            let override_align = options
                .and_then(|o| o.align.get(index))
                .and_then(|r| r.get(column))
                .copied()
                .filter(|a| !a.is_empty());
            let cell_align = if let Some(a) = override_align {
                a
            } else if percentage {
                match if padded { column % 3 } else { (column + 1) % 3 } {
                    0 => "center",
                    1 => "right",
                    _ => "left",
                }
            } else if alignment == Some('l') || cases || aligned && column % 2 == 1 {
                "left"
            } else if alignment == Some('r') || aligned {
                "right"
            } else {
                "center"
            };
            ams::set_percentage_wrap(&mut cell, widths[column] * scale * 1000.0, cell_align);
            let cell_width = cell
                .intrinsic_width_em
                .unwrap_or_else(|| cell.width_em.unwrap_or(cell.width / 1000.0) / cell_scale);
            let column_width = widths[column] * inverse_row_scale;
            let alignment = aligns.get(column).or_else(|| aligns.last()).copied();

            let dx = if let Some(a) = override_align {
                match a {
                    "left" => cursor,
                    "right" => cursor + column_width - cell_width,
                    _ => cursor + (column_width - cell_width) / 2.0,
                }
            } else if percentage {
                let pattern = if flalign {
                    (column + 1) % 3
                } else if padded {
                    column % 3
                } else {
                    (column + 1) % 3
                };
                match pattern {
                    0 => cursor + (column_width - cell_width) / 2.0,
                    1 => cursor + column_width - cell_width,
                    _ => cursor,
                }
            } else if alignment == Some('l') {
                cursor
            } else if alignment == Some('r') {
                cursor + column_width - cell_width
            } else if alignment == Some('c') {
                cursor + (column_width - cell_width) / 2.0
            } else if cases || aligned && column % 2 == 1 {
                cursor
            } else if aligned {
                cursor + column_width - cell_width
            } else {
                cursor + (column_width - cell_width) / 2.0
            };
            row.groups.push(group(
                cell,
                dx * cell_scale * 1000.0,
                0.0,
                cell_scale,
                "mtd",
            ));
            cursor += column_width
                + column_gaps.get(column).copied().unwrap_or(0.0) * inverse_row_scale
                + if column_lines.get(column).is_some_and(Option::is_some) {
                    0.07 * inverse_row_scale
                } else {
                    0.0
                };
        }
        table.groups.push(group(
            row,
            0.0,
            row_baseline * scale * 1000.0,
            cell_scale,
            "mtr",
        ));
        y -= top_space
            + hd[index].0
            + hd[index].1
            + bottom_space
            + if row_lines.get(index).is_some_and(Option::is_some) {
                0.07
            } else {
                0.0
            };
    }
    let mut line_x = frame_line + frame_x;
    for (column, style) in column_lines.iter().enumerate() {
        let gap = column_gaps[column];
        line_x += widths[column] + gap / 2.0;
        if let Some(style) = style {
            let mut line = group(blank(), 0.0, 0.0, scale, "#table-line");
            line.table_line = Some(TableLine {
                x2: None,
                frame: false,
                x: (line_x + 0.035) * 1000.0,
                y1: -d * 1000.0,
                y2: h * 1000.0,
                dashed: *style == ':',
            });
            line.before_glyph = table.glyphs.len();
            table.groups.push(line);
            line_x += 0.07;
        }
        line_x += gap / 2.0;
    }
    let mut line_y = h - frame_line - frame_y;
    for (i, style) in row_lines.iter().enumerate() {
        line_y -= hd[i].0
            + hd[i].1
            + row_gaps[i] / 2.0
            + if i == 0 { 0.0 } else { row_gaps[i - 1] / 2.0 };
        if let Some(style) = style {
            let mut line = group(blank(), 0.0, 0.0, scale, "#table-line");
            line.table_line = Some(TableLine {
                x: 0.0,
                x2: Some(width * 1000.0),
                frame: false,
                y1: (line_y - 0.035) * 1000.0,
                y2: (line_y - 0.035) * 1000.0,
                dashed: *style == "dashed",
            });
            table.groups.push(line);
            line_y -= 0.07;
        }
    }
    if rules.full {
        let mut line = group(blank(), 0.0, 0.0, scale, "#table-line");
        line.table_line = Some(TableLine {
            x: 35.0,
            x2: Some(width * 1000.0 - 35.0),
            frame: true,
            y1: 35.0 - d * 1000.0,
            y2: h * 1000.0 - 35.0,
            dashed: rules.dashed,
        });
        table.groups.push(line);
    }
    let (mut table, width, h_em, d_em, table_kind) =
        array_rules::enclose(table, width, h, d, scale, &rules);
    let width_em = width * scale;
    let (width, h, d) = (
        width * scale * 1000.0,
        h_em * scale * 1000.0,
        d_em * scale * 1000.0,
    );
    table.width = width;
    table.ascent = h;
    table.depth = d;
    let mut total = width;
    let mut outer = blank();
    let mut ascent = h;
    let mut depth = d;
    if let Some((open, close)) = fences(environment) {
        let mut cursor = 0.0;
        if open != 0 {
            let (w, fh, fd, g) = fence(open, h_em, d_em, scale)?;
            outer.groups.push(g);
            cursor = w;
            total += w;
            ascent = ascent.max(fh);
            depth = depth.max(fd);
        } else {
            outer.groups.push(empty_fence(0.0, scale));
        }
        outer
            .groups
            .push(group(table, cursor, 0.0, scale, table_kind));
        cursor += width;
        if close != 0 {
            let (w, fh, fd, mut g) = fence(close, h_em, d_em, scale)?;
            g.x = cursor;
            g.x_frame = None;
            outer.groups.push(g);
            total += w;
            ascent = ascent.max(fh);
            depth = depth.max(fd);
        } else {
            outer.groups.push(empty_fence(cursor, scale));
        }
        outer.width = total;
        outer.ascent = ascent;
        outer.depth = depth;
        let mut wrapper = group(outer, x, baseline, scale, "mrow");
        wrapper.before_glyph = out.glyphs.len();
        out.groups.push(wrapper);
    } else {
        let mut wrapper = group(table, x, baseline, scale, table_kind);
        wrapper.before_glyph = out.glyphs.len();
        if environment == "crampedsubarray" {
            // ordinary_array.go sets an explicit attribute on the mtable;
            // wrapper.go handleAttributes preserves it through rule enclosures.
            if table_kind == "mtable" {
                wrapper.data_cramped = Some(true);
            } else {
                wrapper.body.groups[0].body.groups[0].data_cramped = Some(true);
            }
        }
        if percentage {
            let state = Some(Percentage {
                natural: widths,
                fitted: (0..columns)
                    .map(|i| {
                        if flalign {
                            i % 3 == 2
                        } else if multline || multlined {
                            false
                        } else if padded {
                            i % 3 == 0
                        } else {
                            i % 3 == 2
                        }
                    })
                    .collect(),
                padding,
                gaps: column_gaps,
                row_align: options.map_or(Vec::new(), |o| o.align.clone()),
                ratio: percent_value.unwrap_or(1.0),
                align: (0..columns)
                    .map(|i| {
                        if flalign {
                            (i + 1) % 3
                        } else if padded {
                            i % 3
                        } else {
                            (i + 1) % 3
                        }
                    })
                    .collect(),
                scale,
                top: false,
                wrap_width: None,
                container_align: "left",
                resolved: false,
                width_includes_label: flalign
                    || multline
                    || matches!(environment, "xalignat" | "xalignat*" | "xxalignat"),
            });
            if table_kind == "mtable" {
                wrapper.percentage = state;
            } else {
                // menclose is the width container. The mtable retains its
                // own percentage state and data-width-includes-label flag.
                wrapper.body.groups[0].body.groups[0].percentage = state;
            }
        }
        out.groups.push(wrapper);
    }
    out.ascent = out.ascent.max(baseline + ascent);
    out.depth = out.depth.max(depth - baseline);
    if fences(environment).is_none() {
        out.width_em = Some(width_em);
    }
    Some(total)
}

pub(super) fn font_glyph(code: u32, variant: usize) -> Option<GlyphData> {
    macro_rules! lookup {
        ($table:expr) => {{
            $table
                .binary_search_by_key(&code, |g| g.codepoint)
                .ok()
                .map(|i| {
                    let g = $table[i];
                    GlyphData {
                        height: g.height,
                        depth: g.depth,
                        width: g.width,
                        path: g.path,
                    }
                })
        }};
    }
    match variant {
        0 => glyph(code),
        1 => lookup!(smallop_glyphs::SMALL_OP_GLYPHS),
        2 => lookup!(large_glyphs::LARGE_GLYPHS),
        3 => lookup!(size3_glyphs::SIZE3_GLYPHS),
        4 => lookup!(size4_glyphs::SIZE4_GLYPHS),
        5 => variant_glyph(code),
        _ => None,
    }
    .or_else(|| glyph(code))
}
fn recipe(code: u32) -> Option<&'static vertical_delimiters::Recipe> {
    vertical_delimiters::RECIPES
        .binary_search_by_key(&code, |r| r.code)
        .ok()
        .map(|i| &vertical_delimiters::RECIPES[i])
}
// Source svg/mo.go canStretch returns false when LookupDelimiter has no recipe.
pub(super) fn can_stretch(code: u32) -> bool {
    recipe(code).is_some()
}
// computeMoBBox initializes an unsized stretchy mo with dimension zero.
pub(super) fn natural_fence(
    code: u32,
    scale: f64,
    symmetric: bool,
) -> Option<(f64, f64, f64, LayoutGroup)> {
    fence_target(code, 0., 0., scale, symmetric, None)
}
pub(super) fn fence(code: u32, h: f64, d: f64, scale: f64) -> Option<(f64, f64, f64, LayoutGroup)> {
    fence_with_symmetry(code, h, d, scale, true)
}
// svg/mo.go: middle tokens inherit symmetry from the operator dictionary;
// only the outer left/right delimiters force axis symmetry.
pub(super) fn fence_with_symmetry(
    code: u32,
    h: f64,
    d: f64,
    scale: f64,
    symmetric: bool,
) -> Option<(f64, f64, f64, LayoutGroup)> {
    let dimension = if symmetric {
        2.0 * (h - 0.25).max(d + 0.25)
    } else {
        h + d
    };
    fence_target(
        code,
        dimension,
        (dimension * 0.901).max(dimension - 0.3),
        scale,
        symmetric,
        Some((h, d)),
    )
}
pub(super) fn fixed_fence(
    code: u32,
    size: f64,
    scale: f64,
) -> Option<(f64, f64, f64, LayoutGroup)> {
    fence_target(code, size, size, scale, true, None)
}
fn fence_target(
    code: u32,
    dimension: f64,
    target: f64,
    scale: f64,
    symmetric: bool,
    dimensions: Option<(f64, f64)>,
) -> Option<(f64, f64, f64, LayoutGroup)> {
    let r = recipe(code)?;
    let selected = r.sizes.iter().position(|size| *size >= target).or_else(|| {
        if r.parts.is_empty() {
            r.sizes.len().checked_sub(1)
        } else {
            None
        }
    });
    let mut g = group(blank(), 0.0, 0.0, scale, "mo");
    let (width, height, depth) = if let Some(index) = selected {
        let variant = r.variants.get(index).copied().unwrap_or(index);
        let paint = r
            .chars
            .get(index)
            .copied()
            .filter(|c| *c != 0)
            .unwrap_or(if r.alias != 0 { r.alias } else { code });
        let glyph = font_glyph(paint, variant)?;
        let shift = if symmetric {
            (glyph.height + glyph.depth) / 2.0 + 0.25 - glyph.height
        } else {
            0.0
        };
        g.delimiter = Some(cd::Delimiter {
            code: paint,
            length: 0.0,
            height: shift * 1000.0,
            depth: 0.0,
            fixed: true,
            font_variant: Some(variant),
        });
        (
            glyph.width * 1000.0,
            (glyph.height + shift) * 1000.0,
            (glyph.depth - shift) * 1000.0,
        )
    } else {
        let mut total = dimension;
        if let Some([extender, ends]) = r.full {
            total = ends + ((dimension - ends).max(0.0) / extender).ceil() * extender;
        }
        total = total.max(r.minimum);
        let depth = if symmetric {
            total / 2.0 - 0.25
        } else if let Some((_, d)) = dimensions.filter(|(h, d)| h + d == total) {
            d
        } else {
            let [h, d] = r.height_depth;
            d * total / (h + d)
        };
        let height = if symmetric {
            total / 2.0 + 0.25
        } else {
            total - depth
        };
        g.delimiter = Some(cd::Delimiter {
            code,
            length: r.width * 1000.0,
            height: height * 1000.0,
            depth: depth * 1000.0,
            fixed: false,
            font_variant: Some(usize::MAX),
        });
        (r.width * 1000.0, height * 1000.0, depth * 1000.0)
    };
    if !symmetric {
        // Preserve the inherited, uncentered bounds for style wrappers too.
        g.body.bounds_em = Some((height / 1000.0, depth / 1000.0));
        g.body.ascent = height;
        g.body.depth = depth;
    }
    Some((width * scale, height * scale, depth * scale, g))
}

pub(super) fn render_fence(svg: &mut String, d: cd::Delimiter) {
    if d.fixed {
        let glyph =
            font_glyph(d.code, d.font_variant.unwrap()).expect("source fixed delimiter font");
        svg.push_str(&format!(
            "<path data-c=\"{:X}\" d=\"M{}Z\"></path>",
            d.code, glyph.path
        ));
        return;
    }
    let r = recipe(d.code).expect("source delimiter recipe");
    let part = |i: usize| {
        r.parts
            .get(i)
            .copied()
            .filter(|c| *c != 0)
            .and_then(|c| font_glyph(c, 4).map(|g| (c, g)))
    };
    // stretch.go addStretchGlyph -> placeChar: wrapper.place installs the
    // part's own transform after its d attribute (none when it rounds to 0,0).
    let draw = |svg: &mut String, c: u32, g: GlyphData, y: f64| {
        let (x, y) = (
            root_number((d.length - g.width * 1000.0) / 2.0),
            root_number(y),
        );
        let place = if x == "0" && y == "0" {
            String::new()
        } else {
            format!(" transform=\"translate({x},{y})\"")
        };
        svg.push_str(&format!(
            "<path data-c=\"{c:X}\" d=\"M{}Z\"{place}></path>",
            g.path
        ))
    };
    let mut top = 0.0;
    let mut bottom = 0.0;
    if let Some((c, g)) = part(0) {
        draw(svg, c, g, d.height - g.height * 1000.0);
        top = (g.height + g.depth) * 1000.0;
    }
    if let Some((c, g)) = part(2) {
        draw(svg, c, g, g.depth * 1000.0 - d.depth);
        bottom = (g.height + g.depth) * 1000.0;
    }
    let extender = |svg: &mut String, h: f64, depth: f64, top: f64, bottom: f64| {
        let Some((c, g)) = part(1) else { return };
        // Original stretch.go computes extent, stretch and viewport center in
        // ems, converting only at fixed(). Unit-space arithmetic changes ties.
        let h = h / 1000.0;
        let depth = depth / 1000.0;
        let top = (top / 1000.0 - 0.1).max(0.0);
        let bottom = (bottom / 1000.0 - 0.1).max(0.0);
        let extent = h + depth - top - bottom;
        if extent <= 0.0 || g.height + g.depth == 0.0 {
            return;
        }
        let stretch = 1.5 * extent / (g.height + g.depth);
        let y = (stretch * (g.height - g.depth) - extent) / 2.0;
        svg.push_str(&format!("<svg width=\"{}\" height=\"{}\" y=\"{}\" x=\"{}\" viewBox=\"0 {} {} {}\"><path data-c=\"{c:X}\" d=\"M{}Z\" transform=\"scale(1,{})\"></path></svg>",root_number(g.width*1000.0),root_number(extent*1000.0),root_number((bottom-depth)*1000.0),root_number((d.length/1000.0-g.width)*500.0),root_number(y*1000.0),root_number(g.width*1000.0),root_number(extent*1000.0),g.path,number(stretch)));
    };
    if let Some((c, g)) = part(3) {
        let y = (g.depth - g.height) * 500.0 + 250.0;
        draw(svg, c, g, y);
        extender(svg, d.height, 0.0, top, g.height * 1000.0 + y);
        extender(svg, 0.0, d.depth, g.depth * 1000.0 - y, bottom);
    } else {
        extender(svg, d.height, d.depth, top, bottom);
    }
}

// Preserve source em-space bbox arithmetic before conversion to SVG units.
pub(super) fn fixed_fence_bounds(d: cd::Delimiter) -> Option<(f64, f64, f64, f64)> {
    if !d.fixed {
        return None;
    }
    let variant = d.font_variant?;
    recipe(d.code)?;
    let glyph = font_glyph(d.code, variant)?;
    let shift = (glyph.height + glyph.depth) / 2.0 + 0.25 - glyph.height;
    let h = glyph.height + shift;
    let depth = glyph.depth - shift;
    Some((h, depth, h * 1000.0, depth * 1000.0))
}

// Source CommonMtable percentage-width state. Natural bbox and late painted
// column widths are separate, so nested tables do not inflate their parent.
#[derive(Clone)]
pub(super) struct Percentage {
    natural: Vec<f64>,
    fitted: Vec<bool>,
    align: Vec<usize>,
    scale: f64,
    padding: f64,
    gaps: Vec<f64>,
    row_align: Vec<Vec<&'static str>>,
    ratio: f64,
    pub top: bool,
    pub wrap_width: Option<f64>,
    pub container_align: &'static str,
    resolved: bool,
    pub width_includes_label: bool,
}
pub(super) fn resolve(layout: &mut Layout) {
    for i in 0..layout.groups.len() {
        let mut delta = 0.0;
        let mut before = 0;
        {
            let g = &mut layout.groups[i];
            if let Some(state) = &mut g.percentage {
                if !state.resolved {
                    let decoration = 2.0 * state.padding + state.gaps.iter().sum::<f64>();
                    let natural = decoration + state.natural.iter().sum::<f64>();
                    let reference = if state.top {
                        80.0 * 0.442
                    } else {
                        state.wrap_width.unwrap_or(g.body.width) / (state.scale * 1000.0)
                    };
                    let target = (reference * state.ratio).max(natural);
                    let fitted = state.fitted.iter().filter(|v| **v).count();
                    let count = if fitted > 0 {
                        fitted
                    } else {
                        state.natural.len()
                    };
                    let extra = if count > 0 {
                        (target - natural).max(0.0) / count as f64
                    } else {
                        0.0
                    };
                    let computed = state
                        .natural
                        .iter()
                        .zip(&state.fitted)
                        .map(|(w, fit)| w + if *fit || fitted == 0 { extra } else { 0.0 })
                        .collect::<Vec<_>>();
                    for (row_index, row) in g.body.groups.iter_mut().enumerate() {
                        let mut old_cursor = 0.0;
                        let mut cursor = 0.0;
                        for (column, cell) in row.body.groups.iter_mut().enumerate() {
                            let old = state.natural[column];
                            let width = computed[column];
                            let alignment = state
                                .row_align
                                .get(row_index)
                                .and_then(|r| r.get(column))
                                .copied()
                                .unwrap_or(match state.align[column] {
                                    0 => "center",
                                    1 => "right",
                                    _ => "left",
                                });
                            let shift = match alignment {
                                "center" => (width - old) / 2.0,
                                "right" => width - old,
                                _ => 0.0,
                            };
                            cell.x += (cursor - old_cursor + shift) * state.scale * 1000.0;
                            cell.x_frame = None;
                            ams::set_wrap_width(&mut cell.body, width * state.scale * 1000.0);

                            ams::set_percentage_wrap(
                                &mut cell.body,
                                width * state.scale * 1000.0,
                                alignment,
                            );
                            cursor += width + state.gaps.get(column).copied().unwrap_or(0.0);
                            old_cursor += old + state.gaps.get(column).copied().unwrap_or(0.0);
                        }
                    }
                    if !state.top {
                        let shift = (target - natural) * state.scale * 1000.0;
                        g.x_frame = None;
                        g.x -= match state.container_align {
                            "center" => shift / 2.0,
                            "right" => shift,
                            _ => 0.0,
                        };
                    }
                    if state.top {
                        let width = target * state.scale * 1000.0;
                        delta = width - g.body.width;
                        g.body.width = width;
                        g.body.width_em = Some(target * state.scale);
                        before = g.before_glyph;
                    }
                    state.resolved = true;
                }
            }
            let transparent = matches!(g.kind, "" | "mrow" | "#inferred-place")
                || matches!(g.kind, "mstyle" | "TeXAtom")
                    && g.body.groups.len() == 1
                    && g.body.glyphs.is_empty()
                    && g.body.rules.is_empty();
            if matches!(g.kind, "mstyle" | "TeXAtom") && !transparent {
                let width = g.body.width;
                ams::set_percentage_wrap(&mut g.body, width, "left");
            }
            let old_width = g.body.width;
            resolve(&mut g.body);
            if transparent {
                delta += (g.body.width - old_width) * g.mathsize.unwrap_or(1.0);
                before = g.before_glyph;
            }
        }
        if delta != 0.0 {
            layout.width += delta;
            layout.width_em = layout.width_em.map(|w| w + delta / 1000.0);
            for g in &mut layout.groups[i + 1..] {
                g.x += delta;
                g.x_frame = None;
            }
            for glyph in &mut layout.glyphs[before..] {
                glyph.x += delta;
            }
        }
    }
}

#[derive(Default, Clone, Debug)]
pub(super) struct MultlineOptions {
    pub column_gap: Option<f64>,
    pub rules: array_rules::Rules,
    pub position: String,
    pub width: String,
    pub align: Vec<Vec<&'static str>>,
    pub row_spacing: Vec<f64>,
}
pub(super) fn parse_multline(
    content: &str,
    environment: &str,
    ams: ams::Shared,
    equation_seen: EquationStack,
    options: &mut MultlineOptions,
) -> Option<Vec<Vec<Atom>>> {
    let body = array_body::parse(content, environment, ams.clone(), equation_seen)?;
    options.rules = body.rules;
    options.row_spacing = body.spacing;
    options.align = body.align;
    let mut rows = body.rows;
    if rows.iter().flatten().any(|a| cd::first_error(a).is_some()) {
        return Some(rows);
    }
    if environment == "multlined" && rows.len() > 1 {
        let skip = |key| {
            let value = ams.borrow().option(key);
            if value.is_empty() {
                ams.borrow().option("multlinegap")
            } else {
                value
            }
        };
        let first = skip("firstline-afterskip");
        let last_skip = skip("lastline-preskip");
        if options.align[0][0] != "right" {
            let atom = rows[0].remove(0);
            rows[0].push(Atom::InferredRow(vec![
                atom,
                Atom::Kern(length::dimension(&first)),
            ]));
        }
        let last = rows.len() - 1;
        if options.align[last][0] != "left" {
            let atom = rows[last].remove(0);
            rows[last].push(Atom::InferredRow(vec![
                Atom::Kern(length::dimension(&last_skip)),
                atom,
            ]));
        }
    }
    if rows.is_empty() {
        return Some(rows);
    }
    if options.align[0][0] == "center" {
        options.align[0][0] = "left";
    }
    let last = options.align.len() - 1;
    if options.align[last][0] == "center" {
        options.align[last][0] = "right";
    }
    Some(rows)
}

// Array-body parsing keeps nested Begin/End owners on the parser stack;
// their Entry/CR tokens belong to the nested environment, not this table.
fn environment_boundary(chars: &[char], index: usize, nesting: &mut usize) {
    for (name, opening) in [("begin", true), ("end", false)] {
        let word: Vec<_> = name.chars().collect();
        if !chars[index + 1..].starts_with(&word) {
            continue;
        }
        let mut cursor = index + 1 + word.len();
        if chars.get(cursor).is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        while chars.get(cursor).is_some_and(|c| c.is_whitespace()) {
            cursor += 1;
        }
        if chars.get(cursor) == Some(&'{') {
            if opening {
                *nesting += 1;
            } else {
                *nesting = nesting.saturating_sub(1);
            }
        }
    }
}

// Source splitTable's raw cell inventory, before environment-specific parsing.
pub(super) fn raw_rows(content: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = content.chars().collect();
    let mut rows = vec![Vec::new()];
    let (mut start, mut i, mut depth, mut environments) = (0, 0, 0usize, 0usize);
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '\\' if chars.get(i + 1) == Some(&'\\') && depth == 0 && environments == 0 => {
                rows.last_mut()
                    .unwrap()
                    .push(chars[start..i].iter().collect());
                i += 2;
                if chars.get(i) == Some(&'[') {
                    if let Some(end) = chars[i..].iter().position(|c| *c == ']') {
                        i += end + 1;
                    }
                }
                rows.push(Vec::new());
                start = i;
                continue;
            }
            '\\' => {
                environment_boundary(&chars, i, &mut environments);
                i += 1;
            }
            '&' if depth == 0 && environments == 0 => {
                rows.last_mut()
                    .unwrap()
                    .push(chars[start..i].iter().collect());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    rows.last_mut()
        .unwrap()
        .push(chars[start..].iter().collect());
    rows
}

pub(super) fn base_spacing(environment: &str) -> f64 {
    if environment == "CD" {
        0.5
    } else if environment.contains("subarray") {
        0.1
    } else if environment.contains("smallmatrix") || environment.contains("cases") {
        0.2
    } else if environment.contains("align")
        || environment.starts_with("eqnarray")
        || environment == "split"
        || environment.contains("gather")
    {
        length::dimension("3pt").em(1., 1.)
    } else if environment.contains("multline") || environment == "displaylines" {
        0.5
    } else {
        0.4
    }
}

// Source MML Walk adds to every contained mtable after parsing the body. Keep
// each pass's clamping/order, including nested spreadlines, rather than summing.
// tex/spread_pop.go: unwrap the popped inferred row once. Authored
// fences, fractions, scripts and groups retain their own table spacing.
pub(super) fn spread(atom: &mut Atom, delta: f64) {
    match atom {
        Atom::Sequence(children) | Atom::InferredRow(children) | Atom::ExplicitRow(children) => {
            for child in children {
                spread_table(child, delta);
            }
        }
        _ => spread_table(atom, delta),
    }
}
fn spread_table(atom: &mut Atom, delta: f64) {
    match atom {
        Atom::Matrix { environment, .. } if fences(environment).is_some() => {}
        Atom::Matrix {
            options: Some(options),
            ..
        } if !options.row_spacing.is_empty() => {
            for value in &mut options.row_spacing {
                *value = alignment::row_spacing_em((*value + delta).max(0.));
            }
        }
        Atom::Matrix {
            environment,
            spacing,
            ..
        }
        | Atom::TaggedTable {
            environment,
            spacing,
            ..
        } => {
            *spacing = Some(alignment::row_spacing_em(
                (spacing.unwrap_or_else(|| base_spacing(environment)) + delta).max(0.0),
            ));
        }
        _ => {}
    }
}

// tex/inheritance.go mtable/mtr: columnalign reaches nested default tables
// through fraction/style/TeXAtom wrappers independently of container alignment.
pub(super) fn inherit_columns(atom: &mut Atom, inherited: char) {
    match atom {
        Atom::Matrix {
            rows,
            environment,
            column_spec,
            ..
        } => {
            if environment.contains("matrix") && column_spec.is_empty() {
                // Array's explicit center overrides an enclosing table's
                // inherited column alignment (tex/environments.go).
                *column_spec = "c".into();
            }
            let aligns: Vec<_> = if environment == "aligned" {
                vec![]
            } else {
                column_spec
                    .chars()
                    .filter(|c| matches!(c, 'l' | 'c' | 'r'))
                    .collect()
            };
            for row in rows {
                for (i, a) in row.iter_mut().enumerate() {
                    let align = aligns
                        .get(i)
                        .or_else(|| aligns.last())
                        .copied()
                        .unwrap_or_else(|| {
                            if environment.contains("cases") {
                                'l'
                            } else if environment.contains("align") || environment == "split" {
                                if i % 2 == 0 { 'r' } else { 'l' }
                            } else {
                                'c'
                            }
                        });
                    inherit_columns(a, align);
                }
            }
        }
        Atom::TaggedTable {
            rows, environment, ..
        } => {
            for row in rows {
                for (i, a) in row.iter_mut().enumerate() {
                    inherit_columns(
                        a,
                        if environment.contains("align") {
                            if i % 2 == 0 { 'r' } else { 'l' }
                        } else {
                            'c'
                        },
                    );
                }
            }
        }
        Atom::Sequence(v)
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v)
        | Atom::Fenced { body: v, .. } => {
            for a in v {
                inherit_columns(a, inherited)
            }
        }
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => {
            inherit_columns(a, inherited);
            inherit_columns(b, inherited)
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            for a in [base, sub, sup] {
                inherit_columns(a, inherited);
            }
            if let Some(v) = post {
                for a in v {
                    inherit_columns(a, inherited);
                }
            }
        }
        Atom::Scripts { base, sub, sup, .. } => {
            inherit_columns(base, inherited);
            for a in [sub, sup].into_iter().flatten() {
                inherit_columns(a, inherited)
            }
        }
        Atom::Choice(v) => {
            for a in v {
                inherit_columns(a, inherited)
            }
        }
        Atom::XArrow { over, under, .. } => {
            inherit_columns(over, inherited);
            if let Some(a) = under {
                inherit_columns(a, inherited)
            }
        }
        Atom::CdArrow { over, under, .. } => {
            for a in [over, under].into_iter().flatten() {
                inherit_columns(a, inherited)
            }
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Reference { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => inherit_columns(a, inherited),
        _ => {}
    }
}
