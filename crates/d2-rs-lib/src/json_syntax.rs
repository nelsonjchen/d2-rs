// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2010 The Go Authors. All rights reserved.
// Copyright 2024 The Go Authors. All rights reserved.
// Source-derived from Go 1.27 encoding/json/{v2_scanner.go,jsontext/decode.go,
// internal/jsonwire/{decode,wire}.go}. See THIRD_PARTY_NOTICES.md for BSD terms.

//! Syntax-first JSON validation and legacy Go string replacement semantics.

use super::D2Error;
use std::borrow::Cow;

#[derive(Clone, Copy)]
enum State {
    Value,
    ObjectKey(bool),
    Colon,
    ObjectEnd,
    ArrayValue(bool),
    ArrayEnd,
    End,
}

struct Scanner<'a> {
    input: &'a [u8],
    pos: usize,
    depth: usize,
    output: Option<Vec<u8>>,
    copied: usize,
}

fn error(message: impl Into<String>) -> D2Error {
    D2Error::GoGraphConversion(message.into())
}
fn eof() -> D2Error {
    error("unexpected end of JSON input")
}

// Like utf8.DecodeRune, invalid encodings consume one byte, not the complete
// invalid subsequence accepted by String::from_utf8_lossy.
fn rune_width(bytes: &[u8]) -> usize {
    let width = match bytes[0] {
        0..=127 => 1,
        194..=223 => 2,
        224..=239 => 3,
        240..=244 => 4,
        _ => return 0,
    };
    if bytes.len() >= width && std::str::from_utf8(&bytes[..width]).is_ok() {
        width
    } else {
        0
    }
}
fn quote_rune(bytes: &[u8]) -> String {
    let width = rune_width(bytes).max(1);
    if bytes[0] == b'\'' {
        return "'\\\''".into();
    }
    if bytes[0] == b'"' {
        return "'\"'".into();
    }
    let quoted = d2_rs_core::quote_go_bytes(&bytes[..width]);
    format!("'{}'", &quoted[1..quoted.len() - 1])
}
fn escape_error(bytes: &[u8]) -> D2Error {
    let quoted = d2_rs_core::quote_go_bytes(bytes);
    // Go uses backticks only for printable, non-whitespace text without a
    // backtick or RuneError. A quoting escape other than quote/backslash means
    // the pinned printability table rejected that rune.
    let printable = quoted[1..quoted.len() - 1]
        .replace("\\\\", "")
        .replace("\\\"", "")
        .find('\\')
        .is_none();
    let plain = std::str::from_utf8(bytes).ok().filter(|s| {
        printable
            && !s
                .chars()
                .any(|c| c == '`' || c == '\u{fffd}' || c.is_whitespace())
    });
    let what = plain.map_or(quoted, |s| format!("`{s}`"));
    error(format!("invalid escape sequence {what} in string"))
}
fn hex(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 4 {
        return None;
    }
    bytes.iter().try_fold(0u16, |value, &byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        Some(value * 16 + u16::from(digit))
    })
}

impl Scanner<'_> {
    fn peek(&self) -> Result<u8, D2Error> {
        self.input.get(self.pos).copied().ok_or_else(eof)
    }
    fn whitespace(&mut self) {
        while self
            .input
            .get(self.pos)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.pos += 1;
        }
    }
    fn invalid(&self, context: &str) -> D2Error {
        if self.pos == self.input.len() {
            return eof();
        }
        error(format!(
            "invalid character {} {context}",
            quote_rune(&self.input[self.pos..])
        ))
    }
    fn replace(&mut self, start: usize, end: usize) {
        let output = self.output.get_or_insert_with(Vec::new);
        output.extend_from_slice(&self.input[self.copied..start]);
        output.extend_from_slice("\u{fffd}".as_bytes());
        self.copied = end;
    }
    fn string(&mut self) -> Result<(), D2Error> {
        self.pos += 1; // caller consumed the opening-quote condition
        loop {
            match self.peek()? {
                b'"' => {
                    self.pos += 1;
                    return Ok(());
                }
                b'\\' => {
                    let start = self.pos;
                    let next = *self.input.get(start + 1).ok_or_else(eof)?;
                    match next {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.pos += 2,
                        b'u' => {
                            let end = (start + 6).min(self.input.len());
                            let digits = &self.input[start + 2..end];
                            if digits.len() < 4 && digits.iter().all(u8::is_ascii_hexdigit) {
                                return Err(eof());
                            }
                            let value =
                                hex(digits).ok_or_else(|| escape_error(&self.input[start..end]))?;
                            self.pos += 6;
                            if (0xd800..=0xdbff).contains(&value)
                                && self.input.get(self.pos..self.pos + 2) == Some(b"\\u")
                                && self
                                    .input
                                    .get(self.pos + 2..self.pos + 6)
                                    .and_then(hex)
                                    .is_some_and(|second| (0xdc00..=0xdfff).contains(&second))
                            {
                                self.pos += 6; // retain a valid surrogate pair unchanged
                            } else if (0xd800..=0xdfff).contains(&value) {
                                self.replace(start, self.pos);
                            }
                        }
                        _ => return Err(escape_error(&self.input[start..start + 2])),
                    }
                }
                0..=31 => return Err(self.invalid("in string")),
                128..=255 => {
                    let width = rune_width(&self.input[self.pos..]);
                    if width == 0 {
                        self.replace(self.pos, self.pos + 1);
                        self.pos += 1;
                    } else {
                        self.pos += width;
                    }
                }
                _ => self.pos += 1,
            }
        }
    }
    fn literal(&mut self, literal: &str) -> Result<(), D2Error> {
        for expected in literal.bytes() {
            if self.peek()? != expected {
                return Err(self.invalid(&format!(
                    "in literal {literal} (expecting '{}')",
                    char::from(expected)
                )));
            }
            self.pos += 1;
        }
        Ok(())
    }
    fn digits(&mut self) -> Result<(), D2Error> {
        if !self.peek()?.is_ascii_digit() {
            return Err(self.invalid("in numeric literal"));
        }
        while self.input.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        Ok(())
    }
    fn number(&mut self) -> Result<(), D2Error> {
        if self.peek()? == b'-' {
            self.pos += 1;
        }
        if self.peek()? == b'0' {
            self.pos += 1;
        } else {
            self.digits()?;
        }
        if self.input.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            self.digits()?;
        }
        if self
            .input
            .get(self.pos)
            .is_some_and(|c| matches!(c, b'e' | b'E'))
        {
            self.pos += 1;
            if self
                .input
                .get(self.pos)
                .is_some_and(|c| matches!(c, b'+' | b'-'))
            {
                self.pos += 1;
            }
            self.digits()?;
        }
        Ok(())
    }
}

/// Validate all syntax before typed graph conversion. Return unchanged bytes
/// unless Go's string decoder would replace invalid Unicode with RuneError.
pub(super) fn prepare(input: &[u8]) -> Result<Cow<'_, [u8]>, D2Error> {
    let mut scan = Scanner {
        input,
        pos: 0,
        depth: 0,
        output: None,
        copied: 0,
    };
    let mut states = vec![State::End, State::Value];
    while let Some(state) = states.pop() {
        scan.whitespace();
        if matches!(state, State::End) {
            if scan.pos != input.len() {
                return Err(scan.invalid("after top-level value"));
            }
            break;
        }
        let byte = scan.peek()?;
        match state {
            State::Value => match byte {
                b'{' | b'[' => {
                    scan.depth += 1;
                    if scan.depth > 10000 {
                        return Err(error("exceeded max depth"));
                    }
                    scan.pos += 1;
                    states.push(if byte == b'{' {
                        State::ObjectKey(true)
                    } else {
                        State::ArrayValue(true)
                    });
                }
                b'"' => scan.string()?,
                b'n' => scan.literal("null")?,
                b't' => scan.literal("true")?,
                b'f' => scan.literal("false")?,
                b'-' | b'0'..=b'9' => scan.number()?,
                _ => return Err(scan.invalid("looking for beginning of value")),
            },
            State::ObjectKey(allow_end) => {
                if byte == b'}' && allow_end {
                    scan.pos += 1;
                    scan.depth -= 1;
                } else if byte == b'"' {
                    scan.string()?;
                    states.push(State::Colon);
                } else {
                    return Err(scan.invalid("looking for beginning of object key string"));
                }
            }
            State::Colon => {
                if byte != b':' {
                    return Err(scan.invalid("after object key"));
                }
                scan.pos += 1;
                states.extend([State::ObjectEnd, State::Value]);
            }
            State::ObjectEnd => match byte {
                b'}' => {
                    scan.pos += 1;
                    scan.depth -= 1;
                }
                b',' => {
                    scan.pos += 1;
                    states.push(State::ObjectKey(false));
                }
                _ => return Err(scan.invalid("after object key:value pair")),
            },
            State::ArrayValue(allow_end) => {
                if byte == b']' && allow_end {
                    scan.pos += 1;
                    scan.depth -= 1;
                } else {
                    states.extend([State::ArrayEnd, State::Value]);
                }
            }
            State::ArrayEnd => match byte {
                b']' => {
                    scan.pos += 1;
                    scan.depth -= 1;
                }
                b',' => {
                    scan.pos += 1;
                    states.push(State::ArrayValue(false));
                }
                _ => return Err(scan.invalid("after array element")),
            },
            State::End => unreachable!(),
        }
    }
    Ok(match scan.output {
        Some(mut output) => {
            output.extend_from_slice(&input[scan.copied..]);
            Cow::Owned(output)
        }
        None => Cow::Borrowed(input),
    })
}
