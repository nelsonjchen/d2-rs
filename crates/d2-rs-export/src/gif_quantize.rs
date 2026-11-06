// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
//! D2 lib/xgif palette selection, dithering and canvas normalization.
//! Median cut follows Eric Pauley's go-quantize (ae555eb2afa4), quantize/
//! {mediancut,bucket}.go. See THIRD_PARTY_NOTICES.md for its BSD notice.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};

/// A multiply-rotate hasher for the quantizer's small integer keys (slot
/// numbers and dithered colors). These maps are private memo tables whose
/// iteration order is never observed, so only speed depends on the hash.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, tail) = bytes.as_chunks::<8>();
        for word in words {
            self.write_u64(u64::from_le_bytes(*word));
        }
        for &byte in tail {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

type KeyMap<K, V> = HashMap<K, V, BuildHasherDefault<KeyHasher>>;

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
    let mut slots = KeyMap::<usize, Color>::default();
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
    // Ascending slot order, as buildBucket traverses its table.
    let mut slots = slots.into_iter().collect::<Vec<_>>();
    slots.sort_unstable_by_key(|&(slot, _)| slot);
    let mut colors = slots
        .into_iter()
        .map(|(_, color)| color)
        .collect::<Vec<_>>();
    // One bucket holding every color, as go-quantize starts.
    #[allow(clippy::single_range_in_vec_init)]
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

/// draw.Palette.Index under xgif's metric: the first palette entry with the
/// least sum over `dimensions` channels of (difference^2 >> 2). Terms only
/// grow the sum, so an entry is dropped once its partial sum reaches the best
/// so far, which it could then no longer replace. No sum can wrap: four terms
/// of at most 0xffff^2 >> 2 fit in u32.
fn nearest(channels: &[[i32; 4]], dimensions: usize, target: [i32; 4]) -> u8 {
    let mut best = (u32::MAX, 0);
    'colors: for (index, color) in channels.iter().enumerate() {
        let mut distance = 0_u32;
        for axis in 0..dimensions {
            let difference = (target[axis] - color[axis]) as u32;
            distance += difference.wrapping_mul(difference) >> 2;
            if distance >= best.0 {
                continue 'colors;
            }
        }
        best = (distance, index as u8);
    }
    best.1
}

#[derive(Clone, Copy)]
struct PaletteNode {
    channels: [i32; 4],
    index: u8,
    axis: usize,
    left: i16,
    right: i16,
}

/// lib/xgif/xgif.go paletteTree: balanced RGB search with Palette.Index's
/// first-entry tie breaking and conservative split-plane pruning.
struct PaletteTree {
    nodes: Vec<PaletteNode>,
    root: i16,
    unique: bool,
    seeds: Option<[u8; 512]>,
}

impl PaletteTree {
    fn new(channels: &[[i32; 4]], seed: bool) -> Self {
        let mut nodes = channels
            .iter()
            .enumerate()
            .map(|(index, &channels)| PaletteNode {
                channels,
                index: index as u8,
                axis: 0,
                left: -1,
                right: -1,
            })
            .collect::<Vec<_>>();
        let unique = channels
            .iter()
            .enumerate()
            .all(|(i, color)| !channels[..i].contains(color));
        let root = Self::build(&mut nodes, 0);
        let mut tree = Self {
            nodes,
            root,
            unique,
            seeds: None,
        };
        if seed {
            let mut node_for_index = [0; 256];
            for (i, node) in tree.nodes.iter().enumerate() {
                node_for_index[usize::from(node.index)] = i as u8;
            }
            let mut seeds = [0; 512];
            for (i, entry) in seeds.iter_mut().enumerate() {
                let target = [
                    ((i >> 6) << 13 | 1 << 12) as i32,
                    (((i >> 3) & 7) << 13 | 1 << 12) as i32,
                    ((i & 7) << 13 | 1 << 12) as i32,
                    0xffff,
                ];
                *entry = node_for_index[usize::from(tree.nearest(target))];
            }
            tree.seeds = Some(seeds);
        }
        tree
    }

    fn build(nodes: &mut [PaletteNode], offset: usize) -> i16 {
        if nodes.is_empty() {
            return -1;
        }
        let mut minimum = nodes[0].channels;
        let mut maximum = minimum;
        for node in &nodes[1..] {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(node.channels[axis]);
                maximum[axis] = maximum[axis].max(node.channels[axis]);
            }
        }
        let mut axis = 0;
        for next in 1..3 {
            if maximum[next] - minimum[next] > maximum[axis] - minimum[axis] {
                axis = next;
            }
        }
        nodes.sort_unstable_by_key(|node| (node.channels[axis], node.index));
        let middle = nodes.len() / 2;
        let left = Self::build(&mut nodes[..middle], offset);
        let right = Self::build(&mut nodes[middle + 1..], offset + middle + 1);
        nodes[middle].axis = axis;
        nodes[middle].left = left;
        nodes[middle].right = right;
        (offset + middle) as i16
    }

    fn nearest(&self, target: [i32; 4]) -> u8 {
        let difference = |a: i32, b: i32| {
            let delta = (a - b) as u32;
            delta.wrapping_mul(delta) >> 2
        };
        let distance = |node: &PaletteNode| {
            difference(target[0], node.channels[0])
                + difference(target[1], node.channels[1])
                + difference(target[2], node.channels[2])
        };
        let mut best = (u32::MAX, 0);
        if let Some(seeds) = &self.seeds {
            let index =
                ((target[0] >> 13) << 6 | (target[1] >> 13) << 3 | target[2] >> 13) as usize;
            let node = &self.nodes[usize::from(seeds[index])];
            best = (distance(node), node.index);
        }
        // The 256-entry middle-split tree has depth at most 9, as in Go's
        // paletteTraversalStackCapacity bound.
        let mut stack = [(0_i16, 0_u32); 16];
        stack[0] = (self.root, 0);
        let mut length = 1;
        while length > 0 {
            length -= 1;
            let (index, minimum_distance) = stack[length];
            if minimum_distance > best.0 {
                continue;
            }
            let node = &self.nodes[index as usize];
            let distance = distance(node);
            if distance == 0 && self.unique {
                return node.index;
            }
            best = best.min((distance, node.index));
            let (near, far) = if target[node.axis] >= node.channels[node.axis] {
                (node.right, node.left)
            } else {
                (node.left, node.right)
            };
            if far >= 0 {
                stack[length] = (far, difference(target[node.axis], node.channels[node.axis]));
                length += 1;
            }
            if near >= 0 {
                stack[length] = (near, 0);
                length += 1;
            }
        }
        best.1
    }
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
    let tree = (dimensions == 3 && channels.len() > 16)
        .then(|| PaletteTree::new(&channels, pixels.len() / 4 >= 1024));
    let mut indices = Vec::with_capacity(pixels.len() / 4);
    let mut current = vec![[0_i32; 4]; usize::from(source_width) + 2];
    let mut next = current.clone();
    // A direct-mapped memo of nearest palette entries. The search is a pure
    // function of the dithered color, so the memo only saves work. Dithered
    // channels lie in 0..=0xffff and pack into one word; an empty slot's
    // index is out of palette range.
    const MEMO_BITS: u32 = 12;
    const EMPTY: u16 = u16::MAX;
    let mut nearest_memo = vec![(0_u64, EMPTY); 1 << MEMO_BITS];
    for row in pixels.chunks_exact(usize::from(source_width) * 4) {
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            let target = std::array::from_fn::<_, 4, _>(|axis| {
                (i32::from(pixel[axis]) * 0x101 + current[x + 1][axis] / 16).clamp(0, 0xffff)
            });
            let key = (target[0] as u64) << 48
                | (target[1] as u64) << 32
                | (target[2] as u64) << 16
                | target[3] as u64;
            let slot = (key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - MEMO_BITS)) as usize;
            let index = if nearest_memo[slot].1 != EMPTY && nearest_memo[slot].0 == key {
                nearest_memo[slot].1 as u8
            } else {
                let index = tree.as_ref().map_or_else(
                    || nearest(&channels, dimensions, target),
                    |tree| tree.nearest(target),
                );
                nearest_memo[slot] = (key, u16::from(index));
                index
            };
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
    fn palette_tree_keeps_first_entry_for_equal_distances_and_duplicate_colors() {
        // lib/xgif/xgif.go paletteTree.nearest preserves image/color
        // Palette.Index's first entry on ties, even across split planes.
        let mut channels = (0..33)
            .map(|i| [i * 2000, 0x4000, 0x8000, 0xffff])
            .collect::<Vec<_>>();
        channels[27] = channels[3];
        channels[31] = channels[30];
        for seed in [false, true] {
            let tree = PaletteTree::new(&channels, seed);
            assert_eq!(tree.nearest(channels[3]), 3);
            assert_eq!(tree.nearest(channels[30]), 30);
            assert_eq!(tree.nearest([7000, 0x4000, 0x8000, 0xffff]), 3);
            for r in (0..=0xffff).step_by(257) {
                let target = [r, 0, 0xffff, 0xffff];
                assert_eq!(tree.nearest(target), nearest(&channels, 3, target));
            }
        }
    }

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
