// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Derived from Goldmark v1.8.5 text/reader.go. MIT terms in THIRD_PARTY_NOTICES.md.
use super::bytes::Bytes;
use super::reader::decode_rune;
use super::text::{Segment, is_space};

/// Reader over parsed block source segments. The borrowed segment list cannot
/// be mutated while reading; Go slice alias mutation is not exposed by this API.
pub struct BlockReader<'a> {
    source: Bytes,
    segments: Option<&'a [Segment]>,
    line: isize,
    pos: Segment,
    head: isize,
    last: isize,
    line_offset: isize,
}

impl<'a> BlockReader<'a> {
    pub fn new(source: &[u8], segments: Option<&'a [Segment]>) -> Self {
        Self::with_source(Bytes::new(source), segments)
    }
    pub fn with_source(source: Bytes, segments: Option<&'a [Segment]>) -> Self {
        let mut r = Self {
            source,
            segments: None,
            line: 0,
            pos: Segment::default(),
            head: 0,
            last: 0,
            line_offset: 0,
        };
        if let Some(segments) = segments {
            r.reset(segments);
        }
        r
    }
    pub fn source(&self) -> Bytes {
        self.source.clone()
    }
    fn count(&self) -> isize {
        self.segments.map_or(0, |s| s.len() as isize)
    }
    pub fn reset(&mut self, segments: &'a [Segment]) {
        self.segments = Some(segments);
        self.reset_position();
    }
    pub fn reset_position(&mut self) {
        self.line = -1;
        self.head = 0;
        self.last = 0;
        self.line_offset = -1;
        self.pos.start = -1;
        self.pos.stop = -1;
        self.pos.padding = 0;
        if let Some(last) = self.segments.and_then(|s| s.last()) {
            self.last = last.stop;
        }
        self.advance_line();
    }
    pub fn value(&self, seg: Segment) -> Vec<u8> {
        let segments = self.segments.expect("nil segments");
        let mut line = self.count() - 1;
        while line >= 0 && seg.start < segments[line as usize].start {
            line -= 1;
        }
        let mut result = Vec::new();
        let mut i = seg.start;
        while line < self.count() {
            let s = segments[usize::try_from(line).unwrap()];
            if i < 0 {
                i = s.start;
            }
            s.concat_padding(&mut result);
            while i < seg.stop && i < s.stop {
                result.push(self.source.bytes()[usize::try_from(i).unwrap()]);
                i += 1;
            }
            i = -1;
            if s.stop > seg.stop {
                break;
            }
            line += 1;
        }
        result
    }
    pub fn preceding_character(&self) -> char {
        if self.pos.padding != 0 {
            return ' ';
        }
        let segments = self.segments.expect("nil segments");
        if segments.is_empty() || (self.line == 0 && self.pos.start <= segments[0].start) {
            return '\n';
        }
        let mut i = self.pos.start - 1;
        while i >= 0 && i < self.source.len() as isize {
            if self.source.bytes()[i as usize] & 0xc0 != 0x80 {
                break;
            }
            i -= 1;
        }
        if i < 0 || i >= self.source.len() as isize {
            return '\n';
        }
        decode_rune(&self.source.bytes()[i as usize..]).0
    }
    pub fn line_offset(&mut self) -> isize {
        if self.line_offset < 0 {
            let mut width = 0;
            for i in self.head..self.pos.start {
                width += if self.source.bytes()[i as usize] == b'\t' {
                    4 - width % 4
                } else {
                    1
                };
            }
            self.line_offset = width - self.pos.padding;
        }
        self.line_offset
    }
    fn in_range(&self) -> bool {
        self.line < self.count() && self.pos.start >= 0 && self.pos.start < self.last
    }
    pub fn peek(&self) -> u8 {
        if !self.in_range() {
            return 0xff;
        }
        if self.pos.padding != 0 {
            b' '
        } else {
            self.source.bytes()[self.pos.start as usize]
        }
    }
    pub fn peek_line(&mut self) -> (Option<Bytes>, Segment) {
        (
            if self.in_range() {
                Some(self.pos.value_shared(&self.source))
            } else {
                None
            },
            self.pos,
        )
    }
    pub fn advance(&mut self, mut n: isize) {
        self.line_offset = -1;
        if n < self.pos.stop - self.pos.start && self.pos.padding == 0 {
            self.pos.start += n;
            return;
        }
        while n > 0 {
            n -= 1;
            if self.pos.padding != 0 {
                self.pos.padding -= 1;
                continue;
            }
            if self.pos.start >= self.pos.stop - 1 && self.pos.stop < self.last {
                self.advance_line();
                continue;
            }
            self.pos.start += 1;
        }
    }
    pub fn advance_and_set_padding(&mut self, n: isize, padding: isize) {
        self.advance(n);
        if padding > self.pos.padding {
            self.set_padding(padding);
        }
    }
    pub fn advance_to_eol(&mut self) {
        self.line_offset = -1;
        self.pos.padding = 0;
        self.pos.start =
            if self.source.bytes()[usize::try_from(self.pos.stop - 1).unwrap()] == b'\n' {
                self.pos.stop - 1
            } else {
                self.pos.stop
            };
    }
    pub fn advance_line(&mut self) {
        self.set_position(self.line + 1, Segment::new(-1, -1));
        self.head = self.pos.start;
    }
    pub fn position(&self) -> (isize, Segment) {
        (self.line, self.pos)
    }
    pub fn set_position(&mut self, line: isize, pos: Segment) {
        self.line_offset = -1;
        self.line = line;
        if pos.start == -1 {
            if self.line < self.count() {
                let s = self.segments.unwrap()[self.line as usize];
                self.head = s.start;
                self.pos = s;
            }
        } else {
            self.pos = pos;
            if self.line < self.count() {
                self.head = self.segments.unwrap()[self.line as usize].start;
            }
        }
    }
    pub fn set_padding(&mut self, padding: isize) {
        self.line_offset = -1;
        self.pos.padding = padding;
    }
    pub fn read_rune(&mut self) -> Option<(char, usize)> {
        let (line, _) = self.peek_line();
        let (rune, size) = decode_rune(&line?.bytes());
        if rune == '\u{fffd}' {
            return None;
        }
        self.advance(size as isize);
        Some((rune, size))
    }
    pub fn skip_blank_lines(&mut self) -> (Segment, usize, bool) {
        let mut lines = 0;
        loop {
            let (line, seg) = self.peek_line();
            let Some(line) = line else {
                return (seg, lines, false);
            };
            if !line.bytes().iter().all(|b| is_space(*b)) {
                return (seg, lines, true);
            }
            lines += 1;
            self.advance_line();
        }
    }
    pub fn skip_spaces(&mut self) -> (Segment, usize, bool) {
        let mut chars = 0;
        loop {
            let (line, seg) = self.peek_line();
            if line.is_none() {
                return (seg, chars, false);
            }
            let line = line.unwrap();
            for (i, &byte) in line.bytes().iter().enumerate() {
                if is_space(byte) {
                    chars += 1;
                    self.advance(1);
                } else {
                    return (seg.with_start(seg.start + i as isize + 1), chars, true);
                }
            }
        }
    }
}
