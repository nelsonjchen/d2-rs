// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned parseBlockString, including delimiter byte slicing and rune lookahead.
use super::*;
use d2_rs_core::ast::{BlockComment, BlockString};
impl<R: RuneReader> Parser<R> {
    /// The three opening quotes must already have been consumed.
    pub fn parse_block_comment(&mut self) -> BlockComment {
        let start = (0..3).fold(self.position, |p, _| p.subtract('"', self.utf16_positions));
        let mut comment = BlockComment {
            range: Range {
                path: self.path.clone(),
                start,
                ..Default::default()
            },
            ..Default::default()
        };
        self.depth += 1;
        let mut body = String::new();
        self.parse_block_comment_contents(start, &mut body);
        self.depth -= 1;
        comment.range.end = self.position;
        comment.value =
            indent::trim_common_indent(&indent::trim_space_after_last_newline(body.as_bytes()))
                .into();
        comment
    }
    fn parse_block_comment_contents(&mut self, start: Position, body: &mut String) {
        const MESSAGE: &str = "block comments must be terminated with \"\"\"";
        loop {
            let Some(r) = self.peek() else {
                self.error(start, self.reader_position, MESSAGE);
                return;
            };
            if !is_space(r) {
                self.rewind();
                break;
            }
            self.commit();
            if r == '\n' {
                break;
            }
        }
        loop {
            let Some(r) = self.read() else {
                self.error(start, self.reader_position, MESSAGE);
                return;
            };
            if r != '"' {
                body.push(r);
                continue;
            }
            let (text, eof) = self.peek_n(2);
            if eof {
                self.error(start, self.reader_position, MESSAGE);
                return;
            }
            if text != "\"\"" {
                body.push('"');
                self.rewind();
                continue;
            }
            self.commit();
            return;
        }
    }
    pub fn parse_block_string(&mut self) -> BlockString {
        let mut block = BlockString {
            range: Range {
                path: self.path.clone(),
                start: self.position.subtract('|', self.utf16_positions),
                ..Default::default()
            },
            ..Default::default()
        };
        self.depth += 1;
        let mut body = String::new();
        self.parse_block_contents(&mut block, &mut body);
        self.depth -= 1;
        block.range.end = self.position;
        block.value =
            indent::trim_common_indent(&indent::trim_space_after_last_newline(body.as_bytes()))
                .into();
        block
    }
    fn block_error(&mut self, block: &BlockString) {
        self.error(
            block.range.start,
            self.reader_position,
            &format!("block string must be terminated with {}|", block.quote),
        );
    }
    fn parse_block_contents(&mut self, block: &mut BlockString, body: &mut String) {
        loop {
            let Some(r) = self.peek() else {
                self.block_error(block);
                return;
            };
            if is_space(r) || unicode::is_letter_or_digit(r) || r == '_' {
                self.rewind();
                break;
            }
            self.commit();
            block.quote.push(r);
        }
        loop {
            let Some(r) = self.peek() else {
                self.block_error(block);
                return;
            };
            if is_space(r) {
                self.rewind();
                break;
            }
            self.commit();
            block.tag.push(r);
        }
        if block.tag.is_empty() {
            block.tag = "md".into();
        }
        loop {
            let Some(r) = self.peek() else {
                self.block_error(block);
                return;
            };
            if !is_space(r) {
                body.push_str(&" ".repeat(self.depth * 2));
                self.rewind();
                break;
            }
            self.commit();
            if r == '\n' {
                break;
            }
        }
        let mut end_hint = '|';
        let mut end_rest = Vec::new();
        if let Some(last) = block.quote.chars().next_back() {
            end_hint = last;
            // Upstream removes last-rune width from the *front* of Quote.
            end_rest.extend_from_slice(&block.quote.as_bytes()[last.len_utf8()..]);
            end_rest.push(b'|');
        }
        loop {
            let Some(r) = self.read() else {
                self.block_error(block);
                return;
            };
            if r != end_hint {
                body.push(r);
                continue;
            }
            let mut peeked = String::new();
            // Upstream peekn consumes this byte length as a rune count.
            for _ in 0..end_rest.len() {
                let Some(r) = self.peek() else {
                    self.block_error(block);
                    return;
                };
                peeked.push(r);
            }
            if peeked.as_bytes() != end_rest {
                body.push(end_hint);
                self.rewind();
                continue;
            }
            self.commit();
            return;
        }
    }
}
