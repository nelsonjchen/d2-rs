// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Derived from github.com/yuin/goldmark v1.8.5 text/segment.go.
// Complete MIT terms are in THIRD_PARTY_NOTICES.md.
use super::bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// A range of source bytes with synthetic indentation and an optional EOF newline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Segment {
    pub start: isize,
    pub stop: isize,
    pub padding: isize,
    pub force_newline: bool,
}

impl Segment {
    /// Source-backed value, preserving Go append writes and slice aliases.
    pub fn value_shared(&self, source: &Bytes) -> Bytes {
        let start = usize::try_from(self.start).unwrap();
        let stop = usize::try_from(self.stop).unwrap();
        let mut result = if self.padding == 0 {
            source.slice(start, stop)
        } else {
            let padding = usize::try_from(self.padding).unwrap();
            let mut bytes = vec![b' '; padding];
            bytes.extend_from_slice(&source.slice(start, stop).bytes());
            Bytes::with_capacity(&bytes, padding + stop - start + 1)
        };
        let newline =
            self.force_newline && !result.is_empty() && result.bytes().last() != Some(&b'\n');
        if newline {
            result = result.append_byte(b'\n');
        }
        result
    }
    pub const fn new(start: isize, stop: isize) -> Self {
        Self::with_padding(start, stop, 0)
    }

    pub const fn with_padding(start: isize, stop: isize, padding: isize) -> Self {
        Self {
            start,
            stop,
            padding,
            force_newline: false,
        }
    }

    fn source_range<'a>(&self, source: &'a [u8]) -> &'a [u8] {
        &source[usize::try_from(self.start).unwrap()..usize::try_from(self.stop).unwrap()]
    }

    /// Read-only byte view. Parser paths use `value_shared` to preserve source writes.
    pub fn value<'a>(&self, source: &'a [u8]) -> Cow<'a, [u8]> {
        let bytes = self.source_range(source);
        let mut result = if self.padding == 0 {
            Cow::Borrowed(bytes)
        } else {
            let mut result = vec![b' '; usize::try_from(self.padding).unwrap()];
            result.extend_from_slice(bytes);
            Cow::Owned(result)
        };
        if self.force_newline && !result.is_empty() && result.last() != Some(&b'\n') {
            result.to_mut().push(b'\n');
        }
        result
    }

    /// Goldmark's logical length excludes the optional synthetic newline.
    pub const fn len(&self) -> isize {
        self.stop - self.start + self.padding
    }

    pub const fn is_empty(&self) -> bool {
        self.start >= self.stop && self.padding == 0
    }

    pub fn between(&self, other: Self) -> Self {
        assert_eq!(self.stop, other.stop, "invalid state");
        Self::with_padding(self.start, other.start, self.padding - other.padding)
    }

    pub fn trim_right_space(&self, source: &[u8]) -> Self {
        let bytes = self.source_range(source);
        let trim = bytes.iter().rev().take_while(|b| is_space(**b)).count();
        if trim == bytes.len() {
            Self::new(self.start, self.start)
        } else {
            Self::with_padding(self.start, self.stop - trim as isize, self.padding)
        }
    }

    pub fn trim_left_space(&self, source: &[u8]) -> Self {
        let trim = self
            .source_range(source)
            .iter()
            .take_while(|b| is_space(**b))
            .count();
        Self::new(self.start + trim as isize, self.stop)
    }

    pub fn trim_left_space_width(&self, mut width: isize, source: &[u8]) -> Self {
        let mut padding = self.padding;
        while width > 0 {
            if padding == 0 {
                break;
            }
            padding -= 1;
            width -= 1;
        }
        if width == 0 {
            return Self::with_padding(self.start, self.stop, padding);
        }
        let mut start = self.start;
        for &byte in self.source_range(source) {
            // Upstream leaves the final byte even when it is a space/tab.
            if start >= self.stop - 1 || width <= 0 {
                break;
            }
            match byte {
                b' ' => width -= 1,
                b'\t' => width -= 4,
                _ => break,
            }
            start += 1;
        }
        if width < 0 {
            padding = -width;
        }
        Self::with_padding(start, self.stop, padding)
    }

    pub const fn with_start(&self, start: isize) -> Self {
        Self::with_padding(start, self.stop, self.padding)
    }

    pub const fn with_stop(&self, stop: isize) -> Self {
        Self::with_padding(self.start, stop, self.padding)
    }

    pub fn concat_padding(&self, value: &mut Vec<u8>) {
        if self.padding > 0 {
            value.resize(value.len() + self.padding as usize, b' ');
        }
    }
}

// Goldmark excludes vertical tab and form feed from its byte whitespace table.
pub(super) fn is_space(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\r' | b' ')
}
