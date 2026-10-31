// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/raw_html.go delimiter-based forms.
use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    reader_api::ReaderCursor,
    text::Segment,
};
impl Context {
    /// Comments, processing instructions, declarations and CDATA. Open/close
    /// tags use the separate regexp-reader path, which is not implemented here.
    pub fn parse_raw_html_delimited(&mut self, block: &mut impl ReaderCursor) -> Option<NodeId> {
        let (line, _) = block.peek_line();
        let line = line?;
        let bytes = line.bytes();
        let (closer, comment) = if bytes.starts_with(b"<!--") {
            (b"-->".as_slice(), true)
        } else if bytes.starts_with(b"<?") {
            (b"?>".as_slice(), false)
        } else if bytes.len() > 2 && bytes[1] == b'!' && bytes[2].is_ascii_uppercase() {
            (b">".as_slice(), false)
        } else if bytes.starts_with(b"<![CDATA[") {
            (b"]]>".as_slice(), false)
        } else {
            return None;
        };
        let saved = block.position();
        let node = self.arena.insert(NodeKind::RawHtml {
            segments: Vec::new(),
        });
        if comment {
            let n = if bytes.starts_with(b"<!-->") {
                Some(5)
            } else if bytes.starts_with(b"<!--->") {
                Some(6)
            } else {
                None
            };
            if let Some(n) = n {
                let (_, segment) = block.peek_line();
                self.append_raw_html_segment(node, segment.with_stop(segment.start + n));
                block.advance(n);
                return Some(node);
            }
        }
        let mut offset = if comment { 4 } else { 0 };
        loop {
            let (line, segment) = block.peek_line();
            let Some(line) = line else { break };
            let bytes = line.bytes();
            let found = bytes[offset..]
                .windows(closer.len())
                .position(|v| v == closer);
            if let Some(index) = found {
                let n = (offset + index + closer.len()) as isize;
                self.append_raw_html_segment(node, segment.with_stop(segment.start + n));
                block.advance(n);
                return Some(node);
            }
            self.append_raw_html_segment(node, segment);
            offset = 0;
            block.advance_line();
        }
        block.set_position(saved.0, saved.1);
        None
    }
    fn append_raw_html_segment(&mut self, node: NodeId, segment: Segment) {
        let NodeKind::RawHtml { segments } = &mut self.arena.node_mut(node).data else {
            unreachable!()
        };
        segments.push(segment);
    }
}

/// Prefix language of Goldmark's openTagRegexp/closeTagRegexp. This accepts
/// byte slices like regexp.FindIndex; reader-specific EOF/lookahead semantics
/// belong to the separate Markdown reader adapter.
pub fn match_tag_prefix(source: &[u8], closing: bool) -> Option<usize> {
    let prefix: &[u8] = if closing { b"</" } else { b"<" };
    if !source.starts_with(prefix) {
        return None;
    }
    let mut i = prefix.len();
    if !source.get(i).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    i += 1;
    while source
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        i += 1;
    }
    loop {
        let mut end = i;
        while end < source.len() {
            match source[end] {
                b' ' | b'\t' | b'\n' => end += 1,
                b'\r' if source.get(end + 1) == Some(&b'\n') => end += 2,
                _ => break,
            }
        }
        if !closing && source.get(end) == Some(&b'/') {
            end += 1;
        }
        if source.get(end) == Some(&b'>') {
            return Some(end + 1);
        }
        if closing {
            return None;
        }
        let before = i;
        while source
            .get(i)
            .is_some_and(|b| matches!(b, b'\r' | b'\n' | b' ' | b'\t'))
        {
            i += 1;
        }
        if i == before
            || !source
                .get(i)
                .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'_' | b':'))
        {
            return None;
        }
        i += 1;
        while source
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
        {
            i += 1;
        }
        let after_name = i;
        while source
            .get(i)
            .is_some_and(|b| matches!(b, b'\r' | b'\n' | b' ' | b'\t'))
        {
            i += 1;
        }
        if source.get(i) != Some(&b'=') {
            i = after_name;
            continue;
        }
        i += 1;
        while source
            .get(i)
            .is_some_and(|b| matches!(b, b'\r' | b'\n' | b' ' | b'\t'))
        {
            i += 1;
        }
        let first = *source.get(i)?;
        if matches!(first, b'\'' | b'"') {
            i += 1;
            while source.get(i).is_some_and(|&b| b != first) {
                i += 1;
            }
            if source.get(i) != Some(&first) {
                return None;
            }
            i += 1;
        } else {
            let start = i;
            while source
                .get(i)
                .is_some_and(|&b| b > 32 && !matches!(b, b'"' | b'\'' | b'=' | b'<' | b'>' | 96))
            {
                i += 1;
            }
            if i == start {
                return None;
            }
        }
    }
}

impl Context {
    /// Complete Goldmark rawHTMLParser entry point for the block reader used
    /// during inline parsing.
    pub fn parse_raw_html(
        &mut self,
        block: &mut super::block_reader::BlockReader<'_>,
    ) -> Option<NodeId> {
        let (line, _) = block.peek_line();
        let line = line?;
        let bytes = line.bytes();
        let closing = if bytes.len() > 1 && bytes[1].is_ascii_alphanumeric() {
            false
        } else if bytes.len() > 2 && bytes[1] == b'/' && bytes[2].is_ascii_alphanumeric() {
            true
        } else {
            return self.parse_raw_html_delimited(block);
        };
        let saved = block.position();
        let matched = super::tag_matcher::match_tag_reader(closing, || block.read_rune());
        block.set_position(saved.0, saved.1);
        let end = matched?;
        block.advance(end as isize);
        let node = self.arena.insert(NodeKind::RawHtml {
            segments: Vec::new(),
        });
        let finish = block.position();
        block.set_position(saved.0, saved.1);
        loop {
            let (line, segment) = block.peek_line();
            if line.is_none() {
                break;
            }
            let current = block.position().0;
            let start = if current == saved.0 {
                saved.1.start
            } else {
                segment.start
            };
            let stop = if current == finish.0 {
                finish.1.start
            } else {
                segment.stop
            };
            self.append_raw_html_segment(node, Segment::new(start, stop));
            if current == finish.0 {
                block.advance(stop - start);
                break;
            }
            block.advance_line();
        }
        Some(node)
    }
}
