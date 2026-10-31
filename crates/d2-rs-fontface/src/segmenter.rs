// SPDX-License-Identifier: Unlicense OR BSD-3-Clause
// Copyright 2021 The go-text authors; Rust translation Copyright 2026 Nelson
// Chen.
//! go-text/typesetting v0.3.4 segmenter: the UAX #29 grapheme-boundary part
//! of computeBreakAttributes (unicode29_rules.go applyGraphemeBoundaryRules
//! with its GB9c, GB11 and GB12/13 state machines) over go-text's own
//! Unicode 17 property data (segmenter_tables.rs).

#[path = "segmenter_tables.rs"]
mod tables;

const GB_CR: u16 = 1 << 0;
const GB_CONTROL: u16 = 1 << 1;
const GB_EXTEND: u16 = 1 << 2;
const GB_L: u16 = 1 << 3;
const GB_LF: u16 = 1 << 4;
const GB_LV: u16 = 1 << 5;
const GB_LVT: u16 = 1 << 6;
const GB_PREPEND: u16 = 1 << 7;
const GB_REGIONAL_INDICATOR: u16 = 1 << 8;
const GB_SPACING_MARK: u16 = 1 << 9;
const GB_T: u16 = 1 << 10;
const GB_V: u16 = 1 << 11;
const GB_ZWJ: u16 = 1 << 12;

const ICB_LINKER: u8 = 1 << 0;
const ICB_CONSONANT: u8 = 1 << 1;
const ICB_EXTEND: u8 = 1 << 2;

const PARAGRAPH_SEPARATOR: u32 = 0x2029;

fn lookup(table: &[(u32, u32, u8)], value: u32) -> u8 {
    match table.binary_search_by(|&(first, last, _)| {
        if value < first {
            std::cmp::Ordering::Greater
        } else if value > last {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => table[index].2,
        Err(_) => 0,
    }
}

fn grapheme_break(value: u32) -> u16 {
    match lookup(tables::GRAPHEME_BREAK, value) {
        0 => 0,
        index => 1 << (index - 1),
    }
}

fn indic_conjunct_break(value: u32) -> u8 {
    match lookup(tables::INDIC_CONJUNCT_BREAK, value) {
        0 => 0,
        index => 1 << (index - 1),
    }
}

fn is_extended_pictographic(value: u32) -> bool {
    lookup(tables::EXTENDED_PICTOGRAPHIC, value) == 1
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Picto {
    None,
    InExtend,
    SeenZwj,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Indic {
    None,
    In,
    Seen,
}

/// The grapheme clusters of `text` as (rune offset, rune count), as
/// `Segmenter.GraphemeIterator` yields them.
pub(crate) fn graphemes(text: &[char]) -> Vec<(usize, usize)> {
    let mut boundaries = vec![false; text.len() + 1];
    let mut r = 0u32;
    let mut grapheme = 0u16;
    let mut prev_ri_odd = false;
    let mut picto = Picto::None;
    let mut indic = Indic::None;
    for i in 0..=text.len() {
        let prev = std::mem::replace(
            &mut r,
            text.get(i).map_or(PARAGRAPH_SEPARATOR, |&c| u32::from(c)),
        );
        let extended_pictographic = is_extended_pictographic(r);
        let icb = indic_conjunct_break(r);
        let prev_grapheme = std::mem::replace(&mut grapheme, grapheme_break(r));

        // updateIndicConjunctBreakSequence (GB9c)
        let trigger_gb9c = match indic {
            Indic::None => {
                if icb == ICB_CONSONANT {
                    indic = Indic::In;
                }
                false
            }
            Indic::In => {
                if icb == ICB_EXTEND || icb == ICB_CONSONANT {
                } else if icb == ICB_LINKER {
                    indic = Indic::Seen;
                } else {
                    indic = Indic::None;
                }
                false
            }
            Indic::Seen => {
                if icb & (ICB_EXTEND | ICB_LINKER) != 0 {
                    false
                } else if icb == ICB_CONSONANT {
                    indic = Indic::In;
                    true
                } else {
                    indic = Indic::None;
                    false
                }
            }
        };
        // updatePictoSequence (GB11)
        let trigger_gb11 = match picto {
            Picto::None => {
                if extended_pictographic {
                    picto = Picto::InExtend;
                }
                false
            }
            Picto::InExtend => {
                if grapheme == GB_EXTEND {
                } else if grapheme == GB_ZWJ {
                    picto = Picto::SeenZwj;
                } else {
                    picto = Picto::None;
                }
                false
            }
            Picto::SeenZwj => {
                if extended_pictographic {
                    picto = Picto::InExtend;
                    true
                } else {
                    picto = Picto::None;
                    false
                }
            }
        };
        // updateGraphemeRIOdd (GB12, GB13)
        let trigger_gb12_13 = if grapheme == GB_REGIONAL_INDICATOR {
            let trigger = prev_ri_odd;
            prev_ri_odd = !prev_ri_odd;
            trigger
        } else {
            prev_ri_odd = false;
            false
        };

        let (br0, br1) = (prev_grapheme, grapheme);
        let controls = GB_CONTROL | GB_CR | GB_LF;
        boundaries[i] = if r == u32::from('\n') && prev == u32::from('\r') {
            false
        } else if br0 & controls != 0 || br1 & controls != 0 {
            true
        } else if br0 == GB_L && br1 & (GB_L | GB_V | GB_LV | GB_LVT) != 0 {
            false
        } else if br0 & (GB_LV | GB_V) != 0 && br1 & (GB_V | GB_T) != 0 {
            false
        } else if br0 & (GB_LVT | GB_T) != 0 && br1 == GB_T {
            false
        } else if br1 & (GB_EXTEND | GB_ZWJ) != 0 || br1 == GB_SPACING_MARK || br0 == GB_PREPEND {
            false
        } else {
            !(trigger_gb9c || trigger_gb11 || trigger_gb12_13)
        };
    }
    boundaries[0] = true;
    boundaries[text.len()] = true;
    let mut clusters = Vec::new();
    let mut last = 0;
    for (pos, &boundary) in boundaries.iter().enumerate().skip(1) {
        if boundary {
            clusters.push((last, pos - last));
            last = pos;
        }
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::graphemes;

    fn offsets(text: &str) -> Vec<usize> {
        let runes: Vec<char> = text.chars().collect();
        graphemes(&runes)
            .into_iter()
            .map(|(offset, _)| offset)
            .collect()
    }

    #[test]
    fn clusters_match_go_text_segmenter() {
        // Observed from go-text v0.3.4 Segmenter.GraphemeIterator: Unicode 17
        // makes MYANMAR SIGN VIRAMA an Indic_Conjunct_Break linker (GB9c),
        // while U+102C stays outside SpacingMark.
        assert_eq!(
            offsets("\u{1019}\u{1004}\u{103a}\u{1039}\u{1002}\u{101c}\u{102c}\u{1015}\u{102b} ("),
            [0, 1, 5, 6, 7, 8, 9, 10]
        );
        assert_eq!(offsets("a\r\nb"), [0, 1, 3]);
        assert_eq!(
            offsets("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x"),
            [0, 5]
        );
        assert_eq!(offsets("\u{1f1fa}\u{1f1f8}\u{1f1fa}"), [0, 2]);
        assert_eq!(offsets("e\u{301}\u{308}\u{1100}\u{1161}\u{11a8}"), [0, 3]);
    }
}
