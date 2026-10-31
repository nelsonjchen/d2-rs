// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/link.go parseLinkDestination and parseLinkTitle.
use super::{block_reader::BlockReader, bytes::Bytes, closure::FindClosureOptions, text::is_space};

/// Parse on the block reader used by inline links and reference definitions.
/// The value and success flag are separate: failure may retain an empty slice.
pub fn parse_link_destination(block: &mut BlockReader<'_>) -> (Option<Bytes>, bool) {
    block.skip_spaces();
    let (line, _) = block.peek_line();
    if block.peek() == b'<' {
        let line = line.expect("missing angle destination line");
        let bytes = line.bytes();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_punctuation() {
                i += 2;
                continue;
            }
            if bytes[i] == b'>' {
                block.advance(i as isize + 1);
                return (Some(line.slice(1, i)), true);
            }
            i += 1;
        }
        return (None, false);
    }
    let Some(line) = line else {
        block.advance(0);
        return (None, false);
    };
    let bytes = line.bytes();
    let mut opened = 0;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_punctuation() {
            i += 2;
            continue;
        }
        if byte == b'(' {
            opened += 1;
        } else if byte == b')' {
            opened -= 1;
            if opened < 0 {
                break;
            }
        } else if is_space(byte) {
            break;
        }
        i += 1;
    }
    block.advance(i as isize);
    (Some(line.slice(0, i)), i != 0)
}

/// Title bytes are copied from block segments, as in the original block-reader
/// path. Allocation capacity is not part of this result's API.
pub fn parse_link_title(block: &mut BlockReader<'_>) -> (Option<Vec<u8>>, bool) {
    block.skip_spaces();
    let opener = block.peek();
    if !matches!(opener, b'"' | b'\'' | b'(') {
        return (None, false);
    }
    let closer = if opener == b'(' { b')' } else { opener };
    block.advance(1);
    let segments = block.find_closure(
        opener,
        closer,
        FindClosureOptions {
            newline: true,
            advance: true,
            ..FindClosureOptions::default()
        },
    );
    let Some(segments) = segments else {
        return (None, false);
    };
    let mut value = Vec::new();
    for segment in segments {
        value.extend_from_slice(&block.value(segment));
    }
    (Some(value), true)
}

/// Parsed reference before paragraph extraction and context registration.
#[derive(Debug)]
pub struct LinkReferenceDefinition {
    pub label: Vec<u8>,
    pub destination: Bytes,
    pub title: Option<Vec<u8>>,
    pub first_segment: super::text::Segment,
}

/// Goldmark parser/link_ref.go's definition parser. The caller registers a
/// successful reference and extracts its paragraph lines. Failure deliberately
/// retains the source reader's partially consumed position.
pub fn parse_link_reference_definition(
    block: &mut BlockReader<'_>,
) -> (Option<LinkReferenceDefinition>, isize, isize) {
    let failed = || (None, -1, -1);
    block.skip_spaces();
    let (line, _) = block.peek_line();
    let Some(line) = line else { return failed() };
    let start_line = block.position().0;
    let (mut width, mut pos) = (0, 0);
    for &byte in line.bytes().iter() {
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4 - width % 4,
            _ => break,
        }
        pos += 1;
    }
    if width > 3 {
        return failed();
    }
    if width != 0 {
        pos += 1;
    }
    if line.bytes()[pos] != b'[' {
        return failed();
    }
    let first_segment = block.position().1;
    block.advance(pos as isize + 1);
    let options = FindClosureOptions {
        newline: true,
        advance: true,
        ..Default::default()
    };
    let Some(segments) = block.find_closure(b'[', b']', options) else {
        return failed();
    };
    let label: Vec<u8> = segments.into_iter().flat_map(|s| block.value(s)).collect();
    if label.iter().all(|&b| is_space(b)) || block.peek() != b':' {
        return failed();
    }
    block.advance(1);
    block.skip_spaces();
    let (destination, ok) = parse_link_destination(block);
    if !ok {
        return failed();
    }
    let destination = destination.expect("successful destination");
    let (line, _) = block.peek_line();
    let is_newline = line.is_none_or(|v| v.bytes().iter().all(|&b| is_space(b)));
    let end_line = block.position().0;
    let (_, spaces, _) = block.skip_spaces();
    let opener = block.peek();
    let make = |title| {
        Some(LinkReferenceDefinition {
            label,
            destination,
            title,
            first_segment,
        })
    };
    if !matches!(opener, b'"' | b'\'' | b'(') {
        return if is_newline {
            (make(None), start_line, end_line + 1)
        } else {
            failed()
        };
    }
    if spaces == 0 {
        return failed();
    }
    block.advance(1);
    let closer = if opener == b'(' { b')' } else { opener };
    let Some(segments) = block.find_closure(opener, closer, options) else {
        if !is_newline {
            return failed();
        }
        block.advance_line();
        return (make(None), start_line, end_line + 1);
    };
    let title = segments.into_iter().flat_map(|s| block.value(s)).collect();
    let (line, _) = block.peek_line();
    if line.is_some_and(|v| !v.bytes().iter().all(|&b| is_space(b))) {
        return if is_newline {
            (make(Some(title)), start_line, end_line)
        } else {
            failed()
        };
    }
    (make(Some(title)), start_line, block.position().0 + 1)
}

#[derive(Debug)]
pub struct ExtractedReference {
    pub definition: LinkReferenceDefinition,
    pub lines: Vec<super::text::Segment>,
    pub blank_previous_lines: bool,
}

/// Extract reference nodes in insertion order and update the remaining paragraph
/// lines. An empty line list tells the AST caller to remove the paragraph. The
/// caller must register definitions in its reference context in this same order.
pub fn extract_link_references(
    source: Bytes,
    lines: &mut Vec<super::text::Segment>,
    blank_previous_lines: bool,
) -> Vec<ExtractedReference> {
    let mut references = Vec::new();
    let mut removes = Vec::new();
    {
        let mut block = BlockReader::with_source(source.clone(), Some(lines));
        loop {
            let (definition, start, mut end) = parse_link_reference_definition(&mut block);
            if start < 0 {
                break;
            }
            let definition = definition.expect("successful reference definition");
            let mut reference_lines = vec![definition.first_segment];
            for i in start + 1..end {
                reference_lines.push(lines[i as usize]);
            }
            let last = reference_lines.last_mut().unwrap();
            *last = last.trim_right_space(&source.bytes());
            references.push(ExtractedReference {
                definition,
                lines: reference_lines,
                blank_previous_lines: start == 0 && blank_previous_lines,
            });
            if start == end {
                end += 1;
            }
            removes.push((start, end));
        }
    }
    let mut offset = 0;
    for (start, end) in removes {
        if lines.is_empty() {
            break;
        }
        let suffix = lines[(end - offset) as usize..].to_vec();
        lines.truncate((start - offset) as usize);
        lines.extend(suffix);
        offset = end;
    }
    references
}

/// Goldmark's byte-preserving full case folding, followed by reference-label
/// whitespace normalization. Invalid UTF-8 remains byte-for-byte intact.
pub fn normalize_reference_label(value: &[u8]) -> Vec<u8> {
    let start = value
        .iter()
        .position(|&b| !matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|&b| !matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .map_or(start, |i| i + 1);
    let value = &value[start..end];
    let mut folded = Vec::new();
    let mut i = 0;
    while i < value.len() {
        let byte = value[i];
        if byte.is_ascii_uppercase() {
            folded.push(byte + 32);
            i += 1;
            continue;
        }
        if byte >= 0xb5 && byte & 0xc0 != 0x80 {
            let (r, n) = super::reader::decode_rune(&value[i..]);
            if r != '\u{fffd}'
                && let Ok(index) =
                    super::case_folding::FOLDINGS.binary_search_by_key(&(r as u32), |&(key, _)| key)
            {
                folded.extend_from_slice(super::case_folding::FOLDINGS[index].1);
                i += n;
                continue;
            }
        }
        folded.push(byte);
        i += 1;
    }
    let mut result = Vec::new();
    let mut in_space = false;
    for byte in folded {
        if is_space(byte) {
            if !in_space {
                result.push(b' ');
            }
            in_space = true;
        } else {
            result.push(byte);
            in_space = false;
        }
    }
    result
}
