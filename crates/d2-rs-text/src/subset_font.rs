// SPDX-License-Identifier: MPL-2.0
//! Literal port of D2 v0.9.0 `lib/font/subsetFont.go` (`font.UTF8CutFont`).
//!
//! The Go file is derived from `github.com/jung-kurt/gofpdf`; its notice is
//! retained below and in `THIRD_PARTY_NOTICES.md`:
//!
//! ```text
//! The following code is part of https://github.com/jung-kurt/gofpdf
//!
//! Copyright (c) 2019 Arteom Korotkiy (Gmail: arteomkorotkiy)
//!
//! Permission to use, copy, modify, and distribute this software for any
//! purpose with or without fee is hereby granted, provided that the above
//! copyright notice and this permission notice appear in all copies.
//!
//! THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
//! WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
//! MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
//! ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
//! WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
//! ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
//! OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//! ```
//!
//! Structure and names follow the Go source function by function. Go `int`
//! is modelled as `i64`. Where Go would panic (out-of-range slicing, a nil
//! table description, a missing `post` table) the Rust port returns `None`;
//! where Go returns a nil font (no Unicode format-4 cmap) it also returns
//! `None`. Both outcomes abort Go's `GetEncodedSubset`.
//!
//! Go slices returned by `fileReader.Read` alias the input buffer and keep
//! its spare capacity, so two `append` calls in the Go code write into the
//! (copied) input font: `getMetrics` for glyphs past `numberOfHMetrics`, and
//! `generateChecksum` padding of tables that are still input slices. Both
//! writes are reproduced here ([`OutTable::Input`], [`Utf8FontFile::get_metrics`])
//! so later reads observe the same bytes Go observes.

use std::collections::BTreeMap;

// flags (subsetFont.go)
const SYMBOL_WORDS: i64 = 1 << 0;
const SYMBOL_SCALE: i64 = 1 << 3;
const SYMBOL_CONTINUE: i64 = 1 << 5;
const SYMBOL_ALL_SCALE: i64 = 1 << 6;
const SYMBOL_2X2: i64 = 1 << 7;

/// `fileReader` (subsetFont.go). `array` is the caller's private copy of the
/// font; Go mutates it through aliasing appends.
struct FileReader {
    reader_position: i64,
    array: Vec<u8>,
}

impl FileReader {
    /// `fileReader.Read`: returns the aliased range `[start, end)`.
    fn read(&mut self, s: i64) -> Option<(usize, usize)> {
        let start = usize::try_from(self.reader_position).ok()?;
        let end = start.checked_add(usize::try_from(s).ok()?)?;
        if end > self.array.len() {
            return None;
        }
        self.reader_position += s;
        Some((start, end))
    }

    fn read_bytes(&mut self, s: i64) -> Option<&[u8]> {
        let (start, end) = self.read(s)?;
        Some(&self.array[start..end])
    }
}

/// `tableDescription` (subsetFont.go).
#[derive(Clone, Copy)]
struct TableDescription {
    position: i64,
    size: i64,
}

/// An entry of `utf8FontFile.outTablesData`. Go stores either an input slice
/// (aliasing the font buffer up to its end) or a freshly allocated slice.
enum OutTable {
    Input { start: usize, len: usize },
    Owned(Vec<u8>),
}

impl OutTable {
    fn bytes<'a>(&'a self, array: &'a [u8]) -> &'a [u8] {
        match self {
            OutTable::Input { start, len } => &array[*start..*start + *len],
            OutTable::Owned(data) => data,
        }
    }
}

/// `locaOffsets` (subsetFont.go).
#[derive(Clone, Default)]
struct LocaOffsets {
    data: Vec<u8>,
    width: i64,
}

impl LocaOffsets {
    /// `locaOffsets.at`, including its glyph-index bounds check.
    fn at(&self, index: i64) -> Option<i64> {
        let count = if self.width != 0 {
            self.data.len() as i64 / self.width
        } else {
            0
        };
        if index < 0 || index >= count {
            return None;
        }
        let offset = (index * self.width) as usize;
        if self.width == 2 {
            let value = u16::from_be_bytes(self.data.get(offset..offset + 2)?.try_into().ok()?);
            return Some(value as i64 * 2);
        }
        let value = u32::from_be_bytes(self.data.get(offset..offset + 4)?.try_into().ok()?);
        Some(value as i64)
    }
}

/// `utf8FontFile` (subsetFont.go).
struct Utf8FontFile {
    file_reader: FileReader,
    table_descriptions: BTreeMap<Vec<u8>, TableDescription>,
    out_tables_data: BTreeMap<String, OutTable>,
    symbol_offsets: LocaOffsets,
    char_symbol_dictionary: BTreeMap<i64, i64>,
    fpdf_cmap_length: bool,
}

/// `newUTF8Font` (subsetFont.go).
fn new_utf8_font(reader: FileReader) -> Utf8FontFile {
    Utf8FontFile {
        file_reader: reader,
        table_descriptions: BTreeMap::new(),
        out_tables_data: BTreeMap::new(),
        symbol_offsets: LocaOffsets::default(),
        char_symbol_dictionary: BTreeMap::new(),
        fpdf_cmap_length: false,
    }
}

/// Results of `parseSymbols`: `(cidSymbolPairCollection, symbolArray,
/// symbolCollection, symbolCollectionKeys)`.
type ParsedSymbols = (
    BTreeMap<i64, i64>,
    BTreeMap<i64, i64>,
    BTreeMap<i64, i64>,
    Vec<i64>,
);

impl Utf8FontFile {
    /// `generateTableDescriptions`.
    fn generate_table_descriptions(&mut self) -> Option<()> {
        let tables_count = self.read_uint16()?;
        let _ = self.read_uint16()?;
        let _ = self.read_uint16()?;
        let _ = self.read_uint16()?;
        self.table_descriptions = BTreeMap::new();

        for _ in 0..tables_count {
            let name = self.file_reader.read_bytes(4)?.to_vec();
            self.skip(4); // Skip the input table checksum; output checksums are recalculated.
            let position = self.read_uint32()?;
            let size = self.read_uint32()?;
            self.table_descriptions
                .insert(name, TableDescription { position, size });
        }
        Some(())
    }

    /// `readUint16`.
    fn read_uint16(&mut self) -> Option<i64> {
        let s = self.file_reader.read_bytes(2)?;
        Some(((s[0] as i64) << 8) + s[1] as i64)
    }

    /// `readUint32`.
    fn read_uint32(&mut self) -> Option<i64> {
        let s = self.file_reader.read_bytes(4)?;
        Some((s[0] as i64) * 16777216 + ((s[1] as i64) << 16) + ((s[2] as i64) << 8) + s[3] as i64)
    }

    /// `calcInt32`.
    fn calc_int32(&self, mut x: [i64; 2], y: [i64; 2]) -> [i64; 2] {
        let mut answer = [0_i64; 2];
        if y[1] > x[1] {
            x[1] += 1 << 16;
            x[0] += 1;
        }
        answer[1] = x[1] - y[1];
        if y[0] > x[0] {
            x[0] += 1 << 16;
        }
        answer[0] = x[0] - y[0];
        answer[0] &= 0xFFFF;
        answer
    }

    /// `generateChecksum` for a table. Go pads `data` with `append`; for an
    /// input slice that writes zeros into the font buffer's spare capacity.
    fn generate_table_checksum(&mut self, name: &str) -> [i64; 2] {
        let mut padded = self.out_tables_data[name]
            .bytes(&self.file_reader.array)
            .to_vec();
        if let OutTable::Input { start, len } = self.out_tables_data[name] {
            let end = start + len;
            let padding = (4 - len % 4) % 4;
            let available = self.file_reader.array.len() - end;
            for byte in &mut self.file_reader.array[end..end + padding.min(available)] {
                *byte = 0;
            }
        }
        while !padded.len().is_multiple_of(4) {
            padded.push(0);
        }
        generate_checksum(&padded)
    }

    /// `seek`.
    fn seek(&mut self, shift: i64) {
        self.file_reader.reader_position = shift;
    }

    /// `skip`.
    fn skip(&mut self, delta: i64) {
        self.file_reader.reader_position += delta;
    }

    /// `SeekTable`.
    fn seek_table(&mut self, name: &[u8]) -> Option<i64> {
        let position = self.table_descriptions.get(name)?.position;
        self.seek(position);
        Some(self.file_reader.reader_position)
    }

    /// `readInt16`.
    fn read_int16(&mut self) -> Option<i16> {
        let s = self.file_reader.read_bytes(2)?;
        Some(i16::from_be_bytes([s[0], s[1]]))
    }

    /// `getUint16`.
    fn get_uint16(&mut self, pos: i64) -> Option<i64> {
        self.seek(pos);
        let s = self.file_reader.read_bytes(2)?;
        Some(((s[0] as i64) << 8) + s[1] as i64)
    }

    /// `getTableData`: `None` for a missing or empty table (Go's nil).
    fn get_table_data(&mut self, name: &[u8]) -> Option<Option<(usize, usize)>> {
        let Some(description) = self.table_descriptions.get(name).copied() else {
            return Some(None);
        };
        if description.size == 0 {
            return Some(None);
        }
        self.seek(description.position);
        self.file_reader.read(description.size).map(Some)
    }

    fn get_table_bytes(&mut self, name: &[u8]) -> Option<Option<Vec<u8>>> {
        Some(
            self.get_table_data(name)?
                .map(|(start, end)| self.file_reader.array[start..end].to_vec()),
        )
    }

    /// `setOutTable`.
    fn set_out_table(&mut self, name: &str, data: Option<OutTable>) {
        let Some(mut data) = data else {
            return;
        };
        if name == "head" {
            let bytes = data.bytes(&self.file_reader.array).to_vec();
            // Go panics for a head table shorter than 12 bytes; such data
            // cannot reach here because `insertUint16(head, 50, ..)` failed.
            data = OutTable::Owned(splice(&bytes, 8, &[0, 0, 0, 0]).unwrap_or(bytes));
        }
        self.out_tables_data.insert(name.to_owned(), data);
    }

    fn set_out_table_from_input(&mut self, name: &str) -> Option<()> {
        let data = self
            .get_table_data(name.as_bytes())?
            .map(|(start, end)| OutTable::Input {
                start,
                len: end - start,
            });
        self.set_out_table(name, data);
        Some(())
    }

    /// `generateCMAP`.
    fn generate_cmap(&mut self) -> Option<Option<BTreeMap<i64, i64>>> {
        let cmap_position = self.seek_table(b"cmap")?;
        self.skip(2);
        let cmap_table_count = self.read_uint16()?;
        let mut rune_cmap_position = 0;
        for _ in 0..cmap_table_count {
            let system = self.read_uint16()?;
            let coder = self.read_uint16()?;
            let position = self.read_uint32()?;
            let old_position = self.file_reader.reader_position;
            if (system == 3 && coder == 1) || system == 0 {
                let format = self.get_uint16(cmap_position + position)?;
                if format == 4 {
                    rune_cmap_position = cmap_position + position;
                    break;
                }
            }
            self.seek(old_position);
        }

        if rune_cmap_position == 0 {
            // Go prints "Font does not have cmap for Unicode" and returns nil.
            return Some(None);
        }

        let mut char_symbol_dictionary = BTreeMap::new();
        self.generate_char_symbol_dictionary(rune_cmap_position, &mut char_symbol_dictionary)?;

        Some(Some(char_symbol_dictionary))
    }

    /// `parseSymbols`.
    fn parse_symbols(&mut self, used_runes: &[i64]) -> Option<ParsedSymbols> {
        let mut symbol_collection = BTreeMap::from([(0_i64, 0_i64)]);
        let mut char_symbol_pair_collection = BTreeMap::new();
        for &char in used_runes {
            if let Some(&symbol) = self.char_symbol_dictionary.get(&char) {
                symbol_collection.insert(symbol, char);
                char_symbol_pair_collection.insert(char, symbol);
            }
        }

        let begin = self.table_descriptions.get(b"glyf".as_slice())?.position;

        let mut symbol_array = BTreeMap::new();
        let mut symbol_collection_keys: Vec<i64> = symbol_collection.keys().copied().collect();

        for (symbol_counter, &old_symbol_index) in symbol_collection_keys.iter().enumerate() {
            symbol_array.insert(old_symbol_index, symbol_counter as i64);
        }
        let mut rune_symbol_pair_collection = BTreeMap::new();
        for (&runa, symbol) in &char_symbol_pair_collection {
            rune_symbol_pair_collection.insert(runa, *symbol_array.get(symbol).unwrap_or(&0));
        }

        // Go ranges over the slice value captured before the loop, while the
        // recursive calls append composite components to the returned keys.
        let top_level_keys = symbol_collection_keys.clone();
        for old_symbol_index in top_level_keys {
            self.get_symbols(
                old_symbol_index,
                begin,
                &mut symbol_array,
                &mut symbol_collection,
                &mut symbol_collection_keys,
            )?;
        }

        Some((
            rune_symbol_pair_collection,
            symbol_array,
            symbol_collection,
            symbol_collection_keys,
        ))
    }

    /// `generateCMAPTable`.
    fn generate_cmap_table(
        &self,
        cid_symbol_pair_collection: &BTreeMap<i64, i64>,
        num_symbols: i64,
    ) -> Vec<u8> {
        let mut cid_id = 0_i64;
        let mut cid_array: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        let mut prev_cid = -2_i64;
        let mut prev_symbol = -1_i64;
        for (&cid, &symbol) in cid_symbol_pair_collection {
            if cid == prev_cid + 1 && symbol == prev_symbol + 1 {
                cid_array.entry(cid_id).or_default().push(symbol);
            } else {
                cid_id = cid;
                cid_array.insert(cid_id, vec![symbol]);
            }
            prev_cid = cid;
            prev_symbol = symbol;
        }
        let cid_array_keys: Vec<i64> = cid_array.keys().copied().collect();
        let seg_count = cid_array.len() as i64 + 1;

        let mut search_range = 1_i64;
        let mut entry_selector = 0_i64;
        while search_range * 2 <= seg_count {
            search_range *= 2;
            entry_selector += 1;
        }
        search_range *= 2;
        let range_shift = seg_count * 2 - search_range;

        let mut data: Vec<i64> = vec![0, 1, 3, 1, 0, 12, 4];

        let mut cmap: Vec<i64> = vec![seg_count * 2, search_range, entry_selector, range_shift];
        for start in &cid_array_keys {
            let end_code = start + (cid_array[start].len() as i64 - 1);
            cmap.push(end_code);
        }
        cmap.push(0xFFFF);
        cmap.push(0);

        cmap.extend_from_slice(&cid_array_keys);
        cmap.push(0xFFFF);
        for cid_key in &cid_array_keys {
            let id_delta = -(cid_key - cid_array[cid_key][0]);
            cmap.push(id_delta);
        }
        cmap.push(1);
        cmap.extend(std::iter::repeat_n(0, cid_array.len()));
        cmap.push(0);
        for start in &cid_array_keys {
            cmap.extend_from_slice(&cid_array[start]);
        }
        cmap.push(0);

        // The 3 extra fields are format, length and language, which are also
        // part of the cmap subtable.
        let length = if self.fpdf_cmap_length {
            // The PDF dependency retains the older formula. D2's SVG font
            // helper corrects it; preserve each caller's source contract.
            16 + 8 * seg_count + num_symbols + 1
        } else {
            (3 + cmap.len() as i64) * 2
        };
        data.push(length);
        data.push(0);
        data.extend_from_slice(&cmap);

        let mut cmapstr = Vec::with_capacity(data.len() * 2);
        for cm in data {
            cmapstr.extend_from_slice(&pack_uint16(cm));
        }
        cmapstr
    }

    /// `GenerateCutFont`.
    fn generate_cut_font(&mut self, used_runes: &[i64]) -> Option<Vec<u8>> {
        self.file_reader.reader_position = 0;
        self.out_tables_data = BTreeMap::new();
        self.skip(4);
        self.generate_table_descriptions()?;

        self.seek_table(b"head")?;
        self.skip(50);
        let mut loca_format = self.read_uint16()?;

        self.seek_table(b"hhea")?;
        self.skip(34);
        let mut metrics_count = self.read_uint16()?;
        let old_metrics = metrics_count;

        self.seek_table(b"maxp")?;
        self.skip(4);
        let mut num_symbols = self.read_uint16()?;

        self.char_symbol_dictionary = self.generate_cmap()??;

        self.parse_loca_table(loca_format, num_symbols)?;

        let (
            mut cid_symbol_pair_collection,
            symbol_array,
            symbol_collection,
            symbol_collection_keys,
        ) = self.parse_symbols(used_runes)?;

        metrics_count = symbol_collection.len() as i64;
        num_symbols = metrics_count;

        self.set_out_table_from_input("name")?;
        self.set_out_table_from_input("cvt ")?;
        self.set_out_table_from_input("fpgm")?;
        self.set_out_table_from_input("prep")?;
        self.set_out_table_from_input("gasp")?;

        // `postTable[4:16]` may read past the table into the input slice's
        // capacity, but panics for a nil (missing or empty) table.
        let (post_start, _) = self.get_table_data(b"post")??;
        let mut post_table = vec![0x00, 0x03, 0x00, 0x00];
        post_table.extend_from_slice(
            self.file_reader
                .array
                .get(post_start + 4..post_start + 16)?,
        );
        post_table.extend_from_slice(&[0; 16]);
        self.set_out_table("post", Some(OutTable::Owned(post_table)));

        cid_symbol_pair_collection.remove(&0);

        let cmap = self.generate_cmap_table(&cid_symbol_pair_collection, num_symbols);
        self.set_out_table("cmap", Some(OutTable::Owned(cmap)));

        // `symbolData` aliases the input; Go may slice it up to its capacity
        // (the end of the font buffer), and later reads see earlier writes.
        let symbol_data_start = self.get_table_data(b"glyf")?.map(|(start, _)| start);

        let mut offsets: Vec<i64> = Vec::new();
        let mut glyf_data: Vec<u8> = Vec::new();
        let mut pos = 0_i64;
        let mut hmtx_data: Vec<u8> = Vec::new();

        for &original_symbol_idx in &symbol_collection_keys {
            let hm = self.get_metrics(old_metrics, original_symbol_idx)?;
            hmtx_data.extend_from_slice(&hm);

            offsets.push(pos);
            let symbol_pos = self.symbol_offsets.at(original_symbol_idx)?;
            let symbol_len = self.symbol_offsets.at(original_symbol_idx + 1)? - symbol_pos;
            let symbol_end = symbol_pos + symbol_len;
            if symbol_pos < 0 || symbol_end < symbol_pos {
                return None;
            }
            let (data_start, data_capacity_end) = match symbol_data_start {
                Some(start) => (start + symbol_pos as usize, self.file_reader.array.len()),
                // A nil slice only admits empty slicing.
                None if symbol_end == 0 => (0, 0),
                None => return None,
            };
            if data_start + symbol_len as usize > data_capacity_end {
                return None;
            }
            let mut data =
                self.file_reader.array[data_start..data_start + symbol_len as usize].to_vec();
            let mut up = 0;
            if symbol_len > 0 {
                // `data[0:2]` may extend into the input slice's capacity.
                up = unpack_uint16(self.file_reader.array.get(data_start..data_start + 2)?);
            }

            if symbol_len > 2 && (up & (1 << 15)) != 0 {
                let mut pos_in_symbol = 10_usize;
                let mut flags = SYMBOL_CONTINUE;
                while (flags & SYMBOL_CONTINUE) != 0 {
                    up = unpack_uint16(data.get(pos_in_symbol..pos_in_symbol + 2)?);
                    flags = up;
                    up = unpack_uint16(data.get(pos_in_symbol + 2..pos_in_symbol + 4)?);
                    let symbol_idx = up;
                    // A missing map entry reads as Go's zero value.
                    let new_idx = *symbol_array.get(&symbol_idx).unwrap_or(&0);
                    data = self.insert_uint16(&data, pos_in_symbol + 2, new_idx)?;
                    pos_in_symbol += 4;
                    if (flags & SYMBOL_WORDS) != 0 {
                        pos_in_symbol += 4;
                    } else {
                        pos_in_symbol += 2;
                    }
                    if (flags & SYMBOL_SCALE) != 0 {
                        pos_in_symbol += 2;
                    } else if (flags & SYMBOL_ALL_SCALE) != 0 {
                        pos_in_symbol += 4;
                    } else if (flags & SYMBOL_2X2) != 0 {
                        pos_in_symbol += 8;
                    }
                }
            }

            glyf_data.extend_from_slice(&data);
            pos += symbol_len;
            if pos % 4 != 0 {
                let padding = 4 - (pos % 4);
                glyf_data.resize(glyf_data.len() + padding as usize, 0);
                pos += padding;
            }
        }

        offsets.push(pos);
        self.set_out_table("glyf", Some(OutTable::Owned(glyf_data)));

        self.set_out_table("hmtx", Some(OutTable::Owned(hmtx_data)));

        let mut loca_data = Vec::new();
        if ((pos + 1) >> 1) > 0xFFFF {
            loca_format = 1;
            for offset in &offsets {
                loca_data.extend_from_slice(&pack_uint32(*offset));
            }
        } else {
            loca_format = 0;
            for offset in &offsets {
                loca_data.extend_from_slice(&pack_uint16(offset / 2));
            }
        }
        self.set_out_table("loca", Some(OutTable::Owned(loca_data)));

        let head_data = self.get_table_bytes(b"head")?.unwrap_or_default();
        let head_data = self.insert_uint16(&head_data, 50, loca_format)?;
        self.set_out_table("head", Some(OutTable::Owned(head_data)));

        let hhea_data = self.get_table_bytes(b"hhea")?.unwrap_or_default();
        let hhea_data = self.insert_uint16(&hhea_data, 34, metrics_count)?;
        self.set_out_table("hhea", Some(OutTable::Owned(hhea_data)));

        let maxp = self.get_table_bytes(b"maxp")?.unwrap_or_default();
        let maxp = self.insert_uint16(&maxp, 4, num_symbols)?;
        self.set_out_table("maxp", Some(OutTable::Owned(maxp)));

        self.set_out_table_from_input("OS/2")?;

        self.assemble_tables()
    }

    /// `insertUint16`.
    fn insert_uint16(&self, stream: &[u8], offset: usize, value: i64) -> Option<Vec<u8>> {
        splice(stream, offset, &(value as u16).to_be_bytes())
    }

    /// `getSymbols`. `symbol_set` is `symbolArray` and `symbols_collection` is
    /// `symbolCollection`; both are Go maps shared by reference.
    fn get_symbols(
        &mut self,
        original_symbol_idx: i64,
        start: i64,
        symbol_set: &mut BTreeMap<i64, i64>,
        symbols_collection: &mut BTreeMap<i64, i64>,
        symbols_collection_keys: &mut Vec<i64>,
    ) -> Option<()> {
        let symbol_pos = self.symbol_offsets.at(original_symbol_idx)?;
        let symbol_size = self.symbol_offsets.at(original_symbol_idx + 1)? - symbol_pos;
        if symbol_size == 0 {
            return Some(());
        }
        self.seek(start + symbol_pos);

        let line_count = self.read_int16()?;

        if line_count < 0 {
            self.skip(8);
            let mut flags = SYMBOL_CONTINUE;
            while flags & SYMBOL_CONTINUE != 0 {
                flags = self.read_uint16()?;
                let symbol_index = self.read_uint16()?;
                if let std::collections::btree_map::Entry::Vacant(e) =
                    symbol_set.entry(symbol_index)
                {
                    e.insert(symbols_collection.len() as i64);
                    symbols_collection.insert(symbol_index, 1);
                    symbols_collection_keys.push(symbol_index);
                }
                let old_position = self.file_reader.reader_position;
                self.get_symbols(
                    symbol_index,
                    start,
                    symbol_set,
                    symbols_collection,
                    symbols_collection_keys,
                )?;
                self.seek(old_position);
                if flags & SYMBOL_WORDS != 0 {
                    self.skip(4);
                } else {
                    self.skip(2);
                }
                if flags & SYMBOL_SCALE != 0 {
                    self.skip(2);
                } else if flags & SYMBOL_ALL_SCALE != 0 {
                    self.skip(4);
                } else if flags & SYMBOL_2X2 != 0 {
                    self.skip(8);
                }
            }
        }
        Some(())
    }

    /// `getMetrics`. For `gid >= metricCount`, Go's
    /// `append(Read(2), Read(2)...)` writes the left side bearing into the
    /// input buffer directly after the last long metric's advance width.
    fn get_metrics(&mut self, metric_count: i64, gid: i64) -> Option<Vec<u8>> {
        let start = self.seek_table(b"hmtx")?;
        if gid < metric_count {
            self.seek(start + gid * 4);
            return Some(self.file_reader.read_bytes(4)?.to_vec());
        }
        self.seek(start + (metric_count - 1) * 4);
        let (advance_start, advance_end) = self.file_reader.read(2)?;
        self.seek(start + metric_count * 2 + gid * 2);
        let (lsb_start, lsb_end) = self.file_reader.read(2)?;
        let lsb = [
            self.file_reader.array[lsb_start],
            self.file_reader.array[lsb_end - 1],
        ];
        // The aliasing append fits in the buffer's capacity (cap == len).
        if advance_end + 2 > self.file_reader.array.len() {
            let mut metrics = self.file_reader.array[advance_start..advance_end].to_vec();
            metrics.extend_from_slice(&lsb);
            return Some(metrics);
        }
        self.file_reader.array[advance_end..advance_end + 2].copy_from_slice(&lsb);
        Some(self.file_reader.array[advance_start..advance_end + 2].to_vec())
    }

    /// `parseLOCATable`.
    fn parse_loca_table(&mut self, format: i64, num_symbols: i64) -> Option<()> {
        let start = self.seek_table(b"loca")?;
        self.symbol_offsets = LocaOffsets::default();
        let width = match format {
            0 => 2,
            1 => 4,
            _ => {
                // Go prints "Unknown loca table format %d" and leaves the
                // offsets empty; the first glyph lookup then panics.
                return Some(());
            }
        };
        let data = self.get_range(start, (num_symbols + 1) * width)?;
        self.symbol_offsets = LocaOffsets { data, width };
        Some(())
    }

    /// `getRange`.
    fn get_range(&mut self, pos: i64, length: i64) -> Option<Vec<u8>> {
        self.seek(pos);
        if length < 1 {
            return Some(Vec::new());
        }
        Some(self.file_reader.read_bytes(length)?.to_vec())
    }

    /// `generateCharSymbolDictionary`.
    fn generate_char_symbol_dictionary(
        &mut self,
        rune_cmap_position: i64,
        char_symbol_dictionary: &mut BTreeMap<i64, i64>,
    ) -> Option<()> {
        self.seek(rune_cmap_position + 2);
        let size = self.read_uint16()?;
        let rim = rune_cmap_position + size;
        self.skip(2);

        let segment_size = self.read_uint16()? / 2;
        self.skip(6);
        let mut completers = Vec::new();
        for _ in 0..segment_size {
            completers.push(self.read_uint16()?);
        }
        self.skip(2);
        let mut beginners = Vec::new();
        for _ in 0..segment_size {
            beginners.push(self.read_uint16()?);
        }
        let mut sizes = Vec::new();
        for _ in 0..segment_size {
            sizes.push(self.read_int16()? as i64);
        }
        let reader_position_start = self.file_reader.reader_position;
        let mut positions = Vec::new();
        for _ in 0..segment_size {
            positions.push(self.read_uint16()?);
        }
        for n in 0..segment_size as usize {
            let complete_position = completers[n] + 1;
            let mut char = beginners[n];
            while char < complete_position {
                let symbol = if positions[n] == 0 {
                    (char + sizes[n]) & 0xFFFF
                } else {
                    let position = (char - beginners[n]) * 2 + positions[n];
                    let position = reader_position_start + 2 * n as i64 + position;
                    if position >= rim {
                        0
                    } else {
                        let symbol = self.get_uint16(position)?;
                        if symbol != 0 {
                            (symbol + sizes[n]) & 0xFFFF
                        } else {
                            symbol
                        }
                    }
                };
                char_symbol_dictionary.insert(char, symbol);
                char += 1;
            }
        }
        Some(())
    }

    /// `assembleTables`.
    fn assemble_tables(&mut self) -> Option<Vec<u8>> {
        let mut answer = Vec::new();
        let tables_count = self.out_tables_data.len() as i64;
        let mut find_size = 1_i64;
        let mut writer = 0_i64;
        while find_size * 2 <= tables_count {
            find_size *= 2;
            writer += 1;
        }
        find_size *= 16;
        let r_offset = tables_count * 16 - find_size;

        answer.extend_from_slice(&pack_header(
            0x0001_0000,
            tables_count,
            find_size,
            writer,
            r_offset,
        ));

        // keySortStrings: Go's sort.Strings is a byte-wise order, like BTreeMap<String>.
        let tables_names: Vec<String> = self.out_tables_data.keys().cloned().collect();

        let mut offset = 12 + tables_count * 16;
        let mut begin = 0_i64;

        for name in &tables_names {
            if name == "head" {
                begin = offset;
            }
            answer.extend_from_slice(name.as_bytes());
            let checksum = self.generate_table_checksum(name);
            answer.extend_from_slice(&pack2_uint16(checksum[0], checksum[1]));
            let length = self.out_tables_data[name]
                .bytes(&self.file_reader.array)
                .len() as i64;
            answer.extend_from_slice(&pack2_uint32(offset, length));
            let padded_length = (length + 3) & !3;
            offset += padded_length;
        }

        for key in &tables_names {
            let mut data = self.out_tables_data[key]
                .bytes(&self.file_reader.array)
                .to_vec();
            data.extend_from_slice(&[0, 0, 0]);
            let aligned = data.len() & !3;
            answer.extend_from_slice(&data[..aligned]);
        }

        let checksum = generate_checksum(&answer);
        let checksum = self.calc_int32([0xB1B0, 0xAFBA], checksum);
        let begin = usize::try_from(begin + 8).ok()?;
        splice(&answer, begin, &pack2_uint16(checksum[0], checksum[1]))
    }
}

/// `generateChecksum` over data already padded to a multiple of four bytes.
fn generate_checksum(data: &[u8]) -> [i64; 2] {
    let mut answer = [0x0000_i64, 0x0000];
    for chunk in data.as_chunks::<4>().0 {
        answer[0] += ((chunk[0] as i64) << 8) + chunk[1] as i64;
        answer[1] += ((chunk[2] as i64) << 8) + chunk[3] as i64;
        answer[0] += answer[1] >> 16;
        answer[1] &= 0xFFFF;
        answer[0] &= 0xFFFF;
    }
    answer
}

/// `splice`: copies `stream`, overwriting `value.len()` bytes at `offset`.
/// Go panics when the overwritten range runs past the end of `stream`.
fn splice(stream: &[u8], offset: usize, value: &[u8]) -> Option<Vec<u8>> {
    if offset + value.len() > stream.len() {
        return None;
    }
    let mut out = stream.to_vec();
    out[offset..offset + value.len()].copy_from_slice(value);
    Some(out)
}

/// `unpackUint16`.
fn unpack_uint16(data: &[u8]) -> i64 {
    u16::from_be_bytes([data[0], data[1]]) as i64
}

/// `packHeader`.
fn pack_header(n: u32, n1: i64, n2: i64, n3: i64, n4: i64) -> Vec<u8> {
    let mut answer = n.to_be_bytes().to_vec();
    for value in [n1, n2, n3, n4] {
        answer.extend_from_slice(&(value as u16).to_be_bytes());
    }
    answer
}

/// `pack2Uint16`.
fn pack2_uint16(n1: i64, n2: i64) -> [u8; 4] {
    let a = (n1 as u16).to_be_bytes();
    let b = (n2 as u16).to_be_bytes();
    [a[0], a[1], b[0], b[1]]
}

/// `pack2Uint32`.
fn pack2_uint32(n1: i64, n2: i64) -> [u8; 8] {
    let mut out = [0; 8];
    out[..4].copy_from_slice(&(n1 as u32).to_be_bytes());
    out[4..].copy_from_slice(&(n2 as u32).to_be_bytes());
    out
}

/// `packUint32`.
fn pack_uint32(n1: i64) -> [u8; 4] {
    (n1 as u32).to_be_bytes()
}

/// `packUint16`.
fn pack_uint16(n1: i64) -> [u8; 2] {
    (n1 as u16).to_be_bytes()
}

/// `UTF8CutFont` (lib/font/subsetFont.go): generates a TrueType font composed
/// only of the runes included in `cutset`.
///
/// `in_buf` is copied, mirroring `GetEncodedSubset`'s private copy of the
/// face; the Go code mutates that copy. Returns `None` where Go returns nil
/// or panics.
pub fn utf8_cut_font(in_buf: &[u8], cutset: &str) -> Option<Vec<u8>> {
    cut_font(in_buf, cutset, false)
}

/// FPDF v0.12.0 GenerateCutFont, whose format-4 cmap length formula predates
/// D2's correction in lib/font/subsetFont.go. Used only by PDF font embedding.
pub fn utf8_cut_font_fpdf(in_buf: &[u8], cutset: &str) -> Option<Vec<u8>> {
    cut_font(in_buf, cutset, true)
}

fn cut_font(in_buf: &[u8], cutset: &str, fpdf_cmap_length: bool) -> Option<Vec<u8>> {
    let mut f = new_utf8_font(FileReader {
        reader_position: 0,
        array: in_buf.to_vec(),
    });
    f.fpdf_cmap_length = fpdf_cmap_length;
    // Go keys the runes by byte index; only the set of values is observable.
    let runes: Vec<i64> = cutset.chars().map(|r| r as i64).collect();
    f.generate_cut_font(&runes)
}
