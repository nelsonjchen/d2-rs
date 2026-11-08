// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/html_block.go.

use super::{
    arena::NodeId,
    context::{Context, NodeKind},
    paragraph::BlockState,
    reader_api::BlockCursor,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmlBlockType {
    Type1,
    Type2,
    Type3,
    Type4,
    Type5,
    Type6,
    Type7,
}

fn indentation(line: &[u8]) -> Option<usize> {
    let count = line.iter().take_while(|&&byte| byte == b' ').count();
    (count <= 3).then_some(count)
}

fn tag_name(line: &[u8]) -> Option<(usize, usize, bool)> {
    let mut start = indentation(line)?;
    if start >= line.len() || line[start] != b'<' {
        return None;
    }
    start += 1;
    let close_tag = line.get(start) == Some(&b'/');
    if close_tag {
        start += 1;
        while line.get(start) == Some(&b' ') {
            start += 1;
        }
    }
    // Goldmark's type 6/7 regexes require [a-zA-Z] before the remaining
    // alphanumeric/hyphen tag-name bytes (parser/html_block.go).
    if !line.get(start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let end = line[start..]
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'-')?
        + start;
    (end > start).then_some((start, end, close_tag))
}

fn allowed(name: &[u8]) -> bool {
    matches!(
        name,
        b"address"
            | b"article"
            | b"aside"
            | b"base"
            | b"basefont"
            | b"blockquote"
            | b"body"
            | b"caption"
            | b"center"
            | b"col"
            | b"colgroup"
            | b"dd"
            | b"details"
            | b"dialog"
            | b"dir"
            | b"div"
            | b"dl"
            | b"dt"
            | b"fieldset"
            | b"figcaption"
            | b"figure"
            | b"footer"
            | b"form"
            | b"frame"
            | b"frameset"
            | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"head"
            | b"header"
            | b"hr"
            | b"html"
            | b"iframe"
            | b"legend"
            | b"li"
            | b"link"
            | b"main"
            | b"menu"
            | b"menuitem"
            | b"meta"
            | b"nav"
            | b"noframes"
            | b"ol"
            | b"optgroup"
            | b"option"
            | b"p"
            | b"param"
            | b"search"
            | b"section"
            | b"summary"
            | b"table"
            | b"tbody"
            | b"td"
            | b"tfoot"
            | b"th"
            | b"thead"
            | b"title"
            | b"tr"
            | b"track"
            | b"ul"
    )
}

fn strip_line_ending(line: &[u8]) -> Option<&[u8]> {
    if line.ends_with(b"\r\n") {
        Some(&line[..line.len() - 2])
    } else if line.ends_with(b"\n") {
        Some(&line[..line.len() - 1])
    } else {
        Some(line)
    }
}

fn is_attribute_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':')
}

fn is_attribute_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-')
}

fn is_attribute_space(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n' | b' ' | b'\t')
}

fn valid_attribute_value(line: &[u8], mut index: usize) -> Option<usize> {
    let whitespace_start = index;
    while line
        .get(index)
        .is_some_and(|&byte| is_attribute_space(byte))
    {
        index += 1;
    }
    if line.get(index) != Some(&b'=') {
        return Some(whitespace_start);
    }
    index += 1;
    while line
        .get(index)
        .is_some_and(|&byte| is_attribute_space(byte))
    {
        index += 1;
    }
    match line.get(index).copied()? {
        b'\'' => {
            index += 1;
            while line.get(index).is_some_and(|&byte| byte != b'\'') {
                index += 1;
            }
            (line.get(index) == Some(&b'\'')).then_some(index + 1)
        }
        b'"' => {
            index += 1;
            while line.get(index).is_some_and(|&byte| byte != b'"') {
                index += 1;
            }
            (line.get(index) == Some(&b'"')).then_some(index + 1)
        }
        _ => {
            let start = index;
            while let Some(&byte) = line.get(index) {
                if byte == b'"'
                    || byte == b'\''
                    || byte == b'='
                    || byte == b'<'
                    || byte == b'>'
                    || byte == b'`'
                    || byte <= b' '
                {
                    break;
                }
                index += 1;
            }
            (index > start).then_some(index)
        }
    }
}

fn valid_standalone_tail(tail: &[u8]) -> Option<bool> {
    let line = strip_line_ending(tail)?;
    let mut index = 0;
    let mut has_attribute = false;
    loop {
        let &byte = line.get(index)?;
        if byte == b'>' {
            return line[index + 1..]
                .iter()
                .all(|&value| value == b' ')
                .then_some(has_attribute);
        }
        if byte == b'/' && line.get(index + 1) == Some(&b'>') {
            return line[index + 2..]
                .iter()
                .all(|&value| value == b' ')
                .then_some(has_attribute);
        }
        if !is_attribute_space(byte) {
            return None;
        }
        let whitespace_start = index;
        while line
            .get(index)
            .is_some_and(|&value| is_attribute_space(value))
        {
            index += 1;
        }
        if line.get(index) == Some(&b'>')
            || (line.get(index) == Some(&b'/') && line.get(index + 1) == Some(&b'>'))
        {
            let whitespace_only = line[whitespace_start..index]
                .iter()
                .all(|&value| value == b' ');
            let close_end = if line[index] == b'>' {
                index + 1
            } else {
                index + 2
            };
            return (whitespace_only && line[close_end..].iter().all(|&value| value == b' '))
                .then_some(has_attribute);
        }
        line.get(index)?;
        if !is_attribute_name_start(line[index]) {
            return None;
        }
        has_attribute = true;
        index += 1;
        while line
            .get(index)
            .is_some_and(|&value| is_attribute_name_byte(value))
        {
            index += 1;
        }
        index = valid_attribute_value(line, index)?;
    }
}

fn valid_type1_tail(tail: &[u8]) -> bool {
    let Some(line) = strip_line_ending(tail) else {
        return false;
    };
    line.is_empty()
        || line.first().is_some_and(|&byte| {
            byte.is_ascii_whitespace() || byte == b'>' || line.starts_with(b"/>")
        })
}

fn valid_type6_tail(tail: &[u8]) -> bool {
    let Some(line) = strip_line_ending(tail) else {
        return false;
    };
    line.is_empty() || line.starts_with(b" ") || line.starts_with(b">") || line.starts_with(b"/>")
}

fn classify(line: &[u8]) -> Option<HtmlBlockType> {
    let indent = indentation(line)?;
    let trimmed = &line[indent..];
    if trimmed.starts_with(b"<!--") {
        return Some(HtmlBlockType::Type2);
    }
    if trimmed.starts_with(b"<?") {
        return Some(HtmlBlockType::Type3);
    }
    if trimmed.starts_with(b"<![CDATA[") {
        return Some(HtmlBlockType::Type5);
    }
    if trimmed.starts_with(b"<!") {
        let mut index = 2;
        while trimmed.get(index).is_some_and(u8::is_ascii_uppercase) {
            index += 1;
        }
        if index > 2 {
            return Some(HtmlBlockType::Type4);
        }
    }
    if let Some((start, end, close_tag)) = tag_name(line) {
        let name = line[start..end].to_ascii_lowercase();
        if matches!(name.as_slice(), b"script" | b"pre" | b"style" | b"textarea")
            && valid_type1_tail(&line[end..])
        {
            return Some(HtmlBlockType::Type1);
        }
        if let Some(has_attribute) = valid_standalone_tail(&line[end..]) {
            if allowed(&name) {
                return Some(HtmlBlockType::Type6);
            }
            if !(close_tag && has_attribute) {
                return Some(HtmlBlockType::Type7);
            }
        }
        if allowed(&name) && valid_type6_tail(&line[end..]) {
            return Some(HtmlBlockType::Type6);
        }
    }
    None
}

/// Whether an HTML block opener is allowed to interrupt a paragraph.
/// Goldmark's type 7 standalone-tag parser is deliberately excluded.
pub fn can_interrupt_paragraph(line: &[u8]) -> bool {
    classify(line).is_some_and(|kind| kind != HtmlBlockType::Type7)
}

fn closure(kind: HtmlBlockType) -> &'static [u8] {
    match kind {
        HtmlBlockType::Type1 => b"",
        HtmlBlockType::Type2 => b"-->",
        HtmlBlockType::Type3 => b"?>",
        HtmlBlockType::Type4 => b">",
        HtmlBlockType::Type5 => b"]]>",
        HtmlBlockType::Type6 | HtmlBlockType::Type7 => b"",
    }
}

fn type1_closes(line: &[u8]) -> bool {
    let names = [b"script".as_slice(), b"pre", b"style", b"textarea"];
    line.windows(2).enumerate().any(|(index, window)| {
        window == b"</"
            && names.iter().any(|name| {
                line.get(index + 2..index + 2 + name.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
                    && line.get(index + 2 + name.len()) == Some(&b'>')
            })
    })
}

impl Context {
    pub fn open_html_block(
        &mut self,
        reader: &mut impl BlockCursor,
    ) -> (Option<NodeId>, BlockState) {
        let (line, segment) = reader.peek_line();
        let Some(line) = line else {
            return (None, BlockState::NO_CHILDREN);
        };
        let bytes = line.bytes();
        let Some(kind) = classify(&bytes) else {
            return (None, BlockState::NO_CHILDREN);
        };
        let node = self.arena.insert(NodeKind::HtmlBlock {
            block_type: kind,
            lines: vec![segment],
            closure_line: None,
        });
        reader.advance_to_eol();
        (Some(node), BlockState::NO_CHILDREN)
    }

    pub fn continue_html_block(
        &mut self,
        node: NodeId,
        reader: &mut impl BlockCursor,
    ) -> BlockState {
        let (line, segment) = reader.peek_line();
        let Some(line) = line else {
            return BlockState::CLOSE;
        };
        let bytes = line.bytes();
        let NodeKind::HtmlBlock {
            block_type,
            lines,
            closure_line,
        } = &mut self.arena.node_mut(node).data
        else {
            panic!("not an HTML block")
        };
        if matches!(block_type, HtmlBlockType::Type6 | HtmlBlockType::Type7) {
            if bytes.iter().all(|&byte| super::text::is_space(byte)) {
                return BlockState::CLOSE;
            }
        } else {
            let first = lines
                .first()
                .map(|value| value.value_shared(&reader.source()));
            let first_closes = if *block_type == HtmlBlockType::Type1 {
                first
                    .as_ref()
                    .is_some_and(|value| type1_closes(&value.bytes()))
            } else {
                let pattern = closure(*block_type);
                first.as_ref().is_some_and(|value| {
                    value
                        .bytes()
                        .windows(pattern.len())
                        .any(|window| window == pattern)
                })
            };
            if first_closes {
                return BlockState::CLOSE;
            }
            let current_closes = if *block_type == HtmlBlockType::Type1 {
                type1_closes(&bytes)
            } else {
                let pattern = closure(*block_type);
                bytes.windows(pattern.len()).any(|window| window == pattern)
            };
            if current_closes {
                *closure_line = Some(segment);
                reader.advance_to_eol();
                return BlockState::CLOSE;
            }
        }
        lines.push(segment);
        reader.advance_to_eol();
        BlockState(18)
    }
}
