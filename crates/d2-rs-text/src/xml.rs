// Copyright 2009 The Go Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause
//! Reachable Go 1.27 encoding/xml.Unmarshal into interface{} validation.
//! Errors expose SyntaxError.Msg, as D2's compileLabel/tooltip do. This is not
//! a replacement XML DOM or SVG importer; the first root's end stops Unmarshal.
use crate::go_xml_names;

struct Reader<'a> {
    source: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.source.get(self.at).copied()
    }
    fn take(&mut self, text: &str) -> bool {
        if self.source[self.at..].starts_with(text.as_bytes()) {
            self.at += text.len();
            true
        } else {
            false
        }
    }
    fn byte(&mut self) -> Result<u8, String> {
        let c = self.peek().ok_or("unexpected EOF")?;
        self.at += 1;
        Ok(c)
    }
    fn space(&mut self) {
        while self
            .peek()
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.at += 1;
        }
    }
    fn name(&mut self, missing: &str, qualified: bool) -> Result<String, String> {
        let start = self.at;
        let is_name =
            |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.' | b'-') || c >= 128;
        self.peek().ok_or("unexpected EOF")?;
        while self.peek().is_some_and(is_name) {
            self.at += 1;
        }
        if self.at == self.source.len() {
            return Err("unexpected EOF".into());
        }
        if self.at == start {
            return Err(missing.into());
        }
        let name = crate::go_quote::text(&self.source[start..self.at]);
        if std::str::from_utf8(&self.source[start..self.at]).is_err() || !go_xml_names::valid(&name)
        {
            return Err(format!("invalid XML name: {name}"));
        }
        if qualified && name.bytes().filter(|b| *b == b':').count() > 1 {
            return Err(missing.into());
        }
        Ok(name)
    }
    fn entity(&mut self) -> Result<char, String> {
        let start = self.at;
        self.at += 1;
        let numeric = self.take("#");
        let hex = numeric && self.take("x");
        let digits = self.at;
        if numeric {
            while self.peek().is_some_and(|c| {
                if hex {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_digit()
                }
            }) {
                self.at += 1;
            }
        } else {
            // readName permits an absent name but rejects invalid name runes.
            while self.peek().is_some_and(|c| {
                c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.' | b'-') || c >= 128
            }) {
                self.at += 1;
            }
        }
        self.peek().ok_or("unexpected EOF")?;
        let end = self.at;
        let semicolon = self.take(";");
        let token = crate::go_quote::text(&self.source[start..self.at]);
        let value = if semicolon {
            if numeric {
                u64::from_str_radix(
                    std::str::from_utf8(&self.source[digits..end]).unwrap_or(""),
                    if hex { 16 } else { 10 },
                )
                .ok()
                .filter(|n| *n <= 0x10ffff)
                .map(|n| char::from_u32(n as u32).unwrap_or('\u{fffd}'))
            } else {
                match std::str::from_utf8(&self.source[digits..end]).unwrap_or("") {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "apos" => Some('\''),
                    "quot" => Some('"'),
                    _ => None,
                }
            }
        } else {
            None
        };
        value.ok_or_else(|| {
            format!(
                "invalid character entity {token}{}",
                if semicolon { "" } else { " (no semicolon)" }
            )
        })
    }
    fn text(&mut self, quote: Option<u8>, cdata: bool) -> Result<(), String> {
        let mut decoded = Vec::new();
        loop {
            if cdata && self.take("]]>") {
                break;
            }
            let Some(c) = self.peek() else {
                if cdata {
                    return Err("unexpected EOF in CDATA section".into());
                }
                break;
            };
            if quote == Some(c) {
                self.at += 1;
                break;
            }
            if !cdata && quote.is_none() && self.source[self.at..].starts_with(b"]]>") {
                return Err("unescaped ]]> not in CDATA section".into());
            }
            if !cdata && c == b'<' {
                if quote.is_some() {
                    return Err("unescaped < inside quoted string".into());
                }
                break;
            }
            if !cdata && c == b'&' {
                decoded.extend_from_slice(self.entity()?.encode_utf8(&mut [0; 4]).as_bytes());
            } else {
                decoded.push(c);
                self.at += 1;
            }
        }
        // Go checks character ranges only after the complete text token; a
        // later lexical failure therefore precedes an earlier illegal rune.
        for ch in std::str::from_utf8(&decoded)
            .map_err(|_| "invalid UTF-8".to_owned())?
            .chars()
        {
            if !matches!(ch as u32,9|10|13|0x20..=0xd7ff|0xe000..=0xfffd|0x10000..=0x10ffff) {
                return Err(format!("illegal character code U+{:04X}", ch as u32));
            }
        }
        Ok(())
    }
    fn comment(&mut self) -> Result<(), String> {
        loop {
            if self.take("--") {
                return if self.byte()? == b'>' {
                    Ok(())
                } else {
                    Err("invalid sequence \"--\" not allowed in comments".into())
                };
            }
            self.byte()?;
        }
    }
    fn directive(&mut self) -> Result<(), String> {
        let mut depth = 0i32;
        let mut quote = None;
        loop {
            if quote.is_none() && self.take("<!--") {
                let end = self.source[self.at..]
                    .windows(3)
                    .position(|w| w == b"-->")
                    .ok_or("unexpected EOF")?;
                self.at += end + 3;
                continue;
            }
            let c = self.byte()?;
            if quote == Some(c) {
                quote = None;
                continue;
            }
            if quote.is_some() {
                continue;
            }
            match c {
                b'\'' | b'"' => quote = Some(c),
                b'<' => depth += 1,
                b'>' if depth == 0 => return Ok(()),
                b'>' => depth -= 1,
                _ => {}
            }
        }
    }
}

fn split_name(name: &str) -> (&str, &str) {
    name.split_once(':')
        .filter(|(prefix, local)| !prefix.is_empty() && !local.is_empty())
        .unwrap_or(("", name))
}
fn instruction_parameter<'a>(name: &str, content: &'a str) -> &'a str {
    let key = format!("{name}=");
    let mut at = 0;
    while let Some(found) = content[at..].find(&key) {
        at += found + key.len();
        let Some(quote) = content.as_bytes().get(at).copied() else {
            return "";
        };
        at += 1;
        if matches!(quote, b'\'' | b'"') {
            return content[at..]
                .find(quote as char)
                .map_or("", |end| &content[at..at + end]);
        }
    }
    ""
}

/// Validate one wrapped Markdown HTML value using Go's strict lexical/stack
/// semantics. Attribute duplicates and undeclared prefixes remain admissible.
pub fn validate_markdown_html(html: &str) -> Result<(), String> {
    validate_markdown_html_bytes(html.as_bytes())
}
/// Preserve raw Go strings until complete XML text tokens are validated;
/// comments and directives intentionally do not apply text-token checks.
pub fn validate_markdown_html_bytes(html: &[u8]) -> Result<(), String> {
    let mut wrapped = Vec::with_capacity(html.len() + 11);
    wrapped.extend_from_slice(b"<div>");
    wrapped.extend_from_slice(html);
    wrapped.extend_from_slice(b"</div>");
    validate_xml_value_bytes(&wrapped)
}

#[doc(hidden)]
pub fn validate_xml_value_bytes(source: &[u8]) -> Result<(), String> {
    let mut r = Reader { source, at: 0 };
    let mut stack: Vec<String> = Vec::new();
    while r.at < source.len() {
        if !r.take("<") {
            r.text(None, false)?;
            continue;
        }
        match r.byte()? {
            b'/' => {
                let name = r.name("expected element name after </", true)?;
                r.space();
                if r.byte()? != b'>' {
                    return Err(format!(
                        "invalid characters between </{} and >",
                        split_name(&name).1
                    ));
                }
                let Some(open) = stack.pop() else {
                    return Err(format!("unexpected end element </{}>", split_name(&name).1));
                };
                let (prefix, local) = split_name(&name);
                let (old_prefix, old_local) = split_name(&open);
                if local != old_local {
                    return Err(format!("element <{old_local}> closed by </{local}>"));
                }
                if prefix != old_prefix {
                    return Err(format!(
                        "element <{old_local}> in space {old_prefix} closed by </{local}> in space {}",
                        if prefix.is_empty() { "\"\"" } else { prefix }
                    ));
                }
                if stack.is_empty() {
                    return Ok(());
                }
            }
            b'?' => {
                let target = r.name("expected target name after <?", false)?;
                r.space();
                let end = r.source[r.at..]
                    .windows(2)
                    .position(|w| w == b"?>")
                    .ok_or("unexpected EOF")?;
                let content = crate::go_quote::text(&r.source[r.at..r.at + end]);
                r.at += end + 2;
                if target == "xml" {
                    let version = instruction_parameter("version", &content);
                    if !version.is_empty() && version != "1.0" {
                        return Err(format!(
                            "xml: unsupported version {}; only version 1.0 is supported",
                            crate::go_quote::quote(version.as_bytes())
                        ));
                    }
                    let encoding = instruction_parameter("encoding", &content);
                    if !encoding.is_empty() && !encoding.eq_ignore_ascii_case("utf-8") {
                        return Err(format!(
                            "xml: encoding {} declared but Decoder.CharsetReader is nil",
                            crate::go_quote::quote(encoding.as_bytes())
                        ));
                    }
                }
            }
            b'!' => match r.byte()? {
                b'-' => {
                    if r.byte()? != b'-' {
                        return Err("invalid sequence <!- not part of <!--".into());
                    }
                    r.comment()?;
                }
                b'[' => {
                    for wanted in b"CDATA[" {
                        if r.byte()? != *wanted {
                            return Err("invalid <![ sequence".into());
                        }
                    }
                    r.text(None, true)?;
                }
                _ => r.directive()?,
            },
            _ => {
                r.at -= 1;
                let name = r.name("expected element name after <", true)?;
                let empty;
                loop {
                    r.space();
                    match r.byte()? {
                        b'>' => {
                            empty = false;
                            break;
                        }
                        b'/' => {
                            if r.byte()? != b'>' {
                                return Err("expected /> in element".into());
                            }
                            empty = true;
                            break;
                        }
                        _ => {
                            r.at -= 1;
                            r.name("expected attribute name in element", true)?;
                            r.space();
                            if r.byte()? != b'=' {
                                return Err("attribute name without = in element".into());
                            }
                            r.space();
                            let quote = r.byte()?;
                            if !matches!(quote, b'\'' | b'"') {
                                return Err("unquoted or missing attribute value in element".into());
                            }
                            r.text(Some(quote), false)?;
                        }
                    }
                }
                if !empty {
                    stack.push(name);
                } else if stack.is_empty() {
                    return Ok(());
                }
            }
        }
    }
    Err("unexpected EOF".into())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_compiler_xml_contract() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/compiler-markdown-xml-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 121);
        for row in rows.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let html = row["html"].as_str().unwrap();
            if row["markdown"].as_bool().unwrap() {
                assert_eq!(
                    crate::markdown::html::render_html(source.as_bytes()),
                    html,
                    "{} projection",
                    row["name"]
                );
            }
            assert_eq!(
                validate_markdown_html(html).err().unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{}",
                row["name"]
            );
        }
    }
}
