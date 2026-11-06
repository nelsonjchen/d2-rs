// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/delimiter.go ScanDelimiter and scalar methods.
use super::{reader::decode_rune, text::Segment};
use serde::{Deserialize, Serialize};
mod unicode;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Delimiter {
    pub can_open: bool,
    pub can_close: bool,
    pub length: isize,
    pub original_length: isize,
    pub char: u8,
    pub segment: Segment,
}

impl Delimiter {
    pub fn consume_characters(&mut self, n: isize) {
        self.length -= n;
        self.segment = self.segment.with_stop(self.segment.start + self.length);
    }
    pub fn calc_consumption(&self, closer: &Self) -> isize {
        if (self.can_close || closer.can_open)
            && (self.original_length + closer.original_length) % 3 == 0
            && closer.original_length % 3 != 0
        {
            return 0;
        }
        if self.length >= 2 && closer.length >= 2 {
            2
        } else {
            1
        }
    }
}

fn classes(rune: i32) -> (bool, bool) {
    let flags = if rune >= 0 {
        let r = rune as u32;
        let i = unicode::RANGES.partition_point(|row| row.1 < r);
        unicode::RANGES
            .get(i)
            .filter(|row| row.0 <= r)
            .map_or(0, |row| row.2)
    } else {
        0
    };
    // Preserve Goldmark's signed <=256 test followed by conversion to byte.
    let byte_space = rune <= 256 && matches!(rune as u8, b'\t' | b'\n' | b'\r' | b' ');
    (flags & 1 != 0, byte_space || flags & 2 != 0)
}

/// Scan a nonempty line using the processor's delimiter-byte set. Linked
/// delimiter processing and inline-node rewrites remain separate work.
pub fn scan_delimiter(
    line: &[u8],
    before: i32,
    minimum: isize,
    allowed: &[u8],
) -> Option<Delimiter> {
    let byte = line[0];
    if !allowed.contains(&byte) {
        return None;
    }
    let length = line.iter().take_while(|b| **b == byte).count();
    if (length as isize) < minimum {
        return None;
    }
    let after = if length == line.len() {
        ' ' as i32
    } else {
        // util.ToRune backs up across continuation bytes before decoding.
        let mut i = length;
        while line[i] & 0xc0 == 0x80 {
            i = i.checked_sub(1).expect("invalid rune range");
        }
        decode_rune(&line[i..]).0 as i32
    };
    let (before_punctuation, before_space) = classes(before);
    let (after_punctuation, after_space) = classes(after);
    let left = !after_space && (!after_punctuation || before_space || before_punctuation);
    let right = !before_space && (!before_punctuation || after_space || after_punctuation);
    let (can_open, can_close) = if byte == b'_' {
        (
            left && (!right || before_punctuation),
            right && (!left || after_punctuation),
        )
    } else {
        (left, right)
    };
    Some(Delimiter {
        can_open,
        can_close,
        length: length as isize,
        original_length: length as isize,
        char: byte,
        segment: Segment::default(),
    })
}
