// SPDX-License-Identifier: MPL-2.0
//! gif.go: one sampled palette, immutable RGB lookup, opaque deltas and delays.
#[cfg(test)]
#[path = "output_api_tests.rs"]
mod output_api_tests;
use super::{
    context::{RenderContext, RenderError},
    gif_capture::{CYCLE_SECONDS, Capture, FRAME_COUNT},
    render::Options,
    svg::{MAX_OUTPUT_BYTES, NativeScene},
};
use crate::target::Diagram;
use d2_rs_core::CancellationToken;
use d2_rs_export::ImageGifEncoder;
use d2_rs_raster::NrgbaImage;
use std::io::Write;

/// image/gif DisposalNone: each frame stays as the next frame's canvas.
pub(crate) const DISPOSAL_NONE: u8 = 0x01;

/// Render an infinitely looping GIF. The caller retains the diagram and all
/// authored animation durations, including source-panel and mask tracks.
pub fn render_gif_with_context(
    token: Option<&CancellationToken>,
    diagram: Option<&Diagram>,
    options: Option<&Options>,
) -> Result<Vec<u8>, RenderError> {
    let mut o = options.cloned().unwrap_or_default();
    o.format = "gif".into();
    let mut capture = Capture::open(token, diagram, Some(&o))?;
    let (frames, cycle) = native_gif_cycle(&capture.scene);
    if frames != FRAME_COUNT {
        capture.scene =
            super::gif_loop::loop_scene(&capture.context, &capture.scene, (cycle * 1e9) as i64)
                .map_err(|e| capture.context.error(e))?;
        capture.static_frame = Default::default();
    }
    let frame_at =
        |seconds: f64| capture.frame_image_at(seconds, seconds * CYCLE_SECONDS / cycle, true);
    let mut samples = vec![0; 256 * 1024 * 4];
    for i in 0..4 {
        let mut seconds = i as f64 * cycle / 4.0;
        if frames != FRAME_COUNT {
            seconds += i as f64 / 4.0;
        }
        let frame = frame_at(seconds)?;
        for y in 0..256 {
            for x in 0..256 {
                let offset =
                    (y * frame.height() / 256) * frame.stride + (x * frame.width() / 256) * 4;
                let out = ((i * 256 + y) * 256 + x) * 4;
                // Native frames are opaque; preserve the RGBA sample contract.
                samples[out..out + 4].copy_from_slice(&frame.pix[offset..offset + 4]);
            }
        }
    }
    capture.context.check()?;
    let palette = d2_rs_export::gif_median_cut_palette(&samples);
    let lookup = palette_lookup(&capture.context, &palette)?;
    let mut writer = BoundedWriter {
        context: &capture.context,
        bytes: Vec::new(),
        remaining: MAX_OUTPUT_BYTES,
    };
    {
        // gif.go renderGIF: gif.EncodeAll with LoopCount 0, the palette as
        // Config.ColorModel and gif.DisposalNone for every frame.
        let error = |e: std::io::Error| capture.context.error(e.to_string());
        let mut encoder = ImageGifEncoder::new(
            &mut writer,
            capture.scene.width as usize,
            capture.scene.height as usize,
            &palette,
            frames,
            0,
        )
        .map_err(error)?;
        let mut previous: Option<Vec<u8>> = None;
        for i in 0..frames {
            let frame = frame_at(i as f64 * cycle / frames as f64)?;
            let indexed = index_frame(&capture.context, &frame, &lookup)?;
            let (left, top, width, height, pixels) = if let Some(before) = &previous {
                frame_delta(before, &indexed, frame.width(), frame.height())
            } else {
                (0, 0, frame.width(), frame.height(), indexed.clone())
            };
            encoder
                .write_frame(
                    left,
                    top,
                    width,
                    height,
                    cycle_frame_delay(i, frames, (cycle * 100.0).round() as usize),
                    DISPOSAL_NONE,
                    &pixels,
                )
                .map_err(error)?;
            previous = Some(indexed);
        }
        encoder.finish().map_err(error)?;
    }
    capture.context.check()?;
    Ok(writer.bytes)
}

pub(crate) fn native_gif_cycle(scene: &NativeScene) -> (usize, f64) {
    if !scene.animated_nodes.is_empty() || scene.panels.iter().any(|p| p.animated) {
        (96, 8.0)
    } else {
        (FRAME_COUNT, CYCLE_SECONDS)
    }
}

pub(crate) fn cycle_frame_delay(i: usize, frames: usize, centiseconds: usize) -> u16 {
    (((i + 1) as f64 * centiseconds as f64 / frames as f64).round()
        - (i as f64 * centiseconds as f64 / frames as f64).round()) as u16
}

pub(crate) fn palette_lookup(
    context: &RenderContext,
    palette: &[[u8; 4]],
) -> Result<Vec<u8>, RenderError> {
    let mut lookup = vec![0; 1 << 18];
    for r in 0..64 {
        context.check()?;
        for g in 0..64 {
            for b in 0..64 {
                let color = [r * 255 / 63, g * 255 / 63, b * 255 / 63, 255];
                let mut best = u32::MAX;
                for (i, p) in palette.iter().enumerate() {
                    // image/color.Palette.Index uses 16-bit RGBA distances,
                    // rounding each squared channel before summing, first tie.
                    let sum: u32 = (0..4)
                        .map(|j| {
                            let delta = color[j] as i32 * 257 - p[j] as i32 * 257;
                            ((delta as i64 * delta as i64) >> 2) as u32
                        })
                        .sum();
                    if sum < best {
                        best = sum;
                        lookup[r << 12 | g << 6 | b] = i as u8;
                    }
                    if best == 0 {
                        break;
                    }
                }
            }
        }
    }
    Ok(lookup)
}

pub(crate) fn index_frame(
    context: &RenderContext,
    frame: &NrgbaImage,
    lookup: &[u8],
) -> Result<Vec<u8>, RenderError> {
    let mut pixels = Vec::with_capacity(frame.width() * frame.height());
    for y in 0..frame.height() {
        if y % 32 == 0 {
            context.check()?;
        }
        for x in 0..frame.width() {
            let p = &frame.pix[y * frame.stride + x * 4..];
            let channel = |c: u8| ((u32::from(c) * 257 * u32::from(p[3]) / 255) >> 10) as usize;
            pixels.push(lookup[channel(p[0]) << 12 | channel(p[1]) << 6 | channel(p[2])]);
        }
    }
    Ok(pixels)
}

pub(crate) fn frame_delta(
    before: &[u8],
    after: &[u8],
    width: usize,
    height: usize,
) -> (usize, usize, usize, usize, Vec<u8>) {
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            if before[y * width + x] != after[y * width + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 {
        (x0, y0, x1, y1) = (0, 0, 1, 1);
    }
    let pixels = (y0..y1)
        .flat_map(|y| after[y * width + x0..y * width + x1].iter().copied())
        .collect();
    (x0, y0, x1 - x0, y1 - y0, pixels)
}

pub(crate) struct BoundedWriter<'a> {
    pub(crate) context: &'a RenderContext,
    pub(crate) bytes: Vec<u8>,
    pub(crate) remaining: usize,
}
impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.context.check().map_err(std::io::Error::other)?;
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other(
                "isometric image exceeds encoded byte limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_deltas_erase_old_particles_and_preserve_static_pixel() {
        let context = RenderContext::default();
        let lookup = palette_lookup(
            &context,
            &[[0, 0, 0, 255], [255, 255, 255, 255], [255, 0, 0, 255]],
        )
        .unwrap();
        let mut frames = Vec::new();
        for i in 0..3 {
            let mut raw = NrgbaImage {
                pix: vec![255; 8 * 4 * 4],
                stride: 8 * 4,
                rect: d2_rs_raster::Rect::new(0, 0, 8, 4),
            };
            raw.pix[(2 * 8 + i + 1) * 4..(2 * 8 + i + 2) * 4].copy_from_slice(&[255, 0, 0, 255]);
            frames.push(index_frame(&context, &raw, &lookup).unwrap());
        }
        let mut canvas = frames[0].clone();
        for after in &frames[1..] {
            let (x, y, w, h, pixels) = frame_delta(&canvas, after, 8, 4);
            for row in 0..h {
                canvas[(y + row) * 8 + x..(y + row) * 8 + x + w]
                    .copy_from_slice(&pixels[row * w..(row + 1) * w]);
            }
            assert_eq!(&canvas, after);
        }
        let (x, y, w, h, pixels) = frame_delta(&canvas, &canvas, 8, 4);
        assert_eq!((x, y, w, h), (0, 0, 1, 1));
        assert_eq!(pixels, canvas[..1]);
        assert_eq!(
            (0..100)
                .map(|i| cycle_frame_delay(i, 100, 833) as usize)
                .sum::<usize>(),
            833
        );
        assert_eq!(
            (0..96)
                .map(|i| cycle_frame_delay(i, 96, 800) as usize)
                .sum::<usize>(),
            800
        );
    }
    #[test]
    fn writer_checks_context_and_rejects_over_budget_before_writing() {
        let context = RenderContext::default();
        let mut writer = BoundedWriter {
            context: &context,
            bytes: vec![],
            remaining: 2,
        };
        assert!(writer.write(b"123").is_err());
        assert!(writer.bytes.is_empty());
        assert_eq!(writer.write(b"12").unwrap(), 2);
        let token = CancellationToken::default();
        token.cancel();
        let context = RenderContext::from_token(&token);
        writer.context = &context;
        assert_eq!(
            writer.write(b"1").unwrap_err().to_string(),
            "context canceled"
        );
        assert_eq!(writer.bytes, b"12");
    }
}
