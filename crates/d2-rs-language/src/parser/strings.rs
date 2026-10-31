// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Recursive key, string and substitution productions from pinned parse.go.
use super::*;
use d2_rs_core::ast::{
    DoubleQuotedString, InterpolationBox, KeyPath, Node, Shared, StringBox, Substitution,
    UnquotedString,
};

fn literal(value: String, raw: Option<String>) -> InterpolationBox {
    InterpolationBox {
        string: Some(Shared::new(value.into())),
        raw_string: raw.map(Shared::new),
        ..Default::default()
    }
}
fn decode_escape(r: char) -> char {
    match r {
        'a' => '\u{7}',
        'b' => '\u{8}',
        'f' => '\u{c}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\u{b}',
        _ => r,
    }
}
impl<R: RuneReader> Parser<R> {
    /// Parse an import after `@` (or `...@`) has been consumed.
    pub fn parse_import(&mut self, spread: bool) -> d2_rs_core::ast::Import {
        // Upstream subtracts "$" here; its byte/column width equals "@".
        let mut start = self.position.subtract('$', self.utf16_positions);
        if spread {
            for _ in 0..3 {
                start = start.subtract('.', self.utf16_positions);
            }
        }
        let mut pre = String::new();
        while let Some(r) = self.peek() {
            if !matches!(r, '.' | '/') {
                self.rewind();
                break;
            }
            pre.push(r);
            self.commit();
        }
        let mut path = self.parse_key().and_then(|key| key.path);
        if let Some(parts) = &path
            && parts.len() > 1
        {
            let first = parts.get(0).unwrap().read().clone().unwrap();
            let second = parts.get(1).unwrap().read().clone().unwrap();
            let remove = first.read().unquoted_string.is_some()
                && second.read().unquoted_string.is_some()
                && second.read().scalar_bytes() == b"d2";
            if remove {
                let mut entries: Vec<_> = parts.iter().map(|p| p.read().clone()).collect();
                entries.remove(1);
                path = Some(entries.into());
            }
        }
        d2_rs_core::ast::Import {
            range: Range {
                path: self.path.clone(),
                start,
                end: self.position,
            },
            spread,
            pre,
            path,
        }
    }
    pub fn parse_string(&mut self, in_key: bool) -> StringBox {
        let (r, newlines) = self.peek_not_space();
        if r.is_none() || newlines > 0 {
            self.rewind();
            return StringBox::default();
        }
        let r = r.unwrap();
        self.commit();
        match r {
            '"' => StringBox {
                double_quoted_string: Some(Shared::new(self.parse_double_quoted_string(in_key))),
                ..Default::default()
            },
            '\'' => StringBox {
                single_quoted_string: Some(Shared::new(self.parse_single_quoted_string())),
                ..Default::default()
            },
            '|' => StringBox {
                block_string: Some(Shared::new(self.parse_block_string())),
                ..Default::default()
            },
            _ => {
                self.replay(r);
                StringBox {
                    unquoted_string: self.parse_unquoted_string(in_key).map(Shared::new),
                    ..Default::default()
                }
            }
        }
    }
    pub fn parse_key(&mut self) -> Option<KeyPath> {
        let mut range = Range {
            path: self.path.clone(),
            start: self.position,
            ..Default::default()
        };
        let mut path = Vec::new();
        loop {
            let (r, newlines) = self.peek_not_space();
            let Some(r) = r else {
                break;
            };
            if newlines > 0 || r == '(' {
                self.rewind();
                break;
            }
            if r == '.' {
                continue;
            }
            self.rewind();
            let sb = self.parse_string(true);
            let Some(s) = sb.unbox() else {
                break;
            };
            if sb.unquoted_string.is_some() && sb.scalar_bytes().starts_with(b"@") {
                let text = sb.scalar_string();
                self.error(
                    s.range().start,
                    s.range().end,
                    &format!("{text} is not a valid import, did you mean ...{text}?"),
                );
            }
            if path.is_empty() {
                range.start = s.range().start;
            }
            path.push(Some(Shared::new(sb)));
            let (r, newlines) = self.peek_not_space();
            let Some(r) = r else {
                break;
            };
            if newlines > 0 || r != '.' {
                self.rewind();
                break;
            }
            self.commit();
        }
        if path.is_empty() {
            return None;
        }
        range.end = path
            .last()
            .unwrap()
            .as_ref()
            .unwrap()
            .read()
            .unbox()
            .unwrap()
            .range()
            .end;
        for part in &path {
            let length = part.as_ref().unwrap().read().scalar_bytes().len();
            if length > 518 {
                self.error(
                    range.start,
                    range.end,
                    &format!("key length {length} exceeds maximum allowed length of 518"),
                );
                break;
            }
        }
        Some(KeyPath {
            range,
            path: Some(path.into()),
        })
    }
    pub fn parse_substitution(&mut self, spread: bool) -> Option<Substitution> {
        let mut start = self.position.subtract('$', self.utf16_positions);
        if spread {
            for _ in 0..3 {
                start = start.subtract('.', self.utf16_positions);
            }
        }
        let (r, newlines) = self.peek_not_space();
        let r = r?;
        if newlines > 0 {
            self.rewind();
            return None;
        }
        if r != '{' {
            self.rewind();
            self.error(start, self.reader_position, "substitutions must begin on {");
            return None;
        }
        self.commit();
        let path = self.parse_key().and_then(|k| k.path);
        let (r, newlines) = self.peek_not_space();
        if r.is_none() {
            self.error(
                start,
                self.reader_position,
                "substitutions must be terminated by }",
            );
        } else if newlines > 0 || r != Some('}') {
            self.rewind();
            self.error(
                start,
                self.position,
                "substitutions must be terminated by }",
            );
        } else {
            self.commit();
        }
        Some(Substitution {
            range: Range {
                path: self.path.clone(),
                start,
                end: self.position,
            },
            spread,
            path,
        })
    }
    pub fn parse_double_quoted_string(&mut self, in_key: bool) -> DoubleQuotedString {
        let start = self.position.subtract('"', self.utf16_positions);
        let mut value = Vec::new();
        let mut decoded = String::new();
        let mut raw = String::new();
        loop {
            let Some(r) = self.peek() else {
                self.error(
                    start,
                    self.reader_position,
                    "double quoted strings must be terminated with \"",
                );
                break;
            };
            if r == '\n' {
                self.rewind();
                self.error(
                    start,
                    self.position,
                    "double quoted strings must be terminated with \"",
                );
                break;
            }
            self.commit();
            if !in_key
                && r == '$'
                && let Some(sub) = self.parse_substitution(false)
            {
                if !decoded.is_empty() {
                    value.push(literal(std::mem::take(&mut decoded), None));
                }
                // Upstream does not clear the raw builder at a substitution.
                value.push(InterpolationBox {
                    substitution: Some(Shared::new(sub)),
                    ..Default::default()
                });
                continue;
            }
            if r == '"' {
                break;
            }
            if r != '\\' {
                decoded.push(r);
                raw.push(r);
                continue;
            }
            let Some(r2) = self.read() else {
                self.error(
                    self.position.subtract('\\', self.utf16_positions),
                    self.reader_position,
                    "unfinished escape sequence",
                );
                self.error(
                    start,
                    self.reader_position,
                    "double quoted strings must be terminated with \"",
                );
                break;
            };
            if r2 == '\n' {
                continue;
            }
            decoded.push(decode_escape(r2));
            raw.push('\\');
            raw.push(r2);
        }
        if !decoded.is_empty() {
            value.push(literal(decoded, Some(raw)));
        }
        DoubleQuotedString {
            range: Range {
                path: self.path.clone(),
                start,
                end: self.position,
            },
            value: if value.is_empty() {
                None
            } else {
                Some(value.into())
            },
        }
    }
    pub fn parse_unquoted_string(&mut self, in_key: bool) -> Option<UnquotedString> {
        let start = self.position;
        let mut last_non_space = start;
        let mut decoded = String::new();
        let mut raw = String::new();
        let mut has_raw = false;
        let mut values = Vec::new();
        let mut pattern: Option<Vec<d2_rs_core::ast::ByteString>> = None;
        let mut last_pattern = 0;
        let mut import_spread = true;
        for want in "...@".chars() {
            let Some(r) = self.peek() else {
                import_spread = false;
                break;
            };
            if r != want {
                import_spread = false;
            }
        }
        self.rewind();
        if import_spread {
            let end = "...@"
                .chars()
                .fold(self.position, |p, r| p.advance(r, self.utf16_positions));
            self.error(
                self.position,
                end,
                "unquoted strings cannot begin with ...@ as that's import spread syntax",
            );
        }
        while let Some(mut r) = self.peek() {
            if self.in_edge_group && r == ')' {
                let (r2, newlines) = self.peek_not_space();
                if r2.is_none()
                    || newlines > 0
                    || matches!(r2, Some('\n' | '#' | '{' | '}' | '[' | ']' | ':' | '.'))
                {
                    self.rewind();
                    break;
                }
                self.rewind();
                self.peek();
                self.commit();
                last_non_space = self.position;
                decoded.push(r);
                if has_raw {
                    raw.push(r);
                }
                continue;
            }
            if matches!(r, '\n' | ';' | '#' | '{' | '}' | '[' | ']') {
                self.rewind();
                break;
            }
            if in_key {
                if matches!(r, ':' | '.' | '<' | '>' | '&') {
                    self.rewind();
                    break;
                }
                if r == '-' {
                    let Some(r2) = self.peek() else {
                        break;
                    };
                    if matches!(r2, '\n' | ';' | '#' | '{' | '}' | '[' | ']') {
                        self.rewind();
                        self.peek();
                        self.commit();
                        decoded.push(r);
                        if has_raw {
                            raw.push(r);
                        }
                        break;
                    }
                    if matches!(r2, '-' | '>' | '*') {
                        self.rewind();
                        break;
                    }
                    decoded.push(r);
                    if has_raw {
                        raw.push(r);
                    }
                    r = r2;
                }
            }
            if r == '*' {
                let pieces = pattern.get_or_insert_default();
                if !decoded.is_empty() {
                    pieces.push(decoded.as_bytes()[last_pattern..].to_vec().into());
                }
                pieces.push("*".into());
                last_pattern = decoded.len() + 1;
            }
            self.commit();
            if !is_space(r) {
                last_non_space = self.position;
            }
            if !in_key && r == '$' {
                if let Some(sub) = self.parse_substitution(false) {
                    if !decoded.is_empty() {
                        let spelling = if has_raw {
                            std::mem::take(&mut raw)
                        } else {
                            decoded.clone()
                        };
                        values.push(literal(std::mem::take(&mut decoded), Some(spelling)));
                        raw.clear();
                        has_raw = false;
                    }
                    values.push(InterpolationBox {
                        substitution: Some(Shared::new(sub)),
                        ..Default::default()
                    });
                }
                continue;
            }
            if r != '\\' {
                decoded.push(r);
                if has_raw {
                    raw.push(r);
                }
                continue;
            }
            let Some(r2) = self.read() else {
                self.error(
                    self.position.subtract('\\', self.utf16_positions),
                    self.reader_position,
                    "unfinished escape sequence",
                );
                break;
            };
            if r2 == '\n' {
                let (r, newlines) = self.peek_not_space();
                if r.is_none() || newlines > 0 {
                    self.rewind();
                    break;
                }
                self.commit();
                self.replay(r.unwrap());
                continue;
            }
            if !has_raw {
                raw.push_str(&decoded);
                has_raw = true;
            }
            decoded.push(decode_escape(r2));
            raw.push('\\');
            raw.push(r2);
        }
        let decoded = decoded.trim_end_matches(is_space).to_owned();
        let spelling = if has_raw {
            raw.trim_end_matches(is_space).to_owned()
        } else {
            decoded.clone()
        };
        if let Some(pattern) = &mut pattern
            && last_pattern < decoded.len()
        {
            pattern.push(decoded.as_bytes()[last_pattern..].to_vec().into());
        }
        if decoded.is_empty() && values.is_empty() {
            return None;
        }
        if !decoded.is_empty() {
            values.push(literal(decoded, Some(spelling)));
        }
        Some(UnquotedString {
            range: Range {
                path: self.path.clone(),
                start,
                end: last_non_space,
            },
            value: Some(values.into()),
            pattern: pattern.map(Into::into),
        })
    }
}
