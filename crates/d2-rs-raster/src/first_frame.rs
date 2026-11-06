// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Go image/color conversion: see root THIRD_PARTY_NOTICES.md.
//! internal/rasterimage/firstframe.go at the pinned D2 revision.
//! Still-image normalization for authored isometric raster assets.

use crate::png::{PngImage, crc32, decode_png};
use std::io::Cursor;

fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || u64::from(width) > 16_000_000 / u64::from(height) {
        return Err("native SVG raster asset exceeds 16 million pixels".into());
    }
    Ok(())
}

pub(crate) fn decode(data: &[u8], format: &str) -> Result<PngImage, String> {
    let format = format.to_ascii_lowercase();
    match format.strip_prefix("image/").unwrap_or(&format) {
        "png" => png(data),
        "gif" => gif(data),
        "jpeg" | "jpg" => jpeg(data),
        "webp" => webp(data),
        _ => Err("image: unknown format".into()),
    }
}

fn jpeg(data: &[u8]) -> Result<PngImage, String> {
    // Read the SOF dimensions before the shared decoder allocates image planes.
    let mut at = 2;
    let mut gray = false;
    while at + 4 <= data.len() && data[at] == 255 {
        if data[at + 1] == 255 {
            at += 1;
            continue;
        }
        let kind = data[at + 1];
        let size = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        let payload = data.get(at + 4..at + 2 + size).unwrap_or_default();
        if matches!(kind, 0xc0..=0xc2) && payload.len() >= 6 {
            dimensions(
                u16::from_be_bytes([payload[3], payload[4]]) as u32,
                u16::from_be_bytes([payload[1], payload[2]]) as u32,
            )?;
            gray = payload[5] == 1;
            break;
        }
        if kind == 0xda || kind == 0xd9 || size < 2 {
            break;
        }
        at += size + 2;
    }
    let mut input = d2_rs_core::jpeg_first::Bytes { data, offset: 0 };
    let image = d2_rs_core::jpeg_first::decode(data, &mut input, &|| Ok(()))?;
    dimensions(image.width as u32, image.height as u32)?;
    // JPEG's GrayModel encodes 8-bit; YCbCr/CMYK use PNG's default 16-bit case.
    Ok(PngImage::from_rgba(
        image.width as u32,
        image.height as u32,
        image.rgba,
        !gray,
        gray,
    ))
}

fn gif(data: &[u8]) -> Result<PngImage, String> {
    inspect_gif(data)?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut reader = options
        .read_info(Cursor::new(data))
        .map_err(|e| e.to_string())?;
    let (width, height) = (reader.width() as u32, reader.height() as u32);
    dimensions(width, height)?;
    let global = reader.global_palette().map(|p| p.to_vec());
    let frame = reader
        .read_next_frame()
        .map_err(|e| e.to_string())?
        .ok_or("gif: no frames")?;
    let palette = frame
        .palette
        .as_deref()
        .or(global.as_deref())
        .ok_or("gif: no color table")?;
    let (x, y, w, h) = (
        frame.left as u32,
        frame.top as u32,
        frame.width as u32,
        frame.height as u32,
    );
    if w == 0 || h == 0 || x + w > width || y + h > height {
        return Err(format!(
            "GIF first-frame bounds ({x},{y})-({},{}) fall outside logical canvas (0,0)-({width},{height})",
            x + w,
            y + h
        ));
    }
    if x == 0 && y == 0 && w == width && h == height {
        let bits = match palette.len() / 3 {
            0..=2 => 1,
            3..=4 => 2,
            5..=16 => 4,
            _ => 8,
        };
        let row_bytes = (width as usize * bits).div_ceil(8);
        let mut pixels = vec![0; row_bytes * height as usize];
        for yy in 0..height as usize {
            for xx in 0..width as usize {
                pixels[yy * row_bytes + xx * bits / 8] |=
                    frame.buffer[yy * width as usize + xx] << (8 - bits - xx * bits % 8);
            }
        }
        let mut transparency = Vec::new();
        if let Some(index) = frame.transparent {
            transparency.resize(index as usize + 1, 255);
            transparency[index as usize] = 0;
        }
        let mut palette = palette.to_vec();
        if let Some(index) = frame.transparent {
            if let Some(color) = palette.get_mut(index as usize * 3..index as usize * 3 + 3) {
                color.fill(0);
            }
        }
        return Ok(PngImage {
            width,
            height,
            pixels,
            channels: 1,
            out_depth: bits as u8,
            color_type: 3,
            row_bytes,
            palette,
            transparency,
        });
    }
    let mut rgba = vec![[0; 4]; width as usize * height as usize];
    for yy in 0..h as usize {
        for xx in 0..w as usize {
            let index = frame.buffer[yy * w as usize + xx] as usize;
            if frame.transparent == Some(index as u8) {
                continue;
            }
            let p = palette
                .get(index * 3..index * 3 + 3)
                .ok_or("gif: invalid pixel value")?;
            rgba[(yy + y as usize) * width as usize + xx + x as usize] = [
                p[0] as u16 * 257,
                p[1] as u16 * 257,
                p[2] as u16 * 257,
                65535,
            ];
        }
    }
    Ok(PngImage::from_rgba(width, height, rgba, true, false))
}

fn inspect_gif(data: &[u8]) -> Result<(), String> {
    if data.len() < 13 {
        return Err("truncated logical screen descriptor".into());
    }
    let mut at = 13;
    if data[10] & 128 != 0 {
        at += 3 * (1usize << ((data[10] & 7) + 1));
        if at > data.len() {
            return Err("truncated global color table".into());
        }
    }
    let mut frames = 0;
    loop {
        let marker = *data.get(at).ok_or("missing GIF trailer")?;
        at += 1;
        match marker {
            0x3b => {
                return if frames > 0 {
                    Ok(())
                } else {
                    Err("GIF has no image frame".into())
                };
            }
            0x21 => {
                if at >= data.len() {
                    return Err("truncated GIF extension".into());
                }
                at += 1;
            }
            0x2c => {
                frames += 1;
                if data.len() - at < 9 {
                    return Err("truncated GIF image descriptor".into());
                }
                let packed = data[at + 8];
                at += 9;
                if packed & 128 != 0 {
                    at += 3 * (1usize << ((packed & 7) + 1));
                    if at > data.len() {
                        return Err("truncated local color table".into());
                    }
                }
                if at >= data.len() {
                    return Err("missing GIF LZW code size".into());
                }
                at += 1;
            }
            _ => return Err(format!("unexpected GIF block marker 0x{marker:02x}")),
        }
        loop {
            let size = *data.get(at).ok_or("truncated GIF sub-blocks")? as usize;
            at += 1;
            if size == 0 {
                break;
            }
            if size > data.len() - at {
                return Err("truncated GIF sub-block data".into());
            }
            at += size;
        }
    }
}

#[derive(Default)]
struct PngPlan {
    canvas: (u32, u32),
    frame: (u32, u32, u32, u32),
    animated: bool,
    uses_idat: bool,
    source_blend: bool,
    prefix: usize,
    frame_bytes: usize,
}

fn be(data: &[u8]) -> u32 {
    u32::from_be_bytes(data[..4].try_into().unwrap())
}
fn chunk<'a>(data: &'a [u8], at: &mut usize) -> Result<(&'a [u8], &'a [u8], &'a [u8]), String> {
    if data.len().saturating_sub(*at) < 12 {
        return Err("truncated PNG chunk".into());
    }
    let size = be(&data[*at..]) as usize;
    if size > data.len() - *at - 12 {
        return Err("truncated PNG chunk data".into());
    }
    let raw = &data[*at..*at + size + 12];
    *at += size + 12;
    if crc32(&raw[4..size + 8]) != be(&raw[size + 8..]) {
        return Err(format!(
            "invalid PNG {} checksum",
            String::from_utf8_lossy(&raw[4..8])
        ));
    }
    Ok((&raw[4..8], &raw[8..size + 8], raw))
}

fn sequence(payload: &[u8], next: &mut u64) -> Result<(), String> {
    let got = be(payload) as u64;
    if got != *next {
        return Err(format!("APNG sequence number is {got}, expected {}", *next));
    }
    *next += 1;
    Ok(())
}

fn inspect_png(data: &[u8]) -> Result<PngPlan, String> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("invalid PNG signature".into());
    }
    let mut plan = PngPlan::default();
    let (mut ihdr, mut idat, mut actl, mut current, mut current_idat, mut has_data) =
        (false, false, false, false, false, false);
    let (mut declared, mut frames, mut next, mut at) = (0u32, 0u64, 0u64, 8usize);
    loop {
        let start = at;
        let (kind, payload, raw) = chunk(data, &mut at)?;
        match kind {
            b"IHDR" => {
                if ihdr || start != 8 || payload.len() != 13 {
                    return Err("invalid PNG IHDR".into());
                }
                ihdr = true;
                plan.canvas = (be(payload), be(&payload[4..]));
                if plan.canvas.0 == 0 || plan.canvas.1 == 0 {
                    return Err("invalid PNG canvas dimensions".into());
                }
                dimensions(plan.canvas.0, plan.canvas.1)?;
            }
            b"acTL" => {
                if !ihdr || actl || idat || payload.len() != 8 || be(payload) == 0 {
                    return Err("invalid APNG animation control chunk".into());
                }
                actl = true;
                plan.animated = true;
                declared = be(payload);
                if frames > declared as u64 {
                    return Err("APNG frame count exceeds acTL declaration".into());
                }
            }
            b"fcTL" => {
                if !ihdr || payload.len() != 26 || (!actl && (idat || frames != 0)) {
                    return Err("invalid APNG frame control chunk".into());
                }
                if current && !has_data {
                    return Err("APNG frame has no data".into());
                }
                sequence(payload, &mut next)?;
                frames += 1;
                if actl && frames > declared as u64 {
                    return Err("APNG frame count exceeds acTL declaration".into());
                }
                let (w, h, x, y) = (
                    be(&payload[4..]),
                    be(&payload[8..]),
                    be(&payload[12..]),
                    be(&payload[16..]),
                );
                if w == 0
                    || h == 0
                    || x as u64 + w as u64 > plan.canvas.0 as u64
                    || y as u64 + h as u64 > plan.canvas.1 as u64
                {
                    return Err("APNG frame falls outside its logical canvas".into());
                }
                if payload[24] > 2 || payload[25] > 1 {
                    return Err("APNG frame has invalid dispose or blend operation".into());
                }
                current = true;
                has_data = false;
                current_idat = frames == 1 && !idat;
                if frames == 1 {
                    plan.frame = (x, y, w, h);
                    plan.uses_idat = current_idat;
                    plan.source_blend = payload[25] == 0;
                    if current_idat && (x != 0 || y != 0 || (w, h) != plan.canvas) {
                        return Err(
                            "APNG IDAT first frame does not cover its logical canvas".into()
                        );
                    }
                }
            }
            b"IDAT" => {
                if !ihdr {
                    return Err("PNG IDAT precedes IHDR".into());
                }
                if current && !actl {
                    return Err("APNG IDAT data precedes animation control".into());
                }
                if current && !current_idat {
                    return Err("APNG IDAT data follows a non-IDAT frame control".into());
                }
                idat = true;
                if current_idat && !payload.is_empty() {
                    has_data = true;
                }
            }
            b"fdAT" => {
                if !actl || !current {
                    return Err("APNG frame data precedes frame control".into());
                }
                if payload.len() < 4 || current_idat {
                    return Err("invalid APNG frame data chunk".into());
                }
                sequence(payload, &mut next)?;
                if payload.len() > 4 {
                    has_data = true;
                }
                if frames == 1 {
                    plan.frame_bytes = plan
                        .frame_bytes
                        .checked_add(payload.len() - 4)
                        .ok_or("APNG first-frame data bytes overflow")?;
                }
            }
            b"IEND" => {
                if !payload.is_empty() {
                    return Err("invalid PNG IEND".into());
                }
                if !ihdr {
                    return Err("PNG is missing IHDR".into());
                }
                if frames != 0 && !actl {
                    return Err("APNG is missing an animation control chunk".into());
                }
                if !plan.animated {
                    return Ok(plan);
                }
                if frames == 0 {
                    return Err("APNG is missing a first frame control chunk".into());
                }
                if !has_data {
                    return Err("APNG frame has no data".into());
                }
                if frames != declared as u64 {
                    return Err(format!(
                        "APNG contains {frames} frames, acTL declares {declared}"
                    ));
                }
                if !plan.uses_idat && plan.frame_bytes == 0 {
                    return Err("APNG first frame has no data".into());
                }
                return Ok(plan);
            }
            _ => {
                if !idat {
                    plan.prefix = plan
                        .prefix
                        .checked_add(raw.len())
                        .ok_or("APNG prefix bytes overflow")?;
                }
            }
        }
    }
}

fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let start = output.len();
    output.extend_from_slice(kind);
    output.extend_from_slice(payload);
    let crc = crc32(&output[start..]);
    output.extend_from_slice(&crc.to_be_bytes());
}

fn png(data: &[u8]) -> Result<PngImage, String> {
    let plan = inspect_png(data)?;
    if !plan.animated || plan.uses_idat {
        return decode_png(data);
    }
    // Two-pass bounded materialization; no allocation or retained slice per chunk.
    let total = 57usize
        .checked_add(plan.prefix)
        .and_then(|n| n.checked_add(plan.frame_bytes))
        .ok_or("APNG first-frame output size overflow")?;
    if plan.frame_bytes > u32::MAX as usize {
        return Err("APNG first-frame output size overflow".into());
    }
    let mut still = Vec::with_capacity(total);
    still.extend_from_slice(&data[..8]);
    let mut header = data[16..29].to_vec();
    header[..4].copy_from_slice(&plan.frame.2.to_be_bytes());
    header[4..8].copy_from_slice(&plan.frame.3.to_be_bytes());
    png_chunk(&mut still, b"IHDR", &header);
    let (mut at, mut saw_idat, mut collecting, mut started) = (8, false, false, false);
    let mut payload_start = 0;
    while at < data.len() {
        let (kind, payload, raw) = chunk(data, &mut at)?;
        match kind {
            b"IHDR" | b"acTL" => {}
            b"IDAT" => saw_idat = true,
            b"fcTL" => {
                if started {
                    break;
                }
                started = true;
                collecting = true;
                still.extend_from_slice(&(plan.frame_bytes as u32).to_be_bytes());
                payload_start = still.len();
                still.extend_from_slice(b"IDAT");
            }
            b"fdAT" => {
                if collecting {
                    still.extend_from_slice(&payload[4..]);
                }
            }
            b"IEND" => break,
            _ => {
                if !saw_idat {
                    still.extend_from_slice(raw);
                }
            }
        }
    }
    let checksum = crc32(&still[payload_start..]);
    still.extend_from_slice(&checksum.to_be_bytes());
    png_chunk(&mut still, b"IEND", &[]);
    if still.len() != total {
        return Err("APNG first-frame output size overflow".into());
    }
    let frame = decode_png(&still)?;
    if plan.frame.0 == 0 && plan.frame.1 == 0 && (plan.frame.2, plan.frame.3) == plan.canvas {
        return Ok(frame);
    }
    let pixels = png_rgba(&still)?;
    composite(
        plan.canvas,
        plan.frame,
        pixels.0,
        plan.source_blend && pixels.1,
    )
}

fn png_rgba(data: &[u8]) -> Result<(Vec<[u16; 4]>, bool), String> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let direct_nrgba = reader.info().bit_depth != png::BitDepth::Sixteen
        && reader.info().color_type != png::ColorType::Indexed
        && (matches!(
            reader.info().color_type,
            png::ColorType::Rgba | png::ColorType::GrayscaleAlpha
        ) || reader.info().trns.is_some());
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let stride = if info.bit_depth == png::BitDepth::Sixteen {
        2
    } else {
        1
    };
    let mut out = Vec::with_capacity(info.width as usize * info.height as usize);
    for bytes in buffer[..info.buffer_size()].chunks_exact(info.color_type.samples() * stride) {
        let mut values = [0u16; 4];
        for (i, value) in values
            .iter_mut()
            .enumerate()
            .take(info.color_type.samples())
        {
            *value = if stride == 2 {
                u16::from_be_bytes([bytes[2 * i], bytes[2 * i + 1]])
            } else {
                bytes[i] as u16 * 257
            };
        }
        out.push(match info.color_type {
            png::ColorType::Grayscale => [values[0], values[0], values[0], 65535],
            png::ColorType::GrayscaleAlpha => [values[0], values[0], values[0], values[1]],
            png::ColorType::Rgb => [values[0], values[1], values[2], 65535],
            png::ColorType::Rgba => [values[0], values[1], values[2], values[3]],
            _ => unreachable!(),
        });
    }
    Ok((out, direct_nrgba))
}

fn composite(
    canvas: (u32, u32),
    frame: (u32, u32, u32, u32),
    pixels: Vec<[u16; 4]>,
    direct_nrgba: bool,
) -> Result<PngImage, String> {
    dimensions(canvas.0, canvas.1)?;
    let mut out = vec![[0; 4]; canvas.0 as usize * canvas.1 as usize];
    // image/draw writes the transparent NRGBA destination, quantizing to 8 bits.
    for y in 0..frame.3 as usize {
        for x in 0..frame.2 as usize {
            let p = pixels[y * frame.2 as usize + x];
            out[(y + frame.1 as usize) * canvas.0 as usize + x + frame.0 as usize] = if direct_nrgba
            {
                p.map(|c| (c >> 8) * 257)
            } else {
                let alpha = u32::from(p[3]);
                #[allow(clippy::manual_checked_ops)]
                let channel = |value: u16| {
                    if alpha == 0 {
                        0
                    } else {
                        // NRGBA.RGBA64At → draw.Over on transparent black →
                        // NRGBA.SetRGBA64. Preserve both integer divisions.
                        ((((u32::from(value) * alpha / 65535) * 65535 / alpha) >> 8) * 257) as u16
                    }
                };
                [
                    channel(p[0]),
                    channel(p[1]),
                    channel(p[2]),
                    (p[3] >> 8) * 257,
                ]
            };
        }
    }
    Ok(PngImage::from_rgba(canvas.0, canvas.1, out, false, false))
}

fn webp(data: &[u8]) -> Result<PngImage, String> {
    // Implemented below after the container is validated and first ANMF extracted.
    webp_first(data)
}

fn le(data: &[u8]) -> u32 {
    u32::from_le_bytes(data[..4].try_into().unwrap())
}
fn u24(data: &[u8]) -> u32 {
    data[0] as u32 | (data[1] as u32) << 8 | (data[2] as u32) << 16
}
fn webp_chunk<'a>(data: &'a [u8], at: &mut usize) -> Result<(&'a [u8], &'a [u8]), String> {
    if data.len().saturating_sub(*at) < 8 {
        return Err("truncated WebP chunk header".into());
    }
    let size = le(&data[*at + 4..]) as usize;
    let padded = size + (size & 1);
    if padded > data.len() - *at - 8 {
        return Err("truncated WebP chunk data".into());
    }
    let kind = &data[*at..*at + 4];
    let payload = &data[*at + 8..*at + 8 + size];
    *at += 8 + padded;
    Ok((kind, payload))
}
fn append_webp(out: &mut Vec<u8>, kind: &[u8], payload: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

fn webp_first(data: &[u8]) -> Result<PngImage, String> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err("invalid WebP RIFF header".into());
    }
    if le(&data[4..]) as u64 + 8 != data.len() as u64 {
        return Err("invalid WebP RIFF length".into());
    }
    let (mut at, mut vp8x, mut animated, mut anim_count, mut anim_len, mut anmf, mut images) =
        (12, false, false, 0, 0, None, 0);
    let mut canvas = (0, 0);
    while at < data.len() {
        let (kind, payload) = webp_chunk(data, &mut at)?;
        match kind {
            b"VP8X" => {
                if vp8x || payload.len() != 10 {
                    return Err("invalid WebP VP8X chunk".into());
                }
                vp8x = true;
                animated = payload[0] & 2 != 0;
                canvas = (u24(&payload[4..]) + 1, u24(&payload[7..]) + 1);
                dimensions(canvas.0, canvas.1)?;
            }
            b"ANIM" => {
                anim_count += 1;
                if anim_count == 1 {
                    anim_len = payload.len();
                }
            }
            b"ANMF" => {
                if anmf.is_none() {
                    anmf = Some(payload);
                }
            }
            b"VP8 " | b"VP8L" => images += 1,
            _ => {}
        }
    }
    if !animated {
        if images != 1 {
            return Err(format!(
                "expected exactly one static WebP image bitstream, got {images}"
            ));
        }
        return decode_webp(data);
    }
    if !vp8x || anim_count != 1 || anim_len != 6 || anmf.is_none() {
        return Err("incomplete animated WebP container".into());
    }
    let payload = anmf.unwrap();
    if payload.len() < 16 {
        return Err("truncated WebP ANMF header".into());
    }
    let frame = (
        u24(payload) * 2,
        u24(&payload[3..]) * 2,
        u24(&payload[6..]) + 1,
        u24(&payload[9..]) + 1,
    );
    if frame.0 as u64 + frame.2 as u64 > canvas.0 as u64
        || frame.1 as u64 + frame.3 as u64 > canvas.1 as u64
    {
        return Err("WebP first frame falls outside its logical canvas".into());
    }
    let (mut alpha, mut image, mut image_kind, mut at) = (None, None, &b""[..], 16);
    while at < payload.len() {
        let (kind, bytes) = webp_chunk(payload, &mut at)?;
        match kind {
            b"ALPH" => {
                if alpha.is_some() {
                    return Err("multiple WebP ALPH chunks in first frame".into());
                }
                alpha = Some(bytes);
            }
            b"VP8 " | b"VP8L" => {
                if image.is_some() {
                    return Err("multiple WebP bitstreams in first frame".into());
                }
                image = Some(bytes);
                image_kind = kind;
            }
            _ => {}
        }
    }
    let image = image.ok_or("invalid WebP first-frame bitstream")?;
    if alpha.is_some() && image_kind != b"VP8 " {
        return Err("invalid WebP first-frame bitstream".into());
    }
    let size = if image_kind == b"VP8L" {
        if image.len() < 5 || image[0] != 0x2f {
            return Err("invalid WebP VP8L first-frame header".into());
        }
        let bits = le(&image[1..]);
        ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
    } else {
        if image.len() < 10 || &image[3..6] != b"\x9d\x01\x2a" {
            return Err("invalid WebP VP8 first-frame header".into());
        }
        (
            (u16::from_le_bytes([image[6], image[7]]) & 0x3fff) as u32,
            (u16::from_le_bytes([image[8], image[9]]) & 0x3fff) as u32,
        )
    };
    if size != (frame.2, frame.3) {
        return Err(format!(
            "WebP first-frame header is {}x{} but its bitstream is {}x{}",
            frame.2, frame.3, size.0, size.1
        ));
    }
    let mut still = b"RIFF\0\0\0\0WEBP".to_vec();
    if let Some(alpha) = alpha {
        let mut vp8x = [0u8; 10];
        vp8x[0] = 0x10;
        vp8x[4..7].copy_from_slice(&(frame.2 - 1).to_le_bytes()[..3]);
        vp8x[7..10].copy_from_slice(&(frame.3 - 1).to_le_bytes()[..3]);
        append_webp(&mut still, b"VP8X", &vp8x);
        append_webp(&mut still, b"ALPH", alpha);
    }
    append_webp(&mut still, image_kind, image);
    let size = (still.len() - 8) as u32;
    still[4..8].copy_from_slice(&size.to_le_bytes());
    let first = decode_webp(&still)?;
    if frame.0 == 0 && frame.1 == 0 && (frame.2, frame.3) == canvas {
        return Ok(first);
    }
    let pixels = webp_rgba(&still)?;
    composite(canvas, frame, pixels.0, payload[15] & 2 != 0 && !pixels.3)
}

fn decode_webp(data: &[u8]) -> Result<PngImage, String> {
    let (pixels, width, height, sixteen) = webp_rgba(data)?;
    Ok(PngImage::from_rgba(width, height, pixels, sixteen, false))
}

pub(crate) fn webp_rgba(data: &[u8]) -> Result<(Vec<[u16; 4]>, u32, u32, bool), String> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(data)).map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    dimensions(width, height)?;
    let mut out = Vec::with_capacity(width as usize * height as usize);
    if !decoder.is_lossy() {
        let alpha = decoder.has_alpha();
        let channels = if alpha { 4 } else { 3 };
        let mut bytes = vec![
            0;
            decoder
                .output_buffer_size()
                .ok_or("webp: image too large")?
        ];
        decoder.read_image(&mut bytes).map_err(|e| e.to_string())?;
        for p in bytes.chunks_exact(channels) {
            out.push([
                p[0] as u16 * 257,
                p[1] as u16 * 257,
                p[2] as u16 * 257,
                if alpha { p[3] as u16 * 257 } else { 65535 },
            ]);
        }
        return Ok((out, width, height, false));
    }
    let (mut at, mut vp8) = (12, None);
    while at < data.len() {
        let (kind, payload) = webp_chunk(data, &mut at)?;
        if kind == b"VP8 " {
            vp8 = Some(payload);
            break;
        }
    }
    let frame =
        image_webp::vp8::Vp8Decoder::decode_frame(Cursor::new(vp8.ok_or("missing VP8 chunk")?))
            .map_err(|e| e.to_string())?;
    if width != frame.width as u32 || height != frame.height as u32 {
        return Err("WebP frame dimensions differ".into());
    }
    let alpha = if decoder.has_alpha() {
        let mut bytes = vec![
            0;
            decoder
                .output_buffer_size()
                .ok_or("webp: image too large")?
        ];
        decoder.read_image(&mut bytes).map_err(|e| e.to_string())?;
        Some(bytes)
    } else {
        None
    };
    let stride = (width as usize).div_ceil(16) * 16;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let ci = (y / 2) * (stride / 2) + x / 2;
            let c = d2_rs_core::jpeg_decode::ycbcr16(
                frame.ybuf[y * stride + x],
                frame.ubuf[ci],
                frame.vbuf[ci],
            );
            let a = alpha
                .as_ref()
                .map_or(65535, |p| p[(y * width as usize + x) * 4 + 3] as u16 * 257);
            // NYCbCrA.RGBA premultiplies before NRGBA64Model unpremultiplies.
            let channel = |v: u16| {
                if a == 0 {
                    0
                } else {
                    ((v as u32 * a as u32 / 65535) * 65535 / a as u32) as u16
                }
            };
            out.push([channel(c[0]), channel(c[1]), channel(c[2]), a]);
        }
    }
    Ok((out, width, height, true))
}
