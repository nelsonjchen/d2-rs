// Copyright (c) 2020-2022 MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: tex/mathtools_handlers.go and mathtools_handlers_environments.go.
use super::*;
pub(super) fn is_environment(name: &str) -> bool {
    matches!(
        name,
        "align"
            | "align*"
            | "alignat"
            | "alignat*"
            | "xalignat"
            | "xalignat*"
            | "xxalignat"
            | "aligned"
            | "alignedat"
            | "gather"
            | "gather*"
            | "gathered"
            | "multline"
            | "multline*"
            | "lgathered"
            | "rgathered"
    )
}
pub(super) fn aboxed_cells(mut cells: Vec<String>, ams: ams::Shared) -> Option<Vec<Atom>> {
    let raw = cells.pop().unwrap();
    let mut parsed = Vec::new();
    for raw in &cells {
        let cell = parse_unresolved_shared(raw.trim(), ams.clone())?;
        if cd::first_error(&cell).is_some() {
            return Some(vec![cell]);
        }
        parsed.push(cell);
    }
    let start = raw.find("\\Aboxed").unwrap() + "\\Aboxed".len();
    let mut parser = Parser {
        ams: ams.clone(),
        splice_inferred: false,
        chars: raw[start..].chars().collect(),
        index: 0,
        normal_font: false,
        calligraphic_font: false,
        multi_letter_font: false,
        cd_minimums: (Dimension::relative(2.75), Dimension::relative(1.75)),
    };
    let argument = match parser.padded_raw_argument("Aboxed") {
        Ok(v) => v,
        Err(e) => return Some(vec![Atom::ErrorDisplay(e)]),
    };
    // splitTopLevel does not treat row breaks as separators; only the first
    // two top-level ampersand parts participate in the source construction.
    let chars: Vec<_> = argument.chars().collect();
    let (mut start, mut i, mut depth) = (0, 0, 0usize);
    let mut parts = vec![];
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '\\' => i += 1,
            '&' if depth == 0 => {
                parts.push(chars[start..i].iter().collect::<String>());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(chars[start..].iter().collect());
    let left = &parts[0];
    let right = parts.get(1).map_or("", String::as_str);
    if parsed.len() % 2 == 1 {
        parsed.push(Atom::Sequence(vec![]));
    }
    let first = format!("\\rlap{{\\boxed{{{left}{{}}{right}}}}}\\kern.267em\\phantom{{{left}}}");
    let second = format!("\\phantom{{{{}}{right}}}\\kern.267em");
    for raw in [first, second] {
        let cell = parse_unresolved_shared(&raw, ams.clone())?;
        if cd::first_error(&cell).is_some() {
            return Some(vec![cell]);
        }
        parsed.push(cell);
    }
    Some(parsed)
}

pub(super) fn has_special(source: &str) -> bool {
    [
        "\\Aboxed",
        "\\ArrowBetweenLines",
        "\\vdotswithin",
        "\\shortvdotswithin",
        "\\MTFlushSpaceAbove",
        "\\MTFlushSpaceBelow",
        "\\shoveleft",
        "\\shoveright",
    ]
    .iter()
    .any(|s| source.contains(s))
}
fn scanner(raw: &str, ams: ams::Shared) -> Parser {
    Parser {
        ams,
        splice_inferred: false,
        chars: raw.chars().collect(),
        index: 0,
        normal_font: false,
        calligraphic_font: false,
        multi_letter_font: false,
        cd_minimums: (Dimension::relative(2.75), Dimension::relative(1.75)),
    }
}
fn argument(raw: &str, name: &str, ams: ams::Shared) -> Result<String, String> {
    let start = raw.find(&format!("\\{name}")).unwrap() + name.len() + 1;
    scanner(&raw[start..], ams).padded_raw_argument(name)
}
fn dots(raw: &str, flush: bool, ams: ams::Shared) -> Option<Atom> {
    let arg = parse_unresolved_shared(raw, ams)?;
    // mathtoolsVDots ignores parseString's error; unwrapInferred(nil)
    // leaves a nil child in its phantom row. Preserve that state until SVG
    // wrapping, rather than treating it as a top-level parser error display.
    if cd::first_error(&arg).is_some() {
        return Some(Atom::SourceNilChild);
    }
    let mut children = vec![Atom::Identifier(vec![])];
    match arg {
        Atom::Sequence(v) => children.extend(v),
        a => children.push(a),
    }
    children.push(Atom::Identifier(vec![]));
    let inner = Atom::Padded {
        // mathtoolsVDots creates an mo explicitly, rather than parsing a name.
        body: Box::new(Atom::OperatorToken(vec![Atom::Glyph(0x22ee)])),
        padding: padded::Padding {
            width_zero: true,
            lspace_width: -0.5,
            height_add: if flush { -0.6 } else { 0. },
            voffset_em: if flush { -0.18 } else { 0. },
            ..Default::default()
        },
    };
    Some(Atom::Padded {
        body: Box::new(Atom::Sequence(vec![
            inner,
            Atom::Phantom(Box::new(Atom::InferredRow(children))),
        ])),
        padding: padded::Padding {
            lspace_width: 0.5,
            ..Default::default()
        },
    })
}
pub(super) fn parse_rows(
    content: &str,
    environment: &str,
    ams: ams::Shared,
) -> Option<(Vec<Vec<Atom>>, table::MultlineOptions)> {
    let mut rows: Vec<Vec<Atom>> = vec![];
    let mut adjusted = std::collections::BTreeSet::new();
    for cells in table::raw_rows(content) {
        let joined = cells.join("&");
        let joined = joined.trim();
        if joined.starts_with("\\ArrowBetweenLines") {
            let mut p = scanner(joined, ams.clone());
            p.index = 1;
            p.capture_control_sequence();
            while p.chars.get(p.index).is_some_and(|c| c.is_whitespace()) {
                p.index += 1;
            }
            let star = p.chars.get(p.index) == Some(&'*');
            if star {
                p.index += 1;
            }
            while p.chars.get(p.index).is_some_and(|c| c.is_whitespace()) {
                p.index += 1;
            }
            let present = p.chars.get(p.index) == Some(&'[');
            let symbol = match p.padded_brackets() {
                Ok(v) => {
                    if present {
                        v
                    } else {
                        "\\Updownarrow".into()
                    }
                }
                Err(e) => return Some((vec![vec![Atom::ErrorDisplay(e)]], Default::default())),
            };
            let body = parse_unresolved_shared(&symbol, ams.clone())?;
            if cd::first_error(&body).is_some() {
                return Some((vec![vec![body]], Default::default()));
            }
            let mut children = match body {
                Atom::Sequence(v) => v,
                a => vec![a],
            };
            let space = Atom::Style(Box::new(Atom::Kern(Dimension::relative(1.))));
            if star {
                children.insert(0, space)
            } else {
                children.push(space)
            }
            rows.push(vec![Atom::InferredRow(children)]);
            rows.push(vec![Atom::Sequence(vec![])]);
            continue;
        }
        let short = joined.contains("\\shortvdotswithin");
        if short || joined.contains("\\vdotswithin") {
            let raw = match argument(
                joined,
                if short {
                    "shortvdotswithin"
                } else {
                    "vdotswithin"
                },
                ams.clone(),
            ) {
                Ok(v) => v,
                Err(e) => return Some((vec![vec![Atom::ErrorDisplay(e)]], Default::default())),
            };
            let flush = short || joined.contains("\\MTFlushSpaceAbove");
            if flush && !rows.is_empty() {
                adjusted.insert(rows.len() - 1);
            }
            let index = rows.len();
            let d = dots(&raw, flush, ams.clone())?;
            if cd::first_error(&d).is_some() {
                return Some((vec![vec![d]], Default::default()));
            }
            if short {
                rows.push(vec![Atom::Sequence(vec![]), d]);
                rows.push(vec![Atom::Sequence(vec![])]);
                adjusted.insert(index);
            } else {
                rows.push(vec![d]);
                if let Some(i) = joined.find("\\MTFlushSpaceBelow") {
                    adjusted.insert(index);
                    let rest = joined[i + "\\MTFlushSpaceBelow".len()..].trim();
                    if !rest.is_empty() {
                        let mut row = vec![];
                        for raw in table::raw_rows(rest).into_iter().flatten() {
                            let a = parse_unresolved_shared(raw.trim(), ams.clone())?;
                            if cd::first_error(&a).is_some() {
                                return Some((vec![vec![a]], Default::default()));
                            }
                            row.push(a);
                        }
                        rows.push(row);
                    }
                }
            }
            continue;
        }
        let row = if cells.last().is_some_and(|s| s.contains("\\Aboxed")) {
            aboxed_cells(cells, ams.clone())?
        } else {
            let mut row = vec![];
            for raw in cells {
                let a = parse_unresolved_shared(raw.trim(), ams.clone())?;
                if cd::first_error(&a).is_some() {
                    return Some((vec![vec![a]], Default::default()));
                }
                row.push(a);
            }
            row
        };
        if row.iter().any(|a| cd::first_error(a).is_some()) {
            return Some((vec![row], Default::default()));
        }
        rows.push(row);
    }
    for row in &mut rows {
        if !environment.contains("gather") {
            for (column, cell) in row.iter_mut().enumerate() {
                if column % 2 == 1 {
                    let mut v = match std::mem::replace(cell, Atom::Sequence(vec![])) {
                        Atom::Sequence(v) => v,
                        a => vec![a],
                    };
                    if v.first().is_some_and(|a| {
                        matches!(a, Atom::Glyph(..) | Atom::RawOperator { .. })
                            && atom_tex_class(a) == Some(3)
                    }) {
                        v.insert(0, Atom::Identifier(vec![]));
                    }
                    *cell = Atom::Sequence(v);
                }
            }
        }
        if matches!(environment, "xalignat" | "xalignat*") {
            let mut padded = vec![Atom::InferredRow(vec![])];
            for (i, a) in std::mem::take(row).into_iter().enumerate() {
                padded.push(a);
                if i % 2 == 1 {
                    padded.push(Atom::InferredRow(vec![]));
                }
            }
            *row = padded;
        }
    }
    let mut options = table::MultlineOptions::default();
    if !adjusted.is_empty() {
        options.row_spacing = vec![0.3; rows.len()];
        if let Some(first) = options.row_spacing.first_mut() {
            *first = length::dimension("3pt").em(1., 1.);
        };
        for i in adjusted {
            options.row_spacing[i] = 0.1;
        }
    }
    Some((rows, options))
}

// layout.Em serializes row-spacing updates through exact-binary JS ToFixed(3).
pub(super) fn row_spacing_em(value: f64) -> f64 {
    if value < 0.001 {
        return 0.;
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let numerator = (((bits & ((1_u64 << 52) - 1)) | (1_u64 << 52)) as u128) * 1000;
    let rounded = if exponent >= 0 {
        numerator.checked_shl(exponent as u32)
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            Some(0)
        } else {
            let q = numerator >> shift;
            let r = numerator & ((1_u128 << shift) - 1);
            Some(q + u128::from(r >= 1_u128 << (shift - 1)))
        }
    };
    rounded.map_or(value, |v| v as f64 / 1000.)
}
