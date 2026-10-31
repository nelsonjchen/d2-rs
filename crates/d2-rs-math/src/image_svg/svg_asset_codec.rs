// SPDX-License-Identifier: MPL-2.0 AND MIT
// Brotli decoder error names and reader boundary behavior map to the Brotli
// Authors' Go decoder adapter. See THIRD_PARTY_NOTICES.md.
//! D2 v0.9.0 imageasset:probeResource/expandDeclaredSVG reachable SVG codecs.
use std::{borrow::Cow, io::Read};
const CHUNK: usize = 32 * 1024;
pub const CLI_SVG_LIMIT: usize = 64 * 1024;
pub fn declared_svg(locator: &str) -> bool {
    let path = locator
        .split(['?', '#'])
        .next()
        .unwrap_or(locator)
        .to_ascii_lowercase();
    path.ends_with(".svg")
        || path.ends_with(".svgz")
        || locator.get(..19).is_some_and(|prefix| {
            prefix.eq_ignore_ascii_case("data:image/svg+xml;")
                || prefix.eq_ignore_ascii_case("data:image/svg+xml,")
        })
}
fn looks_xml(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    bytes
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        == Some(&b'<')
}
fn limit_error(actual: usize, limit: usize) -> String {
    format!("SVG bytes {actual} exceeds limit {limit}")
}
fn raw(bytes: &[u8], limit: usize) -> Result<(), String> {
    if bytes.len() > limit {
        return Err(limit_error(bytes.len(), limit));
    }
    let source = std::str::from_utf8(bytes).map_err(|_| "malformed SVG XML".to_string())?;
    super::svg_asset_xml::validate(source).map_err(|error| {
        error
            .strip_prefix("malformed SVG: ")
            .unwrap_or(&error)
            .to_owned()
    })
}
pub(super) fn bounded(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    bounded_context(&mut reader, limit, None)
}
pub(super) fn bounded_context(
    mut reader: impl Read,
    limit: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; CHUNK];
    loop {
        super::context(cancel)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > limit.saturating_sub(bytes.len()) {
            return Err(limit_error(bytes.len() + count, limit));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}
fn brotli_error(code: i32) -> &'static str {
    match code {
        -1 => "EXUBERANT_NIBBLE",
        -2 => "RESERVED",
        -3 => "EXUBERANT_META_NIBBLE",
        -4 => "SIMPLE_HUFFMAN_ALPHABET",
        -5 => "SIMPLE_HUFFMAN_SAME",
        -6 => "CL_SPACE",
        -7 => "HUFFMAN_SPACE",
        -8 => "CONTEXT_MAP_REPEAT",
        -9 => "BLOCK_LENGTH_1",
        -10 => "BLOCK_LENGTH_2",
        -11 => "TRANSFORM",
        -12 => "DICTIONARY",
        -13 => "WINDOW_BITS",
        -14 => "PADDING_1",
        -15 => "PADDING_2",
        -16 => "DISTANCE",
        -19 => "DICTIONARY_NOT_SET",
        -20 => "INVALID_ARGUMENTS",
        -21 => "CONTEXT_MODES",
        -22 => "TREE_GROUPS",
        -25 => "CONTEXT_MAP",
        -26 => "RING_BUFFER_1",
        -27 => "RING_BUFFER_2",
        -30 => "BLOCK_TYPE_TREES",
        -31 => "UNREACHABLE",
        _ => "INVALID",
    }
}
pub(super) fn brotli(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    brotli_context(data, limit, None)
}
pub(super) fn brotli_context(
    data: &[u8],
    limit: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<u8>, String> {
    use brotli_decompressor::{BrotliDecompressStream, BrotliResult, BrotliState, StandardAlloc};
    let mut state = BrotliState::new_strict(
        StandardAlloc::default(),
        StandardAlloc::default(),
        StandardAlloc::default(),
    );
    let mut decoded = Vec::new();
    let mut begin = 0;
    let mut end = data.len().min(CHUNK);
    let mut offset = 0;
    let mut available = end;
    let mut total_out = 0;
    loop {
        super::context(cancel)?;
        let mut buffer = [0; CHUNK];
        let mut output_offset = 0;
        let mut output_available = CHUNK;
        let result = BrotliDecompressStream(
            &mut available,
            &mut offset,
            &data[begin..end],
            &mut output_available,
            &mut output_offset,
            &mut buffer,
            &mut total_out,
            &mut state,
        );
        // Source readBounded checks bytes returned before processing read error.
        if output_offset > limit.saturating_sub(decoded.len()) {
            return Err(limit_error(decoded.len() + output_offset, limit));
        }
        decoded.extend_from_slice(&buffer[..output_offset]);
        match result {
            BrotliResult::ResultSuccess => {
                if available != 0 || end != data.len() {
                    return Err("brotli: excessive input".into());
                }
                return Ok(decoded);
            }
            BrotliResult::ResultFailure => {
                let mut code = state.error_code as i32;
                // Go distinguishes header byte-alignment padding from final
                // metablock padding. The Rust decoder labels both -15; retain
                // the failing state to select the Source error, not input bytes.
                if code == -15 && format!("{:?}", state.state) == "BROTLI_STATE_METABLOCK_HEADER" {
                    code = -14;
                }
                return Err(format!("brotli: {}", brotli_error(code)));
            }
            BrotliResult::NeedsMoreOutput => {}
            BrotliResult::NeedsMoreInput => {
                if end == data.len() {
                    return Err("unexpected EOF".into());
                }
                begin = end;
                end = (end + CHUNK).min(data.len());
                offset = 0;
                available = end - begin;
            }
        }
    }
}
/// Declared SVG may contain any Source-supported compressed stream. Original
/// bytes remain untouched; the returned resource is the decompressed SVG.
pub fn prepare(bytes: &[u8], declared: bool, limit: usize) -> Result<Cow<'_, [u8]>, String> {
    if bytes.is_empty() {
        return Err("empty image resource".into());
    }
    let xml = looks_xml(bytes);
    if !declared && !xml {
        return Ok(Cow::Borrowed(bytes));
    }
    let raw_error = if xml {
        raw(bytes, limit).err()
    } else {
        Some("not raw SVG XML".into())
    };
    let Some(raw_error) = raw_error else {
        return Ok(Cow::Borrowed(bytes));
    };
    if raw_error.starts_with("unsupported SVG:") {
        return Err(raw_error);
    }
    if !declared {
        return Err(raw_error);
    }
    let mut first_limit = None;
    let mut last_error = "no compressed data".to_owned();
    for codec in 0..4 {
        let decoded = match codec {
            0 => bounded(flate2::read::MultiGzDecoder::new(bytes), limit),
            1 => bounded(flate2::read::ZlibDecoder::new(bytes), limit),
            2 => bounded(flate2::read::DeflateDecoder::new(bytes), limit),
            _ => brotli(bytes, limit),
        };
        let result = decoded.and_then(|decoded| {
            raw(&decoded, limit)?;
            Ok(decoded)
        });
        match result {
            Ok(decoded) => return Ok(Cow::Owned(decoded)),
            Err(error) if error.starts_with("unsupported SVG:") => return Err(error),
            Err(error) => {
                if error.starts_with("SVG bytes ") && first_limit.is_none() {
                    first_limit = Some(error.clone());
                }
                last_error = error;
            }
        }
    }
    if raw_error.starts_with("SVG bytes ") {
        return Err(raw_error);
    }
    let compression_error = first_limit.unwrap_or_else(|| {
        format!("no supported codec produced SVG XML (last codec brotli): {last_error}")
    });
    if xml {
        return Err(format!("malformed SVG: {raw_error}"));
    }
    Err(format!(
        "malformed SVG (raw validation failed: {raw_error}; compressed fallback failed): {compression_error}"
    ))
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use base64::Engine as _;
    #[test]
    fn complete_svg_codec_resources_and_errors_match_pristine_go() {
        // The artifact belongs to d2-rs-export (recipe renderexport-svg-asset-codec).
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-export"),
            "tests/fixtures/svg-asset-codec-outcomes-source-v090.json",
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 342);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(row["data_base64"].as_str().unwrap())
                .unwrap();
            let actual = super::prepare(
                &bytes,
                row["declared"].as_bool().unwrap(),
                row["limit"].as_u64().unwrap() as usize,
            );
            let expected = row["error"].as_str().unwrap();
            match actual {
                Err(error) if error != expected => {
                    failures.push(format!("{}: {error:?} != {expected:?}", row["name"]))
                }
                Ok(bytes) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    if !expected.is_empty() || encoded != row["decoded_base64"].as_str().unwrap() {
                        failures.push(format!(
                            "{}: unexpected resource or decoded bytes; expected error {expected:?}",
                            row["name"]
                        ));
                    }
                }
                _ => {}
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
