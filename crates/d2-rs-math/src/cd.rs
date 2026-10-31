// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Translated from pinned mathjax-go v0.1.0 tex/environments.go,
// tex/amscd_handlers.go and svg/{mtable,munderover,stretch}.go.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Delimiter {
    pub(super) code: u32,
    pub(super) length: f64,
    pub(super) height: f64,
    pub(super) depth: f64,
    pub(super) fixed: bool,
    pub(super) font_variant: Option<usize>,
}

fn empty() -> Atom {
    Atom::Sequence(Vec::new())
}
fn blank() -> Layout {
    Layout {
        bounds_em: None,
        width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    }
}
fn group(body: Layout, x: f64, baseline: f64, scale: f64, kind: &'static str) -> LayoutGroup {
    LayoutGroup {
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
        scale,
        x,
        baseline,
        color: None,
        background: None,
        kind,
        delimiter: None,
        arrow: None,
        body: Box::new(body),
        lines: Vec::new(),
    }
}

// Split only top-level row separators; CD labels may contain grouped TeX.
pub(super) fn parse_rows(content: &str, minimums: Minimums) -> Option<Vec<Vec<Atom>>> {
    let chars: Vec<char> = content.chars().collect();
    let mut rows = Vec::new();
    let (mut start, mut index, mut depth) = (0, 0, 0usize);
    while index < chars.len() {
        match chars[index] {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '\\' => {
                if depth == 0 && chars.get(index + 1) == Some(&'\\') {
                    rows.push(chars[start..index].iter().collect::<String>());
                    index += 2;
                    // Original splitTable consumes an immediate optional row
                    // annotation, but does not apply it to table spacing.
                    if chars.get(index) == Some(&'[')
                        && let Some(end) = chars[index..].iter().position(|c| *c == ']')
                    {
                        index += end + 1;
                    }
                    start = index;
                    continue;
                }
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    rows.push(chars[start..].iter().collect::<String>());
    rows.iter()
        .enumerate()
        .map(|(i, row)| parse_row(row, i % 2 == 0, minimums))
        .collect()
}

fn append_contents(cells: &mut Vec<Atom>, text: &str, object_row: bool) -> Option<()> {
    if text.trim().is_empty() {
        return Some(());
    }
    let mut contents = parse_unresolved(text.trim())?;
    if object_row && cells.is_empty() {
        contents = Atom::Sequence(vec![contents, Atom::Strut(850.0, 200.0, "mpadded")]);
    }
    if !object_row && matches!(cells.last(), Some(Atom::Sequence(v)) if v.is_empty()) {
        *cells.last_mut()? = contents;
    } else {
        cells.push(contents);
    }
    Some(())
}

fn parse_row(raw: &str, object_row: bool, minimums: Minimums) -> Option<Vec<Atom>> {
    let chars: Vec<char> = raw.chars().collect();
    let mut cells = Vec::new();
    let mut position = 0;
    while let Some(offset) = chars[position..].iter().position(|c| *c == '@') {
        let at = position + offset;
        append_contents(
            &mut cells,
            &chars[position..at].iter().collect::<String>(),
            object_row,
        )?;
        if cells.iter().any(|cell| first_error(cell).is_some()) {
            return Some(cells);
        }
        let Some(kind) = chars.get(at + 1).copied() else {
            return Some(vec![Atom::ErrorDisplay(
                "Missing CD arrow after @".to_owned(),
            )]);
        };
        position = at + 2;
        let arrow = match kind {
            '.' => empty(),
            '=' => Atom::CdArrow {
                code: 0x3D,
                over: None,
                under: None,
                minimums,
            },
            '|' => Atom::CdArrow {
                code: 0x2016,
                over: None,
                under: None,
                minimums,
            },
            '>' | '<' | 'V' | 'A' => {
                let Some((first, next)) = label(&chars, position, kind) else {
                    return Some(vec![Atom::ErrorDisplay(format!(
                        "Missing closing {kind} for CD arrow"
                    ))]);
                };
                let Some((second, next)) = label(&chars, next, kind) else {
                    return Some(vec![Atom::ErrorDisplay(format!(
                        "Missing closing {kind} for CD arrow"
                    ))]);
                };
                position = next;
                let horizontal = matches!(kind, '>' | '<');
                let over = if first.is_empty() {
                    if horizontal {
                        Some(Box::new(Atom::Kern(minimums.0)))
                    } else {
                        None
                    }
                } else {
                    Some(Box::new(parse_unresolved(&first)?))
                };
                let under = if second.is_empty() {
                    None
                } else {
                    Some(Box::new(parse_unresolved(&second)?))
                };
                Atom::CdArrow {
                    code: match kind {
                        '>' => 0x2192,
                        '<' => 0x2190,
                        'V' => 0x2193,
                        _ => 0x2191,
                    },
                    over,
                    under,
                    minimums,
                }
            }
            _ => {
                position = at + 1;
                Atom::Glyph('@' as u32)
            }
        };
        cells.push(arrow);
        if !object_row {
            cells.push(empty());
        }
    }
    append_contents(
        &mut cells,
        &chars[position..].iter().collect::<String>(),
        object_row,
    )?;
    if cells.is_empty() {
        cells.push(empty());
    }
    // amscdEnvironment commits a pending cell after a final vertical arrow.
    if matches!(
        cells.last(),
        Some(Atom::CdArrow {
            code: 0x2191 | 0x2193 | 0x2016,
            ..
        })
    ) {
        cells.push(empty());
    }
    Some(cells)
}

fn label(chars: &[char], start: usize, delimiter: char) -> Option<(String, usize)> {
    let (mut index, mut depth) = (start, 0usize);
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            c if c == delimiter && depth == 0 => {
                return Some((chars[start..index].iter().collect(), index + 1));
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn label_box(atom: &Atom, scale: f64, prime: bool) -> Option<Layout> {
    let mut body = blank();
    body.width = layout_at_with_style(atom, 0.0, 0.0, scale, &mut body, false, prime)?;
    retain_negative_glyph_depth(&mut body);
    Some(body)
}

pub(super) fn layout_arrow(
    code: u32,
    over: Option<&Atom>,
    under: Option<&Atom>,
    minimums: Minimums,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    layout_arrow_stretched(code, over, under, minimums, x, baseline, scale, out, None)
}

fn layout_arrow_stretched(
    code: u32,
    over: Option<&Atom>,
    under: Option<&Atom>,
    minimums: Minimums,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    target: Option<f64>,
) -> Option<f64> {
    if code == 0x3D && target.is_none() {
        // Original mtable caches the natural cell box before canStretch;
        // moToSVG then selects/assembles the minimum size without expanding
        // that cached table box. Keep measuring and emitted paint separate.
        let data = glyph(code)?;
        let width = data.width * 1000.0 * scale;
        let h = data.height * 1000.0 * scale;
        let d = data.depth * 1000.0 * scale;
        let mut arrow = group(blank(), x, baseline, scale, "mo");
        let requested = minimums.0.minimum().max(0.0) * 1000.0;
        arrow.delimiter = Some(Delimiter {
            font_variant: None,
            code,
            length: requested.max(data.width * 1000.0),
            height: data.height * 1000.0,
            depth: data.depth * 1000.0,
            fixed: requested <= data.width * 1000.0,
        });
        arrow.body.width = width;
        arrow.body.ascent = h;
        arrow.body.depth = d;
        out.groups.push(arrow);
        out.ascent = out.ascent.max(baseline + h);
        out.depth = out.depth.max(d - baseline);
        return Some(width);
    }
    let horizontal = matches!(code, 0x2190 | 0x2192 | 0x3D);
    let script = scale * std::f64::consts::FRAC_1_SQRT_2;
    let above = over
        .map(|a| label_box(a, script, false))
        .transpose_option()?;
    let below = under
        .map(|a| label_box(a, script, horizontal))
        .transpose_option()?;
    let labels_width = above
        .as_ref()
        .map_or(0.0, |a| a.width + 670.0 * script)
        .max(below.as_ref().map_or(0.0, |a| a.width + 670.0 * script));
    let requested = if horizontal {
        target
            .unwrap_or(labels_width)
            .max(minimums.0.minimum() * 1000.0 * scale)
    } else {
        0.0_f64.max(minimums.1.minimum() * 1000.0 * scale)
    };
    let minimum = if horizontal {
        minimums.0.minimum()
    } else {
        minimums.1.minimum()
    };
    let selection = if minimum == 0.0 {
        (requested * 0.901).max(requested - 300.0 * scale)
    } else {
        requested
    };
    let fixed = selection
        <= (if horizontal {
            if code == 0x3D { 778.0 } else { 1000.0 }
        } else if code == 0x2016 {
            1000.0
        } else {
            888.0
        }) * scale;
    let (base_width, h, d) = if fixed {
        let data = fixed_glyph(code, selection / scale);
        let total = (data.height + data.depth) * 1000.0 * scale;
        if horizontal {
            (
                data.width * 1000.0 * scale,
                data.height * 1000.0 * scale,
                data.depth * 1000.0 * scale,
            )
        } else {
            (
                data.width * 1000.0 * scale,
                total / 2.0 + 250.0 * scale,
                total / 2.0 - 250.0 * scale,
            )
        }
    } else if horizontal {
        (requested, 583.0 * scale, 82.0 * scale)
    } else {
        (
            if code == 0x2016 { 556.0 } else { 667.0 } * scale,
            requested / 2.0 + 250.0 * scale,
            requested / 2.0 - 250.0 * scale,
        )
    };
    let width = if horizontal {
        base_width.max(labels_width)
    } else {
        base_width
    };
    let mut body = blank();
    let mut arrow = group(
        blank(),
        if horizontal {
            (width - base_width) / 2.0
        } else {
            0.0
        },
        0.0,
        scale,
        "mo",
    );
    arrow.delimiter = Some(Delimiter {
        font_variant: None,
        code,
        length: if horizontal {
            base_width / scale
        } else {
            (h + d) / scale
        },
        height: h / scale,
        depth: d / scale,
        fixed,
    });
    body.groups.push(arrow);
    let (mut ascent, mut depth) = (h, d);
    // Original svg/munderover.go emits munderover's base, under, then over.
    let mut labels = [(over, above.as_ref(), true), (under, below.as_ref(), false)];
    if horizontal && under.is_some() {
        labels.swap(0, 1);
    }
    for (label, bounds, top) in labels {
        if let (Some(label), Some(bounds)) = (label, bounds) {
            let mut child = blank();
            let (offset_x, offset_y, child_x, child_y, kind) = if horizontal {
                let width_padded = bounds.width + 670.0 * script;
                let y = if top {
                    h + (111.0 * scale).max(200.0 * scale - bounds.depth.max(0.0)) + bounds.depth
                } else {
                    -(d + (167.0 * scale).max(600.0 * scale - bounds.ascent) + bounds.ascent)
                };
                (
                    (width - width_padded) / 2.0,
                    y,
                    330.0 * script,
                    if top { 100.0 * script } else { 0.0 },
                    "mpadded",
                )
            } else {
                (
                    if top { 0.0 } else { width },
                    0.0,
                    if top { -bounds.width } else { 0.0 },
                    0.0,
                    "mpadded",
                )
            };
            // Original svg/mpadded.go paddedToSVG: a nonzero lspace/voffset
            // installs a separate content g placed inside the mpadded node.
            if child_x != 0.0 || child_y != 0.0 {
                let mut inner = blank();
                inner.width = layout_at_with_style(
                    label,
                    0.0,
                    0.0,
                    script,
                    &mut inner,
                    false,
                    horizontal && !top,
                )?;
                child.width = inner.width;
                child.ascent = inner.ascent + child_y;
                child.depth = inner.depth - child_y;
                child.groups.push(group(inner, child_x, child_y, script, ""));
            } else {
                child.width = layout_at_with_style(
                    label,
                    child_x,
                    child_y,
                    script,
                    &mut child,
                    false,
                    horizontal && !top,
                )?;
            }
            if horizontal {
                body.groups
                    .push(group(child, offset_x, offset_y, script, kind));
            } else {
                // Original parseCDArrow: parseString("\\scriptstyle\\llap{..}")
                // (or \\rlap) yields mstyle > TeXAtom(ORD) > mpadded.
                let (width, ascent, depth) = (child.width, child.ascent, child.depth);
                let wrap = |inner: Layout, kind: &'static str| {
                    let mut outer = blank();
                    outer.width = width;
                    outer.ascent = ascent;
                    outer.depth = depth;
                    outer.groups.push(group(inner, 0.0, 0.0, script, kind));
                    outer
                };
                let atom = wrap(wrap(child, kind), "TeXAtom");
                body.groups
                    .push(group(atom, offset_x, offset_y, script, "mstyle"));
            }
            // mpadded voffset affects placement, not its implicit H/D.
            ascent = ascent.max(offset_y + bounds.ascent);
            depth = depth.max(bounds.depth - offset_y);
            if horizontal {
                if top {
                    ascent += 100.0 * scale
                } else {
                    depth += 100.0 * scale
                }
            }
        }
    }
    // Source parseCDArrow returns the bare ORD arrow for an unlabeled
    // vertical arrow, without a forced row.
    if !horizontal && above.is_none() && below.is_none() {
        let mut arrow = body.groups.pop()?;
        arrow.x += x;
        arrow.baseline += baseline;
        arrow.before_glyph = out.glyphs.len();
        out.ascent = out.ascent.max(baseline + ascent);
        out.depth = out.depth.max(depth - baseline);
        out.groups.push(arrow);
        return Some(width);
    }
    // Source parseCDArrow returns left label, arrow, right label for vertical
    // arrows. Horizontal mover/munderover keeps its base before labels.
    if !horizontal && above.is_some() {
        body.groups.swap(0, 1);
    }
    body.width = width;
    body.ascent = ascent;
    body.depth = depth;
    out.ascent = out.ascent.max(baseline + ascent);
    out.depth = out.depth.max(depth - baseline);
    out.groups.push(group(
        body,
        x,
        baseline,
        scale,
        if code == 0x3D {
            "mrow"
        } else if horizontal {
            if under.is_some() {
                "munderover"
            } else {
                "mover"
            }
        } else {
            "mrow"
        },
    ));
    Some(width)
}

trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(v) => v.map(Some),
        }
    }
}

pub(super) fn layout_table(
    rows: &[Vec<Atom>],
    spacing: Option<f64>,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    let columns = rows.iter().map(Vec::len).max()?;
    let mut widths = vec![0.0_f64; columns];
    let mut hd = Vec::new();
    let mut measured = Vec::new();
    for row in rows {
        let (mut h, mut d) = (750.0 * scale, 250.0 * scale);
        let mut cells = Vec::new();
        for (column, cell) in row.iter().enumerate() {
            let mut body = blank();
            body.width = layout_at(cell, 0.0, 0.0, scale, &mut body)?;
            widths[column] = widths[column].max(body.width);
            h = h.max(body.ascent);
            d = d.max(body.depth);
            cells.push(body);
        }
        hd.push((h, d));
        measured.push(cells);
    }
    if rows.len() > 1 {
        for column in 0..columns {
            let is_horizontal = |a: &Atom| {
                matches!(
                    a,
                    Atom::CdArrow {
                        code: 0x2190 | 0x2192 | 0x3D,
                        ..
                    }
                )
            };
            let stretch_count = rows
                .iter()
                .filter(|r| r.get(column).is_some_and(is_horizontal))
                .count();
            let all = stretch_count > 1 && stretch_count == rows.len();
            let target = rows
                .iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    r.get(column).and_then(|a| {
                        (all || !is_horizontal(a)).then_some(measured[i][column].width)
                    })
                })
                .fold(0.0_f64, f64::max);
            for (i, row) in rows.iter().enumerate() {
                if let Some(Atom::CdArrow {
                    code: 0x2190 | 0x2192 | 0x3D,
                    over,
                    under,
                    minimums,
                }) = row.get(column)
                {
                    let Atom::CdArrow { code, .. } = &row[column] else {
                        unreachable!()
                    };
                    let mut body = blank();
                    body.width = layout_arrow_stretched(
                        *code,
                        over.as_deref(),
                        under.as_deref(),
                        *minimums,
                        0.0,
                        0.0,
                        scale,
                        &mut body,
                        Some(target),
                    )?;
                    measured[i][column] = body;
                }
            }
        }
    }
    let gap = 500.0 * scale;
    let row_gap = spacing.unwrap_or(0.5) * 1000.0 * scale;
    let width = widths.iter().sum::<f64>() + gap * columns.saturating_sub(1) as f64;
    let height =
        hd.iter().map(|(h, d)| h + d).sum::<f64>() + row_gap * rows.len().saturating_sub(1) as f64;
    let ascent = height / 2.0 + 250.0 * scale;
    let depth = height / 2.0 - 250.0 * scale;
    let mut table = blank();
    let mut y = ascent;
    for (i, cells) in measured.into_iter().enumerate() {
        let mut row = blank();
        y -= hd[i].0;
        let mut cursor = 0.0;
        for (column, cell) in cells.into_iter().enumerate() {
            let dx = cursor + (widths[column] - cell.width) / 2.0;
            row.groups.push(group(cell, dx, 0.0, scale, "mtd"));
            cursor += widths[column] + gap;
        }
        table.groups.push(group(row, 0.0, y, scale, "mtr"));
        y -= hd[i].1 + row_gap;
    }
    table.width = width;
    table.ascent = ascent;
    table.depth = depth;
    out.groups.push(group(table, x, baseline, scale, "mtable"));
    out.ascent = out.ascent.max(baseline + ascent);
    out.depth = out.depth.max(depth - baseline);
    Some(width)
}

fn path(svg: &mut String, code: u32, x: f64, y: f64) {
    let data = glyph(code).expect("source CD assembly glyph");
    let (x, y) = (root_number(x), root_number(y));
    let placement = if x == "0" && y == "0" {
        String::new()
    } else {
        format!(" transform=\"translate({x},{y})\"")
    };
    svg.push_str(&format!(
        "<path data-c=\"{code:X}\" d=\"M{}Z\"{placement}></path>",
        data.path
    ));
}

pub(super) fn render_delimiter(svg: &mut String, delimiter: Delimiter) {
    if delimiter.font_variant.is_some() {
        table::render_fence(svg, delimiter);
        return;
    }
    let Delimiter {
        code,
        length,
        height,
        depth,
        fixed,
        font_variant: _,
    } = delimiter;
    if fixed {
        let data = fixed_glyph(
            code,
            if xarrow::recipe(code).is_some() {
                length
            } else {
                height + depth
            },
        );
        let paint_code = if code == 0x2016 && height + depth > 602.0 {
            0x2225
        } else {
            code
        };
        let y = if xarrow::recipe(code).is_some() {
            0.0
        } else {
            height - data.height * 1000.0
        };
        let y = root_number(y);
        let placement = if y == "0" {
            String::new()
        } else {
            format!(" transform=\"translate(0,{y})\"")
        };
        svg.push_str(&format!(
            "<path data-c=\"{paint_code:X}\" d=\"M{}Z\"{placement}></path>",
            data.path
        ));
        return;
    }
    if matches!(code, 0x23de | 0x23df) {
        render_horizontal_brace(svg, code, length / 1000.);
        return;
    }
    if let Some([left_code, stem_code, right_code]) = xarrow::recipe(code) {
        let width = length / 1000.0;
        let mut left = 0.0;
        let mut right = 0.0;
        if left_code != 0 {
            let end = glyph(left_code).expect("horizontal left delimiter part");
            path(svg, left_code, 0., 0.);
            left = end.width;
        }
        if right_code != 0 {
            let end = glyph(right_code).expect("horizontal right delimiter part");
            path(svg, right_code, (width - end.width) * 1000., 0.);
            right = end.width;
        }
        let left = (left - 0.1).max(0.);
        let right = (right - 0.1).max(0.);
        let extent = width - left - right;
        let stem = glyph(stem_code).expect("CD stem");
        if extent <= 0. || stem.width == 0. {
            return;
        }
        let stretch = 1.5 * extent / stem.width;
        let bottom = -(stem.depth + 0.1);
        let h = stem.height + stem.depth + 0.2;
        svg.push_str(&format!("<svg width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" viewBox=\"{} {} {} {}\"><path data-c=\"{stem_code:X}\" d=\"M{}Z\" transform=\"scale({},1)\"></path></svg>",root_number(extent*1000.),root_number(h*1000.),root_number(left*1000.),root_number(bottom*1000.),root_number(((stretch*stem.width-extent)/2.)*1000.),root_number(bottom*1000.),root_number(extent*1000.),root_number(h*1000.),stem.path,number(stretch)));
    } else {
        let width = if code == 0x2016 { 556.0 } else { 667.0 };
        let (mut top, mut bottom) = (0.0_f64, 0.0_f64);
        if code == 0x2191 {
            let end = glyph(code).unwrap();
            path(
                svg,
                code,
                (width - end.width * 1000.0) / 2.0,
                height - end.height * 1000.0,
            );
            top = (end.height + end.depth) * 1000.0;
        }
        if code == 0x2193 {
            let end = glyph(code).unwrap();
            path(
                svg,
                code,
                (width - end.width * 1000.0) / 2.0,
                end.depth * 1000.0 - depth,
            );
            bottom = (end.height + end.depth) * 1000.0;
        }
        top = (top - 100.0).max(0.0);
        bottom = (bottom - 100.0).max(0.0);
        let extent = height + depth - top - bottom;
        // Original normal-font U+23D0 is inherited by Size4; double-bar uses U+2225.
        let (stem_code, stem_h, stem_d, stem_w, stem_path) = if code == 0x2016 {
            let stem = glyph(0x2225).unwrap();
            (
                0x2225,
                stem.height * 1000.0,
                stem.depth * 1000.0,
                stem.width * 1000.0,
                stem.path,
            )
        } else {
            (0x23D0, 602.0, 0.0, 667.0, "312 0V602H355V0H312")
        };
        // Original svg/stretch.go addExtenderVertical computes in em and
        // serializes with fixed(v*1000); keep that order for exact ties.
        let (extent_em, stem_h_em, stem_d_em) = (extent / 1000.0, stem_h / 1000.0, stem_d / 1000.0);
        let stretch = 1.5 * extent_em / (stem_h_em + stem_d_em);
        let y = (stretch * (stem_h_em - stem_d_em) - extent_em) / 2.0 * 1000.0;
        svg.push_str(&format!("<svg width=\"{}\" height=\"{}\" y=\"{}\" x=\"{}\" viewBox=\"0 {} {} {}\"><path data-c=\"{stem_code:X}\" d=\"M{}Z\" transform=\"scale(1,{})\"></path></svg>",root_number(stem_w),root_number(extent),root_number(bottom-depth),root_number((width-stem_w)/2.0),root_number(y),root_number(stem_w),root_number(extent),stem_path,number(stretch)));
    }
}

fn fixed_glyph(code: u32, dimension: f64) -> GlyphData {
    if code == 0x2016 {
        if dimension <= 602.0 {
            return GlyphData {
                height: 0.602,
                depth: 0.0,
                width: 0.778,
                path: "257 0V602H300V0H257ZM478 0V602H521V0H478",
            };
        }
        return glyph(0x2225).unwrap();
    }
    glyph(code).unwrap()
}

pub(super) fn first_error(atom: &Atom) -> Option<&str> {
    match atom {
        Atom::ErrorDisplay(message) => Some(message),
        Atom::XArrow { over, under, .. } => {
            first_error(over).or_else(|| under.as_deref().and_then(first_error))
        }
        Atom::Choice(v) => v.iter().find_map(|a| first_error(a)),
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => v.iter().find_map(first_error),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root { body: a, index: b, .. } => {
            first_error(a).or_else(|| first_error(b))
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Operator(a)
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Class { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Ord(a)
        | Atom::Phantom(a)
        | Atom::Style(a) | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => first_error(a),
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            rows.iter().flatten().find_map(first_error)
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => first_error(base)
            .or_else(|| first_error(sub))
            .or_else(|| first_error(sup))
            .or_else(|| {
                post.as_ref()
                    .and_then(|v| v.iter().find_map(|a| first_error(a)))
            }),
        Atom::Scripts { base, sub, sup, .. } => first_error(base)
            .or_else(|| sub.as_deref().and_then(first_error))
            .or_else(|| sup.as_deref().and_then(first_error)),
        Atom::CdArrow { over, under, .. } => over
            .as_deref()
            .and_then(first_error)
            .or_else(|| under.as_deref().and_then(first_error)),
        _ => None,
    }
}

// A nil child is not a parser diagnostic. A later complete parse error
// discards the tree before SVG construction and therefore takes precedence.
pub(super) fn first_nil_child(atom: &Atom) -> Option<&str> {
    match atom {
        Atom::SourceNilChild => Some("mathjax-go/internal/svg/renderer.wrap:61"),
        Atom::XArrow { over, under, .. } => {
            first_nil_child(over).or_else(|| under.as_deref().and_then(first_nil_child))
        }
        Atom::Choice(v) => v.iter().find_map(|a| first_nil_child(a)),
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => v.iter().find_map(first_nil_child),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root { body: a, index: b, .. } => {
            first_nil_child(a).or_else(|| first_nil_child(b))
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Operator(a)
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Class { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Ord(a)
        | Atom::Phantom(a)
        | Atom::Style(a) | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => first_nil_child(a),
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            rows.iter().flatten().find_map(first_nil_child)
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => first_nil_child(base)
            .or_else(|| first_nil_child(sub))
            .or_else(|| first_nil_child(sup))
            .or_else(|| {
                post.as_ref()
                    .and_then(|v| v.iter().find_map(|a| first_nil_child(a)))
            }),
        Atom::Scripts { base, sub, sup, .. } => first_nil_child(base)
            .or_else(|| sub.as_deref().and_then(first_nil_child))
            .or_else(|| sup.as_deref().and_then(first_nil_child)),
        Atom::CdArrow { over, under, .. } => over
            .as_deref()
            .and_then(first_nil_child)
            .or_else(|| under.as_deref().and_then(first_nil_child)),
        _ => None,
    }
}

pub(super) fn render_error(message: &str) -> String {
    // Original merror explicit-font text uses unknownText's 0.6em advance,
    // 0.75em ascent and 0.2em depth, rather than native font glyph metrics.
    let width = message.encode_utf16().count() as f64 * 600.0;
    let bounds = Layout {
        width,
        ascent: 750.0,
        depth: 200.0,
        ..blank()
    };
    let mut svg = render_layout(&bounds);
    let marker = "<g data-mml-node=\"math\">";
    let at = svg.find(marker).unwrap() + marker.len();
    svg.insert_str(at,&format!("<g data-mml-node=\"merror\" data-mjx-error=\"{}\"><rect data-background=\"true\" width=\"{}\" height=\"950\" y=\"-200\"></rect><g data-mml-node=\"mtext\" style=\"font-family: serif;\"><text data-variant=\"-explicitFont\" transform=\"scale(1,-1)\" font-size=\"884px\">{}</text></g></g>",escape(message),root_number(width),escape_text(message)));
    svg
}

// Source stretchHorizontal/addMiddleHorizontal and placeChar composite content.
fn render_horizontal_brace(svg: &mut String, code: u32, width: f64) {
    let (left_code, right_code, middle_code, middle_parts) = if code == 0x23de {
        (0xe150, 0xe151, 0xe155, [0xe153, 0xe152])
    } else {
        (0xe152, 0xe153, 0xe156, [0xe151, 0xe150])
    };
    let part = |c| table::font_glyph(c, 4).expect("source horizontal brace part");
    // placeChar + wrapper.place: the transform follows d, and is omitted
    // when the placement serializes as (0,0).
    let draw = |svg: &mut String, c: u32, x: f64| {
        let g = part(c);
        let x = root_number(x * 1000.);
        let place = if x == "0" {
            String::new()
        } else {
            format!(" transform=\"translate({x},0)\"")
        };
        svg.push_str(&format!("<path data-c=\"{c:X}\" d=\"M{}Z\"{place}></path>", g.path));
    };
    let left = part(left_code).width;
    let right = part(right_code).width;
    draw(svg, left_code, 0.);
    draw(svg, right_code, width - right);
    let middle_width = 0.9;
    let middle_left = (width - middle_width) / 2.;
    let middle_right = middle_left + middle_width;
    svg.push_str(&format!(
        "<g data-c=\"{middle_code:X}\" transform=\"translate({},0)\">",
        root_number(middle_left * 1000.)
    ));
    draw(svg, middle_parts[0], 0.);
    draw(svg, middle_parts[1], part(middle_parts[0]).width);
    svg.push_str("</g>");
    let stem = part(0xe154);
    for (section_width, left, right, x) in [
        (width / 2., left, width / 2. - middle_left, 0.),
        (width / 2., middle_right - width / 2., right, width / 2.),
    ] {
        let left = (left - 0.1).max(0.);
        let right = (right - 0.1).max(0.);
        let extent = section_width - left - right;
        if extent <= 0. || stem.width == 0. {
            continue;
        }
        let stretch = 1.5 * extent / stem.width;
        let bottom = -(stem.depth + 0.1);
        let h = stem.height + stem.depth + 0.2;
        svg.push_str(&format!("<svg width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" viewBox=\"{} {} {} {}\"><path data-c=\"E154\" d=\"M{}Z\" transform=\"scale({},1)\"></path></svg>",root_number(extent*1000.),root_number(h*1000.),root_number((x+left)*1000.),root_number(bottom*1000.),root_number((stretch*stem.width-extent)/2.*1000.),root_number(bottom*1000.),root_number(extent*1000.),root_number(h*1000.),stem.path,number(stretch)));
    }
}
