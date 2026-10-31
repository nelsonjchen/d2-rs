// Copyright 2009 The Go Authors. All rights reserved.
// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
//! Native reachable Go 1.27 encoding/xml lexical/stack contracts, with D2
//! imageasset:validateSVGWithLimits root admission. Budgets/charset byte
//! transport and full importer token construction remain separate stages.
use super::{go_xml_names, svg_import_envelope};
struct Reader<'a> {
    source: &'a str,
    at: usize,
}
impl<'a> Reader<'a> {
    fn bytes(&self) -> &'a [u8] {
        self.source.as_bytes()
    }
    fn skip(&mut self) {
        while self
            .bytes()
            .get(self.at)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, s: &str) -> bool {
        if self.bytes()[self.at..].starts_with(s.as_bytes()) {
            self.at += s.len();
            true
        } else {
            false
        }
    }
    fn name(&mut self, qualified: bool) -> Option<&'a str> {
        let start = self.at;
        while self.bytes().get(self.at).is_some_and(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.' | b'-') || *c >= 128
        }) {
            self.at += 1;
        }
        let name = &self.source[start..self.at];
        (go_xml_names::valid(name)
            && (!qualified || name.bytes().filter(|c| *c == b':').count() <= 1))
            .then_some(name)
    }
    fn entity(&mut self) -> Option<char> {
        let start = self.at;
        let end = self.source[start..].find(';')?;
        let name = &self.source[start..start + end];
        self.at += end + 1;
        let character = match name {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "apos" => '\'',
            "quot" => '"',
            _ => {
                let value = if let Some(hex) = name.strip_prefix("#x") {
                    if hex.is_empty() || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                        return None;
                    }
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    let decimal = name.strip_prefix('#')?;
                    if decimal.is_empty() || !decimal.bytes().all(|c| c.is_ascii_digit()) {
                        return None;
                    }
                    decimal.parse::<u32>().ok()?
                };
                if value > 0x10ffff {
                    return None;
                }
                // Go utf8.EncodeRune substitutes RuneError for surrogate
                // numeric references before XML character-range validation.
                char::from_u32(value).unwrap_or('\u{fffd}')
            }
        };
        xml_character(character).then_some(character)
    }
    fn text(&mut self, quote: Option<u8>, cdata: bool) -> Option<String> {
        let mut result = String::new();
        while self.at < self.source.len() {
            if cdata && self.bytes()[self.at..].starts_with(b"]]>") {
                self.at += 3;
                return Some(result);
            }
            let c = self.bytes()[self.at];
            if quote == Some(c) {
                self.at += 1;
                return Some(result);
            }
            if !cdata && c == b'<' {
                return quote.is_none().then_some(result);
            }
            if !cdata && quote.is_none() && self.bytes()[self.at..].starts_with(b"]]>") {
                return None;
            }
            if !cdata && c == b'&' {
                self.at += 1;
                result.push(self.entity()?);
            } else {
                let character = self.source[self.at..].chars().next()?;
                self.at += character.len_utf8();
                if !xml_character(character) {
                    return None;
                }
                if character == '\r' {
                    result.push('\n');
                    if self.bytes().get(self.at) == Some(&b'\n') {
                        self.at += 1;
                    }
                } else {
                    result.push(character);
                }
            }
        }
        (quote.is_none() && !cdata).then_some(result)
    }
}
fn xml_character(c: char) -> bool {
    matches!(c as u32, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}
/// Error offset selects precedence only; asset error bodies have no byte offset.
/// The original raw XML remains authoritative; namespace declarations and
/// duplicate attributes retain Go admission rather than native-parser policy.
pub fn validate(source: &str) -> Result<(), String> {
    validate_impl(source, None, None)
}

/// imageasset's derived XML work bounds, independent of SVG importer schema.
pub fn validate_bounded(
    source: &str,
    max_bytes: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    validate_impl(source, Some(max_bytes), cancel)
}

fn validate_impl(
    source: &str,
    max_bytes: Option<usize>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let envelope = svg_import_envelope::scan(source);
    let mut envelope_errors = envelope.asset_errors.into_iter().peekable();
    let mut reader = Reader { source, at: 0 };
    let mut stack: Vec<&str> = Vec::new();
    let mut seen_root = false;
    let mut tokens = 0;
    let mut elements = 0;
    let mut attributes = 0;
    let mut attribute_bytes = 0;
    let mut namespaces = std::collections::HashMap::new();
    let mut namespace_stack = Vec::new();
    let check = |name: &str, actual: usize, ceiling: usize| {
        if actual > ceiling {
            Err(format!("{name} {actual} exceeds limit {ceiling}"))
        } else {
            Ok(())
        }
    };
    let malformed = || "malformed SVG: malformed SVG XML".to_string();
    while reader.at < source.len() {
        super::context(cancel)?;
        tokens += 1;
        if let Some(max_bytes) = max_bytes {
            check("SVG XML tokens", tokens, max_bytes / 2 + 2)?;
        }
        let start = reader.at;
        // Decoder lexical errors on this token precede its asset admission.
        // Once the token is complete, deferred envelope failures run in order.
        if reader.take("<!--") {
            let end = source[reader.at..].find("-->").ok_or_else(malformed)?;
            if source[reader.at..reader.at + end].contains("--")
                || source[reader.at..reader.at + end].ends_with('-')
            {
                return Err(malformed());
            }
            reader.at += end + 3;
        } else if reader.take("<![CDATA[") {
            let text = reader.text(None, true).ok_or_else(malformed)?;
            if stack.is_empty() && !text.trim().is_empty() {
                return Err("malformed SVG: text outside svg root element".into());
            }
        } else if reader.take("<?") {
            reader.name(false).ok_or_else(malformed)?;
            reader.skip();
            let end = source[reader.at..].find("?>").ok_or_else(malformed)?;
            reader.at += end + 2;
        } else if reader.bytes()[reader.at..].starts_with(b"<!-")
            || reader.bytes()[reader.at..].starts_with(b"<![")
        {
            return Err(malformed());
        } else if reader.bytes()[reader.at..].starts_with(b"<!") {
            reader.at =
                svg_import_envelope::markup_end(source, reader.at, true).ok_or_else(malformed)?;
        } else if reader.take("</") {
            let name = reader.name(true).ok_or_else(malformed)?;
            reader.skip();
            if !reader.take(">") || stack.pop() != Some(name) {
                return Err(malformed());
            }
            if max_bytes.is_some() {
                namespaces = namespace_stack.pop().unwrap_or_default();
            }
        } else if reader.take("<") {
            let name = reader.name(true).ok_or_else(malformed)?;
            let mut attrs = Vec::new();
            let empty;
            loop {
                reader.skip();
                if reader.take("/>") {
                    empty = true;
                    break;
                }
                if reader.take(">") {
                    empty = false;
                    break;
                }
                let attribute = reader.name(true).ok_or_else(malformed)?;
                reader.skip();
                if !reader.take("=") {
                    return Err(malformed());
                }
                reader.skip();
                let quote = *reader.bytes().get(reader.at).ok_or_else(malformed)?;
                if !matches!(quote, b'\'' | b'"') {
                    return Err(malformed());
                }
                reader.at += 1;
                let value = reader.text(Some(quote), false).ok_or_else(malformed)?;
                attrs.push((attribute, value));
            }
            let previous = if let Some(max_bytes) = max_bytes {
                elements += 1;
                check("SVG elements", elements, max_bytes / 3 + 1)?;
                attributes += attrs.len();
                check("SVG attributes", attributes, max_bytes / 3 + 1)?;
                let previous = namespaces.clone();
                for (name, value) in &attrs {
                    if *name == "xmlns" {
                        namespaces.insert(String::new(), value.clone());
                    } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                        namespaces.insert(prefix.into(), value.clone());
                    }
                }
                for (name, value) in &attrs {
                    let (namespace, local) = expanded(name, &namespaces, false);
                    attribute_bytes += namespace.len() + local.len() + value.len();
                    check("SVG attribute bytes", attribute_bytes, max_bytes)?;
                }
                Some(previous)
            } else {
                None
            };
            if let Some((at, _)) = envelope_errors.peek() {
                if *at <= start {
                    return Err(envelope_errors.next().unwrap().1);
                }
            }
            if stack.is_empty() {
                if seen_root {
                    return Err("malformed SVG: multiple root elements".into());
                }
                let (prefix, local) = name.split_once(':').unwrap_or(("", name));
                if local != "svg" {
                    return Err("malformed SVG: root element is not svg".into());
                }
                let namespace = if prefix == "xml" {
                    "http://www.w3.org/XML/1998/namespace"
                } else {
                    attrs
                        .iter()
                        .rev()
                        .find(|(key, _)| {
                            if prefix.is_empty() {
                                *key == "xmlns"
                            } else {
                                key.strip_prefix("xmlns:") == Some(prefix)
                            }
                        })
                        .map_or(prefix, |(_, value)| value.as_str())
                };
                if !matches!(namespace, "" | "http://www.w3.org/2000/svg") {
                    return Err("malformed SVG: svg root has an unexpected namespace".into());
                }
                seen_root = true;
            }
            if max_bytes.is_some() {
                check("SVG XML depth", stack.len() + 1, 256)?;
                if empty {
                    tokens += 1;
                    check("SVG XML tokens", tokens, max_bytes.unwrap() / 2 + 2)?;
                    namespaces = previous.unwrap();
                } else {
                    namespace_stack.push(previous.unwrap());
                }
            }
            if !empty {
                stack.push(name);
            }
        } else {
            let text = reader.text(None, false).ok_or_else(malformed)?;
            if stack.is_empty() && !text.trim().is_empty() {
                return Err("malformed SVG: text outside svg root element".into());
            }
        }
        if let Some((at, _)) = envelope_errors.peek() {
            if *at <= start {
                return Err(envelope_errors.next().unwrap().1);
            }
        }
    }
    super::context(cancel)?;
    if !seen_root {
        return Err("malformed SVG: missing or unclosed svg root element".into());
    }
    if !stack.is_empty() {
        return Err(malformed());
    }
    if let Some((_, error)) = envelope_errors.next() {
        return Err(error);
    }
    Ok(())
}

#[derive(Debug)]
pub struct Attribute {
    pub namespace: String,
    pub name: String,
    pub value: String,
}
#[derive(Debug)]
pub enum Event {
    Start {
        namespace: String,
        name: String,
        attributes: Vec<Attribute>,
        end: usize,
    },
    End,
    Text {
        value: String,
        end: usize,
    },
    Comment,
    Envelope {
        start: usize,
    },
}
// Source encoding/xml:Token applies every declaration before translating the
// element/attribute names. Last declarations win in the decoder; duplicate
// expanded names are checked later by d2svgimport:attributes, in source order.
fn expanded(
    name: &str,
    namespaces: &std::collections::HashMap<String, String>,
    element: bool,
) -> (String, String) {
    let (prefix, local) = name.split_once(':').unwrap_or(("", name));
    let namespace = match prefix {
        "xmlns" => "xmlns".into(),
        "xml" => "http://www.w3.org/XML/1998/namespace".into(),
        "" if !element || local == "xmlns" => String::new(),
        _ => namespaces
            .get(prefix)
            .cloned()
            .unwrap_or_else(|| prefix.into()),
    };
    (namespace, local.into())
}
pub fn events(source: &str) -> Option<Vec<Event>> {
    events_with_separators(source, None)
}
/// Renderer-only lexical transport for Source-accepted adjacent attributes.
/// Keep original bytes for importer admission and diagnostics. Attribute
/// values, names, namespace declarations and every other token stay intact.
pub fn renderer_attribute_separators(source: &str) -> std::borrow::Cow<'_, str> {
    let mut positions = Vec::new();
    if events_with_separators(source, Some(&mut positions)).is_none() || positions.is_empty() {
        return std::borrow::Cow::Borrowed(source);
    }
    let mut rendered = source.to_owned();
    for position in positions.into_iter().rev() {
        rendered.insert(position, ' ');
    }
    std::borrow::Cow::Owned(rendered)
}
fn events_with_separators(
    source: &str,
    mut separators: Option<&mut Vec<usize>>,
) -> Option<Vec<Event>> {
    let mut reader = Reader { source, at: 0 };
    let mut result = Vec::new();
    let mut namespaces = std::collections::HashMap::new();
    let mut stack = Vec::new();
    while reader.at < source.len() {
        let start = reader.at;
        if reader.take("<!--") {
            let end = source[reader.at..].find("-->")?;
            let text = &source[reader.at..reader.at + end];
            if text.contains("--") || text.ends_with('-') {
                return None;
            }
            reader.at += end + 3;
            result.push(Event::Comment);
        } else if reader.take("<![CDATA[") {
            let value = reader.text(None, true)?;
            result.push(Event::Text {
                value,
                end: reader.at,
            });
        } else if reader.take("<?") {
            reader.name(false)?;
            reader.skip();
            let end = source[reader.at..].find("?>")?;
            reader.at += end + 2;
            result.push(Event::Envelope { start });
        } else if reader.bytes()[reader.at..].starts_with(b"<!-")
            || reader.bytes()[reader.at..].starts_with(b"<![")
        {
            return None;
        } else if reader.bytes()[reader.at..].starts_with(b"<!") {
            reader.at = svg_import_envelope::markup_end(source, reader.at, true)?;
            result.push(Event::Envelope { start });
        } else if reader.take("</") {
            let name = reader.name(true)?;
            reader.skip();
            if !reader.take(">") {
                return None;
            }
            let (previous, original) = stack.pop()?;
            if original != name {
                return None;
            }
            namespaces = previous;
            result.push(Event::End);
        } else if reader.take("<") {
            let name = reader.name(true)?;
            let mut raw_attributes = Vec::new();
            let empty;
            loop {
                let before_space = reader.at;
                reader.skip();
                if reader.take("/>") {
                    empty = true;
                    break;
                }
                if reader.take(">") {
                    empty = false;
                    break;
                }
                if !raw_attributes.is_empty() && before_space == reader.at {
                    if let Some(positions) = separators.as_mut() {
                        positions.push(reader.at);
                    }
                }
                let attribute = reader.name(true)?;
                reader.skip();
                if !reader.take("=") {
                    return None;
                }
                reader.skip();
                let quote = *reader.bytes().get(reader.at)?;
                if !matches!(quote, b'\'' | b'"') {
                    return None;
                }
                reader.at += 1;
                let value = reader.text(Some(quote), false)?;
                raw_attributes.push((attribute, value));
            }
            let previous = namespaces.clone();
            for (name, value) in &raw_attributes {
                if *name == "xmlns" {
                    namespaces.insert(String::new(), value.clone());
                } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                    namespaces.insert(prefix.into(), value.clone());
                }
            }
            let (namespace, local) = expanded(name, &namespaces, true);
            let attributes = raw_attributes
                .into_iter()
                .map(|(name, value)| {
                    let (namespace, name) = expanded(name, &namespaces, false);
                    Attribute {
                        namespace,
                        name,
                        value,
                    }
                })
                .collect();
            result.push(Event::Start {
                namespace,
                name: local,
                attributes,
                end: reader.at,
            });
            if empty {
                namespaces = previous;
                result.push(Event::End);
            } else {
                stack.push((previous, name));
            }
        } else {
            let value = reader.text(None, false)?;
            result.push(Event::Text {
                value,
                end: reader.at,
            });
        }
    }
    stack.is_empty().then_some(result)
}

#[cfg(test)]
mod separator_tests {
    use super::*;
    #[test]
    fn renderer_copy_preserves_tokens_and_only_inserts_missing_attribute_spaces() {
        let cases = [
            (
                "<svg width=\"1\"height='1'/>",
                "<svg width=\"1\" height='1'/>",
            ),
            (
                "<svg xmlns='http://www.w3.org/2000/svg'width='2'height='2'><g id='é'fill='red'><rect width='1'height='1'/></g></svg>",
                "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><g id='é' fill='red'><rect width='1' height='1'/></g></svg>",
            ),
            (
                "<svg width='1'height='1'><title><![CDATA[width=\"1\"height=\"1\"]]></title><!-- x='a'y='b' --><rect width='1' height='1' data-a='&quot;'/></svg>",
                "<svg width='1' height='1'><title><![CDATA[width=\"1\"height=\"1\"]]></title><!-- x='a'y='b' --><rect width='1' height='1' data-a='&quot;'/></svg>",
            ),
            ("<svg width='1' height='1'/>", "<svg width='1' height='1'/>"),
            ("<svg width='1'height='1'", "<svg width='1'height='1'"),
            (
                "<svg width='1'height='&bad;'/>",
                "<svg width='1'height='&bad;'/>",
            ),
        ];
        for (raw, expected) in cases {
            let rendered = renderer_attribute_separators(raw);
            assert_eq!(rendered, expected);
            if raw == expected {
                assert!(matches!(rendered, std::borrow::Cow::Borrowed(_)));
            }
            if let Some(original) = events(raw) {
                let changed = events(&rendered).unwrap();
                let attributes = |events: Vec<Event>| {
                    events
                        .into_iter()
                        .filter_map(|event| {
                            if let Event::Start {
                                namespace,
                                name,
                                attributes,
                                ..
                            } = event
                            {
                                Some((
                                    namespace,
                                    name,
                                    attributes
                                        .into_iter()
                                        .map(|a| (a.namespace, a.name, a.value))
                                        .collect::<Vec<_>>(),
                                ))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(attributes(original), attributes(changed));
            }
        }
    }
}
