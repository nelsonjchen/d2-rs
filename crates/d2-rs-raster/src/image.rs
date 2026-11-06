// SPDX-License-Identifier: MPL-2.0
//! d2raster/image.go and internal/rasterimage: raster asset validation and
//! decoding (first frame, EXIF orientation), placement, 4x4 supersampled
//! bilinear sampling and the premultiplied compositors.
//!
//! Decoded pixels are held as Go's `color.Color.RGBA()` values of the Go
//! decoder's concrete image type. Go's per-type native-size fast paths
//! reduce to the generic per-pixel compositor for every type except
//! `*image.NRGBA`, whose premultiplication rounds differently; that type
//! keeps its raw bytes and dedicated path.

use d2_rs_scene::{
    AspectAlign, AspectFit, AspectRatio, Box as SceneBox, Matrix, Point, go_max, go_min,
};

use crate::fma::{finite, finite_point, go_fmadd, go_fmsub, gomath, mpoint};
use crate::pixels::{Pt, Rect, Rgba};
use crate::raster::validate_raster_point;

const IMAGE_SAMPLES_PER_AXIS: i64 = 4;

/// A decoded raster asset on its logical canvas.
#[derive(Debug)]
pub struct PreparedRasterAsset {
    pub bounds: Rect,
    pub width: i64,
    /// Premultiplied 16-bit RGBA per pixel, as Go's `At(x, y).RGBA()`
    /// (every value fits in sixteen bits).
    pub rgba64: Vec<[u16; 4]>,
    /// Raw straight-alpha bytes when Go decodes to `*image.NRGBA`.
    pub nrgba: Option<Vec<u8>>,
}

impl PreparedRasterAsset {
    fn at(&self, x: i64, y: i64) -> [u32; 4] {
        if !self.bounds.contains(Pt::new(x, y)) {
            return [0; 4];
        }
        self.rgba64[((y - self.bounds.min.y) * self.width + (x - self.bounds.min.x)) as usize]
            .map(u32::from)
    }
}

/// Decoded `At().RGBA()` samples, stored at their sixteen-bit width.
struct Samples(Vec<[u16; 4]>);

impl Samples {
    fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    fn push(&mut self, value: [u32; 4]) {
        self.0.push(narrow(value));
    }
}

/// A sixteen-bit `RGBA()` value; every channel is at most 0xffff.
fn narrow(value: [u32; 4]) -> [u16; 4] {
    debug_assert!(value.iter().all(|&channel| channel <= 0xffff));
    value.map(|channel| channel as u16)
}

#[derive(Clone, Copy, Debug)]
pub struct InverseAffine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

#[derive(Debug)]
pub struct PreparedImage {
    pub asset: std::rc::Rc<PreparedRasterAsset>,
    pub r#box: SceneBox,
    pub placement: SceneBox,
    pub inverse: InverseAffine,
    pub bounds: Rect,
}

// ----------------------------------------------------------- validation

pub fn prepare_raster_asset(
    id: &str,
    mime_type: &str,
    data: &[u8],
    pixel_width: i64,
    pixel_height: i64,
    declared_decoded_bytes: i64,
    available_bytes: i64,
) -> Result<(PreparedRasterAsset, i64), String> {
    let prefix = format!("d2raster: raster asset {id:?}");
    if data.is_empty() {
        return Err(format!("{prefix} has no data"));
    }
    if pixel_width <= 0 || pixel_height <= 0 {
        return Err(format!(
            "{prefix} has invalid declared dimensions {pixel_width}x{pixel_height}"
        ));
    }
    let Some(want_format) = raster_format_for_mime(mime_type) else {
        return Err(format!(
            "{prefix} has unsupported MIME type {mime_type:?}; supported raster MIME types are image/png, image/jpeg, image/gif, and image/webp"
        ));
    };
    let got_signature = raster_data_format(data);
    if got_signature.is_empty() {
        return Err(format!("{prefix} data has no supported raster signature"));
    }
    if got_signature != want_format {
        return Err(format!(
            "{prefix} declares MIME type {mime_type:?} but data has {} signature",
            got_signature.to_uppercase()
        ));
    }
    let decoded = decode_first(data, want_format).map_err(|e| {
        format!(
            "{prefix} has malformed {} data: {e}",
            want_format.to_uppercase()
        )
    })?;
    let (width, height) = (decoded.asset.bounds.dx(), decoded.asset.bounds.dy());
    if width != pixel_width || height != pixel_height {
        return Err(format!(
            "{prefix} declares dimensions {pixel_width}x{pixel_height} but encoded data is {width}x{height}"
        ));
    }
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| format!("{prefix}: decoded pixel count overflows int64"))?;
    let bytes_per_pixel = if decoded.sixteen_bit { 8 } else { 4 };
    let mut decoded_bytes = pixels
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| format!("{prefix}: decoded byte count overflows int64"))?;
    if declared_decoded_bytes != 0 {
        if declared_decoded_bytes < decoded_bytes {
            return Err(format!(
                "{prefix} declares a {declared_decoded_bytes}-byte decoded footprint below the required {decoded_bytes} bytes"
            ));
        }
        decoded_bytes = declared_decoded_bytes;
    }
    if available_bytes < 0 || decoded_bytes > available_bytes {
        return Err(format!(
            "d2raster: raster asset {id:?} decoded storage requires {decoded_bytes} bytes, exceeding the {} bytes remaining under MaxDecodedAssetBytes",
            available_bytes.max(0)
        ));
    }
    Ok((decoded.asset, decoded_bytes))
}

fn raster_format_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpeg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn raster_data_format(data: &[u8]) -> &'static str {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        "png"
    } else if data.len() >= 3 && data[0] == 0xff && data[1] == 0xd8 && data[2] == 0xff {
        "jpeg"
    } else if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        "gif"
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        ""
    }
}

struct Decoded {
    asset: PreparedRasterAsset,
    sixteen_bit: bool,
}

fn rgba_color(r: u8, g: u8, b: u8, a: u8) -> [u32; 4] {
    let c = |v: u8| v as u32 * 0x101;
    [c(r), c(g), c(b), c(a)]
}

/// color.NRGBA.RGBA().
fn nrgba_color(r: u8, g: u8, b: u8, a: u8) -> [u32; 4] {
    let alpha = a as u32;
    let c = |v: u8| (v as u32 * 0x101) * alpha / 0xff;
    [c(r), c(g), c(b), alpha * 0x101]
}

/// color.NRGBA64.RGBA().
fn nrgba64_color(r: u32, g: u32, b: u32, a: u32) -> [u32; 4] {
    [r * a / 0xffff, g * a / 0xffff, b * a / 0xffff, a]
}

fn decode_first(data: &[u8], format: &str) -> Result<Decoded, String> {
    match format {
        "png" => decode_png(data),
        "gif" => decode_gif(data),
        "jpeg" => decode_jpeg(data),
        "webp" => decode_webp(data),
        _ => Err("unsupported format".into()),
    }
}

/// image/png's decoder output types (Gray, NRGBA, RGBA, Paletted, Gray16,
/// NRGBA64, RGBA64) and their `At().RGBA()` values. APNG first frames that
/// do not use IDAT are not yet distinguished.
fn decode_png(data: &[u8]) -> Result<Decoded, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let info = reader.info();
    let (width, height) = (frame.width as usize, frame.height as usize);
    let line = frame.line_size;
    let depth = info.bit_depth as u8;
    let trns = info.trns.as_deref().map(|t| t.to_vec());
    let palette = info.palette.as_deref().map(|p| p.to_vec());
    let mut rgba64 = Samples::with_capacity(width * height);
    let mut nrgba: Option<Vec<u8>> = None;
    let mut sixteen_bit = false;
    let sample = |row: &[u8], x: usize, depth: u8| -> u8 {
        match depth {
            1 => (row[x / 8] >> (7 - x % 8)) & 1,
            2 => (row[x / 4] >> (6 - 2 * (x % 4))) & 3,
            4 => (row[x / 2] >> (4 - 4 * (x % 2))) & 15,
            _ => row[x],
        }
    };
    let be16 = |row: &[u8], i: usize| (row[i] as u32) << 8 | row[i + 1] as u32;
    match info.color_type {
        png::ColorType::Grayscale if depth <= 8 => {
            let scale = match depth {
                1 => 0xff,
                2 => 0x55,
                4 => 0x11,
                _ => 1,
            };
            let transparent = trns
                .as_ref()
                .map(|t| (t.get(1).copied().unwrap_or(0) as u32 * scale as u32) as u8);
            let mut raw = Vec::new();
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    let v = (sample(row, x, depth) as u32 * scale as u32) as u8;
                    match transparent {
                        Some(t) => {
                            let a = if v == t { 0 } else { 0xff };
                            raw.extend_from_slice(&[v, v, v, a]);
                            rgba64.push(nrgba_color(v, v, v, a));
                        }
                        None => rgba64.push(rgba_color(v, v, v, 0xff)),
                    }
                }
            }
            if transparent.is_some() {
                nrgba = Some(raw);
            }
        }
        png::ColorType::Grayscale => {
            sixteen_bit = true;
            let transparent = trns.as_ref().map(|t| be16(t, 0));
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    let v = be16(row, 2 * x);
                    match transparent {
                        Some(t) => {
                            rgba64.push(nrgba64_color(v, v, v, if v == t { 0 } else { 0xffff }))
                        }
                        None => rgba64.push([v, v, v, 0xffff]),
                    }
                }
            }
        }
        png::ColorType::GrayscaleAlpha if depth == 8 => {
            let mut raw = Vec::with_capacity(width * height * 4);
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    let (v, a) = (row[2 * x], row[2 * x + 1]);
                    raw.extend_from_slice(&[v, v, v, a]);
                    rgba64.push(nrgba_color(v, v, v, a));
                }
            }
            nrgba = Some(raw);
        }
        png::ColorType::GrayscaleAlpha => {
            sixteen_bit = true;
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    let (v, a) = (be16(row, 4 * x), be16(row, 4 * x + 2));
                    rgba64.push(nrgba64_color(v, v, v, a));
                }
            }
        }
        png::ColorType::Rgb if depth == 8 => {
            let transparent = trns
                .as_ref()
                .filter(|t| t.len() >= 6)
                .map(|t| (t[1], t[3], t[5]));
            let mut raw = Vec::new();
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    let (r, g, b) = (row[3 * x], row[3 * x + 1], row[3 * x + 2]);
                    match transparent {
                        Some(t) => {
                            let a = if (r, g, b) == t { 0 } else { 0xff };
                            raw.extend_from_slice(&[r, g, b, a]);
                            rgba64.push(nrgba_color(r, g, b, a));
                        }
                        None => rgba64.push(rgba_color(r, g, b, 0xff)),
                    }
                }
            }
            if transparent.is_some() {
                nrgba = Some(raw);
            }
        }
        png::ColorType::Rgb => {
            sixteen_bit = true;
            let transparent = trns
                .as_ref()
                .filter(|t| t.len() >= 6)
                .map(|t| (be16(t, 0), be16(t, 2), be16(t, 4)));
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    let (r, g, b) = (be16(row, 6 * x), be16(row, 6 * x + 2), be16(row, 6 * x + 4));
                    match transparent {
                        Some(t) => rgba64.push(nrgba64_color(
                            r,
                            g,
                            b,
                            if (r, g, b) == t { 0 } else { 0xffff },
                        )),
                        None => rgba64.push([r, g, b, 0xffff]),
                    }
                }
            }
        }
        png::ColorType::Rgba if depth == 8 => {
            let mut raw = Vec::with_capacity(width * height * 4);
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                raw.extend_from_slice(&row[..width * 4]);
                for x in 0..width {
                    rgba64.push(nrgba_color(
                        row[4 * x],
                        row[4 * x + 1],
                        row[4 * x + 2],
                        row[4 * x + 3],
                    ));
                }
            }
            nrgba = Some(raw);
        }
        png::ColorType::Rgba => {
            sixteen_bit = true;
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    rgba64.push(nrgba64_color(
                        be16(row, 8 * x),
                        be16(row, 8 * x + 2),
                        be16(row, 8 * x + 4),
                        be16(row, 8 * x + 6),
                    ));
                }
            }
        }
        png::ColorType::Indexed => {
            let palette = palette.ok_or("png: invalid format: missing palette")?;
            let entries = palette.len() / 3;
            let mut colors: Vec<[u32; 4]> = (0..256)
                .map(|i| {
                    if i < entries {
                        rgba_color(palette[3 * i], palette[3 * i + 1], palette[3 * i + 2], 0xff)
                    } else {
                        rgba_color(0, 0, 0, 0xff)
                    }
                })
                .collect();
            if let Some(t) = &trns {
                for (i, alpha) in t.iter().enumerate().take(256) {
                    let (r, g, b) = if i < entries {
                        (palette[3 * i], palette[3 * i + 1], palette[3 * i + 2])
                    } else {
                        (0, 0, 0)
                    };
                    colors[i] = nrgba_color(r, g, b, *alpha);
                }
            }
            for y in 0..height {
                let row = &buffer[y * line..(y + 1) * line];
                for x in 0..width {
                    rgba64.push(colors[sample(row, x, depth) as usize]);
                }
            }
        }
    }
    let bounds = Rect::new(0, 0, width as i64, height as i64);
    Ok(Decoded {
        asset: PreparedRasterAsset {
            bounds,
            width: width as i64,
            rgba64: rgba64.0,
            nrgba,
        },
        sixteen_bit,
    })
}

/// image/gif's first frame (`*image.Paletted`) on its logical screen.
fn decode_gif(data: &[u8]) -> Result<Decoded, String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options
        .read_info(std::io::Cursor::new(data))
        .map_err(|e| e.to_string())?;
    let (canvas_width, canvas_height) = (decoder.width() as i64, decoder.height() as i64);
    let global = decoder.global_palette().map(|p| p.to_vec());
    let frame = decoder
        .read_next_frame()
        .map_err(|e| e.to_string())?
        .ok_or("gif: no frames")?
        .clone();
    let palette = frame
        .palette
        .clone()
        .or(global)
        .ok_or("gif: no color table")?;
    let mut colors: Vec<[u32; 4]> = palette
        .chunks(3)
        .map(|c| rgba_color(c[0], c[1], c[2], 0xff))
        .collect();
    if let Some(index) = frame.transparent {
        if (index as usize) < colors.len() {
            colors[index as usize] = [0; 4];
        }
    }
    let frame_bounds = Rect::new(
        frame.left as i64,
        frame.top as i64,
        frame.left as i64 + frame.width as i64,
        frame.top as i64 + frame.height as i64,
    );
    let canvas = Rect::new(0, 0, canvas_width, canvas_height);
    if frame_bounds.empty() || !frame_bounds.is_in(canvas) {
        return Err(format!(
            "GIF first-frame bounds ({},{})-({},{}) fall outside logical canvas (0,0)-({canvas_width},{canvas_height})",
            frame_bounds.min.x, frame_bounds.min.y, frame_bounds.max.x, frame_bounds.max.y
        ));
    }
    let mut rgba64 = vec![[0u16; 4]; (canvas_width * canvas_height) as usize];
    for y in 0..frame.height as i64 {
        for x in 0..frame.width as i64 {
            let index = frame.buffer[(y * frame.width as i64 + x) as usize] as usize;
            let color = colors.get(index).copied().unwrap_or([0; 4]);
            rgba64[((frame_bounds.min.y + y) * canvas_width + frame_bounds.min.x + x) as usize] =
                narrow(color);
        }
    }
    Ok(Decoded {
        asset: PreparedRasterAsset {
            bounds: canvas,
            width: canvas_width,
            rgba64,
            nrgba: None,
        },
        sixteen_bit: false,
    })
}

/// JPEG via zune-jpeg. Go's image/jpeg integer IDCT and YCbCr conversion are
/// not yet reproduced bit-for-bit (no corpus board embeds a JPEG).
fn decode_jpeg(data: &[u8]) -> Result<Decoded, String> {
    let mut input = d2_rs_core::jpeg_first::Bytes { data, offset: 0 };
    let image = d2_rs_core::jpeg_first::decode(data, &mut input, &|| Ok(()))?;
    let (width, height) = (image.width as i64, image.height as i64);
    let mut rgba64 = Samples::with_capacity(image.rgba.len());
    for pixel in image.rgba {
        rgba64.push(pixel.map(u32::from));
    }
    Ok(Decoded {
        asset: PreparedRasterAsset {
            bounds: Rect::new(0, 0, width, height),
            width,
            rgba64: rgba64.0,
            nrgba: None,
        },
        sixteen_bit: false,
    })
}

/// WebP via image-webp; lossy NYCbCrA conversion is not yet bit-exact.
fn decode_webp(data: &[u8]) -> Result<Decoded, String> {
    let mut decoder =
        image_webp::WebPDecoder::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
    // x/image/webp exposes opaque VP8 as image.YCbCr. Its RGBA samples
    // retain sixteen-bit conversion precision, which is needed before the
    // rasterizer interpolates scaled images.
    if !decoder.is_animated() && !decoder.has_alpha() && decoder.is_lossy() {
        let (pixels, width, height, _) = crate::first_frame::webp_rgba(data)?;
        let mut rgba64 = Samples::with_capacity(pixels.len());
        for pixel in pixels {
            rgba64.push(pixel.map(u32::from));
        }
        return Ok(Decoded {
            asset: PreparedRasterAsset {
                bounds: Rect::new(0, 0, i64::from(width), i64::from(height)),
                width: i64::from(width),
                rgba64: rgba64.0,
                nrgba: None,
            },
            sixteen_bit: false,
        });
    }
    let (width, height) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let mut buffer = vec![
        0u8;
        decoder
            .output_buffer_size()
            .ok_or("webp: image too large")?
    ];
    decoder.read_image(&mut buffer).map_err(|e| e.to_string())?;
    let (width, height) = (width as usize, height as usize);
    let mut rgba64 = Samples::with_capacity(width * height);
    let mut raw = Vec::with_capacity(width * height * 4);
    for i in 0..width * height {
        let (r, g, b, a) = if alpha {
            (
                buffer[4 * i],
                buffer[4 * i + 1],
                buffer[4 * i + 2],
                buffer[4 * i + 3],
            )
        } else {
            (buffer[3 * i], buffer[3 * i + 1], buffer[3 * i + 2], 0xff)
        };
        raw.extend_from_slice(&[r, g, b, a]);
        rgba64.push(nrgba_color(r, g, b, a));
    }
    let bounds = Rect::new(0, 0, width as i64, height as i64);
    Ok(Decoded {
        asset: PreparedRasterAsset {
            bounds,
            width: width as i64,
            rgba64: rgba64.0,
            nrgba: Some(raw),
        },
        sixteen_bit: false,
    })
}

// ------------------------------------------------------------- placement

pub fn image_placement(
    r#box: SceneBox,
    source_width: i64,
    source_height: i64,
    aspect: AspectRatio,
) -> Result<SceneBox, String> {
    if aspect.align == AspectAlign::NONE || r#box.width == 0.0 || r#box.height == 0.0 {
        return Ok(r#box);
    }
    let (source_width, source_height) = (source_width as f64, source_height as f64);
    let sx = r#box.width / source_width;
    let sy = r#box.height / source_height;
    let mut scale = go_min(sx, sy);
    if aspect.fit == AspectFit::SLICE {
        scale = go_max(sx, sy);
    }
    let width = source_width * scale;
    let height = source_height * scale;
    if !finite(scale) || !finite(width) || !finite(height) || scale <= 0.0 {
        return Err("placement is outside the finite positive domain".into());
    }
    let Some((x_factor, y_factor)) = aspect_alignment_factors(aspect.align) else {
        return Err(format!("unsupported alignment {}", aspect.align.0));
    };
    // ARM64 image.go:392–393: the sizes are re-fused into the remainders.
    let placement = SceneBox {
        x: go_fmadd(
            go_fmsub(source_width, scale, r#box.width),
            x_factor,
            r#box.x,
        ),
        y: go_fmadd(
            go_fmsub(source_height, scale, r#box.height),
            y_factor,
            r#box.y,
        ),
        width,
        height,
    };
    // validateImagePlacement uses the rounded sizes (image.go:400–401).
    let x = go_fmadd(r#box.width - width, x_factor, r#box.x);
    let y = go_fmadd(r#box.height - height, y_factor, r#box.y);
    if !finite(x) || !finite(y) {
        return Err("placement origin is outside the finite numeric domain".into());
    }
    Ok(placement)
}

fn aspect_alignment_factors(align: AspectAlign) -> Option<(f64, f64)> {
    Some(match align {
        AspectAlign::X_MIN_Y_MIN => (0.0, 0.0),
        AspectAlign::X_MID_Y_MIN => (0.5, 0.0),
        AspectAlign::X_MAX_Y_MIN => (1.0, 0.0),
        AspectAlign::X_MIN_Y_MID => (0.0, 0.5),
        AspectAlign::X_MID_Y_MID => (0.5, 0.5),
        AspectAlign::X_MAX_Y_MID => (1.0, 0.5),
        AspectAlign::X_MIN_Y_MAX => (0.0, 1.0),
        AspectAlign::X_MID_Y_MAX => (0.5, 1.0),
        AspectAlign::X_MAX_Y_MAX => (1.0, 1.0),
        _ => return None,
    })
}

pub fn make_inverse_affine(transform: &Matrix) -> Result<Option<InverseAffine>, String> {
    let maximum = go_max(
        go_max(transform.a.abs(), transform.b.abs()),
        go_max(transform.c.abs(), transform.d.abs()),
    );
    if !finite(maximum) {
        return Err("linear coefficients are non-finite".into());
    }
    if maximum == 0.0 {
        return Ok(None);
    }
    let a = transform.a / maximum;
    let b = transform.b / maximum;
    let c = transform.c / maximum;
    let d = transform.d / maximum;
    // ARM64 image.go:448.
    let determinant = go_fmsub(b, c, a * d);
    if determinant == 0.0 {
        return Ok(None);
    }
    let inverse = InverseAffine {
        a: (d / determinant) / maximum,
        b: (-b / determinant) / maximum,
        c: (-c / determinant) / maximum,
        d: (a / determinant) / maximum,
        e: transform.e,
        f: transform.f,
    };
    if !finite(inverse.a) || !finite(inverse.b) || !finite(inverse.c) || !finite(inverse.d) {
        return Err("inverse is outside the finite numeric domain".into());
    }
    Ok(Some(inverse))
}

impl InverseAffine {
    pub const ZERO: InverseAffine = InverseAffine {
        a: 0.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 0.0,
        f: 0.0,
    };

    fn point(&self, point: Point) -> Option<Point> {
        let dx = point.x - self.e;
        let dy = point.y - self.f;
        let local = Point::new(
            self.a.mul_add(dx, self.c * dy),
            self.b.mul_add(dx, self.d * dy),
        );
        finite_point(local).then_some(local)
    }
}

pub fn transformed_image_pixel_bounds(
    r#box: SceneBox,
    transform: &Matrix,
    canvas: Rect,
) -> Result<Rect, String> {
    let points = [
        mpoint(transform, Point::new(r#box.x, r#box.y)),
        mpoint(transform, Point::new(r#box.x + r#box.width, r#box.y)),
        mpoint(
            transform,
            Point::new(r#box.x + r#box.width, r#box.y + r#box.height),
        ),
        mpoint(transform, Point::new(r#box.x, r#box.y + r#box.height)),
    ];
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for point in points {
        validate_raster_point(point)?;
        min_x = go_min(min_x, point.x);
        min_y = go_min(min_y, point.y);
        max_x = go_max(max_x, point.x);
        max_y = go_max(max_y, point.y);
    }
    if max_x <= canvas.min.x as f64
        || max_y <= canvas.min.y as f64
        || min_x >= canvas.max.x as f64
        || min_y >= canvas.max.y as f64
    {
        return Ok(Rect::default());
    }
    let min_x = go_max(min_x, canvas.min.x as f64);
    let min_y = go_max(min_y, canvas.min.y as f64);
    let max_x = go_min(max_x, canvas.max.x as f64);
    let max_y = go_min(max_y, canvas.max.y as f64);
    Ok(Rect::new(
        min_x.floor() as i64,
        min_y.floor() as i64,
        max_x.ceil() as i64,
        max_y.ceil() as i64,
    )
    .intersect(canvas))
}

// ---------------------------------------------------------------- drawing

pub fn draw_prepared_image(destination: &mut Rgba<'_>, prepared: &PreparedImage) {
    let bounds = prepared.bounds.intersect(destination.bounds());
    if bounds.empty() {
        return;
    }
    if let Some(origin) = native_raster_image_origin(prepared) {
        draw_native_size(destination, &prepared.asset, bounds, origin);
        return;
    }
    draw_sampled(destination, prepared, bounds);
}

fn native_raster_image_origin(prepared: &PreparedImage) -> Option<Pt> {
    let inverse = &prepared.inverse;
    let asset = &prepared.asset;
    if inverse.a <= 0.0
        || inverse.d <= 0.0
        || inverse.b != 0.0
        || inverse.c != 0.0
        || prepared.placement != prepared.r#box
        || prepared.r#box.width != inverse.a * asset.width as f64
        || prepared.r#box.height != inverse.d * asset.bounds.dy() as f64
        || !power_of_two(inverse.a)
        || !power_of_two(inverse.d)
    {
        return None;
    }
    let max_exact_sample_coordinate = (1u64 << 47) as f64;
    let local_device_x = prepared.r#box.x / inverse.a;
    let local_device_y = prepared.r#box.y / inverse.d;
    for value in [local_device_x, local_device_y, inverse.e, inverse.f] {
        if value != value.trunc() || value.abs() > max_exact_sample_coordinate {
            return None;
        }
    }
    let x = local_device_x + inverse.e;
    let y = local_device_y + inverse.f;
    let max_int = i64::MAX as f64;
    let min_int = -max_int - 1.0;
    if x != x.trunc() || y != y.trunc() || x < min_int || x > max_int || y < min_int || y > max_int
    {
        return None;
    }
    Some(Pt::new(x as i64, y as i64))
}

fn power_of_two(value: f64) -> bool {
    gomath::frexp(value).0 == 0.5
}

fn draw_native_size(
    destination: &mut Rgba<'_>,
    asset: &PreparedRasterAsset,
    bounds: Rect,
    origin: Pt,
) {
    for y in bounds.min.y..bounds.max.y {
        for x in bounds.min.x..bounds.max.x {
            let source_x = asset.bounds.min.x + x - origin.x;
            let source_y = asset.bounds.min.y + y - origin.y;
            let offset = destination.pix_offset(x, y);
            if let Some(raw) = &asset.nrgba {
                // compositeNativeNRGBA.
                let i = (((source_y - asset.bounds.min.y) * asset.width
                    + (source_x - asset.bounds.min.x))
                    * 4) as usize;
                let (r, g, b, a) = (raw[i], raw[i + 1], raw[i + 2], raw[i + 3]);
                if a == 0xff {
                    destination.pix[offset..offset + 4].copy_from_slice(&[r, g, b, a]);
                } else if a != 0 {
                    let alpha = a as u32;
                    let premultiply = |c: u8| ((c as u32 * alpha + 127) / 255) as u8;
                    composite_premultiplied_rgba8(
                        &mut destination.pix[offset..offset + 4],
                        premultiply(r),
                        premultiply(g),
                        premultiply(b),
                        a,
                    );
                }
                continue;
            }
            let source = asset.at(source_x, source_y);
            if source[3] == 0 {
                continue;
            }
            composite_premultiplied_rgba64(&mut destination.pix[offset..offset + 4], source);
        }
    }
}

fn composite_premultiplied_rgba8(destination: &mut [u8], red: u8, green: u8, blue: u8, alpha: u8) {
    let source_alpha = alpha as u32;
    let inverse_alpha = 255 - source_alpha;
    let mul255 = |value: u8| (value as u32 * inverse_alpha + 127) / 255;
    destination[0] = ((red as u32).min(source_alpha) + mul255(destination[0])) as u8;
    destination[1] = ((green as u32).min(source_alpha) + mul255(destination[1])) as u8;
    destination[2] = ((blue as u32).min(source_alpha) + mul255(destination[2])) as u8;
    destination[3] = (source_alpha + mul255(destination[3])) as u8;
}

fn draw_sampled(destination: &mut Rgba<'_>, prepared: &PreparedImage, bounds: Rect) {
    const SAMPLE_COUNT: u64 = (IMAGE_SAMPLES_PER_AXIS * IMAGE_SAMPLES_PER_AXIS) as u64;
    let asset_width = prepared.asset.width as f64;
    let asset_height = prepared.asset.bounds.dy() as f64;
    let placement = prepared.placement;
    for y in bounds.min.y..bounds.max.y {
        for x in bounds.min.x..bounds.max.x {
            let mut valid_samples = 0u64;
            let (mut source_x_sum, mut source_y_sum) = (0.0, 0.0);
            for sample_y in 0..IMAGE_SAMPLES_PER_AXIS {
                for sample_x in 0..IMAGE_SAMPLES_PER_AXIS {
                    let device = Point::new(
                        x as f64 + (sample_x as f64 + 0.5) / IMAGE_SAMPLES_PER_AXIS as f64,
                        y as f64 + (sample_y as f64 + 0.5) / IMAGE_SAMPLES_PER_AXIS as f64,
                    );
                    let Some(local) = prepared.inverse.point(device) else {
                        continue;
                    };
                    if !point_in_image_box(local, prepared.r#box) {
                        continue;
                    }
                    let ratio_x = (local.x - placement.x) / placement.width;
                    let ratio_y = (local.y - placement.y) / placement.height;
                    let source_x = ratio_x * asset_width;
                    let source_y = ratio_y * asset_height;
                    if !finite(source_x)
                        || !finite(source_y)
                        || source_x < 0.0
                        || source_y < 0.0
                        || source_x >= asset_width
                        || source_y >= asset_height
                    {
                        continue;
                    }
                    valid_samples += 1;
                    // ARM64 image.go:1054–1055: the sums re-fuse the products.
                    source_x_sum = go_fmadd(ratio_x, asset_width, source_x_sum);
                    source_y_sum = go_fmadd(ratio_y, asset_height, source_y_sum);
                }
            }
            if valid_samples == 0 {
                continue;
            }
            let sample = bilinear_premultiplied(
                &prepared.asset,
                source_x_sum / valid_samples as f64,
                source_y_sum / valid_samples as f64,
            );
            let mut source = [0u32; 4];
            for channel in 0..4 {
                source[channel] = ((sample[channel] as u64 * valid_samples + SAMPLE_COUNT / 2)
                    / SAMPLE_COUNT) as u32;
            }
            if source[3] == 0 {
                continue;
            }
            let offset = destination.pix_offset(x, y);
            composite_premultiplied_rgba64(&mut destination.pix[offset..offset + 4], source);
        }
    }
}

fn point_in_image_box(point: Point, r#box: SceneBox) -> bool {
    point.x >= r#box.x
        && point.y >= r#box.y
        && point.x < r#box.x + r#box.width
        && point.y < r#box.y + r#box.height
}

fn bilinear_premultiplied(asset: &PreparedRasterAsset, x: f64, y: f64) -> [u32; 4] {
    let height = asset.bounds.dy();
    let center_x = x - 0.5;
    let center_y = y - 0.5;
    let mut x0 = center_x.floor() as i64;
    let mut y0 = center_y.floor() as i64;
    let weight_x = center_x - x0 as f64;
    let weight_y = center_y - y0 as f64;
    let (mut x1, mut y1) = (x0 + 1, y0 + 1);
    x0 = x0.clamp(0, asset.width - 1);
    x1 = x1.clamp(0, asset.width - 1);
    y0 = y0.clamp(0, height - 1);
    y1 = y1.clamp(0, height - 1);
    if weight_x == 0.0 {
        x1 = x0;
    }
    if weight_y == 0.0 {
        y1 = y0;
    }
    let (x0, x1) = (x0 + asset.bounds.min.x, x1 + asset.bounds.min.x);
    let (y0, y1) = (y0 + asset.bounds.min.y, y1 + asset.bounds.min.y);
    let c00 = asset.at(x0, y0);
    let c10 = if x1 == x0 { c00 } else { asset.at(x1, y0) };
    let (c01, c11) = if y1 == y0 {
        (c00, c10)
    } else {
        let c01 = asset.at(x0, y1);
        (c01, if x1 == x0 { c01 } else { asset.at(x1, y1) })
    };
    if x0 == x1 && y0 == y1 {
        return c00;
    }
    let clamp_round = |value: f64| go_max(0.0, go_min(65535.0, value)).round() as u32;
    let mut result = [0u32; 4];
    for channel in 0..4 {
        let (a00, a10, a01, a11) = (
            c00[channel] as f64,
            c10[channel] as f64,
            c01[channel] as f64,
            c11[channel] as f64,
        );
        result[channel] = if x0 == x1 {
            clamp_round(go_fmadd(a01 - a00, weight_y, a00))
        } else if y0 == y1 {
            clamp_round(go_fmadd(a10 - a00, weight_x, a00))
        } else {
            let top = go_fmadd(a10 - a00, weight_x, a00);
            let bottom = go_fmadd(a11 - a01, weight_x, a01);
            clamp_round(go_fmadd(bottom - top, weight_y, top))
        };
    }
    result
}

pub fn composite_premultiplied_rgba64(destination: &mut [u8], source: [u32; 4]) {
    let to_byte = |value: u32| (value + 128) / 257;
    if source[3] == 0xffff {
        destination[0] = to_byte(source[0]) as u8;
        destination[1] = to_byte(source[1]) as u8;
        destination[2] = to_byte(source[2]) as u8;
        destination[3] = 0xff;
        return;
    }
    let source_alpha = to_byte(source[3]);
    if source_alpha == 0 {
        return;
    }
    let inverse_alpha = 255 - source_alpha;
    let mul255 = |left: u32, right: u32| (left * right + 127) / 255;
    for channel in 0..3 {
        let source_channel = to_byte(source[channel]).min(source_alpha);
        destination[channel] =
            (source_channel + mul255(destination[channel] as u32, inverse_alpha)).min(255) as u8;
    }
    destination[3] = (source_alpha + mul255(destination[3] as u32, inverse_alpha)).min(255) as u8;
}
