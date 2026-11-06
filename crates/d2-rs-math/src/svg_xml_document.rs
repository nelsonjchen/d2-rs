// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// decoder_field: Copyright 2009 The Go Authors. All rights reserved.
// Source: D2 d2renderers/d2svgimport/import.go, lib/svg/doctype.go.
use super::svg_xml_admission::{Attribute, Attributes, State};
use quick_xml::{Reader, events::Event};
use std::collections::BTreeMap;
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Element {
    pub name: String,
    pub attributes: Attributes,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    pub text: Vec<u8>,
}
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Document {
    pub namespace: String,
    pub elements: Vec<Element>,
    pub ids: BTreeMap<String, usize>,
    pub counts: [usize; 4],
}
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}
// Go encoding/xml:procInst searches name= before the importer's stricter field
// parser. Preserve this lookup, including its deliberately permissive syntax.
fn decoder_field<'a>(name: &str, mut input: &'a str) -> &'a str {
    let key = format!("{name}=");
    while let Some(at) = input.find(&key) {
        let rest = &input[at + key.len()..];
        let Some(&quote) = rest.as_bytes().first() else {
            return "";
        };
        if matches!(quote, b'\'' | b'"') {
            return rest[1..]
                .find(quote as char)
                .map_or("", |end| &rest[1..end + 1]);
        }
        input = &rest[rest.chars().next().unwrap().len_utf8()..];
    }
    ""
}
struct Fields<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Fields<'a> {
    fn skip(&mut self) -> bool {
        let before = self.at;
        while self.at < self.bytes.len() && space(self.bytes[self.at]) {
            self.at += 1;
        }
        self.at != before
    }
    fn take(&mut self, value: &[u8]) -> bool {
        if self.bytes[self.at..].starts_with(value) {
            self.at += value.len();
            true
        } else {
            false
        }
    }
    fn quoted(&mut self) -> Option<&'a [u8]> {
        let quote = *self.bytes.get(self.at)?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        self.at += 1;
        let start = self.at;
        while self.at < self.bytes.len() && self.bytes[self.at] != quote {
            self.at += 1;
        }
        if self.at == self.bytes.len() {
            return None;
        }
        let result = &self.bytes[start..self.at];
        self.at += 1;
        Some(result)
    }
}
pub fn declaration(
    input: &str,
    end: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let version = decoder_field("version", input);
    let encoding = decoder_field("encoding", input);
    if (!version.is_empty() && version != "1.0")
        || (!encoding.is_empty()
            && !encoding.eq_ignore_ascii_case("UTF-8")
            && !encoding.eq_ignore_ascii_case("ISO-8859-1"))
    {
        return Err(format!("invalid XML near byte {end}"));
    }
    let invalid = || "invalid XML declaration".to_string();
    let mut p = Fields {
        bytes: input.as_bytes(),
        at: 0,
    };
    skip_declaration_space(&mut p, cancel)?;
    let mut fields = Vec::new();
    while p.at < p.bytes.len() {
        context(cancel)?;
        let start = p.at;
        while p.at < p.bytes.len() && p.bytes[p.at].is_ascii_alphabetic() {
            p.at += 1;
            if p.at & 4095 == 0 {
                context(cancel)?;
            }
        }
        if start == p.at {
            return Err(invalid());
        }
        let name = &p.bytes[start..p.at];
        skip_declaration_space(&mut p, cancel)?;
        if !p.take(b"=") {
            return Err(invalid());
        }
        skip_declaration_space(&mut p, cancel)?;
        let quote = *p.bytes.get(p.at).ok_or_else(invalid)?;
        if !matches!(quote, b'\'' | b'"') {
            return Err(invalid());
        }
        p.at += 1;
        let start = p.at;
        while p.at < p.bytes.len() && p.bytes[p.at] != quote {
            p.at += 1;
            if p.at & 4095 == 0 {
                context(cancel)?;
            }
        }
        if p.at == p.bytes.len() {
            return Err(invalid());
        }
        let value = &p.bytes[start..p.at];
        p.at += 1;
        fields.push((name, value));
        if p.at < p.bytes.len() && !space(p.bytes[p.at]) {
            return Err(invalid());
        }
        skip_declaration_space(&mut p, cancel)?;
    }
    if fields.is_empty()
        || fields.len() > 3
        || fields[0] != (b"version".as_slice(), b"1.0".as_slice())
    {
        return Err(invalid());
    }
    let mut next = 1;
    if fields.get(next).is_some_and(|f| f.0 == b"encoding") {
        if !fields[next].1.eq_ignore_ascii_case(b"UTF-8")
            && !fields[next].1.eq_ignore_ascii_case(b"ISO-8859-1")
        {
            return Err("only UTF-8 and ISO-8859-1 XML declarations are supported".into());
        }
        next += 1;
    }
    if fields.get(next).is_some_and(|f| f.0 == b"standalone") {
        if !matches!(fields[next].1, b"yes" | b"no") {
            return Err(invalid());
        }
        next += 1;
    }
    if next != fields.len() {
        return Err(invalid());
    }
    Ok(())
}
fn skip_declaration_space(
    p: &mut Fields<'_>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    while p.at < p.bytes.len() && space(p.bytes[p.at]) {
        p.at += 1;
        if p.at & 4095 == 0 {
            context(cancel)?;
        }
    }
    Ok(())
}
fn doctype(raw: &str, start: usize) -> bool {
    // Original helper caps both the raw declaration and prolog scan. No
    // internal subset, entity expansion, system-only locator or DTD resolution.
    if raw.len() > 256 || start + 9 > 16 << 10 {
        return false;
    }
    let mut p = Fields {
        bytes: raw.as_bytes(),
        at: 0,
    };
    if !p.take(b"<!DOCTYPE")
        || !p.skip()
        || !p.take(b"svg")
        || !p.skip()
        || !p.take(b"PUBLIC")
        || !p.skip()
    {
        return false;
    }
    let Some(public) = p.quoted() else {
        return false;
    };
    if !p.skip() {
        return false;
    }
    let Some(system) = p.quoted() else {
        return false;
    };
    p.skip();
    if !p.take(b">") || p.at != p.bytes.len() {
        return false;
    }
    (public == b"-//W3C//DTD SVG 1.0//EN"
        && system == b"http://www.w3.org/TR/2001/REC-SVG-20010904/DTD/svg10.dtd")
        || (public == b"-//W3C//DTD SVG 1.1//EN"
            && system == b"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd")
}
fn resolve(name: &str, bindings: &BTreeMap<String, String>, attribute: bool) -> (String, String) {
    if attribute && name == "xmlns" {
        return (String::new(), name.into());
    }
    if let Some((prefix, local)) = name.split_once(':') {
        let namespace = if attribute && prefix == "xmlns" {
            "xmlns".into()
        } else if prefix == "xml" {
            super::svg_xml_admission::XML.into()
        } else {
            bindings
                .get(prefix)
                .cloned()
                .unwrap_or_else(|| prefix.into())
        };
        (namespace, local.into())
    } else {
        (
            if attribute {
                String::new()
            } else {
                bindings.get("").cloned().unwrap_or_default()
            },
            name.into(),
        )
    }
}
// Go text() rejects unknown entities immediately after consuming their
// semicolon, before the closing quote/start-tag terminator is read.
fn attribute_decode_error(raw: &[u8], offset: usize) -> usize {
    for (index, &byte) in raw.iter().enumerate() {
        if byte == b'<' {
            return offset + index + 1;
        }
        if byte == b'&' {
            let tail = &raw[index..];
            if let Some(end) = tail.iter().position(|&b| b == b';') {
                if std::str::from_utf8(&tail[..=end])
                    .ok()
                    .is_some_and(|text| quick_xml::escape::unescape(text).is_err())
                {
                    return offset + index + end + 1;
                }
            }
        }
    }
    offset + raw.len() + 1
}
fn decoded(raw: &[u8], entities: bool) -> Result<Vec<u8>, ()> {
    let text = std::str::from_utf8(raw).map_err(|_| ())?;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = if entities {
        quick_xml::escape::unescape(&normalized)
            .map_err(|_| ())?
            .into_owned()
    } else {
        normalized
    };
    if text.chars().any(|c| {
        !matches!(c, '\t' | '\n' | '\r') && (c < '\u{20}' || c == '\u{fffe}' || c == '\u{ffff}')
    }) {
        return Err(());
    }
    Ok(text.into_bytes())
}
/// Native parsed tree and declared ID inventory. Limits have the same order as
/// structural_work. Source is the already-redacted importer diagnostic label.
pub fn parse(
    source: &str,
    input: &str,
    limits: [usize; 9],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Document, String> {
    parse_bytes(source, input.as_bytes(), limits, cancel)
}
pub fn parse_bytes(
    source: &str,
    input: &[u8],
    limits: [usize; 9],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Document, String> {
    context(cancel)?;
    if limits.contains(&0) {
        return Err("d2svgimport: all import limits must be positive".into());
    }
    if input.len() > limits[0] {
        return Err(format!(
            "d2svgimport: {source} is {} bytes, exceeding limit {}",
            input.len(),
            limits[0]
        ));
    }
    let input = input.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(input);
    let declaration_end = if input.starts_with(b"<?xml") {
        input
            .windows(2)
            .position(|pair| pair == b"?>")
            .map(|end| end + 2)
    } else {
        None
    };
    let latin1 = declaration_end
        .and_then(|end| std::str::from_utf8(&input[5..end - 2]).ok())
        .is_some_and(|header| decoder_field("encoding", header).eq_ignore_ascii_case("ISO-8859-1"));
    let converted;
    let input = if latin1 {
        let end = declaration_end.unwrap();
        let prefix = std::str::from_utf8(&input[..end])
            .map_err(|_| format!("d2svgimport: {source}: invalid XML near byte {end}"))?;
        let mut output = prefix.to_owned();
        for chunk in input[end..].chunks(4096) {
            context(cancel)?;
            output.extend(chunk.iter().copied().map(char::from));
        }
        converted = output;
        converted.as_str()
    } else {
        std::str::from_utf8(input).map_err(|_| {
            format!(
                "d2svgimport: {source}: invalid XML near byte {}",
                input.len()
            )
        })?
    };
    parse_decoded(source, input, limits, cancel)
}
fn parse_decoded(
    source: &str,
    input: &str,
    limits: [usize; 9],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Document, String> {
    // ImportNode removes exactly one BOM. quick-xml would remove another
    // automatically; encoding/xml instead returns that remaining BOM as text.
    if input.starts_with('\u{feff}') {
        let end = input.find('<').unwrap_or(input.len());
        return Err(format!(
            "d2svgimport: {source}: text content is unsupported near byte {end}"
        ));
    }
    let mut reader = Reader::from_str(input);
    reader.config_mut().check_comments = true;
    let mut scopes: Vec<BTreeMap<String, String>> = Vec::new();
    let mut document = Document {
        namespace: String::new(),
        elements: Vec::new(),
        ids: BTreeMap::new(),
        counts: [0; 4],
    };
    let mut state = State::default();
    let mut stack: Vec<usize> = Vec::new();
    let mut closed = false;
    let mut declaration_allowed = true;
    let mut seen_declaration = false;
    let mut seen_doctype = false;
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    loop {
        context(cancel)?;
        let start = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|_| {
            let mut at = reader.buffer_position() as usize;
            if input.as_bytes()[start..].starts_with(b"<!--") {
                if let Some(position) = input[start + 4..].find("--") {
                    at = start + 4 + position + 3;
                }
            }
            error(format!("invalid XML near byte {at}"))
        })?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(node) | Event::Empty(node) => {
                let mut raw_attrs = Vec::new();
                for item in node.attributes().with_checks(false) {
                    let item = item.map_err(|reason| {
                        use quick_xml::events::attributes::AttrError;
                        let at = match reason {
                            AttrError::UnquotedValue(position)
                            | AttrError::ExpectedEq(position) => start + 2 + position,
                            _ => end,
                        };
                        error(format!("invalid XML near byte {at}"))
                    })?;
                    let name = std::str::from_utf8(item.key.as_ref())
                        .map_err(|_| error(format!("invalid XML near byte {end}")))?
                        .to_owned();
                    let value = decoded(item.value.as_ref(), true).map_err(|_| {
                        let offset = (item.value.as_ptr() as usize)
                            .checked_sub(input.as_ptr() as usize)
                            .filter(|&offset| offset <= input.len())
                            .unwrap_or(start);
                        error(format!(
                            "invalid XML near byte {}",
                            attribute_decode_error(item.value.as_ref(), offset)
                        ))
                    })?;
                    raw_attrs.push((name, String::from_utf8(value).unwrap()));
                }
                let empty = input.as_bytes().get(end.saturating_sub(2)..end) == Some(b"/>");
                declaration_allowed = false;
                if closed {
                    return Err(error(format!("multiple root elements near byte {end}")));
                }
                if stack.len() + 1 > limits[1] {
                    return Err(error(format!("XML depth exceeds limit {}", limits[1])));
                }
                document.counts[0] += 1;
                if document.counts[0] > limits[2] {
                    return Err(error(format!("element count exceeds limit {}", limits[2])));
                }
                let mut bindings = scopes.last().cloned().unwrap_or_default();
                for (name, value) in &raw_attrs {
                    if name == "xmlns" {
                        bindings.insert(String::new(), value.clone());
                    } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                        bindings.insert(prefix.into(), value.clone());
                    }
                }
                let qname = node.name();
                let qname = std::str::from_utf8(qname.as_ref())
                    .map_err(|_| error(format!("invalid XML near byte {end}")))?;
                let (ns, local) = resolve(qname, &bindings, false);
                let name = state.element(source, &ns, &local)?;
                if document.elements.is_empty() && name != "svg" {
                    return Err(error(format!("root element must be <svg>, got <{name}>")));
                }
                let attrs = raw_attrs
                    .into_iter()
                    .map(|(name, value)| {
                        let (namespace, name) = resolve(&name, &bindings, true);
                        Attribute {
                            namespace,
                            name,
                            value,
                        }
                    })
                    .collect::<Vec<_>>();
                let attributes =
                    state.attributes(source, &name, &attrs, limits[3], limits[4], cancel)?;
                let index = document.elements.len();
                if !attributes.id.is_empty() {
                    if document.ids.contains_key(&attributes.id) {
                        return Err(error("duplicate id".into()));
                    }
                    if document.ids.len() >= limits[8] {
                        return Err(error(format!(
                            "ID resource count exceeds limit {}",
                            limits[8]
                        )));
                    }
                    document.ids.insert(attributes.id.clone(), index);
                }
                let parent = stack.last().copied();
                if let Some(parent) = parent {
                    document.elements[parent].children.push(index);
                }
                document.elements.push(Element {
                    name,
                    attributes,
                    children: Vec::new(),
                    parent,
                    text: Vec::new(),
                });
                if empty {
                    if stack.is_empty() {
                        closed = true;
                    }
                } else {
                    stack.push(index);
                    scopes.push(bindings);
                }
            }
            Event::End(node) => {
                declaration_allowed = false;
                if stack.is_empty() {
                    return Err(error(format!("unexpected closing element near byte {end}")));
                }
                let qname = node.name();
                let qname = std::str::from_utf8(qname.as_ref())
                    .map_err(|_| error(format!("invalid XML near byte {end}")))?;
                let (ns, local) = resolve(qname, scopes.last().unwrap(), false);
                let name = state.element(source, &ns, &local)?;
                let current = *stack.last().unwrap();
                if document.elements[current].name != name {
                    return Err(error(format!(
                        "closing </{name}> does not match <{}>",
                        document.elements[current].name
                    )));
                }
                stack.pop();
                scopes.pop();
                if stack.is_empty() {
                    closed = true;
                }
            }
            Event::Text(text) => text_event(
                &mut document,
                &stack,
                &decoded(text.as_ref(), true)
                    .map_err(|_| error(format!("invalid XML near byte {end}")))?,
                limits[0],
                source,
                end,
                &mut declaration_allowed,
                cancel,
            )?,
            Event::CData(text) => text_event(
                &mut document,
                &stack,
                &decoded(text.as_ref(), false)
                    .map_err(|_| error(format!("invalid XML near byte {end}")))?,
                limits[0],
                source,
                end,
                &mut declaration_allowed,
                cancel,
            )?,
            Event::Comment(_) => {
                if stack
                    .last()
                    .is_some_and(|&index| document.elements[index].name == "style")
                {
                    return Err(error("CSS comments in <style> are unsupported".into()));
                }
                declaration_allowed = false;
            }
            Event::Decl(_) => {
                if seen_declaration
                    || !declaration_allowed
                    || !document.elements.is_empty()
                    || !stack.is_empty()
                {
                    return Err(error("XML processing instructions are forbidden".into()));
                }
                declaration(&input[start + 5..end - 2], end, cancel).map_err(&error)?;
                seen_declaration = true;
                declaration_allowed = false;
            }
            Event::PI(_) => return Err(error("XML processing instructions are forbidden".into())),
            Event::DocType(_) => {
                if !document.elements.is_empty()
                    || !stack.is_empty()
                    || seen_doctype
                    || !doctype(&input[start..end], start)
                {
                    return Err(error("unsupported XML directive or DTD".into()));
                }
                seen_doctype = true;
                declaration_allowed = false;
            }
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(error(format!("invalid XML near byte {end}")));
                }
                break;
            }
        }
    }
    if document.elements.is_empty() {
        return Err(error("document has no root <svg> element".into()));
    }
    if let Some(&index) = stack.last() {
        return Err(error(format!(
            "unterminated <{}> element",
            document.elements[index].name
        )));
    }
    if !closed {
        return Err(error("unterminated root element".into()));
    }
    document.namespace = state.namespace.unwrap_or_default();
    document.counts[1] = state.attributes;
    document.counts[2] = state.attribute_bytes;
    Ok(document)
}
fn text_event(
    document: &mut Document,
    stack: &[usize],
    value: &[u8],
    maximum: usize,
    source: &str,
    end: usize,
    declaration_allowed: &mut bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let current = stack.last().copied();
    let name = current
        .map(|index| document.elements[index].name.as_str())
        .unwrap_or("");
    if matches!(name, "style" | "text" | "dc:format") {
        let kind = match name {
            "style" => "stylesheet",
            "text" => "painted",
            _ => "metadata",
        };
        let stylesheet = name == "style";
        let index = current.unwrap();
        for chunk in value.chunks(4096) {
            context(cancel)?;
            let used = if stylesheet {
                document.counts[3]
            } else {
                document.elements[index].text.len()
            };
            if used > maximum || chunk.len() > maximum - used {
                return Err(format!(
                    "d2svgimport: {source}: {kind} text exceeds byte limit {maximum}"
                ));
            }
            document.elements[index].text.extend_from_slice(chunk);
            if stylesheet {
                document.counts[3] += chunk.len();
            }
        }
        context(cancel)?;
    } else if name == "title" {
        for _ in value.chunks(4096) {
            context(cancel)?;
        }
        context(cancel)?;
    } else {
        for (index, &byte) in value.iter().enumerate() {
            if !space(byte) {
                return Err(format!(
                    "d2svgimport: {source}: text content is unsupported near byte {end}"
                ));
            }
            if index & 4095 == 0 {
                context(cancel)?;
            }
        }
    }
    if !value.is_empty() {
        *declaration_allowed = false;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_context_precedes_limits_and_byte_work() {
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            parse_bytes("math", b"<svg/>", [0; 9], Some(&token)).unwrap_err(),
            "context canceled"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.expire();
        assert_eq!(
            parse_bytes("math", b"<svg/>", [1; 9], Some(&token)).unwrap_err(),
            "context deadline exceeded"
        );
        assert_eq!(
            parse_bytes("math", b"<svg/>", [0; 9], None).unwrap_err(),
            "d2svgimport: all import limits must be positive"
        );
        assert_eq!(
            parse_bytes("math", b"<svg/>", [1; 9], None).unwrap_err(),
            "d2svgimport: math is 6 bytes, exceeding limit 1"
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_xml_document_trees_and_errors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-xml-document-v090.json"
        ))
        .unwrap();
        let mut mismatches = Vec::new();
        for row in rows.as_array().unwrap() {
            let limits = serde_json::from_value(row["limits"].clone()).unwrap();
            let bytes: Vec<u8> = serde_json::from_value(row["input_bytes"].clone()).unwrap();
            let result = parse_bytes("math", &bytes, limits, None);
            let actual_error = result.as_ref().err().map(String::as_str).unwrap_or("");
            if actual_error != row["error"].as_str().unwrap() {
                mismatches.push(format!(
                    "{} {:?}: expected {:?}, got {:?}",
                    row["mode"], row["input"], row["error"], actual_error
                ));
                continue;
            }
            if let Ok(document) = result {
                let expected: Document = serde_json::from_value(row["output"].clone()).unwrap();
                assert_eq!(document, expected, "{row}");
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
}
