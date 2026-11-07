// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 tex/{environments,amscd_handlers}.go;
// svg/{mtable,munderover,stretch}.go.
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
        intrinsic_width_em: None,
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

// Pinned environments.go parseCD{,Row,Segment}: @ boundaries are raw, but
// Entry/CR are parser events. Argument readers own their tokens before Array.
pub(super) fn parse_rows(
    content: &str,
    ams: ams::Shared,
    equation_seen: EquationStack,
) -> Option<array_body::Body> {
    let rules = array_rules::Shared::default();
    let mut source = content.to_owned();
    let mut rows = Vec::new();
    let mut align = Vec::new();
    let mut adjustments = Vec::new();
    loop {
        let object = rows.len() % 2 == 0;
        let mut cells = Vec::new();
        let mut entry_align = Vec::new();
        let mut stop = None;
        loop {
            let chars: Vec<char> = source.chars().collect();
            let at = chars
                .windows(2)
                .position(|v| v[0] == '@' && "><VA.|=".contains(v[1]));
            let end = at.unwrap_or(chars.len());
            let mut parser = Parser {
                in_root: false,
                root_moves: Default::default(),
                physics_after: None,
                physics_auto: None,
                command_position: None,
                pending_cell: None,
                pending_infix: None,
                row_delimiter: None,
                active_color: None,
                equation_seen: equation_seen.clone(),
                array_cell: Some(array_body::CellState {
                    environment: "CD".into(),
                    fills: Default::default(),
                    rules: Some(rules.clone()),
                    rule_allowed: true,
                    equation: None,
                    at_start: true,
                    shove_allowed: false,
                    stop: None,
                    shove: None,
                    cd_end: at.map(|_| object && cells.is_empty()),
                }),
                vector_alias: false,
                vector_font: None,
                ams: ams.clone(),
                splice_inferred: false,
                chars: chars[..end].to_vec(),
                index: 0,
                active_font: None,
                normal_font: false,
                calligraphic_font: false,
                multi_letter_font: false,
                operator_letters: false,
                operator_pattern: false,
                no_auto_operator: false,
            };
            let contents = parser.sequence(None)?;
            if first_error(&contents).is_some() {
                return Some(array_body::Body {
                    rows: vec![vec![contents]],
                    align: Vec::new(),
                    spacing: Vec::new(),
                    rules: Default::default(),
                });
            }
            let state = parser.array_cell.take()?;
            let mut fills = Some(state.fills);
            let content_align = fills
                .as_ref()?
                .alignment(hfill::size(&contents))
                .unwrap_or("");
            if let Some(item) = state.stop {
                entry_align.push(content_align);
                cells.push(contents);
                source = parser.chars[parser.index..]
                    .iter()
                    .chain(chars[end..].iter())
                    .collect();
                if item.cr {
                    stop = Some(item);
                    break;
                }
                continue;
            }
            if at.is_none() {
                if !cells.is_empty() || !empty_contents(&contents) {
                    entry_align.push(content_align);
                    cells.push(contents);
                }
                break;
            }
            if !empty_contents(&contents) {
                entry_align.push(content_align);
                cells.push(contents);
                fills = None;
            }
            if (cells.len() % 2 == 0) == object {
                entry_align.push(fills.as_ref().and_then(|f| f.alignment(0)).unwrap_or(""));
                cells.push(empty());
                fills = None;
            }
            let minimums = minimums(&ams);
            let (arrow, next) = parse_arrow(&chars, end, minimums, ams.clone())?;
            if first_error(&arrow).is_some() {
                return Some(array_body::Body {
                    rows: vec![vec![arrow]],
                    align: Vec::new(),
                    spacing: Vec::new(),
                    rules: Default::default(),
                });
            }
            entry_align.push(
                fills
                    .as_ref()
                    .and_then(|f| f.alignment(usize::from(!empty_contents(&arrow))))
                    .unwrap_or(""),
            );
            cells.push(arrow);
            source = chars[next..].iter().collect();
        }
        if !cells.is_empty() {
            align.push(entry_align);
            rows.push(cells);
            rules.borrow_mut().rows = rows.len();
        }
        if let Some(item) = stop {
            if let Some(value) = item.spacing {
                adjustments.push((
                    rows.len().saturating_sub(1),
                    length::dimension(&value).em(0., 1.),
                ));
            }
        } else {
            break;
        }
    }
    // amscdEnvironment walks the completed table and fills a final vertical
    // arrow cell, including arrows nested inside ordinary authored content.
    let minimums = minimums(&ams);
    for row in &mut rows {
        let mut tail_vertical = false;
        let last = row.len().saturating_sub(1);
        for (i, cell) in row.iter_mut().enumerate() {
            let vertical = finish_arrows(cell, minimums, true);
            if i == last {
                tail_vertical = vertical;
            }
        }
        if tail_vertical {
            row.push(empty());
        }
    }
    let mut spacing = Vec::new();
    if !adjustments.is_empty() {
        spacing = vec![alignment::row_spacing_em(0.5); rows.len()];
        for (row, value) in adjustments {
            spacing[row] = alignment::row_spacing_em((0.5 + value).max(0.));
        }
    }
    Some(array_body::Body {
        rows,
        align,
        spacing,
        rules: rules.borrow().clone(),
    })
}

fn empty_contents(atom: &Atom) -> bool {
    matches!(atom, Atom::Sequence(v) if v.is_empty())
}
fn minimums(ams: &ams::Shared) -> Minimums {
    ams.borrow()
        .cd_minimums
        .unwrap_or((Dimension::relative(2.75), Dimension::relative(1.75)))
}

// amscdEnvironment resets every matching mo's attributes after parsing the
// whole table. Settings in a nested argument therefore affect earlier arrows.
// The returned flag implements amscdCellHasVerticalArrow's subtree walk.
fn finish_arrows(atom: &mut Atom, minimums: Minimums, stretch_horizontal: bool) -> bool {
    let replacement = match atom {
        Atom::Glyph(code) | Atom::RawOperator { code, .. }
            if matches!(*code, 0x2190 | 0x2192 | 0x3d | 0x2191 | 0x2193 | 0x2016) =>
        {
            Some(*code)
        }
        _ => None,
    };
    if let Some(code) =
        replacement.filter(|code| stretch_horizontal || matches!(*code, 0x2191 | 0x2193 | 0x2016))
    {
        let texclass = if matches!(code, 0x2191 | 0x2193 | 0x2016) {
            Some(0)
        } else {
            atom_tex_class(atom)
        };
        *atom = Atom::CdArrow {
            code,
            texclass,
            over: None,
            under: None,
            minimums,
        };
    }
    match atom {
        Atom::CdArrow {
            code,
            over,
            under,
            minimums: sizes,
            ..
        } => {
            *sizes = minimums;
            let nested = over
                .iter_mut()
                .chain(under.iter_mut())
                .fold(false, |found, child| {
                    finish_arrows(child, minimums, false) | found
                });
            matches!(*code, 0x2191 | 0x2193 | 0x2016) || nested
        }
        Atom::XArrow { over, under, .. } => {
            finish_arrows(over, minimums, false)
                | under
                    .as_mut()
                    .is_some_and(|a| finish_arrows(a, minimums, false))
        }
        Atom::Sequence(v) | Atom::InferredRow(v) | Atom::ExplicitRow(v) => {
            let core = stretch_horizontal
                && v.iter()
                    .filter(|a| !operator_dictionary::flag_spacelike(a))
                    .count()
                    == 1;
            v.iter_mut()
                .fold(false, |found, a| finish_arrows(a, minimums, core) | found)
        }
        Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v)
        | Atom::StyledText(v)
        | Atom::Fenced { body: v, .. } => v
            .iter_mut()
            .fold(false, |found, a| finish_arrows(a, minimums, false) | found),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => finish_arrows(a, minimums, false) | finish_arrows(b, minimums, false),
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
        | Atom::Class { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. }
        | Atom::Reference { body: a, .. } => finish_arrows(a, minimums, stretch_horizontal),
        Atom::Choice(v) => v
            .iter_mut()
            .fold(false, |found, a| finish_arrows(a, minimums, false) | found),
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => rows
            .iter_mut()
            .flatten()
            .fold(false, |found, a| finish_arrows(a, minimums, false) | found),
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            let found = finish_arrows(base, minimums, stretch_horizontal)
                | finish_arrows(sub, minimums, false)
                | finish_arrows(sup, minimums, false);
            post.iter_mut()
                .flatten()
                .fold(found, |found, a| finish_arrows(a, minimums, false) | found)
        }
        Atom::Scripts { base, sub, sup, .. } => {
            let found = finish_arrows(base, minimums, stretch_horizontal);
            sub.iter_mut()
                .chain(sup.iter_mut())
                .fold(found, |found, a| finish_arrows(a, minimums, false) | found)
        }
        _ => false,
    }
}

fn parse_arrow(
    chars: &[char],
    at: usize,
    minimums: Minimums,
    ams: ams::Shared,
) -> Option<(Atom, usize)> {
    let kind = chars[at + 1];
    let position = at + 2;
    if kind == '.' {
        return Some((empty(), position));
    }
    let code = match kind {
        '=' => 0x3d,
        '|' => 0x2016,
        '>' => 0x2192,
        '<' => 0x2190,
        'V' => 0x2193,
        _ => 0x2191,
    };
    if matches!(kind, '=' | '|') {
        return Some((
            Atom::CdArrow {
                code,
                over: None,
                under: None,
                minimums,
                texclass: None,
            },
            position,
        ));
    }
    let Some((first, next)) = label(chars, position, kind) else {
        return Some((
            Atom::ErrorDisplay(format!("Missing closing {kind} for CD arrow")),
            chars.len(),
        ));
    };
    let Some((second, next)) = label(chars, next, kind) else {
        return Some((
            Atom::ErrorDisplay(format!("Missing closing {kind} for CD arrow")),
            chars.len(),
        ));
    };
    let over = if first.is_empty() {
        matches!(kind, '>' | '<').then(|| Box::new(Atom::Kern(minimums.0)))
    } else {
        Some(Box::new(parse_unresolved_shared(&first, ams.clone())?))
    };
    let under = if second.is_empty() {
        None
    } else {
        Some(Box::new(parse_unresolved_shared(&second, ams)?))
    };
    Some((
        Atom::CdArrow {
            code,
            over,
            under,
            minimums,
            texclass: None,
        },
        next,
    ))
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
    let horizontal = matches!(code, 0x2190 | 0x2192 | 0x3D);
    let script = scale * styles::SCRIPT_MULTIPLIER;
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
                child
                    .groups
                    .push(group(inner, child_x, child_y, script, ""));
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
    if above.is_none() && below.is_none() {
        let mut arrow = body.groups.pop()?;
        arrow.x += x;
        arrow.x_frame = None;
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
    options: Option<&table::MultlineOptions>,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
) -> Option<f64> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
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
                    ..
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
    let rules = options
        .map(|o| &o.rules)
        .cloned()
        .unwrap_or_default()
        .finish(rows.len(), "");
    let row_lines: Vec<_> = (0..rows.len().saturating_sub(1))
        .map(|i| {
            rules
                .lines
                .get(i)
                .or_else(|| rules.lines.last())
                .copied()
                .filter(|s| *s != "none")
        })
        .collect();
    let unit = 1000.0 * scale;
    let frame_line = if rules.full { 0.07 * unit } else { 0.0 };
    let frame_x = if rules.frame.is_empty() {
        0.0
    } else {
        0.4 * unit
    };
    let frame_y = if rules.frame.is_empty() {
        0.0
    } else {
        length::dimension("0.5ex").em(0.0, scale) * unit
    };
    let gap = 500.0 * scale;
    let row_gaps: Vec<f64> = (0..rows.len().saturating_sub(1))
        .map(|i| {
            options
                .and_then(|o| o.row_spacing.get(i))
                .copied()
                .unwrap_or(spacing.unwrap_or(0.5))
                * 1000.
                * scale
        })
        .collect();
    let width = widths.iter().sum::<f64>()
        + gap * columns.saturating_sub(1) as f64
        + 2.0 * (frame_line + frame_x);
    let height = hd.iter().map(|(h, d)| h + d).sum::<f64>()
        + row_gaps.iter().sum::<f64>()
        + 2.0 * (frame_line + frame_y)
        + row_lines.iter().filter(|s| s.is_some()).count() as f64 * 0.07 * unit;
    let ascent = height / 2.0 + 250.0 * scale;
    let depth = height / 2.0 - 250.0 * scale;
    let mut table = blank();
    let mut y = ascent - frame_line - frame_y;
    for (i, cells) in measured.into_iter().enumerate() {
        let mut row = blank();
        y -= hd[i].0;
        let mut cursor = frame_line + frame_x;
        for (column, cell) in cells.into_iter().enumerate() {
            let dx = cursor
                + match options
                    .and_then(|o| o.align.get(i))
                    .and_then(|r| r.get(column))
                    .copied()
                    .unwrap_or("")
                {
                    "left" => 0.,
                    "right" => widths[column] - cell.width,
                    _ => (widths[column] - cell.width) / 2.,
                };
            row.groups.push(group(cell, dx, 0.0, scale, "mtd"));
            cursor += widths[column] + gap;
        }
        table.groups.push(group(row, 0.0, y, scale, "mtr"));
        y -= hd[i].1
            + row_gaps.get(i).copied().unwrap_or(0.)
            + if row_lines.get(i).is_some_and(Option::is_some) {
                0.07 * unit
            } else {
                0.0
            };
    }
    let mut line_y = ascent - frame_line - frame_y;
    for (i, style) in row_lines.iter().enumerate() {
        line_y -= hd[i].0
            + hd[i].1
            + row_gaps[i] / 2.0
            + if i == 0 { 0.0 } else { row_gaps[i - 1] / 2.0 };
        if let Some(style) = style {
            let mut line = group(blank(), 0.0, 0.0, scale, "#table-line");
            line.table_line = Some(TableLine {
                x: 0.0,
                x2: Some(width / scale),
                frame: false,
                y1: line_y / scale - 35.0,
                y2: line_y / scale - 35.0,
                dashed: *style == "dashed",
            });
            table.groups.push(line);
            line_y -= 0.07 * unit;
        }
    }
    if rules.full {
        let mut line = group(blank(), 0.0, 0.0, scale, "#table-line");
        line.table_line = Some(TableLine {
            x: 35.0,
            x2: Some(width / scale - 35.0),
            frame: true,
            y1: 35.0 - depth / scale,
            y2: ascent / scale - 35.0,
            dashed: rules.dashed,
        });
        table.groups.push(line);
    }
    table.width = width;
    table.ascent = ascent;
    table.depth = depth;
    let (mut table, width, ascent, depth, kind) = if rules.frame.is_empty() {
        (table, width, ascent, depth, "mtable")
    } else {
        let (table, w, h, d, kind) = array_rules::enclose(
            table,
            width / unit,
            ascent / unit,
            depth / unit,
            scale,
            &rules,
        );
        (table, w * unit, h * unit, d * unit, kind)
    };
    table.width = width;
    table.ascent = ascent;
    table.depth = depth;
    out.groups.push(group(table, x, baseline, scale, kind));
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
        render_horizontal_extender(
            svg,
            stem_code,
            glyph(stem_code).expect("horizontal extender part"),
            width,
            left,
            right,
            0.,
        );
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
    match first_error_node(atom)? {
        Atom::ErrorDisplay(message) | Atom::SourceError(message) => Some(message),
        _ => unreachable!(),
    }
}

pub(super) fn first_error_node(atom: &Atom) -> Option<&Atom> {
    match atom {
        Atom::ErrorDisplay(_) | Atom::SourceError(_) => Some(atom),
        Atom::XArrow { over, under, .. } => {
            first_error_node(over).or_else(|| under.as_deref().and_then(first_error_node))
        }
        Atom::Choice(v) => v.iter().find_map(|a| first_error_node(a)),
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => v.iter().find_map(first_error_node),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => first_error_node(a).or_else(|| first_error_node(b)),
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
        | Atom::Class { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => first_error_node(a),
        Atom::Matrix { rows, .. } => rows.iter().flatten().find_map(first_error_node),
        Atom::TaggedTable { rows, tags, .. } => rows
            .iter()
            .flatten()
            .find_map(first_error_node)
            .or_else(|| {
                tags.iter()
                    .flatten()
                    .filter_map(|t| t.content.as_deref())
                    .find_map(first_error_node)
            }),
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => first_error_node(base)
            .or_else(|| first_error_node(sub))
            .or_else(|| first_error_node(sup))
            .or_else(|| {
                post.as_ref()
                    .and_then(|v| v.iter().find_map(|a| first_error_node(a)))
            }),
        Atom::Scripts { base, sub, sup, .. } => first_error_node(base)
            .or_else(|| sub.as_deref().and_then(first_error_node))
            .or_else(|| sup.as_deref().and_then(first_error_node)),
        Atom::CdArrow { over, under, .. } => over
            .as_deref()
            .and_then(first_error_node)
            .or_else(|| under.as_deref().and_then(first_error_node)),
        _ => None,
    }
}

// A nil child is not a parser diagnostic. A later complete parse error
// discards the tree before SVG construction and therefore takes precedence.
pub(super) fn first_nil_child(atom: &Atom) -> Option<&str> {
    match atom {
        Atom::XArrow { over, under, .. } => {
            first_nil_child(over).or_else(|| under.as_deref().and_then(first_nil_child))
        }
        Atom::Choice(v) => v.iter().find_map(|a| first_nil_child(a)),
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => v.iter().find_map(first_nil_child),
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => first_nil_child(a).or_else(|| first_nil_child(b)),
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
        | Atom::Class { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Mrow(a)
        | Atom::Ord(a)
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => first_nil_child(a),
        Atom::Matrix { rows, .. } => rows.iter().flatten().find_map(first_nil_child),
        Atom::TaggedTable { rows, tags, .. } => {
            rows.iter().flatten().find_map(first_nil_child).or_else(|| {
                tags.iter()
                    .flatten()
                    .filter_map(|t| t.content.as_deref())
                    .find_map(first_nil_child)
            })
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
        svg.push_str(&format!(
            "<path data-c=\"{c:X}\" d=\"M{}Z\"{place}></path>",
            g.path
        ));
    };
    let left = part(left_code).width;
    let right = part(right_code).width;
    draw(svg, left_code, 0.);
    draw(svg, right_code, width - right);
    let middle_width = 0.9;
    let middle_left = (width - middle_width) / 2.;
    let middle_right = (width + middle_width) / 2.;
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
        render_horizontal_extender(svg, 0xe154, stem, section_width, left, right, x);
    }
}

// Pinned stretch.go addExtenderHorizontal, shared by three- and four-part
// recipes. Keep all dimensions in em, including the unrounded clip product.
fn render_horizontal_extender(
    svg: &mut String,
    code: u32,
    stem: GlyphData,
    width: f64,
    left: f64,
    right: f64,
    x: f64,
) {
    let right = (right - 0.1).max(0.);
    let left = (left - 0.1).max(0.);
    let extent = width - left - right;
    if extent <= 0. || stem.width == 0. {
        return;
    }
    let height = stem.height + stem.depth + 0.2;
    let stretch = 1.5 * (extent / stem.width);
    let clip = (stretch * stem.width - extent) / 2.;
    let bottom = -(stem.depth + 0.1);
    svg.push_str(&format!("<svg width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" viewBox=\"{} {} {} {}\"><path data-c=\"{code:X}\" d=\"M{}Z\" transform=\"scale({},1)\"></path></svg>",root_number(extent*1000.),root_number(height*1000.),root_number((x+left)*1000.),root_number(bottom*1000.),root_number(clip*1000.),root_number(bottom*1000.),root_number(extent*1000.),root_number(height*1000.),stem.path,number(stretch)));
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn cd_entries_follow_argument_readers_and_row_annotations() {
        // EndTable drops the pending empty row; a real CR commits it.
        let empty = render_exact(r"\begin{CD}\end{CD}").unwrap();
        assert!(!empty.contains("data-mml-node=\"mtr\""));
        let row = render_exact(r"\begin{CD}\\\end{CD}").unwrap();
        assert!(row.contains("data-mml-node=\"mtr\""));
        assert_eq!(
            render_exact(r"\begin{CD}\frac{a}&b\end{CD}"),
            render_exact("&"),
        );
        assert_eq!(
            render_exact(r"\begin{CD}A\cr B\end{CD}"),
            render_exact(r"\begin{CD}A\\B\end{CD}"),
        );
        let ordinary = measure(r"\begin{CD}A\\B\end{CD}").unwrap();
        let spaced = measure(r"\begin{CD}A\\[2em]B\end{CD}").unwrap();
        assert_eq!(ordinary.width, spaced.width);
        assert!(spaced.height > ordinary.height);
    }

    #[test]
    fn cd_minimums_survive_argument_scopes_and_apply_after_the_table_parse() {
        let ordinary = measure(r"\begin{CD}A@>>>B\end{CD}").unwrap();
        let wide = measure(r"\minCDarrowwidth{4em}\begin{CD}A@>>>B\end{CD}").unwrap();
        assert!(wide.width > ordinary.width);
        for expression in [
            r"{\minCDarrowwidth{4em}}\begin{CD}A@>>>B\end{CD}",
            r"\begin{CD}A@>>>\minCDarrowwidth{4em}B\end{CD}",
        ] {
            let size = measure(expression).unwrap();
            assert_eq!((size.width, size.height), (wide.width, wide.height));
        }
    }
}
