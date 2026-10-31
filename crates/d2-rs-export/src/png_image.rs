// SPDX-License-Identifier: MPL-2.0
//! Source d2raster/image.go PNG decoding, coverage and premultiplied sampling.
use resvg::usvg;
use std::io::Cursor;

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
        let dx = (-encoded_width).mul_add(scale, image_box[2]);
        let dy = (-encoded_height).mul_add(scale, image_box[3]);
        let placement = [
            dx.mul_add(ax, image_box[0]),
            dy.mul_add(ay, image_box[1]),
            width,
            height,
        ];
        (scale.is_finite() && scale > 0.0 && placement.iter().all(|v| v.is_finite()))
            .then_some(placement)
    }
}

struct Asset {
    width: usize,
    height: usize,
    pixels: Vec<[u32; 4]>,
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

fn decode(data: &[u8]) -> Option<Asset> {
    let adapted = empty_dictionary_decoder_bytes(data);
    let data = adapted.as_ref();
    let mut decoder = png::Decoder::new(Cursor::new(data));
    // Expand palettes/sub-byte samples without stripping sixteen-bit precision.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    let width = usize::try_from(info.width).ok()?;
    let height = usize::try_from(info.height).ok()?;
    let count = width.checked_mul(height)?;
    if width == 0 || height == 0 {
        return None;
    }
    let sample_bytes = match info.bit_depth {
        png::BitDepth::Eight => 1,
        png::BitDepth::Sixteen => 2,
        _ => return None,
    };
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let stride = channels * sample_bytes;
    if info.buffer_size() != count.checked_mul(stride)? {
        return None;
    }
    let mut pixels = Vec::with_capacity(count);
    for pixel in bytes[..info.buffer_size()].chunks_exact(stride) {
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
            4 => (sample(0), sample(1), sample(2), sample(3)),
            _ => return None,
        };
        let premultiply = |value: u32| (u64::from(value) * u64::from(a) / 65535) as u32;
        pixels.push([premultiply(r), premultiply(g), premultiply(b), a]);
    }
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
        let c00 = self.pixels[iy0 * self.width + ix0];
        let c10 = self.pixels[iy0 * self.width + ix1];
        let c01 = self.pixels[iy1 * self.width + ix0];
        let c11 = self.pixels[iy1 * self.width + ix1];
        if ix0 == ix1 && iy0 == iy1 {
            return c00;
        }
        std::array::from_fn(|channel| {
            let value = if ix0 == ix1 {
                (f64::from(c01[channel]) - f64::from(c00[channel]))
                    .mul_add(wy, f64::from(c00[channel]))
            } else if iy0 == iy1 {
                (f64::from(c10[channel]) - f64::from(c00[channel]))
                    .mul_add(wx, f64::from(c00[channel]))
            } else {
                let top = (f64::from(c10[channel]) - f64::from(c00[channel]))
                    .mul_add(wx, f64::from(c00[channel]));
                let bottom = (f64::from(c11[channel]) - f64::from(c01[channel]))
                    .mul_add(wx, f64::from(c01[channel]));
                (bottom - top).mul_add(wy, top)
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
    for y in top..bottom {
        for x in left..right {
            let mut valid = 0_u64;
            let (mut sum_x, mut sum_y) = (0.0, 0.0);
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = x as f64 + (sx as f64 + 0.5) / 4.0 - e;
                    let dy = y as f64 + (sy as f64 + 0.5) / 4.0 - f;
                    let local_x = ia.mul_add(dx, ic * dy);
                    let local_y = ib.mul_add(dx, id * dy);
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

#[cfg(test)]
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

    #[cfg(feature = "oracle-fixtures")]
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
                (actual.width, actual.height, actual.pixels),
                (equivalent.width, equivalent.height, equivalent.pixels)
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
        let rows: Vec<Record> = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/vector-image-placement-source-master.json"))
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
