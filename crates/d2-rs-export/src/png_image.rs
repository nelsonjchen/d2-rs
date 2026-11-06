// SPDX-License-Identifier: MPL-2.0
//! Source d2raster/image.go PNG decoding, coverage and premultiplied sampling.
use resvg::usvg;
use std::io::Cursor;

use d2_rs_render::go_float::go_fmadd as go_mul_add;

/// d2svgimport/viewport.go and d2raster/image.go keep alignment separate
/// from the original clip box and from the node's affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageAspect {
    alignment: Option<[f64; 2]>,
    slice: bool,
}

impl ImageAspect {
    /// Vector images map the imported viewport in scene f64 coordinates;
    /// raster texel placement below is a separate source contract.
    pub(crate) fn vector_matrix(self, source: [f64; 4], destination: [f64; 4]) -> Option<[f64; 6]> {
        let align = self
            .alignment
            .map_or(0, |[x, y]| 1 + (x * 2.0) as u8 + (y * 6.0) as u8);
        d2_rs_math::svg_viewport::aspect_matrix(source, destination, [align, u8::from(self.slice)])
            .ok()
    }

    pub(crate) fn parse(raw: Option<&str>) -> Option<Self> {
        let raw = raw.unwrap_or("");
        if raw.len() > 128 {
            return None;
        }
        let fields: Vec<_> = raw.split_whitespace().collect();
        let align = fields.first().copied().unwrap_or("xMidYMid");
        let alignment = match align {
            "none" => None,
            "xMinYMin" => Some([0.0, 0.0]),
            "xMidYMin" => Some([0.5, 0.0]),
            "xMaxYMin" => Some([1.0, 0.0]),
            "xMinYMid" => Some([0.0, 0.5]),
            "xMidYMid" => Some([0.5, 0.5]),
            "xMaxYMid" => Some([1.0, 0.5]),
            "xMinYMax" => Some([0.0, 1.0]),
            "xMidYMax" => Some([0.5, 1.0]),
            "xMaxYMax" => Some([1.0, 1.0]),
            _ => return None,
        };
        if fields.len() > 2 {
            return None;
        }
        let slice = match fields.get(1).copied().unwrap_or("meet") {
            "meet" => false,
            "slice" => true,
            _ => return None,
        };
        Some(Self { alignment, slice })
    }

    fn placement(self, image_box: [f64; 4], width: usize, height: usize) -> Option<[f64; 4]> {
        let Some([ax, ay]) = self.alignment else {
            return Some(image_box);
        };
        if image_box[2] == 0.0 || image_box[3] == 0.0 {
            return Some(image_box);
        }
        let sx = image_box[2] / width as f64;
        let sy = image_box[3] / height as f64;
        let scale = if self.slice { sx.max(sy) } else { sx.min(sy) };
        let encoded_width = width as f64;
        let encoded_height = height as f64;
        let width = encoded_width * scale;
        let height = encoded_height * scale;
        // Pinned Go arm64 lowers the placement subtraction and translation
        // to fused operations; keep the rounded extent separately for sampling.
        #[cfg(not(target_arch = "aarch64"))]
        let (dx, dy) = (image_box[2] - width, image_box[3] - height);
        #[cfg(target_arch = "aarch64")]
        let (dx, dy) = (
            (-encoded_width).mul_add(scale, image_box[2]),
            (-encoded_height).mul_add(scale, image_box[3]),
        );
        #[cfg(not(target_arch = "aarch64"))]
        let origin = [image_box[0] + dx * ax, image_box[1] + dy * ay];
        #[cfg(target_arch = "aarch64")]
        let origin = [dx.mul_add(ax, image_box[0]), dy.mul_add(ay, image_box[1])];
        let placement = [origin[0], origin[1], width, height];
        (scale.is_finite() && scale > 0.0 && placement.iter().all(|v| v.is_finite()))
            .then_some(placement)
    }
}

pub(crate) struct Asset {
    width: usize,
    height: usize,
    pixels: AssetPixels,
}

/// Decoded image samples, at the source depth.
#[derive(Debug, PartialEq)]
enum AssetPixels {
    // Straight (unpremultiplied) eight-bit RGBA, as decoded.
    Eight(Vec<[u8; 4]>),
    // Premultiplied sixteen-bit samples; every value fits in a u16.
    Sixteen(Vec<[u16; 4]>),
    // Lossy WebP planes, converted per texel like image/color.YCbCr.RGBA.
    YCbCr(YCbCrPlanes),
}

/// x/image/webp's lossy image model: VP8 4:2:0 luma and chroma planes (with
/// the decoder's macroblock strides) and an optional eight-bit alpha plane.
#[derive(Debug, PartialEq)]
pub(crate) struct YCbCrPlanes {
    pub(crate) width: usize,
    pub(crate) stride: usize,
    pub(crate) chroma_stride: usize,
    pub(crate) y: Vec<u8>,
    pub(crate) cb: Vec<u8>,
    pub(crate) cr: Vec<u8>,
    pub(crate) alpha: Option<Vec<u8>>,
}

impl YCbCrPlanes {
    /// Straight sixteen-bit RGBA of pixel (x, y): image/color.YCbCr.RGBA on
    /// the nearest 4:2:0 chroma sample, alpha widened by 257.
    pub(crate) fn rgba16(&self, x: usize, y: usize) -> [u16; 4] {
        let yy = i32::from(self.y[y * self.stride + x]) * 0x10101;
        let ci = (y / 2) * self.chroma_stride + x / 2;
        let cb = i32::from(self.cb[ci]) - 128;
        let cr = i32::from(self.cr[ci]) - 128;
        [
            ((yy + 91881 * cr) >> 8).clamp(0, 65535) as u16,
            ((yy - 22554 * cb - 46802 * cr) >> 8).clamp(0, 65535) as u16,
            ((yy + 116130 * cb) >> 8).clamp(0, 65535) as u16,
            self.alpha
                .as_ref()
                .map_or(65535, |a| u16::from(a[y * self.width + x]) * 257),
        ]
    }
}

/// Premultiply straight sixteen-bit RGBA exactly as sixteen-bit PNG samples
/// are at decode.
fn premultiply16([r, g, b, a]: [u32; 4]) -> [u16; 4] {
    let premultiply = |value: u32| (u64::from(value) * u64::from(a) / 65535) as u16;
    [premultiply(r), premultiply(g), premultiply(b), a as u16]
}

impl Asset {
    /// Straight eight-bit RGBA pixels, as an eight-bit RGBA PNG decodes.
    pub(crate) fn straight_rgba8(width: usize, height: usize, rgba: Vec<[u8; 4]>) -> Option<Self> {
        (width > 0 && height > 0 && width.checked_mul(height)? == rgba.len()).then_some(Self {
            width,
            height,
            pixels: AssetPixels::Eight(rgba),
        })
    }

    /// Lossy WebP planes; texels equal those of their sixteen-bit RGBA PNG.
    pub(crate) fn ycbcr(width: usize, height: usize, planes: YCbCrPlanes) -> Option<Self> {
        (width > 0 && height > 0 && planes.width == width).then_some(Self {
            width,
            height,
            pixels: AssetPixels::YCbCr(planes),
        })
    }

    /// internal/rasterimage compositeFrame: an animation's first frame at
    /// `(x, y)` on a transparent NRGBA logical canvas. A frame that fills the
    /// canvas is returned as decoded. draw.Draw copies NRGBA bytes for a
    /// non-blending (Src) NRGBA frame; every other case reaches the
    /// RGBA64Image fallback, which on the transparent canvas stores
    /// NRGBA.SetRGBA64(src.RGBA64At) for both Over and Src.
    pub(crate) fn on_canvas(
        self,
        canvas_width: usize,
        canvas_height: usize,
        x: usize,
        y: usize,
        source_blend: bool,
    ) -> Option<Self> {
        if x == 0 && y == 0 && self.width == canvas_width && self.height == canvas_height {
            return Some(self);
        }
        if x.checked_add(self.width)? > canvas_width || y.checked_add(self.height)? > canvas_height
        {
            return None;
        }
        let mut canvas = vec![[0_u8; 4]; canvas_width.checked_mul(canvas_height)?];
        for row in 0..self.height {
            for column in 0..self.width {
                let index = row * self.width + column;
                let pixel = match &self.pixels {
                    AssetPixels::Eight(pixels) if source_blend => pixels[index],
                    _ => {
                        let [r, g, b, a] = self.texel(index);
                        let straight = |value: u32| {
                            if a != 0 && a != 0xffff {
                                (value * 0xffff / a) as u16
                            } else {
                                value as u16
                            }
                        };
                        [
                            (straight(r) >> 8) as u8,
                            (straight(g) >> 8) as u8,
                            (straight(b) >> 8) as u8,
                            (a >> 8) as u8,
                        ]
                    }
                };
                canvas[(y + row) * canvas_width + x + column] = pixel;
            }
        }
        Self::straight_rgba8(canvas_width, canvas_height, canvas)
    }

    /// Premultiplied sixteen-bit RGBA of pixel `index`. Eight-bit samples
    /// widen by 257 and premultiply exactly as sixteen-bit ones are at decode.
    fn texel(&self, index: usize) -> [u32; 4] {
        match &self.pixels {
            AssetPixels::Sixteen(pixels) => pixels[index].map(u32::from),
            AssetPixels::Eight(pixels) => {
                let [r, g, b, a] = pixels[index].map(|value| u32::from(value) * 257);
                let premultiply = |value: u32| (u64::from(value) * u64::from(a) / 65535) as u32;
                [premultiply(r), premultiply(g), premultiply(b), a]
            }
            AssetPixels::YCbCr(planes) => premultiply16(
                planes
                    .rgba16(index % self.width, index / self.width)
                    .map(u32::from),
            )
            .map(u32::from),
        }
    }
}

/// Decoded assets handed from raster asset normalization to the sampler
/// without a PNG round trip. The SVG carries a placeholder PNG whose header
/// gives the image's dimensions and depth (all that SVG parsing reads) and
/// whose private chunk names the registered asset; it has no image data, so
/// no other PNG reader can mistake it for pixels.
static HANDOFFS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<[u8; 16], std::sync::Arc<Asset>>>,
> = std::sync::LazyLock::new(Default::default);
const HANDOFF_CHUNK: &[u8; 4] = b"d2Rh";
const HANDOFF_PNG_BYTES: usize = 8 + 25 + 28 + 12;

/// Registered handoff assets, released when the scope ends.
#[derive(Default)]
pub(crate) struct HandoffScope {
    keys: Vec<[u8; 16]>,
    prepared: std::collections::HashMap<[u8; 32], Vec<u8>>,
    original_pngs: std::collections::HashMap<[u8; 16], Vec<u8>>,
}

impl HandoffScope {
    /// d2raster's prepared raster assets share decoded pixels across aliases,
    /// image uses and bands. Keep the encoded PNG for the resvg fallback,
    /// whose own decoder must see the original sample precision.
    pub(crate) fn prepare(
        &mut self,
        data: &[u8],
        preserve_png: bool,
        decode: impl FnOnce() -> Option<Asset>,
    ) -> Option<Vec<u8>> {
        use sha2::Digest as _;
        if handed_off(data).is_some() {
            return Some(data.to_vec());
        }
        let digest: [u8; 32] = sha2::Sha256::digest(data).into();
        if let Some(png) = self.prepared.get(&digest) {
            return Some(png.clone());
        }
        let png = self.hand_off(decode()?)?;
        if preserve_png {
            self.original_pngs.insert(*self.keys.last()?, data.to_vec());
        }
        self.prepared.insert(digest, png.clone());
        Some(png)
    }

    /// Register `asset` and return its placeholder PNG, whose header declares
    /// the depth of the PNG the asset would otherwise travel in.
    pub(crate) fn hand_off(&mut self, asset: Asset) -> Option<Vec<u8>> {
        use std::hash::{BuildHasher, Hasher};
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // A per-process random half keeps an input PNG from naming a key.
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(serial);
        let mut key = [0_u8; 16];
        key[..8].copy_from_slice(&serial.to_be_bytes());
        key[8..].copy_from_slice(&hasher.finish().to_be_bytes());
        let png = asset.placeholder_png(&key)?;
        HANDOFFS
            .lock()
            .unwrap()
            .insert(key, std::sync::Arc::new(asset));
        self.keys.push(key);
        Some(png)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// `svg` with every placeholder data URL replaced by a PNG encoding of
    /// its asset (the form the asset otherwise travels in), for a renderer
    /// that decodes images itself.
    pub(crate) fn materialize(&self, svg: &str) -> String {
        use base64::Engine as _;
        let mut svg = svg.to_owned();
        let handoffs = HANDOFFS.lock().unwrap_or_else(|e| e.into_inner());
        for key in &self.keys {
            let Some(asset) = handoffs.get(key) else {
                continue;
            };
            let png = self
                .original_pngs
                .get(key)
                .cloned()
                .or_else(|| asset.encode_png());
            let Some((placeholder, png)) = asset.placeholder_png(key).zip(png) else {
                continue;
            };
            let engine = base64::engine::general_purpose::STANDARD;
            svg = svg.replace(&engine.encode(placeholder), &engine.encode(png));
        }
        svg
    }
}

impl Asset {
    fn placeholder_png(&self, key: &[u8; 16]) -> Option<Vec<u8>> {
        let width = u32::try_from(self.width).ok()?;
        let height = u32::try_from(self.height).ok()?;
        let sixteen_bit = !matches!(self.pixels, AssetPixels::Eight(_));
        Some(placeholder_png(width, height, sixteen_bit, key))
    }

    /// The asset as the straight-alpha PNG it was decoded from.
    fn encode_png(&self) -> Option<Vec<u8>> {
        let (depth, bytes) = match &self.pixels {
            AssetPixels::Eight(pixels) => (png::BitDepth::Eight, pixels.as_flattened().to_vec()),
            AssetPixels::YCbCr(planes) => {
                let mut bytes = Vec::with_capacity(self.width * self.height * 8);
                for y in 0..self.height {
                    for x in 0..self.width {
                        for channel in planes.rgba16(x, y) {
                            bytes.extend_from_slice(&channel.to_be_bytes());
                        }
                    }
                }
                (png::BitDepth::Sixteen, bytes)
            }
            // Premultiplied samples have no exact straight form.
            AssetPixels::Sixteen(_) => return None,
        };
        let mut output = Vec::new();
        let mut encoder = png::Encoder::new(
            &mut output,
            u32::try_from(self.width).ok()?,
            u32::try_from(self.height).ok()?,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(depth);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&bytes).ok()?;
        drop(writer);
        Some(output)
    }
}

fn placeholder_png(width: u32, height: u32, sixteen_bit: bool, key: &[u8; 16]) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], payload: &[u8]| {
        png.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(payload);
        let crc = crc32(kind.iter().chain(payload).copied());
        png.extend_from_slice(&crc.to_be_bytes());
    };
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[if sixteen_bit { 16 } else { 8 }, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(HANDOFF_CHUNK, key);
    chunk(b"IEND", &[]);
    debug_assert_eq!(png.len(), HANDOFF_PNG_BYTES);
    png
}

impl Drop for HandoffScope {
    fn drop(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let mut handoffs = HANDOFFS.lock().unwrap_or_else(|e| e.into_inner());
        for key in &self.keys {
            handoffs.remove(key);
        }
    }
}

fn handed_off(data: &[u8]) -> Option<std::sync::Arc<Asset>> {
    if data.len() != HANDOFF_PNG_BYTES || &data[37..41] != HANDOFF_CHUNK {
        return None;
    }
    let key: [u8; 16] = data[41..57].try_into().ok()?;
    HANDOFFS.lock().ok()?.get(&key).cloned()
}

fn crc32(bytes: impl Iterator<Item = u8>) -> u32 {
    let mut crc = 0xffffffffu32;
    for byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}

/// Adapt Go's legal empty-dictionary zlib header for PNG backends that reject
/// FDICT. The DEFLATE payload and pixels are unchanged; encoded source bytes
/// and their asset identity remain untouched outside the decoder.
pub(crate) fn empty_dictionary_decoder_bytes(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Cow::Borrowed(data);
    }
    // Most assets have no dictionary. Probe at most six zlib header bytes
    // across IDAT chunks before allocating an adapted encoded image.
    let mut header = [0u8; 6];
    let mut filled = 0;
    let mut probe = 8usize;
    while probe + 12 <= data.len() && filled < 6 {
        let length = u32::from_be_bytes(data[probe..probe + 4].try_into().unwrap()) as usize;
        let Some(end) = probe
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .filter(|&v| v <= data.len())
        else {
            return Cow::Borrowed(data);
        };
        if &data[probe + 4..probe + 8] == b"IDAT" {
            for &byte in data[probe + 8..probe + 8 + length].iter().take(6 - filled) {
                header[filled] = byte;
                filled += 1;
            }
            if filled >= 2 && header[1] & 0x20 == 0 {
                return Cow::Borrowed(data);
            }
        }
        probe = end;
    }
    if filled < 6 || header[2..6] != [0, 0, 0, 1] {
        return Cow::Borrowed(data);
    }
    let mut chunks = Vec::new();
    let mut entropy = Vec::new();
    let mut offset = 8usize;
    while offset + 12 <= data.len() {
        let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let Some(end) = offset
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .filter(|&v| v <= data.len())
        else {
            return Cow::Borrowed(data);
        };
        let kind = &data[offset + 4..offset + 8];
        // Never make a corrupt source chunk valid by regenerating its CRC.
        if crc32(data[offset + 4..end - 4].iter().copied())
            != u32::from_be_bytes(data[end - 4..end].try_into().unwrap())
        {
            return Cow::Borrowed(data);
        }
        if kind == b"IDAT" {
            entropy.extend_from_slice(&data[offset + 8..offset + 8 + length]);
        }
        chunks.push((offset, end, kind == b"IDAT"));
        offset = end;
    }
    if offset != data.len()
        || entropy.len() < 6
        || entropy[0] & 15 != 8
        || entropy[0] >> 4 > 7
        || (u16::from(entropy[0]) * 256 + u16::from(entropy[1])) % 31 != 0
        || entropy[1] & 0x20 == 0
        || entropy[2..6] != [0, 0, 0, 1]
    {
        return Cow::Borrowed(data);
    }
    let flags = entropy[1] & 0xc0;
    let check = (31 - (u16::from(entropy[0]) * 256 + u16::from(flags)) % 31) % 31;
    let mut payload = vec![entropy[0], flags | check as u8];
    payload.extend_from_slice(&entropy[6..]);
    let mut out = data[..8].to_vec();
    let mut inserted = false;
    for (start, end, idat) in chunks {
        if !idat {
            out.extend_from_slice(&data[start..end]);
            continue;
        }
        if inserted {
            continue;
        }
        inserted = true;
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(b"IDAT");
        out.extend_from_slice(&payload);
        out.extend_from_slice(&crc32(b"IDAT".iter().chain(payload.iter()).copied()).to_be_bytes());
    }
    Cow::Owned(out)
}

fn decode(data: &[u8]) -> Option<std::sync::Arc<Asset>> {
    if let Some(asset) = handed_off(data) {
        return Some(asset);
    }
    decode_png(data).map(std::sync::Arc::new)
}

pub(crate) fn decode_png(data: &[u8]) -> Option<Asset> {
    let adapted = empty_dictionary_decoder_bytes(data);
    let data = adapted.as_ref();
    let mut decoder = png::Decoder::new(Cursor::new(data));
    // Expand palettes/sub-byte samples without stripping sixteen-bit precision.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let (color_type, bit_depth) = reader.output_color_type();
    let width = usize::try_from(reader.info().width).ok()?;
    let height = usize::try_from(reader.info().height).ok()?;
    let interlaced = reader.info().interlaced;
    let count = width.checked_mul(height)?;
    if width == 0 || height == 0 {
        return None;
    }
    let sample_bytes = match bit_depth {
        png::BitDepth::Eight => 1,
        png::BitDepth::Sixteen => 2,
        _ => return None,
    };
    let channels = match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let stride = channels * sample_bytes;
    let row_bytes = width.checked_mul(stride)?;
    let mut eight = Vec::with_capacity(if sample_bytes == 1 { count } else { 0 });
    let mut pixels = Vec::with_capacity(if sample_bytes == 1 { 0 } else { count });
    let mut convert = |samples: &[u8]| {
        if sample_bytes == 1 {
            // Keep eight-bit samples as decoded (texel premultiplies them).
            for pixel in samples.chunks_exact(stride) {
                eight.push(match channels {
                    1 => [pixel[0], pixel[0], pixel[0], 255],
                    2 => [pixel[0], pixel[0], pixel[0], pixel[1]],
                    3 => [pixel[0], pixel[1], pixel[2], 255],
                    _ => [pixel[0], pixel[1], pixel[2], pixel[3]],
                });
            }
            return;
        }
        for pixel in samples.chunks_exact(stride) {
            let sample = |index: usize| -> u32 {
                if sample_bytes == 1 {
                    u32::from(pixel[index]) * 257
                } else {
                    u32::from(u16::from_be_bytes([pixel[index * 2], pixel[index * 2 + 1]]))
                }
            };
            let (r, g, b, a) = match channels {
                1 => (sample(0), sample(0), sample(0), 65535),
                2 => (sample(0), sample(0), sample(0), sample(1)),
                3 => (sample(0), sample(1), sample(2), 65535),
                _ => (sample(0), sample(1), sample(2), sample(3)),
            };
            let premultiply = |value: u32| (u64::from(value) * u64::from(a) / 65535) as u16;
            pixels.push([premultiply(r), premultiply(g), premultiply(b), a as u16]);
        }
    };
    if interlaced {
        let mut bytes = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut bytes).ok()?;
        if info.buffer_size() != count.checked_mul(stride)? {
            return None;
        }
        convert(&bytes[..info.buffer_size()]);
    } else {
        // Convert row by row rather than holding the complete encoded-depth
        // frame next to the converted samples.
        for _ in 0..height {
            let row = reader.next_row().ok()??;
            if row.data().len() != row_bytes {
                return None;
            }
            convert(row.data());
        }
    }
    let pixels = if sample_bytes == 1 {
        AssetPixels::Eight(eight)
    } else {
        AssetPixels::Sixteen(pixels)
    };
    Some(Asset {
        width,
        height,
        pixels,
    })
}

impl Asset {
    fn bilinear(&self, x: f64, y: f64) -> [u32; 4] {
        let center_x = x - 0.5;
        let center_y = y - 0.5;
        let x0 = center_x.floor();
        let y0 = center_y.floor();
        let wx = center_x - x0;
        let wy = center_y - y0;
        let clamp_x = |x: f64| x.clamp(0.0, (self.width - 1) as f64) as usize;
        let clamp_y = |y: f64| y.clamp(0.0, (self.height - 1) as f64) as usize;
        let ix0 = clamp_x(x0);
        let ix1 = if wx == 0.0 { ix0 } else { clamp_x(x0 + 1.0) };
        let iy0 = clamp_y(y0);
        let iy1 = if wy == 0.0 { iy0 } else { clamp_y(y0 + 1.0) };
        let c00 = self.texel(iy0 * self.width + ix0);
        let c10 = self.texel(iy0 * self.width + ix1);
        let c01 = self.texel(iy1 * self.width + ix0);
        let c11 = self.texel(iy1 * self.width + ix1);
        if ix0 == ix1 && iy0 == iy1 {
            return c00;
        }
        std::array::from_fn(|channel| {
            let value = if ix0 == ix1 {
                go_mul_add(
                    f64::from(c01[channel]) - f64::from(c00[channel]),
                    wy,
                    f64::from(c00[channel]),
                )
            } else if iy0 == iy1 {
                go_mul_add(
                    f64::from(c10[channel]) - f64::from(c00[channel]),
                    wx,
                    f64::from(c00[channel]),
                )
            } else {
                let top = go_mul_add(
                    f64::from(c10[channel]) - f64::from(c00[channel]),
                    wx,
                    f64::from(c00[channel]),
                );
                let bottom = go_mul_add(
                    f64::from(c11[channel]) - f64::from(c01[channel]),
                    wx,
                    f64::from(c01[channel]),
                );
                go_mul_add(bottom - top, wy, top)
            };
            value.clamp(0.0, 65535.0).round() as u32
        })
    }
}

fn composite(destination: &mut [u8], source: [u32; 4]) {
    let to_byte = |value: u32| (value + 128) / 257;
    let alpha = to_byte(source[3]);
    if alpha == 0 {
        return;
    }
    let inverse = 255 - alpha;
    for channel in 0..3 {
        let source_channel = to_byte(source[channel]).min(alpha);
        destination[channel] = (source_channel
            + (u32::from(destination[channel]) * inverse + 127) / 255)
            .min(255) as u8;
    }
    destination[3] = (alpha + (u32::from(destination[3]) * inverse + 127) / 255).min(255) as u8;
}

pub(crate) fn draw(
    image: &usvg::Image,
    data: &[u8],
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    scene: Option<&crate::svg_precision::SceneImagePlacement>,
) -> bool {
    draw_inner(
        Some(image),
        data,
        pixels,
        width,
        height,
        scale,
        opacity,
        scene,
    )
}
pub(crate) fn draw_scene(
    data: &[u8],
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scene: &crate::svg_precision::SceneImagePlacement,
) -> bool {
    draw_inner(None, data, pixels, width, height, 1., 1., Some(scene))
}
fn draw_inner(
    image: Option<&usvg::Image>,
    data: &[u8],
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    scene: Option<&crate::svg_precision::SceneImagePlacement>,
) -> bool {
    if image.is_some_and(|image| !image.is_visible()) {
        return true;
    }
    let Some(asset) = decode(data) else {
        return false;
    };
    draw_asset(image, &asset, pixels, width, height, scale, opacity, scene)
}

/// `draw_scene` for an already decoded asset.
pub(crate) fn draw_scene_asset(
    asset: &Asset,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scene: &crate::svg_precision::SceneImagePlacement,
) -> bool {
    draw_asset(None, asset, pixels, width, height, 1., 1., Some(scene))
}

#[allow(clippy::too_many_arguments)]
fn draw_asset(
    image: Option<&usvg::Image>,
    asset: &Asset,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    scale: f32,
    opacity: f32,
    scene: Option<&crate::svg_precision::SceneImagePlacement>,
) -> bool {
    let transform = image.map(|image| image.abs_transform()).unwrap_or_default();
    let scale = f64::from(scale);
    let matrix = scene.map(|scene| scene.matrix).unwrap_or([
        f64::from(transform.sx),
        f64::from(transform.ky),
        f64::from(transform.kx),
        f64::from(transform.sy),
        f64::from(transform.tx),
        f64::from(transform.ty),
    ]);
    let [a, b, c, d, e, f] = matrix.map(|value| value * scale);
    let image_box = scene.map(|scene| scene.image_box).unwrap_or([
        0.0,
        0.0,
        asset.width as f64,
        asset.height as f64,
    ]);
    let placement = if let Some(scene) = scene {
        let Some(placement) = scene.aspect.placement(image_box, asset.width, asset.height) else {
            return false;
        };
        placement
    } else {
        image_box
    };
    let maximum = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if !maximum.is_finite() {
        return false;
    }
    if maximum == 0.0 {
        return true;
    }
    let (na, nb, nc, nd) = (a / maximum, b / maximum, c / maximum, d / maximum);
    let determinant = na * nd - nb * nc;
    if determinant == 0.0 {
        return true;
    }
    let (ia, ib, ic, id) = (
        (nd / determinant) / maximum,
        (-nb / determinant) / maximum,
        (-nc / determinant) / maximum,
        (na / determinant) / maximum,
    );
    if [ia, ib, ic, id, e, f].iter().any(|v| !v.is_finite()) {
        return false;
    }
    let (left, top, right, bottom) = if scene.is_some() {
        let [x, y, w, h] = image_box;
        let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
            .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
        (
            corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min),
            corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
            corners
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max),
            corners
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max),
        )
    } else {
        let bounds = image.unwrap().abs_bounding_box();
        (
            f64::from(bounds.left()) * scale,
            f64::from(bounds.top()) * scale,
            f64::from(bounds.right()) * scale,
            f64::from(bounds.bottom()) * scale,
        )
    };
    let left = left.floor().clamp(0.0, width as f64) as usize;
    let right = right.ceil().clamp(0.0, width as f64) as usize;
    let top = top.floor().clamp(0.0, height as f64) as usize;
    let bottom = bottom.ceil().clamp(0.0, height as f64) as usize;
    // d2raster drawPreparedImage intersects the image's bounds with its
    // destination band. Filter inputs suspend this window while they draw.
    let (band_top, band_bottom) =
        crate::scanline::frame_band_rows(width, height).unwrap_or((0, height));
    let (top, bottom) = (top.max(band_top), bottom.min(band_bottom));
    for y in top..bottom {
        for x in left..right {
            let mut valid = 0_u64;
            let (mut sum_x, mut sum_y) = (0.0, 0.0);
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = x as f64 + (sx as f64 + 0.5) / 4.0 - e;
                    let dy = y as f64 + (sy as f64 + 0.5) / 4.0 - f;
                    let local_x = go_mul_add(ia, dx, ic * dy);
                    let local_y = go_mul_add(ib, dx, id * dy);
                    let [box_x, box_y, box_width, box_height] = image_box;
                    if local_x < box_x
                        || local_y < box_y
                        || local_x >= box_x + box_width
                        || local_y >= box_y + box_height
                    {
                        continue;
                    }
                    // Source keeps placement division before multiplication by
                    // encoded size, separately from the scene inverse above.
                    let (source_x, source_y) = if scene.is_some() {
                        let [placed_x, placed_y, placed_width, placed_height] = placement;
                        (
                            (local_x - placed_x) / placed_width * asset.width as f64,
                            (local_y - placed_y) / placed_height * asset.height as f64,
                        )
                    } else {
                        (local_x, local_y)
                    };
                    if source_x.is_finite()
                        && source_y.is_finite()
                        && source_x >= 0.0
                        && source_y >= 0.0
                        && source_x < asset.width as f64
                        && source_y < asset.height as f64
                    {
                        valid += 1;
                        if scene.is_some() && cfg!(target_arch = "aarch64") {
                            // Original Go keeps the tested source coordinate
                            // rounded but fuses its encoded-size multiplication
                            // into the sample sum (image.go1054/1055).
                            sum_x = ((local_x - placement[0]) / placement[2])
                                .mul_add(asset.width as f64, sum_x);
                            sum_y = ((local_y - placement[1]) / placement[3])
                                .mul_add(asset.height as f64, sum_y);
                        } else {
                            sum_x += source_x;
                            sum_y += source_y;
                        }
                    }
                }
            }
            if valid == 0 {
                continue;
            }
            let sample = asset.bilinear(sum_x / valid as f64, sum_y / valid as f64);
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var("D2_RS_TRACE_PNG_SAMPLES").ok().as_deref() == Some(&format!("{x},{y}"))
            {
                eprintln!(
                    "d2-rs-png-sample matrix={matrix:?} box={image_box:?} placement={placement:?} xy=({x},{y}) valid={valid} sum=({sum_x:.17e},{sum_y:.17e}) sample={sample:?}"
                );
            }
            let source = sample.map(|channel| {
                let covered = (u64::from(channel) * valid + 8) / 16;
                (covered as f64 * f64::from(opacity)).clamp(0.0, 65535.0) as u32
            });
            let offset = (y * width + x) * 4;
            composite(&mut pixels[offset..offset + 4], source);
        }
    }
    true
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_aspect_contract {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    fn image(group: &usvg::Group) -> Option<&usvg::Image> {
        for child in group.children() {
            match child {
                usvg::Node::Image(value) => return Some(value),
                usvg::Node::Group(value) => {
                    if let Some(found) = image(value) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn original_png_aspect_placement_and_sampled_pixels() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-image-aspect-source-v090.json"
        ))
        .unwrap();
        let mut outcomes = Vec::new();
        for row in rows.as_array().unwrap() {
            let values = |key: &str| {
                row[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect::<Vec<_>>()
            };
            let image_box: [f64; 4] = values("box").try_into().unwrap();
            let matrix: [f64; 6] = values("matrix").try_into().unwrap();
            let aspect = row["aspect"].as_str().unwrap();
            let encoded = row["png"].as_str().unwrap();
            let source = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><image id="probe" data-d2-rs-scene-image="ordinary" x="{}" y="{}" width="{}" height="{}" transform="matrix({} {} {} {} {} {})" preserveAspectRatio="{}" href="data:image/png;base64,{}"/></svg>"#,
                image_box[0],
                image_box[1],
                image_box[2],
                image_box[3],
                matrix[0],
                matrix[1],
                matrix[2],
                matrix[3],
                matrix[4],
                matrix[5],
                aspect,
                encoded
            );
            let (tagged, sources) = crate::svg_precision::tag_embedded_svg_images(&source);
            let scene = sources.scene_image("probe").unwrap();
            assert_eq!(
                scene.matrix.map(f64::to_bits),
                matrix.map(f64::to_bits),
                "{}",
                row["name"]
            );
            let data = STANDARD.decode(encoded).unwrap();
            let asset = decode(&data).unwrap();
            let placement = scene
                .aspect
                .placement(scene.image_box, asset.width, asset.height)
                .unwrap();
            let expected_placement: [f64; 4] = values("placement").try_into().unwrap();
            assert_eq!(
                placement.map(f64::to_bits),
                expected_placement.map(f64::to_bits),
                "{}",
                row["name"]
            );
            // The parsed scene discards singular images. The retained PNG image
            // supplies visibility; draw uses the independently retained matrix.
            let identity_source = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><image width="3" height="2" href="data:image/png;base64,{}"/></svg>"#,
                encoded
            );
            let tree = usvg::Tree::from_str(&identity_source, &usvg::Options::default()).unwrap();
            let _parsed_scene = usvg::Tree::from_str(&tagged, &usvg::Options::default()).unwrap();
            let mut pixels = [17, 29, 43, 255].repeat(24 * 24);
            assert!(draw(
                image(tree.root()).unwrap(),
                &data,
                &mut pixels,
                24,
                24,
                1.0,
                1.0,
                Some(scene)
            ));
            let expected = STANDARD.decode(row["pixels"].as_str().unwrap()).unwrap();
            let matched = pixels == expected;
            outcomes.push(serde_json::json!({"name":row["name"],"placement":placement,"pixels":STANDARD.encode(&pixels),"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_PNG_ASPECT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert_eq!(outcomes.len(), 552);
        let failed: Vec<_> = outcomes
            .iter()
            .filter(|row| row["matched"] != true)
            .map(|row| row["name"].clone())
            .collect();
        assert!(
            failed.is_empty(),
            "original PNG pixel differences: {failed:?}"
        );
    }
}

#[cfg(test)]
mod empty_dictionary_tests {
    use super::*;
    #[test]
    fn source_empty_dictionary_pixels_match_standard_header_without_repairing_crc() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../tests/fixtures/png-empty-dictionary-v090.json"
        ))
        .unwrap();
        for row in rows {
            let original: Vec<u8> = row["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            let adapted = empty_dictionary_decoder_bytes(&original);
            assert!(matches!(adapted, std::borrow::Cow::Owned(_)));
            let actual = decode(&original).expect("Go accepts the empty dictionary");
            let equivalent = decode(&adapted).unwrap();
            assert_eq!(
                (
                    actual.width,
                    actual.height,
                    (0..actual.width * actual.height)
                        .map(|index| actual.texel(index))
                        .collect::<Vec<_>>()
                ),
                (
                    equivalent.width,
                    equivalent.height,
                    (0..equivalent.width * equivalent.height)
                        .map(|index| equivalent.texel(index))
                        .collect::<Vec<_>>()
                )
            );
            assert!(matches!(
                empty_dictionary_decoder_bytes(&adapted),
                std::borrow::Cow::Borrowed(_)
            ));
            let mut bad = original.clone();
            let mut offset = 8;
            loop {
                let length =
                    u32::from_be_bytes(bad[offset..offset + 4].try_into().unwrap()) as usize;
                if &bad[offset + 4..offset + 8] == b"IDAT" {
                    bad[offset + 8 + length] ^= 1;
                    break;
                }
                offset += 12 + length;
            }
            assert!(matches!(
                empty_dictionary_decoder_bytes(&bad),
                std::borrow::Cow::Borrowed(_)
            ));
            assert!(decode(&bad).is_none());
            let asset = d2_rs_math::svg_image::RasterAsset {
                mime: "image/png".into(),
                data: original.clone(),
                width: 1,
                height: 1,
                decoded_bytes: 0,
            };
            assert!(
                crate::raster_asset_validation::prepare_png("source", &asset, 1024, None).is_ok()
            );
            assert_eq!(asset.data, original);
        }
    }
}

#[cfg(test)]
mod vector_placement_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::ImageAspect;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn all_original_vector_aspect_matrix_bits() {
        #[derive(serde::Deserialize)]
        struct Record {
            source: [f64; 4],
            destination: [f64; 4],
            aspect: [u8; 2],
            bits: [u64; 6],
        }
        let rows: Vec<Record> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/vector-image-placement-source-master.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 120);
        let names = [
            "none", "xMinYMin", "xMidYMin", "xMaxYMin", "xMinYMid", "xMidYMid", "xMaxYMid",
            "xMinYMax", "xMidYMax", "xMaxYMax",
        ];
        for (index, row) in rows.into_iter().enumerate() {
            let raw = format!(
                "{} {}",
                names[row.aspect[0] as usize],
                if row.aspect[1] == 0 { "meet" } else { "slice" }
            );
            let actual = ImageAspect::parse(Some(&raw))
                .unwrap()
                .vector_matrix(row.source, row.destination)
                .unwrap();
            assert_eq!(actual.map(f64::to_bits), row.bits, "control {index}: {raw}");
        }
    }
}

#[cfg(test)]
mod handoff_tests {
    use super::*;

    fn planes(width: usize, height: usize, alpha: bool) -> YCbCrPlanes {
        let stride = width.div_ceil(16) * 16;
        let chroma_stride = stride / 2;
        let rows = height.div_ceil(16) * 16;
        let byte = |index: usize, salt: usize| ((index * 37 + salt * 101) % 256) as u8;
        YCbCrPlanes {
            width,
            stride,
            chroma_stride,
            y: (0..stride * rows).map(|i| byte(i, 1)).collect(),
            cb: (0..chroma_stride * rows / 2).map(|i| byte(i, 2)).collect(),
            cr: (0..chroma_stride * rows / 2).map(|i| byte(i, 3)).collect(),
            alpha: alpha.then(|| (0..width * height).map(|i| byte(i, 4)).collect()),
        }
    }

    /// A handed-off asset samples exactly as the PNG it replaces, the
    /// placeholder resolves only while its scope lives, and materializing
    /// restores that PNG for decoders outside the sampler.
    #[test]
    fn handed_off_assets_sample_as_their_png() {
        let (width, height) = (19, 11);
        let rgba8 = (0..width * height)
            .map(|i| [(i * 3) as u8, (i * 5) as u8, (i * 7) as u8, (i * 11) as u8])
            .collect::<Vec<_>>();
        let assets = [
            Asset::ycbcr(width, height, planes(width, height, true)).unwrap(),
            Asset::ycbcr(width, height, planes(width, height, false)).unwrap(),
            Asset::straight_rgba8(width, height, rgba8).unwrap(),
        ];
        for asset in assets {
            let png = asset.encode_png().unwrap();
            let decoded = decode_png(&png).unwrap();
            let texels = |asset: &Asset| {
                (0..width * height)
                    .map(|i| asset.texel(i))
                    .collect::<Vec<_>>()
            };
            let expected = texels(&decoded);
            assert_eq!(texels(&asset), expected);
            let mut scope = HandoffScope::default();
            let placeholder = scope.hand_off(asset).unwrap();
            let handed = decode(&placeholder).unwrap();
            assert_eq!(texels(&handed), expected);
            use base64::Engine as _;
            let engine = base64::engine::general_purpose::STANDARD;
            let svg = format!(
                "<image href=\"data:image/png;base64,{}\"/>",
                engine.encode(&placeholder)
            );
            // SVG parsing takes the PNG kind and size from the placeholder.
            let document = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\">{svg}</svg>"
            );
            let tree = usvg::Tree::from_str(&document, &usvg::Options::default()).unwrap();
            fn first_image(group: &usvg::Group) -> Option<&usvg::Image> {
                group.children().iter().find_map(|node| match node {
                    usvg::Node::Image(image) => Some(image.as_ref()),
                    usvg::Node::Group(group) => first_image(group),
                    _ => None,
                })
            }
            let image = first_image(tree.root()).expect("placeholder image was not parsed");
            assert!(matches!(image.kind(), usvg::ImageKind::PNG(_)));
            assert_eq!(
                (image.size().width(), image.size().height()),
                (width as f32, height as f32)
            );
            let materialized = scope.materialize(&svg);
            let encoded = materialized
                .strip_prefix("<image href=\"data:image/png;base64,")
                .and_then(|rest| rest.strip_suffix("\"/>"))
                .unwrap();
            let restored = decode_png(&engine.decode(encoded).unwrap()).unwrap();
            assert_eq!(texels(&restored), expected);
            drop(scope);
            assert!(decode(&placeholder).is_none());

            let mut scope = HandoffScope::default();
            let shared = scope.prepare(&png, true, || decode_png(&png)).unwrap();
            let alias = scope
                .prepare(&png.clone(), true, || panic!("decoded the same PNG twice"))
                .unwrap();
            assert!(std::sync::Arc::ptr_eq(
                &decode(&shared).unwrap(),
                &decode(&alias).unwrap()
            ));
            // A fallback renderer receives the original bytes, including
            // sixteen-bit precision, rather than a second PNG conversion.
            assert_eq!(
                scope.materialize(&engine.encode(&shared)),
                engine.encode(&png)
            );
            drop(scope);
            assert!(decode(&shared).is_none());
        }
    }
}

#[cfg(test)]
mod first_frame_canvas_tests {
    use super::*;

    #[test]
    fn first_frames_composite_like_rasterimage_composite_frame() {
        let frame = || Asset::straight_rgba8(1, 2, vec![[1, 1, 1, 1], [10, 20, 30, 255]]).unwrap();
        // A frame that fills its canvas is used as decoded.
        let same = frame().on_canvas(1, 2, 0, 0, false).unwrap();
        assert_eq!(same.pixels, frame().pixels);
        // draw.Src copies NRGBA bytes.
        let copied = frame().on_canvas(3, 3, 2, 1, true).unwrap();
        let AssetPixels::Eight(pixels) = &copied.pixels else {
            panic!("canvas is NRGBA");
        };
        assert_eq!(pixels[3 + 2], [1, 1, 1, 1]);
        assert_eq!(pixels[6 + 2], [10, 20, 30, 255]);
        assert_eq!(pixels[0], [0; 4]);
        // draw.Over stores NRGBA.SetRGBA64(src.RGBA64At): low alpha loses color.
        let over = frame().on_canvas(3, 3, 2, 1, false).unwrap();
        let AssetPixels::Eight(pixels) = &over.pixels else {
            panic!("canvas is NRGBA");
        };
        assert_eq!(pixels[3 + 2], [0, 0, 0, 1]);
        assert_eq!(pixels[6 + 2], [10, 20, 30, 255]);
        assert!(frame().on_canvas(2, 2, 1, 1, false).is_none());
    }
}
