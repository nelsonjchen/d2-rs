// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 archive/zip/{writer,struct,register}.go.
// Copyright 2010 The Go Authors. All rights reserved. BSD-3-Clause;
// see THIRD_PARTY_NOTICES.md.

//! Go's Create/Copy ZIP representation for the in-memory PPTX writer.

use crate::go_png::crc32;

#[derive(Default)]
pub(crate) struct Writer {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

struct Entry {
    central: Vec<u8>,
    offset: u64,
    size: u64,
    compressed: u64,
}

fn u16_at(bytes: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap())
}

impl Writer {
    pub(crate) fn create(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if name.len() > u16::MAX as usize {
            return Err("zip: FileHeader.Name too long".into());
        }
        let directory = name.ends_with('/');
        if directory && !bytes.is_empty() {
            return Err("zip: write to directory".into());
        }
        let flags: u16 = (if directory { 0 } else { 8 })
            | if name.chars().any(|c| !(' '..='}').contains(&c) || c == '\\') {
                0x800
            } else {
                0
            };
        let method: u16 = if directory { 0 } else { 8 };
        let compressed = if directory {
            Vec::new()
        } else {
            // archive/zip/register.go uses level 5, not DefaultCompression.
            d2_rs_goflate::flate_compress(bytes, 5)
        };
        let mut central = vec![0; 46];
        central[..4].copy_from_slice(&0x02014b50u32.to_le_bytes());
        central[4..6].copy_from_slice(&20u16.to_le_bytes());
        central[6..8].copy_from_slice(&20u16.to_le_bytes());
        central[8..10].copy_from_slice(&flags.to_le_bytes());
        central[10..12].copy_from_slice(&method.to_le_bytes());
        central[16..20].copy_from_slice(&crc32(bytes).to_le_bytes());
        central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        self.append(central, &compressed, bytes.len() as u64);
        Ok(())
    }

    /// Copy the immutable, checked-in template's raw members as zip.Writer.Copy
    /// does, retaining their metadata, extras, comments and compressed bytes.
    pub(crate) fn copy_template(&mut self, bytes: &[u8], skip: &[&str]) -> Result<(), String> {
        let end = bytes
            .windows(4)
            .rposition(|b| b == b"PK\x05\x06")
            .ok_or("invalid PPTX template ZIP end record")?;
        let end = bytes
            .get(end..)
            .filter(|b| b.len() >= 22)
            .ok_or("truncated PPTX template ZIP end record")?;
        let mut position = u32_at(end, 16) as usize;
        for _ in 0..u16_at(end, 10) {
            let head = bytes
                .get(position..position + 46)
                .filter(|b| b.starts_with(b"PK\x01\x02"))
                .ok_or("invalid PPTX template ZIP central header")?;
            let name_len = usize::from(u16_at(head, 28));
            let len = 46 + name_len + usize::from(u16_at(head, 30)) + usize::from(u16_at(head, 32));
            let mut central = bytes
                .get(position..position + len)
                .ok_or("truncated PPTX template ZIP central header")?
                .to_vec();
            position += len;
            let name =
                std::str::from_utf8(&central[46..46 + name_len]).map_err(|e| e.to_string())?;
            if skip.contains(&name) {
                continue;
            }
            let offset = u32_at(&central, 42) as usize;
            let local = bytes
                .get(offset..offset + 30)
                .filter(|b| b.starts_with(b"PK\x03\x04"))
                .ok_or("invalid PPTX template ZIP local header")?;
            let data_offset =
                offset + 30 + usize::from(u16_at(local, 26)) + usize::from(u16_at(local, 28));
            let compressed_len = u32_at(&central, 20) as usize;
            let data = bytes
                .get(data_offset..data_offset + compressed_len)
                .ok_or("truncated PPTX template ZIP data")?;
            let size = u64::from(u32_at(&central, 24));
            // Go does not propagate the input's disk/internal attributes.
            central[34..38].fill(0);
            self.append(central, data, size);
        }
        Ok(())
    }

    fn append(&mut self, mut central: Vec<u8>, data: &[u8], size: u64) {
        let offset = self.bytes.len() as u64;
        let compressed = data.len() as u64;
        let descriptor = u16_at(&central, 8) & 8 != 0;
        let large = compressed > u64::from(u32::MAX) || size > u64::from(u32::MAX);
        central[20..24]
            .copy_from_slice(&(compressed.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        central[24..28].copy_from_slice(&(size.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        if large && !descriptor {
            central[6..8].copy_from_slice(&45u16.to_le_bytes());
        }
        let name_len = usize::from(u16_at(&central, 28));
        let extra_len = usize::from(u16_at(&central, 30));
        let mut local = Vec::from(&b"PK\x03\x04"[..]);
        local.extend_from_slice(&central[6..16]);
        if descriptor {
            local.extend_from_slice(&[0; 12]);
        } else {
            local.extend_from_slice(&central[16..28]);
        }
        local.extend_from_slice(&(name_len as u16).to_le_bytes());
        local.extend_from_slice(
            &((extra_len + if large && !descriptor { 20 } else { 0 }) as u16).to_le_bytes(),
        );
        local.extend_from_slice(&central[46..46 + name_len + extra_len]);
        if large && !descriptor {
            local.extend_from_slice(&[1, 0, 16, 0]);
            local.extend_from_slice(&size.to_le_bytes());
            local.extend_from_slice(&compressed.to_le_bytes());
        }
        self.bytes.extend_from_slice(&local);
        self.bytes.extend_from_slice(data);
        // Create writes its local header before the streaming sizes are known.
        // Only the central header is upgraded when Close discovers ZIP64.
        if large {
            central[6..8].copy_from_slice(&45u16.to_le_bytes());
        }
        if descriptor {
            self.bytes.extend_from_slice(b"PK\x07\x08");
            self.bytes.extend_from_slice(&central[16..20]);
            if large {
                self.bytes.extend_from_slice(&compressed.to_le_bytes());
                self.bytes.extend_from_slice(&size.to_le_bytes());
            } else {
                self.bytes.extend_from_slice(&central[20..28]);
            }
        }
        self.entries.push(Entry {
            central,
            offset,
            size,
            compressed,
        });
    }

    pub(crate) fn finish(mut self) -> Result<Vec<u8>, String> {
        let start = self.bytes.len() as u64;
        let count = self.entries.len() as u64;
        let mut zip64 = false;
        for mut entry in self.entries {
            let mut extra = Vec::new();
            for value in [entry.size, entry.compressed, entry.offset] {
                if value >= u64::from(u32::MAX) {
                    extra.extend_from_slice(&value.to_le_bytes());
                }
            }
            if !extra.is_empty() {
                zip64 = true;
                let name_len = usize::from(u16_at(&entry.central, 28));
                let old_extra_len = usize::from(u16_at(&entry.central, 30));
                let new_extra_len = old_extra_len + 4 + extra.len();
                let len =
                    u16::try_from(new_extra_len).map_err(|_| "zip: FileHeader.Extra too long")?;
                entry.central[30..32].copy_from_slice(&len.to_le_bytes());
                entry.central[6..8].copy_from_slice(&45u16.to_le_bytes());
                let mut field = Vec::from(1u16.to_le_bytes());
                field.extend_from_slice(&(extra.len() as u16).to_le_bytes());
                field.extend_from_slice(&extra);
                entry.central.splice(
                    46 + name_len + old_extra_len..46 + name_len + old_extra_len,
                    field,
                );
            }
            entry.central[42..46]
                .copy_from_slice(&(entry.offset.min(u64::from(u32::MAX)) as u32).to_le_bytes());
            self.bytes.extend_from_slice(&entry.central);
        }
        let end = self.bytes.len() as u64;
        let len = end - start;
        if zip64
            || count >= u64::from(u16::MAX)
            || len >= u64::from(u32::MAX)
            || start >= u64::from(u32::MAX)
        {
            self.bytes.extend_from_slice(b"PK\x06\x06");
            self.bytes.extend_from_slice(&44u64.to_le_bytes());
            self.bytes
                .extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            for value in [count, count, len, start] {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            self.bytes.extend_from_slice(b"PK\x06\x07");
            self.bytes.extend_from_slice(&0u32.to_le_bytes());
            self.bytes.extend_from_slice(&end.to_le_bytes());
            self.bytes.extend_from_slice(&1u32.to_le_bytes());
        }
        self.bytes.extend_from_slice(b"PK\x05\x06");
        self.bytes.extend_from_slice(&[0; 4]);
        let count = (count.min(u64::from(u16::MAX)) as u16).to_le_bytes();
        self.bytes.extend_from_slice(&count);
        self.bytes.extend_from_slice(&count);
        self.bytes
            .extend_from_slice(&(len.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        self.bytes
            .extend_from_slice(&(start.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        self.bytes.extend_from_slice(&[0; 2]);
        Ok(self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn archive_create_copy_headers_and_compression_match_go() {
        let mut writer = Writer::default();
        writer.create("[Content_Types].xml", b"<Types/>").unwrap();
        writer
            .copy_template(
                include_bytes!("../assets/d2-template.pptx"),
                &[
                    "_rels/.rels",
                    "ppt/slideMasters/_rels/slideMaster1.xml.rels",
                ],
            )
            .unwrap();
        writer.create("empty/", b"").unwrap();
        writer.create("empty.txt", b"").unwrap();
        writer
            .create(
                "日本語/~name.txt",
                b"repeated archive contents repeated archive contents",
            )
            .unwrap();
        let actual = writer.finish().unwrap();
        let expected = d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/zip-encoding-go127.zip");
        assert!(
            actual == expected,
            "lengths {}/{}, first diff {:?}",
            actual.len(),
            expected.len(),
            actual.iter().zip(expected).position(|(a, b)| a != b)
        );
    }
}
