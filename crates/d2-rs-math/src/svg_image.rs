// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/image.go, through content ID.
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Encoded {
    pub bounds: [f64; 4],
    pub aspect: [u8; 2],
    pub mime: String,
    pub data: Vec<u8>,
    pub content_id: String,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn data_uri<'a>(
    source: &str,
    raw: &'a str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(&'a str, &'a str), String> {
    let error = |reason: &str| format!("d2svgimport: {source}: {reason}");
    let remainder=raw.strip_prefix("data:").ok_or_else(||error("element <image> href must be one embedded data: raster; external, file, network, blob, and local references are forbidden"))?;
    let mut comma = None;
    for (index, &byte) in remainder.as_bytes().iter().enumerate().take(65) {
        if index & 31 == 0 {
            context(cancel)?;
        }
        if byte == b',' {
            comma = Some(index);
            break;
        }
    }
    let comma = comma.ok_or_else(|| {
        error(if remainder.len() > 64 {
            "element <image> data URI metadata exceeds limit 64"
        } else {
            "element <image> data URI is missing its comma separator"
        })
    })?;
    let metadata = &remainder[..comma];
    let payload = &remainder[comma + 1..];
    let parts = metadata.split(';').collect::<Vec<_>>();
    if parts.len() != 2 || parts[1] != "base64" {
        return Err(error(
            "element <image> data URI must contain one MIME type followed by one final base64 marker",
        ));
    }
    if !matches!(
        parts[0],
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    ) {
        return Err(error(
            "element <image> has unsupported data URI MIME type; expected image/png, image/jpeg, image/gif, or image/webp",
        ));
    }
    if payload.is_empty() {
        return Err(error("element <image> data payload is empty"));
    }
    let invalid = || error("element <image> has malformed canonical base64 data");
    if payload.len() % 4 != 0 {
        return Err(invalid());
    }
    let mut padding = None;
    for (index, byte) in payload.bytes().enumerate() {
        if index & 4095 == 0 {
            context(cancel)?;
        }
        if byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' {
            continue;
        }
        if byte == b'=' && index >= payload.len() - 2 {
            padding.get_or_insert(index);
            continue;
        }
        return Err(invalid());
    }
    if let Some(start) = padding {
        let length = payload.len() - start;
        if length > 2 || (length == 2 && !payload.ends_with('=')) {
            return Err(invalid());
        }
    }
    context(cancel)?;
    Ok((parts[0], payload))
}
pub fn decode(
    payload: &str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<u8>, String> {
    let padding = usize::from(payload.ends_with('=')) + usize::from(payload.ends_with("=="));
    let length = (payload.len() / 4 * 3)
        .checked_sub(padding)
        .ok_or_else(|| "malformed canonical base64 data".to_owned())?;
    let mut bytes = Vec::with_capacity(length);
    for chunk in payload.as_bytes().chunks(32 << 10) {
        context(cancel)?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(chunk)
            .map_err(|_| "malformed canonical base64 data".to_owned())?;
        bytes.extend(decoded);
    }
    if bytes.len() != length {
        return Err("malformed canonical base64 data".into());
    }
    context(cancel)?;
    Ok(bytes)
}
pub fn signature(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(b"\xff\xd8\xff") {
        "jpeg"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "gif"
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        ""
    }
}
pub fn content_id(
    mime: &str,
    data: &[u8],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update(b"d2svgimport:raster\x00");
    hash.update(mime.as_bytes());
    hash.update([0]);
    for chunk in data.chunks(32 << 10) {
        context(cancel)?;
        hash.update(chunk);
    }
    context(cancel)?;
    Ok(format!("svg-raster:{:x}", hash.finalize()))
}
/// Image geometry plus encoded-resource admission. The returned resource still
/// requires cache/resource budgets, static-format inspection and decoded pixels.
pub fn prepare(
    source: &str,
    attrs: &BTreeMap<String, String>,
    children: usize,
    max_bytes: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Encoded, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    if children != 0 {
        return Err(error(
            "element <image> cannot contain child elements".into(),
        ));
    }
    let raw = attrs
        .get("href")
        .ok_or_else(|| error("element <image> requires one data: href".into()))?;
    if !attrs.contains_key("width") || !attrs.contains_key("height") {
        return Err(error(
            "element <image> requires explicit width and height".into(),
        ));
    }
    let length = |key: &str, nonnegative: bool| -> Result<f64, String> {
        let Some(raw) = attrs.get(key) else {
            return Ok(0.);
        };
        let value = super::svg_length::length(raw, true)
            .map_err(|reason| error(format!("element <image> has invalid {key}: {reason}")))?;
        if nonnegative && value < 0. {
            return Err(error(format!(
                "element <image> has invalid {key}: must not be negative"
            )));
        }
        Ok(value)
    };
    let x = length("x", false)?;
    let y = length("y", false)?;
    let width = length("width", true)?;
    let height = length("height", true)?;
    if width <= 0. || height <= 0. {
        return Err(error(
            "element <image> width and height must be positive".into(),
        ));
    }
    if !(x + width).is_finite() || !(y + height).is_finite() {
        return Err(error(
            "element <image> geometry exceeds the finite numeric domain".into(),
        ));
    }
    let aspect = super::svg_viewport::aspect(
        source,
        attrs
            .get("preserveAspectRatio")
            .map(String::as_str)
            .unwrap_or(""),
        "embedded image",
    )?;
    let (mime, payload) = data_uri(source, raw, cancel)?;
    let data = decode(payload, cancel).map_err(|_| {
        context(cancel)
            .err()
            .unwrap_or_else(|| error("element <image> has malformed canonical base64 data".into()))
    })?;
    context(cancel)?;
    if data.is_empty() {
        return Err(error("element <image> data payload is empty".into()));
    }
    if data.len() > max_bytes {
        return Err(error(format!(
            "embedded raster bytes exceed limit {max_bytes}"
        )));
    }
    let format = mime.strip_prefix("image/").unwrap();
    let actual = signature(&data);
    if actual.is_empty() {
        return Err(error(
            "element <image> data has no supported raster signature".into(),
        ));
    }
    if actual != format {
        return Err(error(
            "element <image> MIME type does not match its raster signature".into(),
        ));
    }
    let content_id = content_id(mime, &data, cancel)?;
    Ok(Encoded {
        bounds: [x, y, width, height],
        aspect,
        mime: mime.into(),
        data,
        content_id,
    })
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_image_geometry_uri_canonical_bytes_signature_and_content_id() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-image-admission-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let attrs = serde_json::from_value(row["attrs"].clone()).unwrap();
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = prepare(
                "math",
                &attrs,
                0,
                row["maximum"].as_u64().unwrap() as usize,
                cancel,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(output) = result {
                let expected: Encoded = serde_json::from_value(row["output"].clone()).unwrap();
                assert_eq!(output, expected, "{row}");
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RasterAsset {
    pub mime: String,
    pub data: Vec<u8>,
    pub width: i64,
    pub height: i64,
    pub decoded_bytes: i64,
}
/// None represents a preexisting asset with a different source asset type.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Assets {
    pub entries: BTreeMap<String, Option<RasterAsset>>,
    pub encoded_bytes: i64,
    pub decoded_bytes: i64,
}
fn equal_bytes(
    a: &[u8],
    b: &[u8],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<bool, String> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (left, right) in a.chunks(32768).zip(b.chunks(32768)) {
        context(cancel)?;
        if left != right {
            return Ok(false);
        }
    }
    context(cancel)?;
    Ok(true)
}
/// Complete embedded-image geometry, resource admission and asset insertion.
/// Asset budgets are charged only after every source validation succeeds.
pub fn compile(
    source: &str,
    attrs: &BTreeMap<String, String>,
    children: usize,
    max_bytes: usize,
    max_resources: usize,
    declared_ids: usize,
    assets: &mut Assets,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Encoded, String> {
    let output = prepare(source, attrs, children, max_bytes, cancel)?;
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    if let Some(existing) = assets.entries.get(&output.content_id) {
        let Some(asset) = existing else {
            return Err(error("embedded raster content ID collision".into()));
        };
        if asset.mime != output.mime {
            return Err(error("embedded raster content ID collision".into()));
        }
        if !equal_bytes(&asset.data, &output.data, cancel)? {
            return Err(error("embedded raster content ID collision".into()));
        }
        return Ok(output);
    }
    if declared_ids.saturating_add(assets.entries.len()) >= max_resources {
        return Err(error(format!(
            "declared resource count exceeds limit {max_resources}"
        )));
    }
    if output.data.len() as i128 > max_bytes as i128 - assets.encoded_bytes as i128 {
        return Err(error(format!(
            "embedded raster bytes exceed limit {max_bytes}"
        )));
    }
    let format = output.mime.strip_prefix("image/").unwrap();
    if let Err(err) = super::svg_image_static::inspect(&output.data, format, cancel) {
        context(cancel)?;
        return Err(error(format!(
            "element <image> has unsupported or malformed {} data: {err}",
            format.to_uppercase()
        )));
    }
    let config =
        super::svg_image_config::admitted(&output.data, format, cancel).map_err(|err| {
            context(cancel).err().unwrap_or_else(|| {
                error(format!(
                    "element <image> has malformed {} dimensions: {err}",
                    format.to_uppercase()
                ))
            })
        })?;
    if config.decoded_bytes as i128 > max_bytes as i128 - assets.decoded_bytes as i128 {
        return Err(error(format!(
            "embedded raster decoded bytes exceed limit {max_bytes}"
        )));
    }
    context(cancel)?;
    assets.entries.insert(
        output.content_id.clone(),
        Some(RasterAsset {
            mime: output.mime.clone(),
            data: output.data.clone(),
            width: config.width,
            height: config.height,
            decoded_bytes: config.decoded_bytes,
        }),
    );
    assets.encoded_bytes += output.data.len() as i64;
    assets.decoded_bytes += config.decoded_bytes;
    Ok(output)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod asset_tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Output {
        bounds: [f64; 4],
        aspect: [u8; 2],
        content_id: String,
    }
    #[derive(serde::Deserialize)]
    struct Call {
        error: String,
        output: Option<Output>,
        assets: Assets,
    }
    #[derive(serde::Deserialize)]
    struct Row {
        attrs: BTreeMap<String, String>,
        maximum: usize,
        resources: usize,
        declared: usize,
        cancel: String,
        before: Assets,
        calls: Vec<Call>,
    }
    #[test]
    fn original_image_cache_collisions_resource_limits_atomic_charges_and_repeat() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-image-assets-v090.json"
        ))
        .unwrap();
        for (i, row) in rows.into_iter().enumerate() {
            let token = d2_rs_core::CancellationToken::new();
            let c = match row.cancel.as_str() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let mut assets = row.before;
            for (n, expected) in row.calls.iter().enumerate() {
                let result = compile(
                    "math",
                    &row.attrs,
                    0,
                    row.maximum,
                    row.resources,
                    row.declared,
                    &mut assets,
                    c,
                );
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    expected.error,
                    "control {i} call {n}"
                );
                assert_eq!(assets, expected.assets, "control {i} call {n}");
                if let Ok(out) = result {
                    let e = expected.output.as_ref().unwrap();
                    assert_eq!(out.bounds, e.bounds);
                    assert_eq!(out.aspect, e.aspect);
                    assert_eq!(out.content_id, e.content_id);
                }
            }
        }
    }
}
