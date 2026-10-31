// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Portions translated from Go 1.27 encoding/base64/base64.go and
// compress/flate/inflate.go. Copyright 2009 The Go Authors.
// Go BSD terms are preserved in THIRD_PARTY_NOTICES.md.
//! urlenc.Decode: padded URL base64, Go's DEFLATE read boundaries, and Go
//! string-to-JSON UTF-8 replacement, bounded by master lib/urlenc/urlenc.go.
//! No Go or JavaScript engine is invoked.

use base64::Engine as _;

/// Maximum uncompressed D2 script size accepted by the master URL decoder.
pub const MAX_DECODED_SCRIPT_SIZE: usize = 16 << 20;

/// Compress a script with Go 1.27 BestCompression and padded URL base64.
pub fn encode(raw: &str) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(d2_rs_goflate::flate_compress(
        raw.as_bytes(),
        d2_rs_goflate::flate::BEST_COMPRESSION,
    ))
}

fn size_error() -> String {
    format!("decoded D2 script exceeds maximum size of {MAX_DECODED_SCRIPT_SIZE} bytes")
}

fn base64_url(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut pos = 0;
    let mut output = Vec::new();
    let invalid = |offset| format!("illegal base64 data at input byte {offset}");
    while pos < input.len() {
        let mut digits = [0u32; 4];
        let mut count = 0;
        let mut length = 4;
        while count < 4 {
            if pos == input.len() {
                if count == 0 {
                    return Ok(output);
                }
                return Err(invalid(pos - count));
            }
            let byte = input[pos];
            pos += 1;
            let digit = match byte {
                b'A'..=b'Z' => Some(byte - b'A'),
                b'a'..=b'z' => Some(byte - b'a' + 26),
                b'0'..=b'9' => Some(byte - b'0' + 52),
                b'-' => Some(62),
                b'_' => Some(63),
                _ => None,
            };
            if let Some(digit) = digit {
                digits[count] = u32::from(digit);
                count += 1;
                continue;
            }
            if matches!(byte, b'\r' | b'\n') {
                continue;
            }
            if byte != b'=' || count < 2 {
                return Err(invalid(pos - 1));
            }
            if count == 2 {
                while pos < input.len() && matches!(input[pos], b'\r' | b'\n') {
                    pos += 1;
                }
                if pos == input.len() {
                    return Err(invalid(pos));
                }
                if input[pos] != b'=' {
                    return Err(invalid(pos - 1));
                }
                pos += 1;
            }
            while pos < input.len() && matches!(input[pos], b'\r' | b'\n') {
                pos += 1;
            }
            if pos < input.len() {
                return Err(invalid(pos));
            }
            length = count;
            break;
        }
        let value = digits[0] << 18 | digits[1] << 12 | digits[2] << 6 | digits[3];
        output.push((value >> 16) as u8);
        if length >= 3 {
            output.push((value >> 8) as u8);
        }
        if length == 4 {
            output.push(value as u8);
        }
    }
    Ok(output)
}

struct Huffman {
    min: u32,
    chunks: [u32; 512],
    links: Vec<Vec<u32>>,
    link_mask: usize,
}

impl Huffman {
    fn new(lengths: &[u32]) -> Option<Self> {
        let mut table = Self {
            min: 0,
            chunks: [0; 512],
            links: Vec::new(),
            link_mask: 0,
        };
        let mut counts = [0usize; 16];
        let (mut min, mut max) = (0, 0);
        for &length in lengths {
            let length = length as usize;
            if length == 0 {
                continue;
            }
            if length >= counts.len() {
                return None;
            }
            if min == 0 || length < min {
                min = length;
            }
            max = max.max(length);
            counts[length] += 1;
        }
        if max == 0 {
            return Some(table);
        }
        let mut code = 0usize;
        let mut next = [0usize; 16];
        for length in min..=max {
            code <<= 1;
            next[length] = code;
            code += counts[length];
        }
        if code != 1 << max && !(code == 1 && max == 1) {
            return None;
        }
        table.min = min as u32;
        if max > 9 {
            let size = 1 << (max - 9);
            table.link_mask = size - 1;
            let link = next[10] >> 1;
            table.links = vec![vec![0; size]; 512 - link];
            for index in link..512 {
                let reversed = (index as u16).reverse_bits() as usize >> 7;
                table.chunks[reversed] = (((index - link) << 4) | 10) as u32;
            }
        }
        for (symbol, &length) in lengths.iter().enumerate() {
            let length = length as usize;
            if length == 0 {
                continue;
            }
            let code = next[length];
            next[length] += 1;
            let chunk = ((symbol << 4) | length) as u32;
            let reversed = (code as u16).reverse_bits() as usize >> (16 - length);
            if length <= 9 {
                for offset in (reversed..512).step_by(1 << length) {
                    table.chunks[offset] = chunk;
                }
            } else {
                let link = (table.chunks[reversed & 511] >> 4) as usize;
                let entries = &mut table.links[link];
                for offset in (reversed >> 9..entries.len()).step_by(1 << (length - 9)) {
                    entries[offset] = chunk;
                }
            }
        }
        Some(table)
    }
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
    bits: u32,
    available: u32,
    output: Vec<u8>,
}

impl<'a> Reader<'a> {
    fn corrupt(&self) -> String {
        format!("flate: corrupt input before offset {}", self.pos)
    }
    fn more(&mut self) -> Result<(), String> {
        let byte = *self.input.get(self.pos).ok_or("unexpected EOF")?;
        self.pos += 1;
        self.bits |= u32::from(byte) << self.available;
        self.available += 8;
        Ok(())
    }
    fn take(&mut self, count: u32) -> Result<u32, String> {
        while self.available < count {
            self.more()?;
        }
        let value = self.bits & ((1 << count) - 1);
        self.bits >>= count;
        self.available -= count;
        Ok(value)
    }
    fn symbol(&mut self, table: &Huffman) -> Result<usize, String> {
        let mut count = table.min;
        loop {
            while self.available < count {
                self.more()?;
            }
            let mut chunk = table.chunks[(self.bits & 511) as usize];
            count = chunk & 15;
            if count > 9 {
                chunk =
                    table.links[(chunk >> 4) as usize][(self.bits >> 9) as usize & table.link_mask];
                count = chunk & 15;
            }
            if count <= self.available {
                if count == 0 {
                    return Err(self.corrupt());
                }
                self.bits >>= count;
                self.available -= count;
                return Ok((chunk >> 4) as usize);
            }
        }
    }
    fn dynamic(&mut self) -> Result<(Huffman, Huffman), String> {
        // Go reads all14 header bits before checking either count.
        while self.available < 14 {
            self.more()?;
        }
        let literals = (self.bits & 31) as usize + 257;
        if literals > 286 {
            return Err(self.corrupt());
        }
        self.bits >>= 5;
        let distances = (self.bits & 31) as usize + 1;
        if distances > 30 {
            return Err(self.corrupt());
        }
        self.bits >>= 5;
        let count = (self.bits & 15) as usize + 4;
        self.bits >>= 4;
        self.available -= 14;
        const ORDER: [usize; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];
        let mut meta = [0; 19];
        for &index in &ORDER[..count] {
            meta[index] = self.take(3)?;
        }
        let meta = Huffman::new(&meta).ok_or_else(|| self.corrupt())?;
        let mut lengths = Vec::with_capacity(literals + distances);
        while lengths.len() < literals + distances {
            let symbol = self.symbol(&meta)?;
            if symbol < 16 {
                lengths.push(symbol as u32);
                continue;
            }
            let (base, extra, value) = match symbol {
                16 => (3, 2, *lengths.last().ok_or_else(|| self.corrupt())?),
                17 => (3, 3, 0),
                18 => (11, 7, 0),
                _ => return Err("flate: internal error: unexpected length code".into()),
            };
            let repeat = base + self.take(extra)? as usize;
            if lengths.len() + repeat > literals + distances {
                return Err(self.corrupt());
            }
            lengths.resize(lengths.len() + repeat, value);
        }
        let mut lit = Huffman::new(&lengths[..literals]).ok_or_else(|| self.corrupt())?;
        let dist = Huffman::new(&lengths[literals..]).ok_or_else(|| self.corrupt())?;
        lit.min = lit.min.max(lengths[256]);
        Ok((lit, dist))
    }
    fn huffman_block(
        &mut self,
        literals: &Huffman,
        distances: Option<&Huffman>,
    ) -> Result<(), String> {
        loop {
            let symbol = self.symbol(literals)?;
            if symbol < 256 {
                if self.output.len() == MAX_DECODED_SCRIPT_SIZE {
                    return Err(size_error());
                }
                self.output.push(symbol as u8);
                continue;
            }
            if symbol == 256 {
                return Ok(());
            }
            let (base, extra) = match symbol {
                257..=264 => (symbol - 254, 0),
                265..=268 => (symbol * 2 - 519, 1),
                269..=272 => (symbol * 4 - 1057, 2),
                273..=276 => (symbol * 8 - 2149, 3),
                277..=280 => (symbol * 16 - 4365, 4),
                281..=284 => (symbol * 32 - 8861, 5),
                285 => (258, 0),
                _ => return Err(self.corrupt()),
            };
            let length = base + self.take(extra)? as usize;
            let distance = if let Some(table) = distances {
                self.symbol(table)?
            } else {
                (self.take(5)? as u8).reverse_bits() as usize >> 3
            };
            let distance = if distance < 4 {
                distance + 1
            } else if distance < 30 {
                let extra = ((distance - 2) >> 1) as u32;
                (1 << (extra + 1)) + 1 + ((distance & 1) << extra) + self.take(extra)? as usize
            } else {
                return Err(self.corrupt());
            };
            if distance > self.output.len().min(32768) {
                return Err(self.corrupt());
            }
            if length > MAX_DECODED_SCRIPT_SIZE - self.output.len() {
                return Err(size_error());
            }
            for _ in 0..length {
                self.output.push(self.output[self.output.len() - distance]);
            }
        }
    }
    fn inflate(mut self) -> Result<Vec<u8>, String> {
        loop {
            let header = self.take(3)?;
            let final_block = header & 1 != 0;
            match header >> 1 {
                0 => {
                    self.bits = 0;
                    self.available = 0;
                    let header = self
                        .input
                        .get(self.pos..self.pos + 4)
                        .ok_or("unexpected EOF")?;
                    self.pos += 4;
                    let length = u16::from_le_bytes([header[0], header[1]]);
                    if u16::from_le_bytes([header[2], header[3]]) != !length {
                        return Err(self.corrupt());
                    }
                    let end = self.pos + usize::from(length);
                    let bytes = self.input.get(self.pos..end).ok_or("unexpected EOF")?;
                    if bytes.len() > MAX_DECODED_SCRIPT_SIZE - self.output.len() {
                        return Err(size_error());
                    }
                    self.output.extend_from_slice(bytes);
                    self.pos = end;
                }
                1 => {
                    let lengths: Vec<_> = (0..288)
                        .map(|i| match i {
                            0..=143 => 8,
                            144..=255 => 9,
                            256..=279 => 7,
                            _ => 8,
                        })
                        .collect();
                    self.huffman_block(&Huffman::new(&lengths).unwrap(), None)?;
                }
                2 => {
                    let (literals, distances) = self.dynamic()?;
                    self.huffman_block(&literals, Some(&distances))?;
                }
                _ => return Err(self.corrupt()),
            }
            if final_block {
                return Ok(self.output);
            }
        }
    }
}

fn go_string(mut bytes: &[u8]) -> String {
    let mut result = String::new();
    loop {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                result.push_str(text);
                return result;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                result.push_str(std::str::from_utf8(&bytes[..valid]).unwrap());
                result.push('\u{fffd}');
                bytes = &bytes[valid + 1..];
            }
        }
    }
}

/// Decode a URL script, rejecting expansion beyond 16 MiB before publication.
///
/// Errors preserve the upstream URL/base64/DEFLATE diagnostic boundary.
pub fn decode(encoded: &str) -> Result<String, String> {
    let run = || {
        let compressed = base64_url(encoded.as_bytes())?;
        Reader {
            input: &compressed,
            pos: 0,
            bits: 0,
            available: 0,
            output: Vec::new(),
        }
        .inflate()
        .map(|bytes| go_string(&bytes))
    };
    run().map_err(|error| format!("failed to decode d2 script: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_decoded_limit_is_accepted_and_next_byte_is_rejected() {
        let script = "x".repeat(MAX_DECODED_SCRIPT_SIZE);
        assert_eq!(decode(&encode(&script)).unwrap(), script);
        let too_large = script + "x";
        assert_eq!(
            decode(&encode(&too_large)).unwrap_err(),
            format!("failed to decode d2 script: {}", size_error())
        );
    }

    #[test]
    fn stored_and_huffman_only_streams_obey_the_same_bound() {
        let script = vec![b'x'; MAX_DECODED_SCRIPT_SIZE + 1];
        for level in [
            d2_rs_goflate::flate::NO_COMPRESSION,
            d2_rs_goflate::flate::HUFFMAN_ONLY,
        ] {
            let compressed = d2_rs_goflate::flate_compress(&script, level);
            let encoded = base64::engine::general_purpose::URL_SAFE.encode(compressed);
            assert_eq!(
                decode(&encoded).unwrap_err(),
                format!("failed to decode d2 script: {}", size_error())
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_url_decode_streams_and_errors_match() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-wasm"), "tests/fixtures/url-codec-source-v090.json"))
        .unwrap();
        let rows = fixture["cases"].as_array().unwrap();
        assert_eq!(rows.len(), 987);
        let mut failures = Vec::new();
        for row in rows {
            let actual = decode(row["encoded"].as_str().unwrap());
            let expected = &row["expected"];
            let matches = match actual {
                Ok(ref text) => {
                    expected.get("error").is_none() && expected["result"] == text.as_str()
                }
                Err(ref message) => expected["error"]["message"] == message.as_str(),
            };
            if !matches {
                failures.push(
                    serde_json::json!({"name":row["name"],"expected":expected,"actual":actual}),
                );
            }
        }
        assert!(
            failures.is_empty(),
            "source codec differences: {}",
            serde_json::to_string(&failures).unwrap()
        );
    }
}
