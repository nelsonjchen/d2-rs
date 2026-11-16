// SPDX-License-Identifier: MPL-2.0
// Go 1.27 runtime/msize.go roundupsize, with the size classes from
// internal/runtime/gc/sizeclasses.go.
// Copyright 2009 The Go Authors. All rights reserved. See root notices.

/// internal/runtime/gc SizeClassToSize.
const SIZE_CLASSES: &[usize] = &[
    0, 8, 16, 24, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 288, 320,
    352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536, 1792,
    2048, 2304, 2688, 3072, 3200, 3456, 4096, 4864, 5376, 6144, 6528, 6784, 6912, 8192, 9472, 9728,
    10240, 10880, 12288, 13568, 14336, 16384, 18432, 19072, 20480, 21760, 24576, 27264, 28672,
    32768,
];
const MAX_SMALL_SIZE: usize = 32768;
const MALLOC_HEADER_SIZE: usize = 8;
const MIN_SIZE_FOR_MALLOC_HEADER: usize = 512;
const PAGE_SIZE: usize = 8192;

/// runtime.roundupsize: the allocation size mallocgc uses for `size` bytes.
pub(crate) fn roundupsize(size: usize, noscan: bool) -> usize {
    if size <= MAX_SMALL_SIZE - MALLOC_HEADER_SIZE {
        let header = if !noscan && size > MIN_SIZE_FOR_MALLOC_HEADER {
            MALLOC_HEADER_SIZE
        } else {
            0
        };
        let request = size + header;
        return SIZE_CLASSES
            .iter()
            .find(|&&class| class >= request)
            .unwrap()
            - header;
    }
    size.checked_add(PAGE_SIZE - 1)
        .map_or(size, |request| request & !(PAGE_SIZE - 1))
}

/// The capacity of `[]rune(s)` for a heap-allocated conversion of `runes`
/// runes (runtime.rawruneslice); the runes past the length are zero.
pub(crate) fn rune_slice_capacity(runes: usize) -> usize {
    roundupsize(runes.saturating_mul(4), true) / 4
}

#[cfg(test)]
mod tests {
    use super::{roundupsize, rune_slice_capacity};

    #[test]
    fn allocation_sizes_follow_go_size_classes() {
        assert_eq!(roundupsize(0, true), 0);
        assert_eq!(roundupsize(1, true), 8);
        assert_eq!(roundupsize(24, true), 24);
        assert_eq!(roundupsize(25, true), 32);
        // 513 + 8 header bytes rounds to the 576-byte class.
        assert_eq!(roundupsize(513, false), 576 - 8);
        assert_eq!(roundupsize(32760, true), 32768);
        // Past the small-object limit, sizes round up to 8 KiB pages.
        assert_eq!(roundupsize(32761, true), 32768);
        assert_eq!(roundupsize(32769, true), 40960);
        // Chroma's raku lexer panics report these capacities.
        assert_eq!(rune_slice_capacity(4), 4);
        assert_eq!(rune_slice_capacity(6), 6);
        assert_eq!(rune_slice_capacity(3), 4);
    }
}
