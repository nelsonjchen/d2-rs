// SPDX-License-Identifier: MPL-2.0
// The subset of Go encoding/xml's strict Decoder.Token behavior that d2svg's
// font-corpus and theme-pruning scans depend on: namespace translation,
// unescaped character data (including CDATA), matched end tags, and errors
// where Go reports them (bad entities, unmatched tags, malformed attributes,
// illegal characters).

use std::collections::HashMap;

pub(crate) const XML_URL: &str = "http://www.w3.org/XML/1998/namespace";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Name {
    pub space: String,
    pub local: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Attr {
    pub name: Name,
    pub value: String,
}

#[derive(Clone, Debug)]
pub(crate) enum Token {
    Start { name: Name, attrs: Vec<Attr> },
    End { name: Name },
    CharData(String),
    Comment,
    ProcInst,
    Directive,
}

/// Tokenizes a complete document; `None` is any decoder error before EOF.
pub(crate) fn tokens(source: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let mut stack: Vec<(String, HashMap<String, String>)> = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Token>| {
        if !text.is_empty() {
            out.push(Token::CharData(std::mem::take(text)));
        }
    };
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = source[i..].find('<').map_or(bytes.len(), |n| n + i);
            text.push_str(&unescape(&source[i..end])?);
            i = end;
            continue;
        }
        let rest = &source[i..];
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>")?;
            check_chars(&after[..end])?;
            text.push_str(&after[..end]);
            i += 9 + end + 3;
            continue;
        }
        flush(&mut text, &mut out);
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->")?;
            out.push(Token::Comment);
            i += 4 + end + 3;
            continue;
        }
        if let Some(after) = rest.strip_prefix("<?") {
            let end = after.find("?>")?;
            out.push(Token::ProcInst);
            i += 2 + end + 2;
            continue;
        }
        if rest.starts_with("<!") {
            let end = rest.find('>')?;
            out.push(Token::Directive);
            i += end + 1;
            continue;
        }
        if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>')?;
            let raw = after[..end].trim_end();
            let (open, namespaces) = stack.pop()?;
            if open != raw {
                return None;
            }
            out.push(Token::End {
                name: translate(raw, &namespaces, true),
            });
            i += 2 + end + 1;
            continue;
        }
        let mut j = i + 1;
        let name_start = j;
        while j < bytes.len() && is_name_byte(bytes[j]) {
            j += 1;
        }
        if j == name_start {
            return None;
        }
        let raw_name = &source[name_start..j];
        let mut raw_attrs = Vec::new();
        let self_closing;
        loop {
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            match bytes.get(j)? {
                b'>' => {
                    self_closing = false;
                    j += 1;
                    break;
                }
                b'/' => {
                    if bytes.get(j + 1) != Some(&b'>') {
                        return None;
                    }
                    self_closing = true;
                    j += 2;
                    break;
                }
                _ => {}
            }
            let attr_start = j;
            while j < bytes.len() && is_name_byte(bytes[j]) {
                j += 1;
            }
            if j == attr_start {
                return None;
            }
            let attr_name = &source[attr_start..j];
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) != Some(&b'=') {
                return None;
            }
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let quote = *bytes.get(j)?;
            if quote != b'"' && quote != b'\'' {
                return None;
            }
            let value_start = j + 1;
            let value_end = source[value_start..].find(quote as char)? + value_start;
            let raw = &source[value_start..value_end];
            if raw.contains('<') {
                return None;
            }
            raw_attrs.push((attr_name, unescape(raw)?));
            j = value_end + 1;
        }
        let mut namespaces = stack.last().map(|(_, ns)| ns.clone()).unwrap_or_default();
        for (name, value) in &raw_attrs {
            if *name == "xmlns" {
                namespaces.insert(String::new(), value.clone());
            } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                namespaces.insert(prefix.to_owned(), value.clone());
            }
        }
        let name = translate(raw_name, &namespaces, true);
        let attrs = raw_attrs
            .into_iter()
            .map(|(attr, value)| Attr {
                name: translate(attr, &namespaces, false),
                value,
            })
            .collect();
        out.push(Token::Start {
            name: name.clone(),
            attrs,
        });
        if self_closing {
            out.push(Token::End { name });
        } else {
            stack.push((raw_name.to_owned(), namespaces));
        }
        i = j;
    }
    flush(&mut text, &mut out);
    if !stack.is_empty() {
        return None;
    }
    Some(out)
}

/// encoding/xml Decoder.translate.
fn translate(raw: &str, namespaces: &HashMap<String, String>, element: bool) -> Name {
    let (prefix, local) = match raw.split_once(':') {
        Some((prefix, local)) if !prefix.is_empty() && !local.is_empty() => (prefix, local),
        _ => ("", raw),
    };
    let space = match prefix {
        "xmlns" => "xmlns".to_owned(),
        "xml" => XML_URL.to_owned(),
        "" if !element => String::new(),
        _ => namespaces
            .get(prefix)
            .cloned()
            .unwrap_or_else(|| prefix.to_owned()),
    };
    if raw == "xmlns" {
        return Name {
            space: String::new(),
            local: "xmlns".into(),
        };
    }
    Name {
        space,
        local: local.to_owned(),
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':') || b >= 0x80
}

fn check_chars(text: &str) -> Option<()> {
    text.chars()
        .all(|c| {
            let r = c as u32;
            r == 0x09
                || r == 0x0A
                || r == 0x0D
                || (0x20..=0xD7FF).contains(&r)
                || (0xE000..=0xFFFD).contains(&r)
                || r >= 0x10000
        })
        .then_some(())
}

/// Character data / attribute unescaping (strict entities; `\r\n` → `\n`).
fn unescape(raw: &str) -> Option<String> {
    check_chars(raw)?;
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let end = rest[index..].find(';')? + index;
        let entity = &rest[index + 1..end];
        let c = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "apos" => '\'',
            "quot" => '"',
            _ => {
                let number = entity.strip_prefix('#')?;
                let code = if let Some(hex) = number.strip_prefix('x') {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    number.parse().ok()?
                };
                char::from_u32(code)?
            }
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out.replace("\r\n", "\n").replace('\r', "\n"))
}
