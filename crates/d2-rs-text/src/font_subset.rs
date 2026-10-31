// SPDX-License-Identifier: MPL-2.0
//! TrueType subsetting used by D2's self-contained SVG renderer.
//!
//! The glyph selection and table rewrite follow D2 v0.9.0's
//! `lib/font/subsetFont.go`, originally derived from gofpdf. Keeping this
//! boundary byte-oriented makes the output independent of host font software.

use std::collections::{BTreeMap, BTreeSet};

const CHECKSUM_MAGIC: u32 = 0xB1B0_AFBA;
const COMPOSITE_MORE_COMPONENTS: u16 = 1 << 5;
const COMPOSITE_ARG_WORDS: u16 = 1;
const COMPOSITE_SCALE: u16 = 1 << 3;
const COMPOSITE_XY_SCALE: u16 = 1 << 6;
const COMPOSITE_2X2: u16 = 1 << 7;

#[derive(Clone, Copy, Debug)]
struct TableRecord {
    offset: usize,
    length: usize,
}

/// Builds the same glyph subset as D2 v0.9.0 from a TrueType font.
///
/// The returned bytes are a valid SFNT/TTF. Characters absent from the font's
/// BMP format-4 cmap are ignored, matching the pinned Go implementation.
pub fn subset_ttf(input: &[u8], characters: &str) -> Option<Vec<u8>> {
    if input.get(..4)? != [0, 1, 0, 0] {
        return None;
    }
    let tables = read_table_records(input)?;
    let cmap = table_slice(input, &tables, *b"cmap")?;
    let char_to_glyph = parse_format4_cmap(cmap)?;
    let glyf = table_slice(input, &tables, *b"glyf")?;
    let loca = table_slice(input, &tables, *b"loca")?;
    let head = table_slice(input, &tables, *b"head")?;
    let maxp = table_slice(input, &tables, *b"maxp")?;
    let hhea = table_slice(input, &tables, *b"hhea")?;
    let hmtx = table_slice(input, &tables, *b"hmtx")?;
    let loca_format = read_u16(head, 50)?;
    let glyph_count = read_u16(maxp, 4)? as usize;
    let long_metrics = read_u16(hhea, 34)? as usize;
    if glyph_count == 0 || long_metrics == 0 || long_metrics > glyph_count {
        return None;
    }
    let glyph_offsets = parse_loca(loca, loca_format, glyph_count)?;

    let mut selected = BTreeSet::from([0_u16]);
    let mut unicode_to_old_glyph = BTreeMap::new();
    for character in characters.chars() {
        let codepoint = character as u32;
        if codepoint <= u16::MAX as u32 {
            if let Some(glyph) = char_to_glyph.get(&(codepoint as u16)).copied() {
                selected.insert(glyph);
                unicode_to_old_glyph.insert(codepoint as u16, glyph);
            }
        }
    }

    // D2 assigns used glyph IDs in sorted order, then appends composite glyphs
    // in depth-first component order. That order is visible in hmtx/glyf/loca.
    let mut old_to_new = BTreeMap::new();
    let mut glyph_order = Vec::new();
    for old_glyph in selected.iter().copied() {
        old_to_new.insert(old_glyph, glyph_order.len() as u16);
        glyph_order.push(old_glyph);
    }
    let selected_glyphs = glyph_order.clone();
    for old_glyph in selected_glyphs {
        collect_composite_glyphs(
            old_glyph,
            glyf,
            &glyph_offsets,
            &mut old_to_new,
            &mut glyph_order,
            0,
        )?;
    }

    let mut subset_glyf = Vec::new();
    let mut subset_offsets = Vec::with_capacity(glyph_order.len() + 1);
    let mut subset_hmtx = Vec::new();
    for old_glyph in glyph_order.iter().copied() {
        let start = *glyph_offsets.get(old_glyph as usize)?;
        let end = *glyph_offsets.get(old_glyph as usize + 1)?;
        let source_glyph = glyf.get(start..end)?;
        subset_offsets.push(subset_glyf.len() as u32);
        let mut glyph_data = source_glyph.to_vec();
        remap_composite_components(&mut glyph_data, &old_to_new)?;
        subset_glyf.extend_from_slice(&glyph_data);
        while subset_glyf.len() % 4 != 0 {
            subset_glyf.push(0);
        }
        append_hmetric(&mut subset_hmtx, hmtx, long_metrics, old_glyph as usize)?;
    }
    subset_offsets.push(subset_glyf.len() as u32);

    let unicode_to_new_glyph = unicode_to_old_glyph
        .into_iter()
        .map(|(unicode, old)| Some((unicode, *old_to_new.get(&old)?)))
        .collect::<Option<BTreeMap<_, _>>>()?;
    let mut output = BTreeMap::<[u8; 4], Vec<u8>>::new();
    for tag in [*b"name", *b"cvt ", *b"fpgm", *b"prep", *b"gasp"] {
        if let Some(data) = table_slice(input, &tables, tag) {
            output.insert(tag, data.to_vec());
        }
    }

    let mut post = table_slice(input, &tables, *b"post")?.to_vec();
    if post.len() < 16 {
        return None;
    }
    let mut compact_post = Vec::with_capacity(32);
    compact_post.extend_from_slice(&[0, 3, 0, 0]);
    compact_post.extend_from_slice(&post[4..16]);
    compact_post.extend_from_slice(&[0; 16]);
    post.clear();
    output.insert(*b"post", compact_post);

    output.insert(*b"cmap", build_format4_cmap(&unicode_to_new_glyph)?);
    output.insert(*b"glyf", subset_glyf);
    output.insert(*b"hmtx", subset_hmtx);
    output.insert(*b"loca", build_loca(&subset_offsets)?);

    let mut head = head.to_vec();
    head.get_mut(8..12)?.fill(0);
    write_u16(
        &mut head,
        50,
        if subset_offsets.last().copied()? > 0x1_FFFE {
            1
        } else {
            0
        },
    )?;
    output.insert(*b"head", head);

    let mut hhea = hhea.to_vec();
    write_u16(&mut hhea, 34, glyph_order.len() as u16)?;
    output.insert(*b"hhea", hhea);

    let mut maxp = maxp.to_vec();
    write_u16(&mut maxp, 4, glyph_order.len() as u16)?;
    output.insert(*b"maxp", maxp);
    if let Some(os2) = table_slice(input, &tables, *b"OS/2") {
        output.insert(*b"OS/2", os2.to_vec());
    }
    assemble_sfnt(output)
}

fn read_table_records(input: &[u8]) -> Option<BTreeMap<[u8; 4], TableRecord>> {
    let count = read_u16(input, 4)? as usize;
    let records_end = 12_usize.checked_add(count.checked_mul(16)?)?;
    input.get(12..records_end)?;
    let mut records = BTreeMap::new();
    for index in 0..count {
        let position = 12 + index * 16;
        let tag: [u8; 4] = input.get(position..position + 4)?.try_into().ok()?;
        let offset = read_u32(input, position + 8)? as usize;
        let length = read_u32(input, position + 12)? as usize;
        input.get(offset..offset.checked_add(length)?)?;
        records.insert(tag, TableRecord { offset, length });
    }
    Some(records)
}

fn table_slice<'a>(
    input: &'a [u8],
    tables: &BTreeMap<[u8; 4], TableRecord>,
    tag: [u8; 4],
) -> Option<&'a [u8]> {
    let record = tables.get(&tag)?;
    input.get(record.offset..record.offset.checked_add(record.length)?)
}

fn parse_format4_cmap(cmap: &[u8]) -> Option<BTreeMap<u16, u16>> {
    let table_count = read_u16(cmap, 2)? as usize;
    let mut selected = None;
    for index in 0..table_count {
        let record = 4_usize.checked_add(index.checked_mul(8)?)?;
        let platform = read_u16(cmap, record)?;
        let encoding = read_u16(cmap, record + 2)?;
        let offset = read_u32(cmap, record + 4)? as usize;
        let format = read_u16(cmap, offset)?;
        if ((platform == 3 && encoding == 1) || platform == 0) && format == 4 {
            selected = Some(offset);
            break;
        }
    }
    let base = selected?;
    let length = read_u16(cmap, base + 2)? as usize;
    let limit = base.checked_add(length)?;
    cmap.get(base..limit)?;
    let segment_count = (read_u16(cmap, base + 6)? / 2) as usize;
    if segment_count == 0 {
        return None;
    }
    let end_codes = base + 14;
    let start_codes = end_codes + segment_count * 2 + 2;
    let deltas = start_codes + segment_count * 2;
    let range_offsets = deltas + segment_count * 2;
    let mut mappings = BTreeMap::new();
    for segment in 0..segment_count {
        let start = read_u16(cmap, start_codes + segment * 2)?;
        let end = read_u16(cmap, end_codes + segment * 2)?;
        let delta = read_i16(cmap, deltas + segment * 2)? as i32;
        let range_offset = read_u16(cmap, range_offsets + segment * 2)? as usize;
        if start > end {
            continue;
        }
        for character in start..=end {
            let glyph = if range_offset == 0 {
                (character as i32 + delta) as u16
            } else {
                let glyph_position = range_offsets
                    .checked_add(segment * 2)?
                    .checked_add(range_offset)?
                    .checked_add((character - start) as usize * 2)?;
                if glyph_position + 2 > limit {
                    0
                } else {
                    let raw = read_u16(cmap, glyph_position)?;
                    if raw == 0 {
                        0
                    } else {
                        (raw as i32 + delta) as u16
                    }
                }
            };
            mappings.insert(character, glyph);
        }
    }
    Some(mappings)
}

fn parse_loca(loca: &[u8], format: u16, glyph_count: usize) -> Option<Vec<usize>> {
    let count = glyph_count.checked_add(1)?;
    let mut offsets = Vec::with_capacity(count);
    match format {
        0 => {
            if loca.len() < count.checked_mul(2)? {
                return None;
            }
            for index in 0..count {
                offsets.push(read_u16(loca, index * 2)? as usize * 2);
            }
        }
        1 => {
            if loca.len() < count.checked_mul(4)? {
                return None;
            }
            for index in 0..count {
                offsets.push(read_u32(loca, index * 4)? as usize);
            }
        }
        _ => return None,
    }
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return None;
    }
    Some(offsets)
}

fn collect_composite_glyphs(
    glyph: u16,
    glyf: &[u8],
    offsets: &[usize],
    old_to_new: &mut BTreeMap<u16, u16>,
    glyph_order: &mut Vec<u16>,
    depth: usize,
) -> Option<()> {
    if depth > 1024 {
        return None;
    }
    let start = *offsets.get(glyph as usize)?;
    let end = *offsets.get(glyph as usize + 1)?;
    let data = glyf.get(start..end)?;
    if data.len() < 10 || read_i16(data, 0)? >= 0 {
        return Some(());
    }
    let mut position = 10;
    loop {
        let flags = read_u16(data, position)?;
        let component = read_u16(data, position + 2)?;
        if !old_to_new.contains_key(&component) {
            old_to_new.insert(component, glyph_order.len() as u16);
            glyph_order.push(component);
            collect_composite_glyphs(component, glyf, offsets, old_to_new, glyph_order, depth + 1)?;
        }
        position += 4;
        position += if flags & COMPOSITE_ARG_WORDS != 0 {
            4
        } else {
            2
        };
        position += if flags & COMPOSITE_SCALE != 0 {
            2
        } else if flags & COMPOSITE_XY_SCALE != 0 {
            4
        } else if flags & COMPOSITE_2X2 != 0 {
            8
        } else {
            0
        };
        if flags & COMPOSITE_MORE_COMPONENTS == 0 {
            break;
        }
    }
    Some(())
}

fn remap_composite_components(glyph: &mut [u8], old_to_new: &BTreeMap<u16, u16>) -> Option<()> {
    if glyph.len() < 10 || read_i16(glyph, 0)? >= 0 {
        return Some(());
    }
    let mut position = 10;
    loop {
        let flags = read_u16(glyph, position)?;
        let old_component = read_u16(glyph, position + 2)?;
        write_u16(glyph, position + 2, *old_to_new.get(&old_component)?)?;
        position += 4;
        position += if flags & COMPOSITE_ARG_WORDS != 0 {
            4
        } else {
            2
        };
        position += if flags & COMPOSITE_SCALE != 0 {
            2
        } else if flags & COMPOSITE_XY_SCALE != 0 {
            4
        } else if flags & COMPOSITE_2X2 != 0 {
            8
        } else {
            0
        };
        if flags & COMPOSITE_MORE_COMPONENTS == 0 {
            break;
        }
    }
    Some(())
}

fn append_hmetric(
    output: &mut Vec<u8>,
    hmtx: &[u8],
    long_metrics: usize,
    glyph: usize,
) -> Option<()> {
    if glyph < long_metrics {
        let start = glyph.checked_mul(4)?;
        output.extend_from_slice(hmtx.get(start..start + 4)?);
    } else {
        let advance_offset = long_metrics.checked_sub(1)?.checked_mul(4)?;
        let lsb_offset = long_metrics
            .checked_mul(4)?
            .checked_add(glyph.checked_sub(long_metrics)?.checked_mul(2)?)?;
        output.extend_from_slice(hmtx.get(advance_offset..advance_offset + 2)?);
        output.extend_from_slice(hmtx.get(lsb_offset..lsb_offset + 2)?);
    }
    Some(())
}

fn build_loca(offsets: &[u32]) -> Option<Vec<u8>> {
    let short = offsets.last().copied()? <= 0x1_FFFE;
    let mut output = Vec::with_capacity(offsets.len() * if short { 2 } else { 4 });
    for offset in offsets {
        if short {
            push_u16(&mut output, (offset / 2) as u16);
        } else {
            push_u32(&mut output, *offset);
        }
    }
    Some(output)
}

fn build_format4_cmap(mappings: &BTreeMap<u16, u16>) -> Option<Vec<u8>> {
    let mut ranges: Vec<(u16, Vec<u16>)> = Vec::new();
    for (character, glyph) in mappings {
        if let Some((start, glyphs)) = ranges.last_mut() {
            let previous_character = (*start as usize).checked_add(glyphs.len())?;
            let previous_glyph = glyphs.last().copied()?;
            if previous_character == *character as usize
                && previous_glyph.checked_add(1) == Some(*glyph)
            {
                glyphs.push(*glyph);
                continue;
            }
        }
        ranges.push((*character, vec![*glyph]));
    }
    let segment_count = ranges.len().checked_add(1)?;
    let mut search_range = 1_usize;
    let mut entry_selector = 0_usize;
    while search_range.checked_mul(2)? <= segment_count {
        search_range *= 2;
        entry_selector += 1;
    }
    search_range *= 2;
    let range_shift = segment_count * 2 - search_range;

    let mut subtable = Vec::new();
    push_u16(&mut subtable, 4);
    push_u16(
        &mut subtable,
        (7 + segment_count * 4 + ranges.iter().map(|(_, glyphs)| glyphs.len()).sum::<usize>())
            as u16
            * 2,
    );
    push_u16(&mut subtable, 0);
    push_u16(&mut subtable, (segment_count * 2) as u16);
    push_u16(&mut subtable, search_range as u16);
    push_u16(&mut subtable, entry_selector as u16);
    push_u16(&mut subtable, range_shift as u16);
    for (start, glyphs) in &ranges {
        push_u16(
            &mut subtable,
            start.checked_add(glyphs.len().checked_sub(1)? as u16)?,
        );
    }
    push_u16(&mut subtable, u16::MAX);
    push_u16(&mut subtable, 0);
    for (start, _) in &ranges {
        push_u16(&mut subtable, *start);
    }
    push_u16(&mut subtable, u16::MAX);
    for (start, glyphs) in &ranges {
        let first_glyph = *glyphs.first()?;
        push_u16(
            &mut subtable,
            start.wrapping_sub(first_glyph).wrapping_neg(),
        );
    }
    push_u16(&mut subtable, 1);
    for _ in &ranges {
        push_u16(&mut subtable, 0);
    }
    push_u16(&mut subtable, 0);
    for (_, glyphs) in &ranges {
        for glyph in glyphs {
            push_u16(&mut subtable, *glyph);
        }
    }
    push_u16(&mut subtable, 0);
    let subtable_length = subtable.len();
    write_u16(&mut subtable, 2, subtable_length as u16)?;

    let mut cmap = Vec::with_capacity(subtable.len() + 12);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 3);
    push_u16(&mut cmap, 1);
    push_u32(&mut cmap, 12);
    cmap.extend_from_slice(&subtable);
    Some(cmap)
}

fn assemble_sfnt(tables: BTreeMap<[u8; 4], Vec<u8>>) -> Option<Vec<u8>> {
    let table_count = tables.len();
    let mut power = 1_usize;
    let mut entry_selector = 0_usize;
    while power.checked_mul(2)? <= table_count {
        power *= 2;
        entry_selector += 1;
    }
    let search_range = power.checked_mul(16)?;
    let range_shift = table_count.checked_mul(16)?.checked_sub(search_range)?;
    let mut output = Vec::new();
    push_u32(&mut output, 0x0001_0000);
    push_u16(&mut output, table_count as u16);
    push_u16(&mut output, search_range as u16);
    push_u16(&mut output, entry_selector as u16);
    push_u16(&mut output, range_shift as u16);

    let mut offset = 12_usize.checked_add(table_count.checked_mul(16)?)?;
    let mut head_adjustment_offset = None;
    for (tag, data) in &tables {
        push_u32(&mut output, u32::from_be_bytes(*tag));
        push_u32(&mut output, checksum(data));
        push_u32(&mut output, offset as u32);
        push_u32(&mut output, data.len() as u32);
        if tag == b"head" {
            head_adjustment_offset = Some(offset.checked_add(8)?);
        }
        offset = offset.checked_add(align4(data.len()))?;
    }
    for data in tables.values() {
        output.extend_from_slice(data);
        output.resize(
            output.len().checked_add(align4(data.len()) - data.len())?,
            0,
        );
    }
    let adjustment = go_check_sum_adjustment(checksum(&output));
    write_u32(&mut output, head_adjustment_offset?, adjustment)?;
    Some(output)
}

fn go_check_sum_adjustment(sum: u32) -> u32 {
    // The pinned Go helper's calcInt32 carries a low-word borrow by
    // incrementing the high word. Preserve that behavior because the emitted
    // subset bytes are part of D2's embedded-font output, even though the
    // resulting checkSumAdjustment differs from the SFNT specification.
    let mut expected_high = (CHECKSUM_MAGIC >> 16) as i32;
    let mut expected_low = (CHECKSUM_MAGIC & 0xffff) as i32;
    let sum_high = (sum >> 16) as i32;
    let sum_low = (sum & 0xffff) as i32;
    if sum_low > expected_low {
        expected_low += 1 << 16;
        expected_high += 1;
    }
    let low = expected_low - sum_low;
    if sum_high > expected_high {
        expected_high += 1 << 16;
    }
    let high = (expected_high - sum_high) & 0xffff;
    ((high as u32) << 16) | (low as u32 & 0xffff)
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0_u32;
    for chunk in data.chunks(4) {
        let mut word = [0_u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

fn align4(length: usize) -> usize {
    (length + 3) & !3
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    Some(i16::from_be_bytes(
        data.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn write_u16(data: &mut [u8], offset: usize, value: u16) -> Option<()> {
    data.get_mut(offset..offset.checked_add(2)?)?
        .copy_from_slice(&value.to_be_bytes());
    Some(())
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) -> Option<()> {
    data.get_mut(offset..offset.checked_add(4)?)?
        .copy_from_slice(&value.to_be_bytes());
    Some(())
}

fn push_u16(data: &mut Vec<u8>, value: u16) {
    data.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(data: &mut Vec<u8>, value: u32) {
    data.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::subset_ttf;
    use sha2::{Digest, Sha256};

    const REGULAR: &[u8] = include_bytes!("../assets/SourceSansPro-Regular.ttf");

    #[test]
    fn source_sans_subset_matches_the_pinned_v090_bytes() {
        let subset = subset_ttf(REGULAR, "D2 diagrams 0123 -> []").unwrap();
        let digest = Sha256::digest(&subset);
        let actual = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual,
            "5f0a17176b18400b80c29eee5d83f3a1db79ce1089629ad5f09f070054c6f6f1"
        );
    }

    #[test]
    fn bundled_faces_match_v090_unicode_composite_glyph_subsets() {
        let cutset = "Ω Ж 中 😀 e\u{301} �";
        let cases: [(&[u8], &str); 10] = [
            (
                include_bytes!("../assets/FuzzyBubbles-Bold.ttf"),
                "b8c3c354dcab3aa83678aa8192f7cf407cf56a361901b85d87335ea469bd769b",
            ),
            (
                include_bytes!("../assets/FuzzyBubbles-Regular.ttf"),
                "e884b076c02f93e8e44bda37555e6cdd471ac03c0d2b362bc5b64e88ce27dea4",
            ),
            (
                include_bytes!("../assets/SourceCodePro-Bold.ttf"),
                "260661fd09136271ccc7cf2f3a15a389b7f25d7a26825a3807f70d48495c2598",
            ),
            (
                include_bytes!("../assets/SourceCodePro-Italic.ttf"),
                "4c543ad6ff3d45bbc03733a1fb67c9b0b0b1de7d2b8617609e322f40544c4dfc",
            ),
            (
                include_bytes!("../assets/SourceCodePro-Regular.ttf"),
                "cc5e945f6e59de735d7aa7180c6e5436aa7e3002fb767cf762b24e78a3920e3b",
            ),
            (
                include_bytes!("../assets/SourceCodePro-Semibold.ttf"),
                "7f045dc5585b27071ac5c38b0513be7e853cb14964146b03ce4678cb9264ec06",
            ),
            (
                include_bytes!("../assets/SourceSansPro-Bold.ttf"),
                "475af17049c15291703e360b5e954446f9ddfe1f2926b6b43bd4918ad6a5f9bf",
            ),
            (
                include_bytes!("../assets/SourceSansPro-Italic.ttf"),
                "97502e4f2d71311bad10813d4401f3cf425c2d70fa2451354870f03b954e7623",
            ),
            (
                include_bytes!("../assets/SourceSansPro-Regular.ttf"),
                "5123bdefb734b6ad3660dda2e4ef431748a4c696878bf92feab868ad1ae53e0d",
            ),
            (
                include_bytes!("../assets/SourceSansPro-Semibold.ttf"),
                "eb6867e5f05bcbdeb0a716b9b95a5723e3ab9d91a3aef369174c5cb1ba536f63",
            ),
        ];
        for (font, expected) in cases {
            let subset = subset_ttf(font, cutset).unwrap();
            let digest = Sha256::digest(subset);
            let actual = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn empty_corpus_keeps_only_the_notdef_glyph() {
        let subset = subset_ttf(REGULAR, "").unwrap();
        let tables = super::read_table_records(&subset).unwrap();
        let maxp = super::table_slice(&subset, &tables, *b"maxp").unwrap();
        let glyph_count = super::read_u16(maxp, 4).unwrap();
        assert_eq!(glyph_count, 1);
    }

    #[test]
    fn non_truetype_input_is_rejected() {
        assert!(subset_ttf(b"OTTO", "text").is_none());
    }
}
