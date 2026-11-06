// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2010 The Go Authors. All rights reserved.
// Rust translation Copyright 2026 Nelson Chen.
//! Name folding from pinned x/net/html Tokenizer.TagName/TagAttr. This is a
//! lexical adapter for existing native Markdown consumers, not HTML tree repair.

/// HTML folds ASCII tag/attribute names, while text and attribute values retain
/// their bytes. Goldmark's HTML projection itself remains unchanged.
pub fn normalize_html_names(input: &str) -> String {
    let mut bytes = input.as_bytes().to_vec();
    let mut cursor = 0;
    let space = |b: u8| matches!(b, b' ' | b'\n' | b'\r' | b'\t' | 12);
    while cursor < bytes.len() {
        if bytes[cursor] != b'<' {
            cursor += 1;
            continue;
        }
        if bytes[cursor..].starts_with(b"<!--") {
            cursor = input[cursor + 4..]
                .find("-->")
                .map_or(bytes.len(), |n| cursor + 4 + n + 3);
            continue;
        }
        let mut index = cursor + 1;
        if bytes.get(index) == Some(&b'/') {
            index += 1;
        }
        if !bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
            cursor += 1;
            continue;
        }
        while index < bytes.len() && !space(bytes[index]) && !matches!(bytes[index], b'/' | b'>') {
            bytes[index].make_ascii_lowercase();
            index += 1;
        }
        loop {
            while index < bytes.len() && (space(bytes[index]) || bytes[index] == b'/') {
                index += 1;
            }
            if index == bytes.len() {
                break;
            }
            if bytes[index] == b'>' {
                index += 1;
                break;
            }
            let start = index;
            while index < bytes.len()
                && !space(bytes[index])
                && !matches!(bytes[index], b'=' | b'/' | b'>')
            {
                bytes[index].make_ascii_lowercase();
                index += 1;
            }
            if index == start {
                index += 1;
                continue;
            }
            while index < bytes.len() && space(bytes[index]) {
                index += 1;
            }
            if bytes.get(index) != Some(&b'=') {
                continue;
            }
            index += 1;
            while index < bytes.len() && space(bytes[index]) {
                index += 1;
            }
            if let Some(quote) = bytes
                .get(index)
                .copied()
                .filter(|b| matches!(b, b'\'' | b'"'))
            {
                index += 1;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                if index < bytes.len() {
                    index += 1;
                }
            } else {
                while index < bytes.len() && !space(bytes[index]) && bytes[index] != b'>' {
                    index += 1;
                }
            }
        }
        cursor = index;
    }
    String::from_utf8(bytes).expect("ASCII name folding preserves UTF-8")
}
