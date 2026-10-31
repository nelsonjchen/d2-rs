// SPDX-License-Identifier: MPL-2.0
//! lib/imageasset/resolve.go decodeContentEncodings, master 6a7b48378.
use d2_rs_core::CancellationToken;
use std::io::Read;

fn bounded(
    mut reader: impl Read,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    let mut buffer = [0; 32768];
    loop {
        super::context(cancel)?;
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        super::context(cancel)?;
        if n == 0 {
            return Ok(data);
        }
        if n > limit.saturating_sub(data.len()) {
            return Err(format!(
                "decompressed bytes {} exceeds limit {limit}",
                data.len() + n
            ));
        }
        data.extend_from_slice(&buffer[..n]);
    }
}

/// Decode a validated canonical encoding list in reverse order. Every decoded
/// intermediate layer, including identity-only output, has the same ceiling.
pub fn decode_http_content(
    data: &[u8],
    encodings: &[String],
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>, String> {
    use std::borrow::Cow;
    let mut data = Cow::Borrowed(data);
    for encoding in encodings.iter().rev() {
        super::context(cancel)?;
        let result = match encoding.as_str() {
            "identity" => continue,
            "gzip" | "x-gzip" => bounded(
                flate2::read::MultiGzDecoder::new(data.as_ref()),
                limit,
                cancel,
            ),
            "deflate" => {
                // Go zlib.NewReader tests the wrapper before falling back to
                // raw DEFLATE. A corrupt body in a valid wrapper never retries.
                let header = data.get(..2);
                let zlib = header.is_some_and(|h| {
                    h[0] & 15 == 8
                        && h[0] >> 4 <= 7
                        && h[1] & 0x20 == 0
                        && u16::from_be_bytes([h[0], h[1]]) % 31 == 0
                });
                if zlib {
                    bounded(flate2::read::ZlibDecoder::new(data.as_ref()), limit, cancel)
                } else {
                    bounded(
                        flate2::read::DeflateDecoder::new(data.as_ref()),
                        limit,
                        cancel,
                    )
                }
            }
            "br" => super::svg_asset_codec::brotli_context(data.as_ref(), limit, cancel)
                .map_err(|e| e.replacen("SVG bytes ", "decompressed bytes ", 1)),
            _ => {
                return Err(format!(
                    "unsupported canonical content encoding {:?}",
                    encoding
                ));
            }
        };
        data = Cow::Owned(result.map_err(|e| format!("decode {encoding} content encoding: {e}"))?);
    }
    if data.len() > limit {
        return Err(format!(
            "decompressed bytes {} exceeds limit {limit}",
            data.len()
        ));
    }
    super::context(cancel)?;
    Ok(data.into_owned())
}
