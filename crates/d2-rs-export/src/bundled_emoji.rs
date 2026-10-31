// SPDX-License-Identifier: MPL-2.0
//! Authenticated bounded Noto loader from d2fonts/noto_color_emoji_native.go.
//! Scene budgets and Source's complete renderable COLRv1 predicate remain open.

use resvg::usvg::fontdb::{Database, ID, Source};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::sync::{Arc, OnceLock};

#[path = "emoji_coverage.rs"]
mod coverage;

const COMPRESSED_SIZE: usize = 2_354_437;
const DECODED_SIZE: usize = 4_991_984;
const COMPRESSED_HASH: [u8; 32] = [
    0x34, 0x2b, 0xef, 0xfe, 0x73, 0xf7, 0xfa, 0x45, 0x0d, 0x24, 0x86, 0xd3, 0xad, 0x62, 0xab, 0x23,
    0x08, 0xdf, 0x68, 0x19, 0x27, 0x18, 0x96, 0x5b, 0xaf, 0x23, 0xf5, 0xa8, 0x5e, 0xef, 0x82, 0x47,
];
pub(super) const DECODED_HASH: [u8; 32] = [
    0x0a, 0xe5, 0x7f, 0xe5, 0x86, 0x45, 0x63, 0x85, 0x23, 0xba, 0x35, 0xf3, 0x88, 0xd9, 0x37, 0x39,
    0xd2, 0x92, 0x53, 0x9a, 0x9a, 0xcb, 0x84, 0xdf, 0x57, 0x00, 0xc8, 0x1b, 0x1e, 0x1a, 0x28, 0xd2,
];

#[cfg(not(target_arch = "wasm32"))]
const COMPRESSED: &[u8] = include_bytes!("../assets/NotoColorEmoji-COLRv1-v2.051.ttf.br");

fn covers(value: char) -> bool {
    let value = u32::from(value);
    coverage::RANGES
        .binary_search_by(|(first, last)| {
            if value < *first {
                std::cmp::Ordering::Greater
            } else if value > *last {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn decode(
    compressed: &[u8],
    compressed_size: usize,
    compressed_hash: [u8; 32],
    decoded_size: usize,
    decoded_hash: [u8; 32],
) -> Result<Vec<u8>, String> {
    if compressed_size == 0 || decoded_size == 0 {
        return Err(format!(
            "invalid compressed/decoded size limits {compressed_size}/{decoded_size}"
        ));
    }
    if compressed.len() != compressed_size {
        return Err(format!(
            "Brotli font size is {}, want {compressed_size}",
            compressed.len()
        ));
    }
    let digest = Sha256::digest(compressed);
    if digest.as_slice() != compressed_hash {
        return Err(format!(
            "Brotli font SHA-256 is {digest:x}, want {:x}",
            sha2::digest::Output::<Sha256>::from_slice(&compressed_hash)
        ));
    }
    // One final backing allocation, followed by a one-byte expansion probe.
    let mut data = vec![0; decoded_size];
    let mut reader = brotli_decompressor::Decompressor::new(Cursor::new(compressed), 4096);
    let mut count = 0;
    while count < decoded_size {
        let n = reader
            .read(&mut data[count..])
            .map_err(|error| format!("read Brotli stream: {error}"))?;
        if n == 0 {
            return Err(format!("decoded font size is {count}, want {decoded_size}"));
        }
        count += n;
    }
    let mut extra = [0];
    if reader
        .read(&mut extra)
        .map_err(|error| format!("read Brotli stream: {error}"))?
        != 0
    {
        return Err(format!("decoded font exceeds limit {decoded_size}"));
    }
    let digest = Sha256::digest(&data);
    if digest.as_slice() != decoded_hash {
        return Err(format!(
            "decoded font SHA-256 is {digest:x}, want {:x}",
            sha2::digest::Output::<Sha256>::from_slice(&decoded_hash)
        ));
    }
    Ok(data)
}

#[cfg(not(target_arch = "wasm32"))]
fn data() -> Result<Arc<Vec<u8>>, String> {
    static CACHE: OnceLock<Result<Arc<Vec<u8>>, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let data = decode(
                COMPRESSED,
                COMPRESSED_SIZE,
                COMPRESSED_HASH,
                DECODED_SIZE,
                DECODED_HASH,
            )?;
            if ttf_parser::fonts_in_collection(&data).is_some() {
                return Err("bundled Noto Color Emoji must contain one face".into());
            }
            ttf_parser::Face::parse(&data, 0)
                .map_err(|error| format!("parse bundled Noto Color Emoji: {error:?}"))?;
            Ok(Arc::new(data))
        })
        .clone()
}

#[derive(Default)]
pub(super) struct Resolver {
    id: Option<ID>,
    pub(super) error: Option<String>,
}

/// Source bundled_fallback.go retains the authenticated Noto face before
/// consulting host fallbacks. A failed upstream shaping run can leave the
/// face loaded without attaching it to any retained glyph.
pub(super) fn loaded_face(value: char, excluded: &[ID], database: &Database) -> Option<ID> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (value, excluded, database);
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !covers(value) {
            return None;
        }
        let authenticated = data().ok()?;
        database.faces().find_map(|face| {
            if excluded.contains(&face.id) || !matches!(face.source, Source::Binary(_)) {
                return None;
            }
            database.with_face_data(face.id, |bytes, index| {
                (index == 0 && bytes == authenticated.as_slice()).then_some(face.id)
            })?
        })
    }
}

impl Resolver {
    pub(super) fn select(
        &mut self,
        value: char,
        excluded: &[ID],
        database: &mut Arc<Database>,
    ) -> Option<ID> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (value, excluded, database);
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !covers(value) {
                return None;
            }
            let id = match self.id {
                Some(id) => id,
                None => {
                    let bytes = match data() {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            self.error = Some(error);
                            return None;
                        }
                    };
                    let ids = Arc::make_mut(database).load_font_source(Source::Binary(bytes));
                    if ids.len() != 1 {
                        self.error = Some("bundled Noto Color Emoji must contain one face".into());
                        return None;
                    }
                    self.id = Some(ids[0]);
                    ids[0]
                }
            };
            if excluded.contains(&id) {
                return None;
            }
            let supported = database
                .with_face_data(id, |data, index| {
                    ttf_parser::Face::parse(data, index)
                        .ok()
                        .and_then(|face| face.glyph_index(value))
                        .is_some_and(|glyph| glyph.0 != 0)
                })
                .unwrap_or(false);
            supported.then_some(id)
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn authenticated_asset_cmap_agrees_with_source_ranges() {
        let bytes = data().unwrap();
        let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
        for value in ['✅', '😀', '❤', 'A', 'あ', '\u{10ffff}'] {
            assert_eq!(
                covers(value),
                face.glyph_index(value).is_some_and(|glyph| glyph.0 != 0)
            );
        }
        assert!(Arc::ptr_eq(&bytes, &data().unwrap()));
    }
    #[test]
    fn decoder_rejects_corruption_and_expansion_before_publication() {
        let mut corrupt = COMPRESSED.to_vec();
        corrupt[10] ^= 1;
        assert!(
            decode(
                &corrupt,
                COMPRESSED_SIZE,
                COMPRESSED_HASH,
                DECODED_SIZE,
                DECODED_HASH
            )
            .unwrap_err()
            .starts_with("Brotli font SHA-256")
        );
        assert_eq!(
            decode(
                COMPRESSED,
                COMPRESSED_SIZE,
                COMPRESSED_HASH,
                DECODED_SIZE - 1,
                DECODED_HASH
            )
            .unwrap_err(),
            format!("decoded font exceeds limit {}", DECODED_SIZE - 1)
        );
        assert_eq!(
            decode(
                COMPRESSED,
                COMPRESSED_SIZE,
                COMPRESSED_HASH,
                DECODED_SIZE + 1,
                DECODED_HASH
            )
            .unwrap_err(),
            format!(
                "decoded font size is {DECODED_SIZE}, want {}",
                DECODED_SIZE + 1
            )
        );
    }
}
