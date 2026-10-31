// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2019 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/image v0.45.0 font/sfnt/gpos.go (GPOS
// pair-positioning kern functions used by Font.Kern).

use super::*;

const HEX_SCRIPT_LATN: u32 = 0x6c61_746e;
const HEX_SCRIPT_DFLT: u32 = 0x4446_4c54;
const HEX_FEATURE_KERN: u32 = 0x6b65_726e;

type IndexLookup = Arc<dyn Fn(GlyphIndex) -> Option<usize> + Send + Sync>;
type ClassLookup = Box<dyn Fn(GlyphIndex) -> i64 + Send + Sync>;

pub(super) fn parse_gpos_kern(src: &Source, gpos: Table) -> Result<Option<Vec<KernFunc>>, Error> {
    if gpos.length == 0 {
        return Ok(None);
    }
    const HEADER_SIZE: u32 = 10;
    if gpos.length < HEADER_SIZE {
        return Err(ERR_INVALID_GPOS_TABLE);
    }
    let buf = src.view(i64::from(gpos.offset), i64::from(HEADER_SIZE))?;
    if u16_at(buf, 0) != 1 || u16_at(buf, 2) > 1 {
        return Err(ERR_UNSUPPORTED_GPOS_TABLE);
    }
    let script_list_offset = i64::from(u16_at(buf, 4));
    let feature_list_offset = i64::from(u16_at(buf, 6));
    let lookup_list_offset = i64::from(u16_at(buf, 8));
    let base = i64::from(gpos.offset);

    let mut feature_idxs = parse_script_features(src, base + script_list_offset, HEX_SCRIPT_LATN)?;
    if feature_idxs.is_empty() {
        feature_idxs = parse_script_features(src, base + script_list_offset, HEX_SCRIPT_DFLT)?;
        if feature_idxs.is_empty() {
            return Ok(None);
        }
    }
    let lookup_idx = parse_features_lookup(
        src,
        base + feature_list_offset,
        &feature_idxs,
        HEX_FEATURE_KERN,
    )?;
    let (lookup_list, num_lookup_tables) = src.var_len_view(base + lookup_list_offset, 2, 0, 2)?;

    let mut kern_funcs: Vec<KernFunc> = Vec::new();
    'lookup_tables: for n in lookup_idx {
        if n > num_lookup_tables {
            return Err(ERR_INVALID_GPOS_TABLE);
        }
        // Go indexes past the view (and panics) when n == count; report
        // the same malformed table as an error instead.
        if 2 + n * 2 + 2 > lookup_list.len() {
            return Err(ERR_INVALID_GPOS_TABLE);
        }
        let table_offset = base + lookup_list_offset + i64::from(u16_at(lookup_list, 2 + n * 2));
        let (buf, num_sub_tables) = src.var_len_view(table_offset, 8, 4, 2)?;
        let flags = u16_at(buf, 2);
        let mut sub_table_offsets: Vec<i64> = (0..num_sub_tables)
            .map(|i| table_offset + i64::from(u16_at(buf, 6 + i * 2)))
            .collect();
        match u16_at(buf, 0) {
            2 => {}
            9 => {
                for offset in &mut sub_table_offsets {
                    let buf = src.view(*offset, 8)?;
                    if u16_at(buf, 0) != 1 {
                        return Err(ERR_UNSUPPORTED_EXTENSION_POS_FORMAT);
                    }
                    if u16_at(buf, 2) != 2 {
                        continue 'lookup_tables;
                    }
                    *offset += i64::from(u32_at(buf, 4));
                }
            }
            _ => continue,
        }
        if flags & 0x0010 > 0 {
            continue;
        }
        for sub_table_offset in sub_table_offsets {
            let buf = src.view(sub_table_offset, 4)?;
            let format = u16_at(buf, 0);
            let coverage = make_coverage_lookup(src, sub_table_offset + i64::from(u16_at(buf, 2)))?;
            match format {
                1 => {
                    if let Some(kern) = parse_pair_pos_format1(src, sub_table_offset, coverage)? {
                        kern_funcs.push(kern);
                    }
                }
                2 => {
                    if let Some(kern) = parse_pair_pos_format2(src, sub_table_offset, coverage)? {
                        kern_funcs.push(kern);
                    }
                }
                _ => {}
            }
        }
    }
    // Go keeps a nil slice when nothing was appended, so Font.Kern then
    // falls back to the kern table.
    Ok((!kern_funcs.is_empty()).then_some(kern_funcs))
}

fn parse_pair_pos_format1(
    src: &Source,
    offset: i64,
    coverage: IndexLookup,
) -> Result<Option<KernFunc>, Error> {
    let (buf, n_pairs) = src.var_len_view(offset, 10, 8, 2)?;
    if u16_at(buf, 4) != 0x04 || u16_at(buf, 6) != 0x00 {
        return Ok(None);
    }
    let mut last_pair_set_offset = 0usize;
    for n in 0..n_pairs {
        let pair_offset = usize::from(u16_at(buf, 10 + n * 2));
        if pair_offset > last_pair_set_offset {
            last_pair_set_offset = pair_offset;
        }
    }
    let buf = src.view(offset + last_pair_set_offset as i64, 2)?;
    let pair_value_count = usize::from(u16_at(buf, 0));
    let length = last_pair_set_offset + 2 + pair_value_count * 4;
    let glyphs = src.view(offset, length as i64)?.to_vec();
    Ok(Some(Box::new(move |a, b| {
        let Some(idx) = coverage(a) else {
            return Err(Error::NotFound);
        };
        if idx >= n_pairs {
            return Err(Error::NotFound);
        }
        let offset = usize::from(u16_at(&glyphs, 10 + idx * 2));
        if offset + 1 >= glyphs.len() {
            return Err(ERR_INVALID_GPOS_TABLE);
        }
        let count = usize::from(u16_at(&glyphs, offset));
        for i in 0..count {
            if offset + 2 + i * 4 + 4 > glyphs.len() {
                return Err(ERR_INVALID_GPOS_TABLE);
            }
            let second = u16_at(&glyphs, offset + 2 + i * 4);
            if second == b {
                return Ok(u16_at(&glyphs, offset + 2 + i * 4 + 2) as i16);
            }
            if second > b {
                return Err(Error::NotFound);
            }
        }
        Err(Error::NotFound)
    })))
}

fn parse_pair_pos_format2(
    src: &Source,
    offset: i64,
    coverage: IndexLookup,
) -> Result<Option<KernFunc>, Error> {
    let buf = src.view(offset, 16)?;
    if u16_at(buf, 4) != 0x04 || u16_at(buf, 6) != 0x00 {
        return Ok(None);
    }
    let num_class1 = usize::from(u16_at(buf, 12));
    let num_class2 = usize::from(u16_at(buf, 14));
    let cdef1_offset = offset + i64::from(u16_at(buf, 8));
    let cdef2_offset = offset + i64::from(u16_at(buf, 10));
    let cdef1 = make_class_lookup(src, cdef1_offset)?;
    let cdef2 = make_class_lookup(src, cdef2_offset)?;
    let glyphs = src
        .view(offset + 16, (num_class1 * num_class2 * 2) as i64)?
        .to_vec();
    Ok(Some(Box::new(move |a, b| {
        if coverage(a).is_none() {
            return Err(Error::NotFound);
        }
        let idxa = cdef1(a);
        let idxb = cdef2(b);
        if idxa < 0 || idxa >= num_class1 as i64 || idxb < 0 || idxb >= num_class2 as i64 {
            return Err(ERR_INVALID_GPOS_TABLE);
        }
        Ok(u16_at(&glyphs, (idxb as usize + idxa as usize * num_class2) * 2) as i16)
    })))
}

fn parse_script_features(src: &Source, offset: i64, script: u32) -> Result<Vec<usize>, Error> {
    let (buf, num_script_tables) = src.var_len_view(offset, 2, 0, 6)?;
    let mut script_table_offset = 0u16;
    for i in 0..num_script_tables {
        if u32_at(buf, 2 + i * 6) == script {
            script_table_offset = u16_at(buf, 2 + i * 6 + 4);
            break;
        }
    }
    if script_table_offset == 0 {
        return Ok(Vec::new());
    }
    let buf = src.view(offset + i64::from(script_table_offset), 2)?;
    let default_lang_sys_offset = u16_at(buf, 0);
    if default_lang_sys_offset == 0 {
        return Ok(Vec::new());
    }
    let (buf, num_features) = src.var_len_view(
        offset + i64::from(script_table_offset) + i64::from(default_lang_sys_offset),
        6,
        4,
        2,
    )?;
    Ok((0..num_features)
        .map(|i| usize::from(u16_at(buf, 6 + i * 2)))
        .collect())
}

fn parse_features_lookup(
    src: &Source,
    offset: i64,
    feature_idxs: &[usize],
    feature: u32,
) -> Result<Vec<usize>, Error> {
    let (buf, num_feature_tables) = src.var_len_view(offset, 2, 0, 6)?;
    let mut lookup_idx = Vec::with_capacity(4);
    for &fidx in feature_idxs {
        if fidx > num_feature_tables {
            return Err(ERR_INVALID_GPOS_TABLE);
        }
        if 2 + fidx * 6 + 6 > buf.len() {
            return Err(ERR_INVALID_GPOS_TABLE);
        }
        if u32_at(buf, 2 + fidx * 6) != feature {
            continue;
        }
        let feature_offset = u16_at(buf, 2 + fidx * 6 + 4);
        let (lookups, num_lookups) =
            src.var_len_view(offset + i64::from(feature_offset), 4, 2, 2)?;
        for i in 0..num_lookups {
            lookup_idx.push(usize::from(u16_at(lookups, 4 + i * 2)));
        }
    }
    Ok(lookup_idx)
}

fn make_coverage_lookup(src: &Source, offset: i64) -> Result<IndexLookup, Error> {
    let buf = src.view(offset, 2)?;
    match u16_at(buf, 0) {
        1 => {
            let (buf, _) = src.var_len_view(offset, 4, 2, 2)?;
            let num = usize::from(u16_at(buf, 2));
            let list = buf[4..].to_vec();
            Ok(Arc::new(move |gi| {
                let idx = search(num, |i| gi <= u16_at(&list, i * 2));
                (idx < num && u16_at(&list, idx * 2) == gi).then_some(idx)
            }))
        }
        2 => {
            let (buf, _) = src.var_len_view(offset, 4, 2, 6)?;
            let num = usize::from(u16_at(buf, 2));
            let ranges = buf[4..].to_vec();
            Ok(Arc::new(move |gi| {
                if num == 0 {
                    return None;
                }
                let mut idx = search(num, |i| gi <= u16_at(&ranges, i * 6));
                if idx < num && gi == u16_at(&ranges, idx * 6) {
                    return Some(usize::from(u16_at(&ranges, idx * 6 + 4)));
                }
                if idx > 0 {
                    idx -= 1;
                    let (start, end) = (u16_at(&ranges, idx * 6), u16_at(&ranges, idx * 6 + 2));
                    if gi >= start && gi <= end {
                        return Some(usize::from(
                            u16_at(&ranges, idx * 6 + 4)
                                .wrapping_add(gi)
                                .wrapping_sub(start),
                        ));
                    }
                }
                None
            }))
        }
        _ => Err(ERR_UNSUPPORTED_COVERAGE_FORMAT),
    }
}

fn make_class_lookup(src: &Source, offset: i64) -> Result<ClassLookup, Error> {
    let buf = src.view(offset, 2)?;
    match u16_at(buf, 0) {
        1 => {
            let (buf, _) = src.var_len_view(offset, 6, 4, 2)?;
            let start_gi = u16_at(buf, 2);
            let num = u16_at(buf, 4);
            let class_ids = buf[6..].to_vec();
            Ok(Box::new(move |gi| {
                if gi < start_gi || gi >= start_gi.wrapping_add(num) {
                    return 0;
                }
                i64::from(u16_at(
                    &class_ids,
                    (usize::from(gi) - usize::from(start_gi)) * 2,
                ))
            }))
        }
        2 => {
            let (buf, _) = src.var_len_view(offset, 4, 2, 6)?;
            let num = usize::from(u16_at(buf, 2));
            let class_ranges = buf[4..].to_vec();
            Ok(Box::new(move |gi| {
                if num == 0 {
                    return 0;
                }
                let mut idx = search(num, |i| gi <= u16_at(&class_ranges, i * 6));
                if idx < num && gi == u16_at(&class_ranges, idx * 6) {
                    return i64::from(u16_at(&class_ranges, idx * 6 + 4));
                }
                if idx > 0 {
                    idx -= 1;
                    let start = u16_at(&class_ranges, idx * 6);
                    let end = u16_at(&class_ranges, idx * 6 + 2);
                    if gi >= start && gi <= end {
                        return i64::from(u16_at(&class_ranges, idx * 6 + 4));
                    }
                }
                0
            }))
        }
        _ => Err(ERR_UNSUPPORTED_CLASS_DEF_FORMAT),
    }
}

/// Go `sort.Search`: the smallest index in [0, n) for which `f` is true.
fn search(n: usize, f: impl Fn(usize) -> bool) -> usize {
    let (mut i, mut j) = (0usize, n);
    while i < j {
        let h = (i + j) >> 1;
        if !f(h) {
            i = h + 1;
        } else {
            j = h;
        }
    }
    i
}
