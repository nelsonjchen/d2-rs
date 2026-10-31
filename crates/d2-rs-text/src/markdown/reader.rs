// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Derived from Goldmark v1.8.5 text/reader.go. MIT terms in THIRD_PARTY_NOTICES.md.
use super::bytes::Bytes;
use super::text::{Segment, is_space};

/// Goldmark's contiguous source reader. Regex searches are pending. Cached
/// lines intentionally survive SetPosition and SetPadding, matching upstream.
pub struct Reader {
    source: Bytes,
    line: isize,
    peeked_line: Option<Bytes>,
    pos: Segment,
    head: isize,
    line_offset: isize,
}

impl Reader {
    pub fn new(source: &[u8]) -> Self {
        Self::with_source(Bytes::new(source))
    }

    pub fn with_source(source: Bytes) -> Self {
        let mut reader = Self {
            source,
            line: 0,
            peeked_line: None,
            pos: Segment::default(),
            head: 0,
            line_offset: 0,
        };
        reader.reset_position();
        reader
    }

    pub fn source(&self) -> Bytes {
        self.source.clone()
    }

    pub fn reset_position(&mut self) {
        self.line = -1;
        self.head = 0;
        self.line_offset = -1;
        self.advance_line();
    }

    pub fn peek(&self) -> u8 {
        if self.pos.start >= 0 && self.pos.start < self.source.len() as isize {
            if self.pos.padding != 0 {
                b' '
            } else {
                self.source.bytes()[self.pos.start as usize]
            }
        } else {
            0xff
        }
    }

    pub fn peek_line(&mut self) -> (Option<Bytes>, Segment) {
        if self.pos.start >= 0 && self.pos.start < self.source.len() as isize {
            if self.peeked_line.is_none() {
                self.peeked_line = Some(self.pos.value_shared(&self.source));
            }
            (self.peeked_line.clone(), self.pos)
        } else {
            (None, self.pos)
        }
    }

    pub fn position(&self) -> (isize, Segment) {
        (self.line, self.pos)
    }

    pub fn set_position(&mut self, line: isize, pos: Segment) {
        self.line_offset = -1;
        self.line = line;
        self.pos = pos;
    }

    pub fn set_padding(&mut self, padding: isize) {
        self.pos.padding = padding;
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

    pub fn preceding_character(&self) -> char {
        if self.pos.start <= 0 {
            return if self.pos.padding != 0 { ' ' } else { '\n' };
        }
        let mut index = self.pos.start - 1;
        while index >= 0 && self.source.bytes()[index as usize] & 0xc0 == 0x80 {
            index -= 1;
        }
        let index = usize::try_from(index).expect("invalid UTF-8 predecessor range");
        decode_rune(&self.source.bytes()[index..]).0
    }

    pub fn advance(&mut self, mut n: isize) {
        self.line_offset = -1;
        if n < self.peeked_line.as_ref().map_or(0, |s| s.len() as isize) && self.pos.padding == 0 {
            self.pos.start += n;
            self.peeked_line = None;
            return;
        }
        self.peeked_line = None;
        while n > 0 && self.pos.start < self.source.len() as isize {
            n -= 1;
            if self.pos.padding != 0 {
                self.pos.padding -= 1;
                continue;
            }
            if self.source.bytes()[self.pos.start as usize] == b'\n' {
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
        if self.pos.start >= self.source.len() as isize {
            return;
        }
        self.line_offset = -1;
        let mut newline = None;
        if let Some(line) = &self.peeked_line {
            self.pos.start += line.len() as isize - self.pos.padding - 1;
            if self.source.bytes()[self.pos.start as usize] == b'\n' {
                newline = Some(0);
            }
        }
        if newline.is_none() {
            newline = self.source.bytes()[self.pos.start as usize..]
                .iter()
                .position(|b| *b == b'\n');
        }
        self.peeked_line = None;
        self.pos.start =
            newline.map_or(self.source.len() as isize, |i| self.pos.start + i as isize);
        self.pos.padding = 0;
    }

    pub fn advance_line(&mut self) {
        self.line_offset = -1;
        self.peeked_line = None;
        self.pos.start = self.pos.stop;
        self.head = self.pos.start;
        if self.pos.start < 0 || self.pos.start >= self.source.len() as isize {
            return;
        }
        self.pos.stop = self.source.len() as isize;
        if let Some(i) = self.source.bytes()[self.pos.start as usize..]
            .iter()
            .position(|b| *b == b'\n')
        {
            self.pos.stop = self.pos.start + i as isize + 1;
        }
        self.line += 1;
        self.pos.padding = 0;
    }

    /// None matches io.EOF, including Goldmark's treatment of U+FFFD.
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
            let Some(_) = line else {
                return (seg, chars, false);
            };
            let line = self.peeked_line.clone().unwrap();
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

pub(super) fn decode_rune(bytes: &[u8]) -> (char, usize) {
    let Some(&first) = bytes.first() else {
        return ('\u{fffd}', 0);
    };
    let size = match first {
        0..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return ('\u{fffd}', 1),
    };
    match bytes
        .get(..size)
        .and_then(|s| std::str::from_utf8(s).ok())
        .and_then(|s| s.chars().next())
    {
        Some(rune) => (rune, size),
        None => ('\u{fffd}', 1),
    }
}
