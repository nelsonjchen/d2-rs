// SPDX-License-Identifier: MPL-2.0
//! Source D2 v0.9.0 d2raster/image.go validateRasterAsset/decoded budget order.
use d2_rs_core::CancellationToken;
use d2_rs_math::svg_image::RasterAsset;
#[derive(Debug, PartialEq, Eq)]
pub struct Validation {
    pub format: &'static str,
    pub decoded_bytes: i64,
}
fn context(cancel: Option<&CancellationToken>) -> Result<(), String> {
    match cancel.and_then(CancellationToken::error) {
        None => Ok(()),
        Some(e) => Err(e.to_string()),
    }
}
pub fn signature(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(&[255, 216, 255]) {
        "jpeg"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "gif"
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        ""
    }
}
pub fn budget(id: &str, decoded: i64, available: i64) -> Result<(), String> {
    if available < 0 || decoded > available {
        Err(format!(
            "d2raster: raster asset {} decoded storage requires {decoded} bytes, exceeding the {} bytes remaining under MaxDecodedAssetBytes",
            d2_rs_core::quote_go_bytes(id.as_bytes()),
            available.max(0)
        ))
    } else {
        Ok(())
    }
}
/// Static resource validation. SVG import has already rejected animations;
/// animated standalone resource configuration remains a separate caller path.
pub fn validate_static(
    id: &str,
    asset: &RasterAsset,
    available: i64,
    cancel: Option<&CancellationToken>,
) -> Result<Validation, String> {
    let prefix = format!(
        "d2raster: raster asset {}",
        d2_rs_core::quote_go_bytes(id.as_bytes())
    );
    context(cancel)?;
    if asset.data.is_empty() {
        return Err(format!("{prefix} has no data"));
    }
    if asset.width <= 0 || asset.height <= 0 {
        return Err(format!(
            "{prefix} has invalid declared dimensions {}x{}",
            asset.width, asset.height
        ));
    }
    let format = match asset.mime.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpeg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => {
            return Err(format!(
                "{prefix} has unsupported MIME type {}; supported raster MIME types are image/png, image/jpeg, image/gif, and image/webp",
                d2_rs_core::quote_go_bytes(asset.mime.as_bytes())
            ));
        }
    };
    let actual = signature(&asset.data);
    if actual.is_empty() {
        return Err(format!("{prefix} data has no supported raster signature"));
    }
    if actual != format {
        return Err(format!(
            "{prefix} declares MIME type {} but data has {} signature",
            d2_rs_core::quote_go_bytes(asset.mime.as_bytes()),
            actual.to_uppercase()
        ));
    }
    let config = (if format == "jpeg" {
        d2_rs_math::svg_image_config::jpeg_config(&asset.data, cancel)
    } else if format == "gif" {
        d2_rs_math::svg_image_config::gif_config(&asset.data, cancel)
    } else {
        d2_rs_math::svg_image_config::static_config(&asset.data, format, cancel)
    })
    .map_err(|e| {
        context(cancel).err().unwrap_or_else(|| {
            format!("{prefix} has malformed {} data: {e}", format.to_uppercase())
        })
    })?;
    if config.width != asset.width || config.height != asset.height {
        return Err(format!(
            "{prefix} declares dimensions {}x{} but encoded data is {}x{}",
            asset.width, asset.height, config.width, config.height
        ));
    }
    let mut decoded = config.decoded_bytes;
    if asset.decoded_bytes != 0 {
        if asset.decoded_bytes < decoded {
            return Err(format!(
                "{prefix} declares a {}-byte decoded footprint below the required {decoded} bytes",
                asset.decoded_bytes
            ));
        }
        decoded = asset.decoded_bytes;
    }
    budget(id, decoded, available)?;
    if decoded as i128 > isize::MAX as i128 {
        return Err(format!(
            "{prefix} decoded storage exceeds the platform integer domain"
        ));
    }
    context(cancel)?;
    Ok(Validation {
        format,
        decoded_bytes: decoded,
    })
}
/// Pixels are decoded only after source metadata/budget validation succeeds.
pub fn prepare_jpeg(
    id: &str,
    asset: &RasterAsset,
    available: i64,
    cancel: Option<&CancellationToken>,
) -> Result<(Vec<u8>, i64), String> {
    let validation = validate_static(id, asset, available, cancel)?;
    assert_eq!(validation.format, "jpeg");
    context(cancel)?;
    let (width, height, data) = crate::jpeg_frame_png(&asset.data, cancel).map_err(|e| {
        context(cancel).err().unwrap_or_else(|| {
            format!(
                "d2raster: raster asset {} has malformed JPEG pixel data: {e}",
                d2_rs_core::quote_go_bytes(id.as_bytes())
            )
        })
    })?;
    if width != asset.width || height != asset.height {
        return Err(format!(
            "d2raster: raster asset {} decoded pixels are {width}x{height}, want declared dimensions {}x{}",
            d2_rs_core::quote_go_bytes(id.as_bytes()),
            asset.width,
            asset.height
        ));
    }
    context(cancel)?;
    Ok((data, validation.decoded_bytes))
}

/// Validate and decode a statically admitted PNG before rendering can silently
/// discard it. Raw encoded bytes remain the asset; this returns its source charge.
pub fn prepare_png(
    id: &str,
    asset: &RasterAsset,
    available: i64,
    cancel: Option<&CancellationToken>,
) -> Result<i64, String> {
    let validation = validate_static(id, asset, available, cancel)?;
    assert_eq!(validation.format, "png");
    context(cancel)?;
    let prefix = format!(
        "d2raster: raster asset {} has malformed PNG pixel data: ",
        d2_rs_core::quote_go_bytes(id.as_bytes())
    );
    // Go zlib.NewReader validates CMF/FLG before decoding any scanline. Read
    // across IDAT boundaries, including empty chunks.
    let mut entropy = Vec::new();
    let mut offset = 8usize;
    while offset + 12 <= asset.data.len() {
        let length =
            u32::from_be_bytes(asset.data[offset..offset + 4].try_into().unwrap()) as usize;
        let end = offset + 12 + length;
        if end > asset.data.len() {
            break;
        }
        if &asset.data[offset + 4..offset + 8] == b"IDAT" {
            entropy.extend_from_slice(&asset.data[offset + 8..offset + 8 + length]);
        }
        offset = end;
    }
    let header = &entropy;
    if header.len() >= 2
        && (header[0] & 15 != 8
            || header[0] >> 4 > 7
            || (u16::from(header[0]) * 256 + u16::from(header[1])) % 31 != 0)
    {
        return Err(prefix + "zlib: invalid header");
    }
    let stream_start = if header.len() >= 2 && header[1] & 0x20 != 0 {
        if header.len() < 6 {
            return Err(prefix + "png: invalid format: not enough pixel data");
        }
        // PNG supplies no preset dictionary: Go compares DICTID against the
        // Adler-32 of the empty dictionary, which is one.
        if u32::from_be_bytes(header[2..6].try_into().unwrap()) != 1 {
            return Err(prefix + "zlib: invalid dictionary");
        }
        6
    } else {
        2
    };
    let error = |e: png::DecodingError| {
        let message = e.to_string();
        let source = if message.starts_with("Unknown filter method ") {
            "png: invalid format: bad filter type".to_owned()
        } else if message == "IDAT or fDAT chunk does not have enough data for image."
            || message == "Corrupt deflate stream. InsufficientInput"
        {
            "png: invalid format: not enough pixel data".to_owned()
        } else {
            message
        };
        context(cancel)
            .err()
            .unwrap_or_else(|| format!("{prefix}{source}"))
    };
    let decoder_bytes = crate::png_image::empty_dictionary_decoder_bytes(&asset.data);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(decoder_bytes.as_ref()));
    decoder.set_limits(png::Limits {
        bytes: (validation.decoded_bytes as usize).max(64 << 20),
    });
    let mut reader = decoder.read_info().map_err(&error)?;
    // Match Go's row reads and the final one-byte read, including data+error
    // results. A complete row ignores a simultaneous terminal error; a short
    // row propagates it before inspecting its filter byte.
    let info = reader.info();
    let bits = info.bit_depth as u64 * info.color_type.samples() as u64;
    let mut stream = PngZlibReader::new(&entropy, stream_start);
    let passes: &[(u64, u64, u64, u64)] = if info.interlaced {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    } else {
        &[(0, 0, 1, 1)]
    };
    for &(x, y, dx, dy) in passes {
        let width = u64::from(info.width).saturating_sub(x).div_ceil(dx);
        let height = u64::from(info.height).saturating_sub(y).div_ceil(dy);
        if width == 0 || height == 0 {
            continue;
        }
        let row_bytes = (width * bits).div_ceil(8) + 1;
        for _ in 0..height {
            context(cancel)?;
            let filter = stream
                .read_row(row_bytes)
                .map_err(|e| format!("{prefix}{e}"))?;
            if filter > 4 {
                return Err(format!("{prefix}png: invalid format: bad filter type"));
            }
        }
    }
    let (extra, terminal) = stream.read_go(&mut [0u8; 1]);
    if let Some(error) = terminal.filter(|e| e != "EOF") {
        return Err(format!("{prefix}png: invalid format: {error}"));
    }
    if extra != 0 || stream.unconsumed_entropy() {
        return Err(format!("{prefix}png: invalid format: too much pixel data"));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("{prefix}png: invalid format: image too large"))?;
    let mut pixels = vec![0; size];
    let frame = reader.next_frame(&mut pixels).map_err(error)?;
    if i64::from(frame.width) != asset.width || i64::from(frame.height) != asset.height {
        return Err(format!(
            "d2raster: raster asset {} decoded pixels are {}x{}, want declared dimensions {}x{}",
            d2_rs_core::quote_go_bytes(id.as_bytes()),
            frame.width,
            frame.height,
            asset.width,
            asset.height
        ));
    }
    context(cancel)?;
    Ok(validation.decoded_bytes)
}

// Go flate delivers its final buffered data together with EOF; zlib then
// verifies the trailer and can return those same bytes with a checksum error.
struct PngZlibReader<'a> {
    raw: flate2::bufread::DeflateDecoder<std::io::Cursor<&'a [u8]>>,
    entropy: &'a [u8],
    start: usize,
    window: Box<[u8; 32768]>,
    pos: usize,
    len: usize,
    pending: Option<u8>,
    terminal: Option<String>,
    a: u32,
    b: u32,
}
impl<'a> PngZlibReader<'a> {
    fn new(entropy: &'a [u8], start: usize) -> Self {
        Self {
            raw: flate2::bufread::DeflateDecoder::new(std::io::Cursor::new(
                &entropy[start.min(entropy.len())..],
            )),
            entropy,
            start,
            window: Box::new([0; 32768]),
            pos: 0,
            len: 0,
            pending: None,
            terminal: None,
            a: 1,
            b: 0,
        }
    }
    fn input_error(error: std::io::Error) -> String {
        // Go's DEFLATE reader obtains its bytes through png.decoder.Read,
        // which reports short IDAT data before a bare compressed-stream EOF.
        if error.kind() == std::io::ErrorKind::UnexpectedEof
            || error.to_string() == "incomplete deflate stream"
        {
            "png: invalid format: not enough pixel data".into()
        } else {
            error.to_string()
        }
    }
    fn fill(&mut self) {
        use std::io::Read;
        self.pos = 0;
        self.len = 0;
        if let Some(byte) = self.pending.take() {
            self.window[0] = byte;
            self.len = 1;
        }
        let mut ended = false;
        while self.len < self.window.len() {
            match self.raw.read(&mut self.window[self.len..]) {
                Ok(0) => {
                    ended = true;
                    break;
                }
                Ok(n) => self.len += n,
                Err(e) => {
                    self.terminal = Some(Self::input_error(e));
                    break;
                }
            }
        }
        if self.len == self.window.len() && self.terminal.is_none() {
            let mut byte = [0];
            match self.raw.read(&mut byte) {
                Ok(0) => ended = true,
                Ok(_) => self.pending = Some(byte[0]),
                Err(e) => self.terminal = Some(Self::input_error(e)),
            }
        }
        for &byte in &self.window[..self.len] {
            self.a = (self.a + u32::from(byte)) % 65521;
            self.b = (self.b + self.a) % 65521;
        }
        if ended {
            let trailer = self.start + self.raw.total_in() as usize;
            self.terminal = Some(if self.entropy.len().saturating_sub(trailer) < 4 {
                "png: invalid format: not enough pixel data".into()
            } else if u32::from_be_bytes(self.entropy[trailer..trailer + 4].try_into().unwrap())
                != ((self.b << 16) | self.a)
            {
                "zlib: invalid checksum".into()
            } else {
                "EOF".into()
            });
        }
    }
    fn read_go(&mut self, out: &mut [u8]) -> (usize, Option<String>) {
        if self.pos == self.len && self.terminal.is_none() {
            self.fill();
        }
        let count = out.len().min(self.len - self.pos);
        out[..count].copy_from_slice(&self.window[self.pos..self.pos + count]);
        self.pos += count;
        (
            count,
            if self.pos == self.len {
                self.terminal.clone()
            } else {
                None
            },
        )
    }
    fn read_row(&mut self, mut remaining: u64) -> Result<u8, String> {
        let mut filter = None;
        let mut buffer = [0u8; 8192];
        while remaining > 0 {
            let size = remaining.min(buffer.len() as u64) as usize;
            let (count, error) = self.read_go(&mut buffer[..size]);
            if filter.is_none() && count > 0 {
                filter = Some(buffer[0]);
            }
            remaining -= count as u64;
            if remaining == 0 {
                break;
            }
            if let Some(error) = error {
                return Err(if error == "EOF" {
                    "png: invalid format: not enough pixel data".into()
                } else {
                    error
                });
            }
        }
        Ok(filter.expect("nonempty PNG row"))
    }
    fn unconsumed_entropy(&self) -> bool {
        self.entropy.len() > self.start + self.raw.total_in() as usize + 4
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    use serde_json::{Value, json};
    #[cfg(feature = "oracle-fixtures")]
    fn project(inputs: Vec<Value>) -> Vec<Value> {
        let outputs:Vec<Value>=inputs.into_iter().map(|i|{let asset=d2_rs_math::svg_image::RasterAsset{mime:i["MIME"].as_str().unwrap().to_owned(),data:i["Data"].as_array().unwrap().iter().map(|v|v.as_u64().unwrap() as u8).collect(),width:i["Width"].as_i64().unwrap(),height:i["Height"].as_i64().unwrap(),decoded_bytes:i["DecodedBytes"].as_i64().unwrap()};let token=d2_rs_core::CancellationToken::new();let c=match i["Cancel"].as_str().unwrap(){"cancelled"=>{token.cancel();Some(&token)},"deadline"=>{token.expire();Some(&token)},_=>None};let mut out=json!({"Format":"","DecodedBytes":0,"Error":"","Width":0,"Height":0,"Pixels":[]});if i["Mode"]=="prepare"{match crate::raster_asset_validation::prepare_jpeg(i["ID"].as_str().unwrap(),&asset,i["Available"].as_i64().unwrap(),c){Err(e)=>out["Error"]=json!(e),Ok((data,charge))=>{let mut r=png::Decoder::new(std::io::Cursor::new(data)).read_info().unwrap();let mut b=vec![0;r.output_buffer_size().unwrap()];let frame=r.next_frame(&mut b).unwrap();let pixels:Vec<[u16;4]>=b[..frame.buffer_size()].chunks_exact(8).map(|p|std::array::from_fn(|j|u16::from_be_bytes([p[2*j],p[2*j+1]]))).collect();out=json!({"Format":"jpeg","DecodedBytes":charge,"Error":"","Width":frame.width,"Height":frame.height,"Pixels":pixels});}}}else{match crate::raster_asset_validation::validate_static(i["ID"].as_str().unwrap(),&asset,i["Available"].as_i64().unwrap(),c){Err(e)=>out["Error"]=json!(e),Ok(v)=>{out["Format"]=json!(v.format);out["DecodedBytes"]=json!(v.decoded_bytes);}}}out}).collect();
        outputs
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn actual_source_metadata_budget_errors_cancellation_and_prepared_colors() {
        let fixtures: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/raster-validation-v090.json"
        ))
        .unwrap();
        assert_eq!(fixtures.len(), 578);
        let inputs = fixtures.iter().map(|f| f["input"].clone()).collect();
        let expected: Vec<Value> = fixtures.iter().map(|f| f["expected"].clone()).collect();
        assert_eq!(project(inputs), expected);
    }
}
