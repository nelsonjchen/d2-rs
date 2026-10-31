// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
//! D2 lib/xgif palette selection, dithering and canvas normalization.
//! Median cut follows Eric Pauley's go-quantize (ae555eb2afa4), quantize/
//! {mediancut,bucket}.go. See THIRD_PARTY_NOTICES.md for its BSD notice.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Clone, Copy, Default)]
struct Color {
    rgba: [u8; 4],
    count: u32,
}

fn split(colors: &mut [Color]) -> usize {
    let mut minimum = [255; 3];
    let mut maximum = [0; 3];
    let mut counts = [[0_u64; 256]; 3];
    let mut total = 0_u64;
    for color in colors.iter() {
        for axis in 0..3 {
            let channel = color.rgba[axis];
            minimum[axis] = minimum[axis].min(channel);
            maximum[axis] = maximum[axis].max(channel);
            counts[axis][usize::from(channel)] += u64::from(color.count);
        }
        total += u64::from(color.count);
    }
    let span = std::array::from_fn::<_, 3, _>(|axis| maximum[axis] - minimum[axis]);
    let axis = if span[0] > span[1] && span[0] > span[2] {
        0
    } else if span[1] > span[2] {
        1
    } else {
        2
    };
    let mut accumulated = 0;
    let mut median = 0;
    for (index, count) in counts[axis].iter().enumerate() {
        median = index as u8;
        if accumulated > total / 2 || accumulated + count == total {
            break;
        }
        accumulated += count;
    }
    let (mut left, mut right) = (0, colors.len() - 1);
    while left < right {
        colors.swap(left, right);
        while colors[left].rgba[axis] < median && left < right {
            left += 1;
        }
        while colors[right].rgba[axis] >= median && left < right {
            right -= 1;
        }
    }
    left.clamp(1, colors.len() - 1)
}

/// go-quantize's default median-cut palette for premultiplied RGBA samples.
/// Shared with the isometric encoder, which supplies its own indexing policy.
pub fn palette(pixels: &[u8]) -> Vec<[u8; 4]> {
    if pixels.is_empty() {
        return Vec::new();
    }
    // buildBucket's slots, including collision probes and ascending slot
    // traversal, determine the first mode on ties. Store occupied slots only.
    let table_size = pixels.len() / 2;
    let mut slots = BTreeMap::<usize, Color>::new();
    for pixel in pixels.chunks_exact(4) {
        let rgba: [u8; 4] = pixel.try_into().unwrap();
        let mut probe =
            usize::from(rgba[0]) << 16 | usize::from(rgba[1]) << 8 | usize::from(rgba[2]);
        let mut step = 1;
        loop {
            let entry = slots.entry(probe % table_size).or_default();
            if entry.count == 0 || entry.rgba == rgba {
                entry.rgba = rgba;
                entry.count += 1;
                break;
            }
            probe += 1 + step;
            step += 1;
        }
    }
    let mut colors = slots.into_values().collect::<Vec<_>>();
    let mut buckets = VecDeque::from([0..colors.len()]);
    while buckets.len() < 256 && buckets.len() < colors.len() {
        let range = buckets.pop_front().unwrap();
        if range.len() < 2 {
            buckets.push_back(range);
            continue;
        }
        let middle = range.start
            + if range.len() == 2 {
                1
            } else {
                split(&mut colors[range.clone()])
            };
        buckets.push_back(range.start..middle);
        buckets.push_back(middle..range.end);
    }
    buckets
        .into_iter()
        .map(|range| {
            let mut best = Color::default();
            for color in &colors[range] {
                if color.count > best.count {
                    best = *color;
                }
            }
            best.rgba
        })
        .collect()
}

/// Input is the premultiplied RGBA raster produced by tiny-skia. Quantize
/// each source frame before centering, as xgif.QuantizeImage does.
pub(crate) fn frame(
    pixels: &[u8],
    source_width: u16,
    source_height: u16,
    width: u16,
    height: u16,
    delay: u16,
) -> gif::Frame<'static> {
    let mut palette = palette(pixels);
    let channels = palette
        .iter()
        .map(|color| color.map(|value| i32::from(value) * 0x101))
        .collect::<Vec<_>>();
    let dimensions = if pixels.chunks_exact(4).all(|pixel| pixel[3] == 255) {
        3
    } else {
        4
    };
    let mut indices = Vec::with_capacity(pixels.len() / 4);
    let mut current = vec![[0_i32; 4]; usize::from(source_width) + 2];
    let mut next = current.clone();
    let mut nearest_cache = HashMap::new();
    for row in pixels.chunks_exact(usize::from(source_width) * 4) {
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            let target = std::array::from_fn::<_, 4, _>(|axis| {
                (i32::from(pixel[axis]) * 0x101 + current[x + 1][axis] / 16).clamp(0, 0xffff)
            });
            let index = *nearest_cache.entry(target).or_insert_with(|| {
                let mut best = (u32::MAX, 0);
                for (index, color) in channels.iter().enumerate() {
                    let distance = (0..dimensions)
                        .map(|axis| {
                            let difference = (target[axis] - color[axis]) as u32;
                            difference.wrapping_mul(difference) >> 2
                        })
                        .sum::<u32>();
                    if distance < best.0 {
                        best = (distance, index as u8);
                    }
                }
                best.1
            });
            indices.push(index);
            for axis in 0..dimensions {
                let error = target[axis] - channels[usize::from(index)][axis];
                next[x][axis] += error * 3;
                next[x + 1][axis] += error * 5;
                next[x + 2][axis] += error;
                current[x + 2][axis] += error * 7;
            }
        }
        std::mem::swap(&mut current, &mut next);
        next.fill([0; 4]);
        // Keep the nearest-color memo bounded independently of frame size.
        if nearest_cache.len() > 65536 {
            nearest_cache.clear();
        }
    }
    // finishQuantizedRaster pads the palette before paletteWithBackground.
    let transparent = palette.iter().position(|color| color[3] == 0);
    palette.resize(palette.len().max(2).next_power_of_two(), [0, 0, 0, 255]);
    let background = if palette.len() == 256 {
        let mut brightest = (0, 0);
        for (index, color) in palette.iter().enumerate() {
            let sum = color[..3].iter().map(|&value| u32::from(value)).sum();
            if sum > brightest.0 {
                brightest = (sum, index);
            }
        }
        palette[brightest.1] = [255; 4];
        brightest.1
    } else {
        let index = palette.len();
        palette.push([255; 4]);
        index
    };
    let mut canvas = vec![background as u8; usize::from(width) * usize::from(height)];
    let offset_x = usize::from((width - source_width) / 2);
    let offset_y = usize::from((height - source_height) / 2);
    for (y, row) in indices.chunks_exact(usize::from(source_width)).enumerate() {
        let start = (y + offset_y) * usize::from(width) + offset_x;
        canvas[start..start + row.len()].copy_from_slice(row);
    }
    gif::Frame {
        width,
        height,
        delay,
        transparent: transparent.map(|index| index as u8),
        palette: Some(
            palette
                .into_iter()
                .flat_map(|color| [color[0], color[1], color[2]])
                .collect(),
        ),
        buffer: Cow::Owned(canvas),
        ..gif::Frame::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn median_cut_dither_and_centering_match_pinned_xgif() {
        // More than 256 colors exercises median cut and nonzero error diffusion.
        // Expected decoded RGB comes from pinned xgif.QuantizeImage followed by
        // NormalizePalettedImage; the reproducer is in the run-26 evidence.
        let mut rgba = Vec::new();
        for y in 0..16 {
            for x in 0..32 {
                rgba.extend_from_slice(&[
                    (x * 7 + y * 11) as u8,
                    (x * 13 + y * 3) as u8,
                    (x * 5 + y * 17) as u8,
                    255,
                ]);
            }
        }
        let frame = frame(&rgba, 32, 16, 36, 20, 1);
        let palette = frame.palette.as_ref().unwrap();
        let rgb = frame
            .buffer
            .iter()
            .flat_map(|&index| {
                let offset = usize::from(index) * 3;
                palette[offset..offset + 3].iter().copied()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            format!("{:x}", Sha256::digest(rgb)),
            "3aaa8187470708bc047bc1a1026cb01d69c7ddab5c5dfe2d55931d2952860cf3"
        );
    }
}
