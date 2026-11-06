// SPDX-License-Identifier: MPL-2.0 AND MIT
// Source: github.com/rivo/uniseg v0.4.7 line.go, linerules.go,
// graphemerules.go and properties.go. Copyright (c) 2019 Oliver Kuederle.
//! Pinned Unicode 15 boundaries used by original D2's Markdown painter.
//! Inputs are valid UTF-8; byte offsets retain source string ownership.
#![allow(non_upper_case_globals)]
#[path = "uniseg_data.rs"]
mod data;
use data::*;

fn property<const N: usize>(table: &[[u32; N]], ch: char) -> [u32; N] {
    let cp = ch as u32;
    let (mut from, mut to) = (0, table.len());
    while to > from {
        let middle = (from + to) / 2;
        let entry = table[middle];
        if cp < entry[0] {
            to = middle;
        } else if cp > entry[1] {
            from = middle + 1;
        } else {
            return entry;
        }
    }
    [0; N]
}
fn line_property(ch: char) -> (i32, i32) {
    match ch {
        'a'..='z' => (prAL, gcLl),
        'A'..='Z' => (prAL, gcLu),
        '0'..='9' => (prNU, gcNd),
        _ => {
            let entry = property(LINE_PROPERTIES, ch);
            (entry[2] as i32, entry[3] as i32)
        }
    }
}
fn grapheme_property(ch: char) -> i32 {
    match ch {
        '\u{20}'..='\u{7e}' => prAny,
        '\n' => prLF,
        '\r' => prCR,
        '\u{0}'..='\u{1f}' | '\u{7f}' => prControl,
        _ => property(GRAPHEME_PROPERTIES, ch)[2] as i32,
    }
}
fn east_asian_width(ch: char) -> i32 {
    match ch {
        '\u{20}'..='\u{7e}' => prNa,
        '\u{0}'..='\u{1f}' | '\u{7f}' => prN,
        _ => property(EAST_ASIAN_WIDTH, ch)[2] as i32,
    }
}
fn transition(table: &[[i32; 5]], state: i32, prop: i32) -> (i32, i32, i32) {
    table
        .iter()
        .find(|row| row[0] == state && row[1] == prop)
        .map_or((-1, -1, -1), |row| (row[2], row[3], row[4]))
}
fn resolve(
    table: &[[i32; 5]],
    state: i32,
    prop: i32,
    any: i32,
    default_break: i32,
) -> (i32, i32, i32) {
    let specific = transition(table, state, prop);
    if specific.0 >= 0 {
        return specific;
    }
    let by_state = transition(table, state, prAny);
    let by_prop = transition(table, any, prop);
    if by_state.0 >= 0 && by_prop.0 >= 0 {
        if by_state.2 < by_prop.2 {
            (by_prop.0, by_state.1, by_state.2)
        } else {
            by_prop
        }
    } else if by_state.0 >= 0 {
        by_state
    } else if by_prop.0 >= 0 {
        by_prop
    } else {
        (any, default_break, 310)
    }
}
fn line_transition(mut state: i32, ch: char, rest: &str) -> (i32, i32) {
    let (mut prop, category) = line_property(ch);
    let cp_non_wide = state >= 0 && state & lbCPeaFWHBit != 0;
    if cp_non_wide {
        state &= !lbCPeaFWHBit;
    }
    let force_no_break = state >= 0 && state & lbZWJBit != 0;
    if force_no_break {
        state &= !lbZWJBit;
    }
    // Go's deferred changes apply to every return, including special rules.
    let finish = |mut next, mut boundary| {
        if next == lbCP || next == lbNUCP {
            if !matches!(east_asian_width(ch), prF | prW | prH) {
                next |= lbCPeaFWHBit;
            }
        }
        if force_no_break {
            boundary = LineDontBreak;
        }
        (next, boundary)
    };
    if matches!(prop, prAI | prSG | prXX) {
        prop = prAL;
    } else if prop == prSA {
        prop = if matches!(category, gcMn | gcMc) {
            prCM
        } else {
            prAL
        };
    } else if prop == prCJ {
        prop = prNS;
    }
    if matches!(prop, prZWJ | prCM) {
        let bit = if prop == prZWJ { lbZWJBit } else { 0 };
        let must_break = state < 0 || matches!(state, lbBK | lbCR | lbLF | lbNL);
        if !must_break && !matches!(state, lbSP | lbZW | lbQUSP | lbCLCPSP | lbB2SP) {
            return finish(state | bit, LineDontBreak);
        }
        return finish(
            lbAL | bit,
            if must_break {
                LineMustBreak
            } else {
                LineCanBreak
            },
        );
    }
    let (next, boundary, rule) = resolve(LINE_TRANSITIONS, state, prop, lbAny, LineCanBreak);
    if rule > 121
        && prop == prGL
        && !matches!(
            state,
            lbSP | lbBA | lbHY | lbLB21a | lbQUSP | lbCLCPSP | lbB2SP
        )
    {
        return finish(lbGL, LineDontBreak);
    }
    if rule > 130 && !matches!(state, lbNU | lbNUNU) {
        let special = match prop {
            prCL => Some(lbCL),
            prCP => Some(lbCP),
            prIS => Some(lbIS),
            prSY => Some(lbSY),
            _ => None,
        };
        if let Some(special) = special {
            return finish(special, LineDontBreak);
        }
    }
    // Preserve original Go precedence, including the unparenthesized HY arm.
    if (rule > 250 && matches!(state, lbPR | lbPO) && prop == prOP) || prop == prHY {
        if let Some(ahead) = rest.chars().next().filter(|ch| *ch != '\u{fffd}') {
            if line_property(ahead).0 == prNU {
                return finish(lbNU, LineDontBreak);
            }
        }
    }
    if rule > 300 {
        if matches!(state, lbAL | lbHL | lbNU | lbNUNU) && prop == prOP {
            if !matches!(east_asian_width(ch), prF | prW | prH) {
                return finish(lbOP, LineDontBreak);
            }
        } else if cp_non_wide {
            let special = match prop {
                prAL => Some(lbAL),
                prHL => Some(lbHL),
                prNU => Some(lbNU),
                _ => None,
            };
            if let Some(special) = special {
                return finish(special, LineDontBreak);
            }
        }
    }
    if next == lbAny && prop == prRI {
        return finish(
            if state == lbOddRI { lbEvenRI } else { lbOddRI },
            if state == lbOddRI {
                LineDontBreak
            } else {
                boundary
            },
        );
    }
    if rule > 302 {
        if prop == prEM && matches!(state, lbEB | lbExtPicCn) {
            return finish(prAny, LineDontBreak);
        }
        if grapheme_property(ch) == prExtendedPictographic && category == gcCn {
            return finish(lbExtPicCn, LineCanBreak);
        }
    }
    finish(next, boundary)
}

/// Every source line-segment end offset, with its must-break flag. End-of-input
/// is always a mandatory boundary, as in FirstLineSegmentInString (LB3).
pub fn line_boundaries(text: &str) -> Vec<(usize, bool)> {
    let mut chars = text.char_indices();
    let Some((_, first)) = chars.next() else {
        return Vec::new();
    };
    let mut state = line_transition(-1, first, &text[first.len_utf8()..]).0;
    let mut result = Vec::new();
    for (offset, ch) in chars {
        let (next, boundary) = line_transition(state, ch, &text[offset + ch.len_utf8()..]);
        state = next;
        if boundary != LineDontBreak {
            result.push((offset, boundary == LineMustBreak));
        }
    }
    result.push((text.len(), true));
    result
}

/// End offsets of original Unicode 15 extended grapheme clusters.
pub fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let mut state = -1;
    let mut result = Vec::new();
    for (offset, ch) in text.char_indices() {
        let (next, boundary, _) = resolve(
            GRAPHEME_TRANSITIONS,
            state,
            grapheme_property(ch),
            grAny,
            grBoundary,
        );
        if offset != 0 && boundary == grBoundary {
            result.push(offset);
        }
        state = next;
    }
    if !text.is_empty() {
        result.push(text.len());
    }
    result
}

fn rune_width(ch: char) -> usize {
    match grapheme_property(ch) {
        prControl | prCR | prLF | prExtend | prZWJ => return 0,
        prRegionalIndicator => return 2,
        prExtendedPictographic => {
            return if property(EMOJI_PRESENTATION, ch)[2] as i32 == prEmojiPresentation {
                2
            } else {
                1
            };
        }
        _ => {}
    }
    match ch {
        '\u{2e3a}' => 3,
        '\u{2e3b}' => 4,
        _ => {
            if matches!(east_asian_width(ch), prW | prF) {
                2
            } else {
                1
            }
        }
    }
}
/// Original StringWidth, with the unchanged default ambiguous width of one.
pub fn string_width(text: &str) -> usize {
    let mut start = 0;
    let mut total = 0;
    for end in grapheme_boundaries(text) {
        let mut chars = text[start..end].chars();
        let first = chars.next().unwrap();
        let first_prop = grapheme_property(first);
        let mut width = rune_width(first);
        for ch in chars {
            if first_prop == prExtendedPictographic {
                if ch == '\u{fe0e}' {
                    width = 1;
                } else if ch == '\u{fe0f}' {
                    width = 2;
                }
            } else if !matches!(first_prop, prRegionalIndicator | prL) {
                width += rune_width(ch);
            }
        }
        total += width;
        start = end;
    }
    total
}
fn in_ranges(table: &[[u32; 3]], ch: char) -> bool {
    let entry = property(table, ch);
    entry[2] != 0 && ((ch as u32) - entry[0]).is_multiple_of(entry[2])
}
pub fn is_symbol(ch: char) -> bool {
    in_ranges(GO_S, ch)
}
pub fn is_mark(ch: char) -> bool {
    in_ranges(GO_MN, ch) || in_ranges(GO_ME, ch)
}
pub fn is_color_emoji(text: &str) -> bool {
    let mut variation = false;
    let mut base = false;
    let mut keycap = false;
    for ch in text.chars() {
        if ch == '\u{fe0f}' {
            variation = true;
            continue;
        }
        if ch == '\u{20e3}' {
            keycap = true;
            continue;
        }
        base |= is_symbol(ch);
        if in_ranges(GO_SO, ch) && string_width(&ch.to_string()) == 2 {
            return true;
        }
    }
    if keycap
        && text
            .chars()
            .any(|ch| ch == '#' || ch == '*' || ch.is_ascii_digit())
    {
        base = true;
    }
    variation && base
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    #[test]
    fn complete_original_unicode15_boundary_controls() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/uniseg-boundaries-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 16652);
        for (index, row) in rows.iter().enumerate() {
            let text = row["input"].as_str().unwrap();
            assert_eq!(
                super::string_width(text) as u64,
                row["string_width"].as_u64().unwrap(),
                "width row {index}: {text:?}"
            );
            assert_eq!(
                serde_json::to_value(super::line_boundaries(text)).unwrap(),
                row["line_boundaries"],
                "line row {index}: {text:?}"
            );
            assert_eq!(
                serde_json::to_value(super::grapheme_boundaries(text)).unwrap(),
                row["grapheme_boundaries"],
                "grapheme row {index}: {text:?}"
            );
        }
    }
}
