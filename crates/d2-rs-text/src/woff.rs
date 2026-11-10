// SPDX-License-Identifier: MPL-2.0
//! Port of D2 v0.9.0 `lib/font/font.go` (`Sfnt2Woff`, itself a Go port of
//! the JavaScript library <https://github.com/fontello/ttf2woff>) and
//! `lib/font/woff_compression.go`.
//!
//! Each table is compressed as an independent zlib stream with Go's
//! `compress/zlib` default-level writer, exactly as
//! `woffCompression.compressTable` does: `Write(data)`, `Flush()`, `Close()`.
//! Byte-identical WOFF output therefore depends on a byte-exact port of Go's
//! encoder; [`sfnt2woff_with`] takes the table compressor as a parameter so
//! the container layout can be verified independently of it.

use d2_rs_core::stable_sort::StableSort;
use std::fmt;

// sfnt2Woff offset
const SFNT_OFFSET_TAG: usize = 0;
const SFNT_OFFSET_CHECKSUM: usize = 4;
const SFNT_OFFSET_OFFSET: usize = 8;
const SFNT_OFFSET_LENGTH: usize = 12;

// sfnt2Woff entry offset
const SFNT_ENTRY_OFFSET_FLAVOR: usize = 0;
const SFNT_ENTRY_OFFSET_VERSION_MAJ: usize = 4;
const SFNT_ENTRY_OFFSET_VERSION_MIN: usize = 6;
const SFNT_ENTRY_OFFSET_CHECKSUM_ADJUSTMENT: usize = 8;

// woff offset
const WOFF_OFFSET_MAGIC: usize = 0;
const WOFF_OFFSET_FLAVOR: usize = 4;
const WOFF_OFFSET_SIZE: usize = 8;
const WOFF_OFFSET_NUM_TABLES: usize = 12;
const WOFF_OFFSET_SFNT_SIZE: usize = 16;
const WOFF_OFFSET_VERSION_MAJ: usize = 20;
const WOFF_OFFSET_VERSION_MIN: usize = 22;
const WOFF_OFFSET_META_OFFSET: usize = 24;
const WOFF_OFFSET_META_LENGTH: usize = 28;
const WOFF_OFFSET_META_ORIG_LENGTH: usize = 32;
const WOFF_OFFSET_PRIV_OFFSET: usize = 36;
const WOFF_OFFSET_PRIV_LENGTH: usize = 40;

// woff entry offset
const WOFF_ENTRY_OFFSET_TAG: usize = 0;
const WOFF_ENTRY_OFFSET_OFFSET: usize = 4;
const WOFF_ENTRY_OFFSET_COMPR_LENGTH: usize = 8;
const WOFF_ENTRY_OFFSET_LENGTH: usize = 12;
const WOFF_ENTRY_OFFSET_CHECKSUM: usize = 16;

// magic
const MAGIC_WOFF: u32 = 0x774f_4646;
const MAGIC_CHECKSUM_ADJUSTMENT: u32 = 0xb1b0_afba;

// sizes
const SIZE_OF_WOFF_HEADER: usize = 44;
const SIZE_OF_WOFF_ENTRY: usize = 20;
const SIZE_OF_SFNT_HEADER: usize = 12;
const SIZE_OF_SFNT_TABLE_ENTRY: usize = 16;

/// Failure modes of `Sfnt2Woff`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sfnt2WoffError {
    /// Go: `fmt.Errorf("checksum error in table: %v", tag)`.
    Checksum { tag: [u8; 4] },
    /// Input too short for a field Go indexes; Go panics here.
    Malformed,
}

impl fmt::Display for Sfnt2WoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Sfnt2WoffError::Checksum { tag } => {
                write!(
                    f,
                    "checksum error in table: {}",
                    String::from_utf8_lossy(tag)
                )
            }
            Sfnt2WoffError::Malformed => f.write_str("malformed sfnt (Go would panic)"),
        }
    }
}

impl std::error::Error for Sfnt2WoffError {}

/// `TableEntry` (font.go).
#[derive(Clone, Copy)]
struct TableEntry {
    tag: [u8; 4],
    check_sum: u32,
    offset: u32,
    length: u32,
}

/// `longAlign`.
fn long_align(n: u32) -> u32 {
    n.wrapping_add(3) & !3
}

/// `calcChecksum`.
fn calc_checksum(buf: &[u8]) -> u32 {
    buf.as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .fold(0_u32, u32::wrapping_add)
}

fn be_u16(buf: &[u8], at: usize) -> Result<u16, Sfnt2WoffError> {
    buf.get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or(Sfnt2WoffError::Malformed)
}

fn be_u32(buf: &[u8], at: usize) -> Result<u32, Sfnt2WoffError> {
    buf.get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(Sfnt2WoffError::Malformed)
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn range(buf: &[u8], start: u32, end: u32) -> Result<std::ops::Range<usize>, Sfnt2WoffError> {
    let (start, end) = (start as usize, end as usize);
    if start > end || end > buf.len() {
        return Err(Sfnt2WoffError::Malformed);
    }
    Ok(start..end)
}

/// `woffCompression.compressTable` (woff_compression.go): an independent
/// zlib stream per table from Go's default-level `zlib.Writer` after
/// `Write`, `Flush` and `Close`. The explicit `Flush` emits a sync marker
/// before the final block, so the stream differs from `Write` + `Close`
/// (`d2_rs_goflate::zlib_compress_default`). The pool and buffer reuse in
/// the Go file do not affect the bytes: each table starts a fresh stream.
pub fn compress_table(data: &[u8]) -> Vec<u8> {
    let mut writer = d2_rs_goflate::zlib::Writer::new(Vec::new());
    // Go ignores these errors; writing to an in-memory buffer cannot fail.
    let _ = writer.write(data);
    let _ = writer.flush();
    let _ = writer.close();
    writer.into_inner()
}

/// `Sfnt2Woff` (lib/font/font.go) with Go's table compressor.
pub fn sfnt2woff(font_buf: &[u8]) -> Result<Vec<u8>, Sfnt2WoffError> {
    sfnt2woff_with(font_buf, compress_table)
}

/// `Sfnt2Woff` with an explicit table compressor (see [`compress_table`]).
///
/// Go writes the computed `checkSumAdjustment` into the caller's `head`
/// table before compressing it; this port applies that write to a private
/// copy, which yields the same output.
pub fn sfnt2woff_with(
    font_buf: &[u8],
    mut compress: impl FnMut(&[u8]) -> Vec<u8>,
) -> Result<Vec<u8>, Sfnt2WoffError> {
    let mut font_buf = font_buf.to_vec();
    let num_tables = be_u16(&font_buf, 4)?;

    let mut woff_header = vec![0_u8; SIZE_OF_WOFF_HEADER];
    put_u32(&mut woff_header, WOFF_OFFSET_MAGIC, MAGIC_WOFF);
    put_u16(&mut woff_header, WOFF_OFFSET_NUM_TABLES, num_tables);
    put_u16(&mut woff_header, WOFF_OFFSET_SFNT_SIZE, 0);
    put_u32(&mut woff_header, WOFF_OFFSET_META_OFFSET, 0);
    put_u32(&mut woff_header, WOFF_OFFSET_META_LENGTH, 0);
    put_u32(&mut woff_header, WOFF_OFFSET_META_ORIG_LENGTH, 0);
    put_u32(&mut woff_header, WOFF_OFFSET_PRIV_OFFSET, 0);
    put_u32(&mut woff_header, WOFF_OFFSET_PRIV_LENGTH, 0);

    let mut entries = Vec::with_capacity(num_tables as usize);
    for i in 0..num_tables as usize {
        let table = SIZE_OF_SFNT_HEADER + i * SIZE_OF_SFNT_TABLE_ENTRY;
        let tag = font_buf
            .get(table + SFNT_OFFSET_TAG..table + SFNT_OFFSET_TAG + 4)
            .ok_or(Sfnt2WoffError::Malformed)?;
        entries.push(TableEntry {
            tag: [tag[0], tag[1], tag[2], tag[3]],
            check_sum: be_u32(&font_buf, table + SFNT_OFFSET_CHECKSUM)?,
            offset: be_u32(&font_buf, table + SFNT_OFFSET_OFFSET)?,
            length: be_u32(&font_buf, table + SFNT_OFFSET_LENGTH)?,
        });
    }
    // Go uses the unstable sort.Slice; tags are unique in valid fonts, where
    // every sort yields the same order.
    entries.stable_sort_by_key(|a| a.tag);

    let mut sfnt_size =
        (SIZE_OF_SFNT_HEADER + num_tables as usize * SIZE_OF_SFNT_TABLE_ENTRY) as u32;
    let mut table_info = vec![0_u8; num_tables as usize * SIZE_OF_WOFF_ENTRY];

    for (i, table_entry) in entries.iter().enumerate() {
        if &table_entry.tag != b"head" {
            let end = table_entry
                .offset
                .wrapping_add(long_align(table_entry.length));
            let align_table = &font_buf[range(&font_buf, table_entry.offset, end)?];

            if calc_checksum(align_table) != table_entry.check_sum {
                return Err(Sfnt2WoffError::Checksum {
                    tag: table_entry.tag,
                });
            }
        }

        let base = i * SIZE_OF_WOFF_ENTRY;
        put_u32(
            &mut table_info,
            base + WOFF_ENTRY_OFFSET_TAG,
            u32::from_be_bytes(table_entry.tag),
        );
        put_u32(
            &mut table_info,
            base + WOFF_ENTRY_OFFSET_LENGTH,
            table_entry.length,
        );
        put_u32(
            &mut table_info,
            base + WOFF_ENTRY_OFFSET_CHECKSUM,
            table_entry.check_sum,
        );

        sfnt_size = sfnt_size.wrapping_add(long_align(table_entry.length));
    }

    let mut sfnt_offset = (SIZE_OF_SFNT_HEADER + entries.len() * SIZE_OF_SFNT_TABLE_ENTRY) as u32;
    let mut csum = calc_checksum(
        font_buf
            .get(..SIZE_OF_SFNT_HEADER)
            .ok_or(Sfnt2WoffError::Malformed)?,
    );
    for table_entry in &entries {
        let mut b = [0_u8; SIZE_OF_SFNT_TABLE_ENTRY];
        put_u32(&mut b, SFNT_OFFSET_TAG, u32::from_be_bytes(table_entry.tag));
        put_u32(&mut b, SFNT_OFFSET_CHECKSUM, table_entry.check_sum);
        put_u32(&mut b, SFNT_OFFSET_OFFSET, sfnt_offset);
        put_u32(&mut b, SFNT_OFFSET_LENGTH, table_entry.length);

        sfnt_offset = sfnt_offset.wrapping_add(long_align(table_entry.length));
        csum = csum.wrapping_add(calc_checksum(&b));
        csum = csum.wrapping_add(table_entry.check_sum);
    }

    let checksum_adjustment = MAGIC_CHECKSUM_ADJUSTMENT.wrapping_sub(csum);

    let mut major_version = 0_u16;
    let mut min_version = 1_u16;
    let mut flavor = 0_u32;
    let mut offset = SIZE_OF_WOFF_HEADER + num_tables as usize * SIZE_OF_WOFF_ENTRY;
    let mut table_bytes = Vec::new();

    for (i, table_entry) in entries.iter().enumerate() {
        let sfnt_range = range(
            &font_buf,
            table_entry.offset,
            table_entry.offset.wrapping_add(table_entry.length),
        )?;
        if &table_entry.tag == b"head" {
            let sfnt_data = &mut font_buf[sfnt_range.clone()];
            major_version = be_u16(sfnt_data, SFNT_ENTRY_OFFSET_VERSION_MAJ)?;
            min_version = be_u16(sfnt_data, SFNT_ENTRY_OFFSET_VERSION_MIN)?;
            flavor = be_u32(sfnt_data, SFNT_ENTRY_OFFSET_FLAVOR)?;
            if sfnt_data.len() < SFNT_ENTRY_OFFSET_CHECKSUM_ADJUSTMENT + 4 {
                return Err(Sfnt2WoffError::Malformed);
            }
            put_u32(
                sfnt_data,
                SFNT_ENTRY_OFFSET_CHECKSUM_ADJUSTMENT,
                checksum_adjustment,
            );
        }
        let sfnt_data = &font_buf[sfnt_range];

        let compressed = compress(sfnt_data);

        // Go computes math.Min over float64 values and truncates to uint32.
        let comp_length = compressed.len().min(sfnt_data.len()) as u32;
        let length = long_align(comp_length) as usize;

        let mut table = vec![0_u8; length];
        // only deflate data if the deflated data is actually smaller
        if compressed.len() >= sfnt_data.len() {
            table[..sfnt_data.len()].copy_from_slice(sfnt_data);
        } else {
            table[..compressed.len()].copy_from_slice(&compressed);
        }

        let base = i * SIZE_OF_WOFF_ENTRY;
        put_u32(
            &mut table_info,
            base + WOFF_ENTRY_OFFSET_OFFSET,
            offset as u32,
        );

        offset += table.len();

        put_u32(
            &mut table_info,
            base + WOFF_ENTRY_OFFSET_COMPR_LENGTH,
            comp_length,
        );

        table_bytes.extend_from_slice(&table);
    }

    let woff_size = (woff_header.len() + table_info.len() + table_bytes.len()) as u32;
    put_u32(&mut woff_header, WOFF_OFFSET_SIZE, woff_size);
    put_u32(&mut woff_header, WOFF_OFFSET_SFNT_SIZE, sfnt_size);
    put_u16(&mut woff_header, WOFF_OFFSET_VERSION_MAJ, major_version);
    put_u16(&mut woff_header, WOFF_OFFSET_VERSION_MIN, min_version);
    put_u32(&mut woff_header, WOFF_OFFSET_FLAVOR, flavor);

    let mut out = woff_header;
    out.extend_from_slice(&table_info);
    out.extend_from_slice(&table_bytes);
    Ok(out)
}
