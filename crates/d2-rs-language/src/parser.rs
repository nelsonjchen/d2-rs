// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Input cursor and single-quoted production from pinned d2parser/parse.go.
use d2_rs_core::ast::{Comment, Error, Position, Range, SingleQuotedString};
use serde::Serialize;
use std::collections::VecDeque;

#[derive(Clone, Debug, Default, Serialize)]
pub struct ParseError {
    #[serde(rename = "errs")]
    pub errors: Option<Vec<Error>>,
}
impl ParseError {
    pub fn is_empty(&self) -> bool {
        self.errors.as_ref().is_none_or(Vec::is_empty)
    }
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.errors.iter().flatten().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}
impl std::error::Error for ParseError {}

/// Rune input boundary; EOF and I/O failures are distinct.
pub trait RuneReader {
    fn read_rune(&mut self) -> Result<Option<char>, String>;
}
/// Go-compatible UTF-8 rune decoding: each invalid byte consumes one position.
pub struct Utf8Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Utf8Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}
impl RuneReader for Utf8Reader<'_> {
    fn read_rune(&mut self) -> Result<Option<char>, String> {
        if self.bytes.is_empty() {
            return Ok(None);
        }
        let (rune, width) = decode_rune(self.bytes);
        self.bytes = &self.bytes[width..];
        Ok(Some(rune))
    }
}

mod block;
mod containers;
mod edges;
pub mod indent;
pub mod numeric;
mod strings;
mod unicode;
/// Pinned Go 1.27 Unicode letter/digit categories shared by source-compatible consumers.
pub use unicode::is_letter_or_digit as is_go_letter_or_digit;

/// Maximum combined map/array nesting, excluding the file map.
pub const MAX_NESTING_DEPTH: usize = 128;
/// Cooperative cancellation checked before the first rune and every 256 reads.
pub type ParseContext = Box<dyn FnMut() -> Option<d2_rs_core::LayoutError>>;

pub(crate) fn decode_rune(bytes: &[u8]) -> (char, usize) {
    let width = match bytes[0] {
        0..=127 => 1,
        194..=223 => 2,
        224..=239 => 3,
        240..=244 => 4,
        _ => 0,
    };
    let rune = if width > 0 && bytes.len() >= width {
        std::str::from_utf8(&bytes[..width])
            .ok()
            .and_then(|s| s.chars().next())
    } else {
        None
    };
    (
        rune.unwrap_or('\u{fffd}'),
        if rune.is_some() { width } else { 1 },
    )
}

pub(crate) fn quote_input(mut bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    while !bytes.is_empty() {
        let (r, width) = decode_rune(bytes);
        if r == '\u{fffd}' && width == 1 {
            out.push_str(&format!("\\x{:02x}", bytes[0]));
            bytes = &bytes[1..];
            continue;
        }
        match r {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            _ if unicode::is_print(r) => out.push(r),
            _ if (r as u32) < 0x20 || r == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", r as u32))
            }
            _ if (r as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", r as u32)),
            _ => out.push_str(&format!("\\U{:08x}", r as u32)),
        }
        bytes = &bytes[width..];
    }
    out.push('"');
    out
}

/// Parser cursor. Production methods assume their opening delimiter was read.
pub struct Parser<R> {
    pub path: String,
    pub position: Position,
    pub utf16_positions: bool,
    pub errors: ParseError,
    pub depth: usize,
    pub in_edge_group: bool,
    reader: R,
    reader_position: Position,
    readahead: VecDeque<char>,
    lookahead: Vec<char>,
    lookahead_position: Position,
    io_error: bool,
    nesting_depth: usize,
    halted: bool,
    read_count: usize,
    pub(crate) context: Option<ParseContext>,
    pub(crate) context_error: Option<d2_rs_core::LayoutError>,
}
impl<R: RuneReader> Parser<R> {
    pub fn peek_not_space(&mut self) -> (Option<char>, usize) {
        let mut newlines = 0;
        loop {
            let Some(r) = self.peek() else {
                return (None, 0);
            };
            if is_space(r) {
                if r == '\n' {
                    newlines += 1;
                }
                continue;
            }
            return (Some(r), newlines);
        }
    }
    pub fn read_not_space(&mut self) -> Option<char> {
        loop {
            let r = self.read()?;
            if !is_space(r) {
                return Some(r);
            }
        }
    }
    /// Parse line comments after their initial `#`, merging adjacent lines.
    pub fn parse_comment(&mut self) -> Comment {
        let start = self.position.subtract('#', self.utf16_positions);
        let mut value = String::new();
        self.parse_comment_line(&mut value);
        loop {
            let (r, newlines) = self.peek_not_space();
            let Some(r) = r else {
                break;
            };
            if r != '#' || newlines >= 2 {
                self.rewind();
                break;
            }
            self.commit();
            if newlines == 1 {
                value.push('\n');
            }
            self.parse_comment_line(&mut value);
        }
        Comment {
            range: Range {
                path: self.path.clone(),
                start,
                end: self.position,
            },
            value,
        }
    }
    fn parse_comment_line(&mut self, value: &mut String) {
        let mut first = true;
        loop {
            let Some(r) = self.peek() else {
                return;
            };
            if r == '\n' {
                self.rewind();
                return;
            }
            self.commit();
            if first {
                first = false;
                if r == ' ' {
                    continue;
                }
            }
            value.push(r);
        }
    }
    pub fn new(path: String, reader: R, utf16_positions: bool) -> Self {
        Self {
            path,
            reader,
            utf16_positions,
            position: Position::default(),
            errors: ParseError::default(),
            depth: 0,
            in_edge_group: false,
            reader_position: Position::default(),
            readahead: VecDeque::new(),
            lookahead: Vec::new(),
            lookahead_position: Position::default(),
            io_error: false,
            nesting_depth: 0,
            halted: false,
            read_count: 0,
            context: None,
            context_error: None,
        }
    }
    fn enter_nesting(&mut self, start: Position) -> bool {
        if self.nesting_depth >= MAX_NESTING_DEPTH {
            self.error(
                start,
                self.position,
                &format!("maximum nesting depth of {MAX_NESTING_DEPTH} exceeded"),
            );
            self.halted = true;
            return false;
        }
        self.nesting_depth += 1;
        true
    }
    fn leave_nesting(&mut self) {
        self.nesting_depth -= 1;
    }
    fn read_rune(&mut self) -> Option<char> {
        if self.halted || self.context_error.is_some() {
            return None;
        }
        self.read_count = self.read_count.wrapping_add(1);
        if self.read_count % 256 == 1
            && let Some(context) = self.context.as_mut()
            && let Some(error) = context()
        {
            self.context_error = Some(error);
            return None;
        }
        if let Some(r) = self.readahead.pop_front() {
            return Some(r);
        }
        if self.io_error {
            self.rewind();
            return None;
        }
        self.reader_position = self.lookahead_position;
        match self.reader.read_rune() {
            Ok(Some(r)) => Some(r),
            result => {
                self.io_error = true;
                if let Err(error) = result {
                    self.errors.errors.get_or_insert_default().push(Error {
                        range: Range {
                            path: self.path.clone(),
                            start: self.reader_position,
                            end: self.reader_position,
                        },
                        message: format!("io error: {error}"),
                    });
                }
                self.rewind();
                None
            }
        }
    }
    pub fn read(&mut self) -> Option<char> {
        let r = self.read_rune()?;
        self.position = self.position.advance(r, self.utf16_positions);
        self.lookahead_position = self.position;
        Some(r)
    }
    /// Every series of peeks must be followed by commit or rewind.
    pub fn peek(&mut self) -> Option<char> {
        let r = self.read_rune()?;
        self.lookahead.push(r);
        self.lookahead_position = self.lookahead_position.advance(r, self.utf16_positions);
        Some(r)
    }
    pub fn peek_n(&mut self, count: usize) -> (String, bool) {
        let mut text = String::new();
        for _ in 0..count {
            let Some(r) = self.peek() else {
                return (text, true);
            };
            text.push(r);
        }
        (text, false)
    }
    pub fn commit(&mut self) {
        self.position = self.lookahead_position;
        self.lookahead.clear();
    }
    pub fn rewind(&mut self) {
        if self.lookahead.is_empty() {
            return;
        }
        for r in self.lookahead.drain(..).rev() {
            self.readahead.push_front(r);
        }
        self.lookahead_position = self.position;
    }
    pub fn replay(&mut self, r: char) {
        self.position = self.position.subtract(r, self.utf16_positions);
        self.lookahead.insert(0, r);
        self.rewind();
    }
    fn error(&mut self, start: Position, end: Position, message: &str) {
        if self.halted || self.context_error.is_some() {
            return;
        }
        let range = Range {
            path: self.path.clone(),
            start,
            end,
        };
        let message = format!("{range}: {message}");
        self.errors
            .errors
            .get_or_insert_default()
            .push(Error { range, message });
    }
    pub fn parse_single_quoted_string(&mut self) -> SingleQuotedString {
        let mut s = SingleQuotedString {
            range: Range {
                path: self.path.clone(),
                start: self.position.subtract('\'', self.utf16_positions),
                ..Default::default()
            },
            ..Default::default()
        };
        loop {
            let Some(r) = self.peek() else {
                self.error(
                    s.range.start,
                    self.reader_position,
                    "single quoted strings must be terminated with '",
                );
                break;
            };
            if r == '\n' {
                self.rewind();
                self.error(
                    s.range.start,
                    self.position,
                    "single quoted strings must be terminated with '",
                );
                break;
            }
            self.commit();
            if r == '\'' {
                let Some(next) = self.peek() else {
                    break;
                };
                if next == '\'' {
                    self.commit();
                    s.value.push('\'');
                    continue;
                }
                self.rewind();
                break;
            }
            if r != '\\' {
                s.value.push(r);
                continue;
            }
            let Some(next) = self.peek() else {
                continue;
            };
            if next == '\n' {
                self.commit();
                continue;
            }
            s.value.push(r);
            self.rewind();
        }
        s.range.end = self.position;
        s
    }
}

/// Go unicode.IsSpace's complete White Space set (including Latin-1 cases).
pub(crate) fn is_space(r: char) -> bool {
    matches!(r, '\u{9}'..='\u{d}' | '\u{20}' | '\u{85}' | '\u{a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

pub(crate) fn go_lower(s: &str) -> String {
    s.chars().map(unicode::to_lower).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    struct FailingReader<'a> {
        reader: Utf8Reader<'a>,
        calls: i64,
        fail: i64,
    }
    impl RuneReader for FailingReader<'_> {
        fn read_rune(&mut self) -> Result<Option<char>, String> {
            if self.calls == self.fail {
                return Err("oracle read failure".into());
            }
            self.calls += 1;
            self.reader.read_rune()
        }
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_container_regressions() {
        let report: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/containers.json"
        ))
        .unwrap();
        let requests = report["requests"].as_array().unwrap();
        let expected = report["reference"].as_array().unwrap();
        assert_eq!(requests.len(), expected.len());
        for (index, (r, expected)) in requests.iter().zip(expected).enumerate() {
            let bytes: Vec<u8> = r["hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
                .collect();
            let mut p = Parser::new(
                "input.d2".into(),
                FailingReader {
                    reader: Utf8Reader::new(&bytes),
                    calls: 0,
                    fail: r["fail"].as_i64().unwrap(),
                },
                r["utf16"].as_bool().unwrap(),
            );
            let node = match r["mode"].as_str().unwrap() {
                "file" => json!(p.parse_map(true)),
                "map" => {
                    p.read();
                    json!(p.parse_map(false))
                }
                "array" => {
                    p.read();
                    json!(p.parse_array())
                }
                "value" => json!(p.parse_value()),
                "mapkey" => json!(p.parse_map_key()),
                "index" => {
                    p.read();
                    json!(p.parse_edge_index())
                }
                _ => panic!("invalid fixture mode"),
            };
            let mut tail = String::new();
            while let Some(r) = p.read() {
                tail.push(r);
            }
            let actual = json!({"node":node,"errors":p.errors,"display":p.errors.to_string(),"position":p.position,"tail":tail,"depth":p.depth,"edge_group":p.in_edge_group});
            assert_eq!(&actual, expected, "case {index}: {r}");
        }
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_recursive_string_regressions() {
        recursive_string_regressions(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/imports.json"
        ));
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_recursive_string_oracle_regressions() {
        recursive_string_regressions(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/strings.json"
        ));
    }
    fn recursive_string_regressions(fixture: &str) {
        {
            let report: Value = serde_json::from_str(fixture).unwrap();
            let requests = report["requests"].as_array().unwrap();
            let expected = report["reference"].as_array().unwrap();
            assert_eq!(requests.len(), expected.len());
            for (index, (r, expected)) in requests.iter().zip(expected).enumerate() {
                let bytes: Vec<u8> = r["hex"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
                    .collect();
                let mut p = Parser::new(
                    "input.d2".into(),
                    FailingReader {
                        reader: Utf8Reader::new(&bytes),
                        calls: 0,
                        fail: r["fail"].as_i64().unwrap(),
                    },
                    r["utf16"].as_bool().unwrap(),
                );
                p.in_edge_group = r["edge"].as_bool().unwrap();
                let in_key = r["in_key"].as_bool().unwrap();
                let mut pattern_hex = None;
                let node = match r["mode"].as_str().unwrap() {
                    "key" => json!(p.parse_key()),
                    "string" => json!(p.parse_string(in_key)),
                    "unquoted" => {
                        let u = p.parse_unquoted_string(in_key);
                        pattern_hex = u.as_ref().and_then(|u| u.pattern.as_ref()).map(|parts| {
                            parts
                                .iter()
                                .map(|part| {
                                    part.read()
                                        .as_bytes()
                                        .iter()
                                        .map(|v| format!("{v:02x}"))
                                        .collect::<String>()
                                })
                                .collect::<Vec<_>>()
                        });
                        json!(u)
                    }
                    "double" => {
                        assert_eq!(p.read(), Some('"'));
                        json!(p.parse_double_quoted_string(in_key))
                    }
                    "import" => {
                        let spread = r["spread"].as_bool().unwrap_or(false);
                        for _ in 0..if spread { 4 } else { 1 } {
                            p.read();
                        }
                        json!(p.parse_import(spread))
                    }
                    "sub" => {
                        let spread = r["spread"].as_bool().unwrap_or(false);
                        for _ in 0..if spread { 4 } else { 1 } {
                            p.read();
                        }
                        json!(p.parse_substitution(spread))
                    }
                    _ => panic!("invalid fixture mode"),
                };
                let mut tail = String::new();
                while let Some(r) = p.read() {
                    tail.push(r);
                }
                let mut actual = json!({"node":node,"errors":p.errors,"display":p.errors.to_string(),"position":p.position,"tail":tail});
                if r["mode"] == "unquoted" {
                    actual["pattern_hex"] = json!(pattern_hex);
                }
                assert_eq!(&actual, expected, "case {index}: {r}");
            }
        }
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_production_regressions() {
        for fixture in [
            d2_rs_testdata::oracle_fixture!("tests/fixtures/single-quoted.json"),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/comment.json"),
        ] {
            production_regressions(fixture);
        }
    }
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn pinned_go_block_production_regressions() {
        for fixture in [
            d2_rs_testdata::oracle_fixture!("tests/fixtures/block.json"),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/block-comment.json"),
        ] {
            production_regressions(fixture);
        }
    }
    fn production_regressions(fixture: &str) {
        {
            let report: Value = serde_json::from_str(fixture).unwrap();
            let requests = report["requests"].as_array().unwrap();
            let reference = report["reference"].as_array().unwrap();
            assert_eq!(requests.len(), reference.len());
            for (index, (request, expected)) in requests.iter().zip(reference).enumerate() {
                let bytes: Vec<u8> = request["hex"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
                    .collect();
                let reader = FailingReader {
                    reader: Utf8Reader::new(&bytes),
                    calls: 0,
                    fail: request["fail"].as_i64().unwrap(),
                };
                let mut p = Parser::new(
                    "input.d2".into(),
                    reader,
                    request["utf16"].as_bool().unwrap(),
                );
                let comment = request["mode"].as_str() == Some("comment");
                let block = request["mode"].as_str() == Some("block");
                let block_comment = request["mode"].as_str() == Some("block_comment");
                p.depth = request["depth"].as_u64().unwrap_or(0) as usize;
                let mut scalar_hex = None;
                assert_eq!(
                    p.read(),
                    Some(if block_comment {
                        '"'
                    } else if block {
                        '|'
                    } else if comment {
                        '#'
                    } else {
                        '\''
                    })
                );
                let node = if block_comment {
                    let (text, eof) = p.peek_n(2);
                    assert!(!eof);
                    assert_eq!(text, "\"\"");
                    p.commit();
                    let b = p.parse_block_comment();
                    scalar_hex = Some(
                        b.value
                            .as_bytes()
                            .iter()
                            .map(|v| format!("{v:02x}"))
                            .collect::<String>(),
                    );
                    json!(b)
                } else if block {
                    let b = p.parse_block_string();
                    scalar_hex = Some(
                        b.value
                            .as_bytes()
                            .iter()
                            .map(|v| format!("{v:02x}"))
                            .collect::<String>(),
                    );
                    json!(b)
                } else if comment {
                    json!(p.parse_comment())
                } else {
                    json!(p.parse_single_quoted_string())
                };
                let mut tail = String::new();
                while let Some(r) = p.read() {
                    tail.push(r);
                }
                let mut actual = json!({"node":node,"errors":p.errors,"display":p.errors.to_string(),"position":p.position,"tail":tail});
                if let Some(hex) = scalar_hex {
                    actual[if block_comment {
                        "value_hex"
                    } else {
                        "scalar_hex"
                    }] = json!(hex);
                    actual["depth"] = json!(p.depth);
                }
                assert_eq!(&actual, expected, "case {index}: {request}");
            }
        }
    }
}
