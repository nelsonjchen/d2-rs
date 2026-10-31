// SPDX-License-Identifier: MPL-2.0
//! Master lib/imageasset/probe.go:39–227,418–590. Resource acquisition only;
//! element/paint/importer support is checked separately by d2svgimport.
use super::{context, svg_asset_codec, svg_asset_xml, svg_import_envelope};
use d2_rs_core::CancellationToken;
use std::borrow::Cow;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub svg_bytes: usize,
    pub encoded_bytes: usize,
    pub decompressed_bytes: usize,
}

impl Limits {
    fn minimum(self) -> (usize, &'static str) {
        let mut result = (self.svg_bytes, "SVG bytes");
        if self.encoded_bytes < result.0 {
            result = (self.encoded_bytes, "encoded bytes");
        }
        if self.decompressed_bytes < result.0 {
            result = (self.decompressed_bytes, "decompressed bytes");
        }
        result
    }
}

fn limit(name: &str, actual: usize, ceiling: usize) -> Result<(), String> {
    if actual > ceiling {
        Err(format!("{name} {actual} exceeds limit {ceiling}"))
    } else {
        Ok(())
    }
}

fn looks_xml(bytes: &[u8]) -> bool {
    bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(bytes)
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        == Some(&b'<')
}

fn raw(bytes: &[u8], ceiling: usize, cancel: Option<&CancellationToken>) -> Result<(), String> {
    context(cancel)?;
    limit("SVG bytes", bytes.len(), ceiling)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let header_end = bytes
        .starts_with(b"<?xml")
        .then(|| bytes.windows(2).position(|p| p == b"?>").map(|at| at + 2))
        .flatten();
    let latin1 = header_end
        .and_then(|end| std::str::from_utf8(&bytes[5..end - 2]).ok())
        .is_some_and(|header| {
            svg_import_envelope::decoder_field("encoding", header)
                .eq_ignore_ascii_case("ISO-8859-1")
        });
    let converted;
    let source = if latin1 {
        let end = header_end.unwrap();
        let prefix = std::str::from_utf8(&bytes[..end]).map_err(|_| "malformed SVG XML")?;
        let mut output = prefix.to_owned();
        for chunk in bytes[end..].chunks(4096) {
            context(cancel)?;
            output.extend(chunk.iter().copied().map(char::from));
        }
        converted = output;
        &converted
    } else {
        std::str::from_utf8(bytes).map_err(|_| "malformed SVG XML")?
    };
    // The caller removed one BOM. A second is ordinary character data to
    // encoding/xml and must not be removed by the shared lexical helper.
    if source.starts_with('\u{feff}') {
        return Err("text outside svg root element".into());
    }
    svg_asset_xml::validate_bounded(source, ceiling, cancel).map_err(|error| {
        error
            .strip_prefix("malformed SVG: ")
            .unwrap_or(&error)
            .to_owned()
    })
}

/// Validate and expand a declared SVG without fetching nested references.
/// Preserve raw source bytes (including ISO-8859-1); compressed resources own
/// their final XML. Limits apply to final encoded bytes, not codec wrappers.
pub fn prepare_resource<'a>(
    bytes: &'a [u8],
    declared: bool,
    limits: Limits,
    cancel: Option<&CancellationToken>,
) -> Result<Cow<'a, [u8]>, String> {
    context(cancel)?;
    if bytes.is_empty() {
        return Err("empty image resource".into());
    }
    let (ceiling, name) = limits.minimum();
    let xml = looks_xml(bytes);
    let raw_error = if xml {
        limit(name, bytes.len(), ceiling)
            .and_then(|()| raw(bytes, ceiling, cancel))
            .err()
    } else {
        Some("not raw SVG XML".into())
    };
    let Some(raw_error) = raw_error else {
        return Ok(Cow::Borrowed(bytes));
    };
    context(cancel)?;
    if raw_error.starts_with("unsupported SVG:")
        || (!declared && raw_error.contains(" exceeds limit "))
    {
        return Err(raw_error);
    }
    if !declared {
        return Err("unsupported or malformed image".into());
    }
    let mut first_limit = None;
    let mut last_error = "no compressed data".to_owned();
    for codec in 0..4 {
        context(cancel)?;
        let decoded = match codec {
            0 => svg_asset_codec::bounded_context(
                flate2::read::MultiGzDecoder::new(bytes),
                ceiling,
                cancel,
            ),
            1 => svg_asset_codec::bounded_context(
                flate2::read::ZlibDecoder::new(bytes),
                ceiling,
                cancel,
            ),
            2 => svg_asset_codec::bounded_context(
                flate2::read::DeflateDecoder::new(bytes),
                ceiling,
                cancel,
            ),
            _ => svg_asset_codec::brotli_context(bytes, ceiling, cancel),
        }
        .map_err(|error| error.replacen("SVG bytes ", &format!("{name} "), 1));
        context(cancel)?;
        let result = decoded.and_then(|decoded| {
            limit("encoded bytes", decoded.len(), limits.encoded_bytes)?;
            raw(&decoded, ceiling, cancel)?;
            Ok(decoded)
        });
        match result {
            Ok(decoded) => return Ok(Cow::Owned(decoded)),
            Err(error) if error.starts_with("unsupported SVG:") => return Err(error),
            Err(error) => {
                if error.contains(" exceeds limit ") && first_limit.is_none() {
                    first_limit = Some(error.clone());
                }
                last_error = error;
            }
        }
    }
    context(cancel)?;
    if raw_error.contains(" exceeds limit ") {
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
