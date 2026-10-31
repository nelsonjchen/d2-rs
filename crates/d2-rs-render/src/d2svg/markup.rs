// SPDX-License-Identifier: MPL-2.0
//! Pinned D2 lib/svg/{markup,text}.go: typed attributes, fragments and local IDs.
use super::svgtext::{escape_text, svg_id};
use std::collections::HashSet;

/// A validated name with an unescaped, caller-owned value.
#[derive(Clone, Debug, Default)]
pub struct Attribute {
    name: String,
    value: Vec<u8>,
}
impl Attribute {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn value(&self) -> &[u8] {
        &self.value
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().filter(|c| *c == b':').count() <= 1
        && name.split(':').all(|part| {
            !part.is_empty()
                && part.bytes().enumerate().all(|(i, c)| {
                    c.is_ascii_alphabetic()
                        || c == b'_'
                        || i > 0 && (c.is_ascii_digit() || matches!(c, b'-' | b'.'))
                })
        })
}
fn valid_attribute_name(name: &str) -> bool {
    valid_name(name)
        && !name
            .rsplit(':')
            .next()
            .unwrap()
            .to_ascii_lowercase()
            .starts_with("on")
}
pub fn attr(name: &str, value: impl AsRef<[u8]>) -> Attribute {
    assert!(
        valid_attribute_name(name),
        "invalid SVG attribute name {name:?}"
    );
    Attribute {
        name: name.into(),
        value: value.as_ref().to_vec(),
    }
}
pub fn float_attr(name: &str, value: f64) -> Attribute {
    assert!(value.is_finite(), "non-finite SVG attribute {name:?}");
    attr(name, super::svgtext::format_float(value))
}
pub fn open_element(name: &str, attributes: &[Attribute]) -> String {
    assert!(
        valid_name(name) && !name.eq_ignore_ascii_case("script"),
        "invalid SVG element name {name:?}"
    );
    format!("<{name}{}>", render_attributes(attributes))
}
pub fn close_element(name: &str) -> String {
    assert!(
        valid_name(name) && !name.eq_ignore_ascii_case("script"),
        "invalid SVG element name {name:?}"
    );
    format!("</{name}>")
}
fn valid_xml_char(c: char) -> bool {
    matches!(c, '\t'|'\n'|'\r'|'\u{20}'..='\u{d7ff}'|'\u{e000}'..='\u{fffd}'|'\u{10000}'..='\u{10ffff}')
}
/// Go ranges over invalid UTF-8 one byte at a time.
fn decode_bytes(mut bytes: &[u8]) -> String {
    let mut out = String::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let n = e.valid_up_to();
                out.push_str(std::str::from_utf8(&bytes[..n]).unwrap());
                out.push('\u{fffd}');
                bytes = &bytes[n + 1..];
            }
        }
    }
    out
}
pub fn escape_attribute(value: impl AsRef<[u8]>) -> String {
    escape_text(&decode_bytes(value.as_ref())).replace("&#39;", "'")
}
pub fn render_attributes(attributes: &[Attribute]) -> String {
    let mut seen = HashSet::new();
    let mut out = String::new();
    for a in attributes {
        assert!(
            valid_attribute_name(&a.name),
            "invalid zero-value SVG attribute"
        );
        assert!(seen.insert(&a.name), "duplicate SVG attribute {:?}", a.name);
        out.push(' ');
        out.push_str(&a.name);
        out.push_str("=\"");
        out.push_str(&escape_attribute(&a.value));
        out.push('"');
    }
    out
}
pub fn href_attributes(value: impl AsRef<[u8]>) -> Vec<Attribute> {
    vec![attr("href", &value), attr("xlink:href", value)]
}
/// Errors represent the same admission boundary as Go; decoder text is not an API contract.
pub fn parse_attributes(fragment: &str) -> Result<Vec<Attribute>, String> {
    let invalid = || "invalid SVG attributes".to_owned();
    let mut rest = fragment.trim_matches(char::is_whitespace);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    while !rest.is_empty() {
        let n = rest
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
            .unwrap_or(rest.len());
        let name = &rest[..n];
        if !valid_attribute_name(name) {
            return Err(format!("invalid SVG attribute name {name:?}"));
        }
        if !seen.insert(name) {
            return Err(format!("duplicate SVG attribute {name:?}"));
        }
        rest = rest[n..].trim_start_matches([' ', '\t', '\n', '\r']);
        rest = rest
            .strip_prefix('=')
            .ok_or_else(invalid)?
            .trim_start_matches([' ', '\t', '\n', '\r']);
        let quote = rest.chars().next().ok_or_else(invalid)?;
        if !matches!(quote, '\'' | '"') {
            return Err(invalid());
        }
        rest = &rest[1..];
        let end = rest.find(quote).ok_or_else(invalid)?;
        let raw = &rest[..end];
        if raw.contains('<') || !raw.chars().all(valid_xml_char) {
            return Err(invalid());
        }
        // Normalize literal newlines before expanding entities, like encoding/xml.
        let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
        let mut value = String::new();
        let mut remaining = raw.as_str();
        while let Some(start) = remaining.find('&') {
            value.push_str(&remaining[..start]);
            let end = remaining[start..].find(';').ok_or_else(invalid)? + start;
            let entity = &remaining[start + 1..end];
            let c = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let number = entity.strip_prefix('#').ok_or_else(invalid)?;
                    let code = if let Some(hex) = number.strip_prefix('x') {
                        u32::from_str_radix(hex, 16)
                    } else {
                        number.parse()
                    }
                    .map_err(|_| invalid())?;
                    char::from_u32(code)
                        .filter(|c| valid_xml_char(*c))
                        .ok_or_else(invalid)?
                }
            };
            value.push(c);
            remaining = &remaining[end + 1..];
        }
        value.push_str(remaining);
        out.push(attr(name, value));
        rest = &rest[end + 1..];
        // RawToken accepts attributes without an intervening space too.
        rest = rest.trim_start_matches([' ', '\t', '\n', '\r']);
    }
    Ok(out)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Id(String);
fn valid_id_part(s: &str) -> bool {
    s.bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':'))
}
pub fn literal_id(value: &str) -> Id {
    assert!(
        !value.is_empty() && valid_id_part(value),
        "invalid SVG id {value:?}"
    );
    Id(value.into())
}
pub fn scoped_id(prefix: &str, component: &str, suffix: &str) -> Id {
    assert!(
        valid_id_part(prefix) && valid_id_part(suffix),
        "invalid SVG id prefix or suffix"
    );
    let component =
        if component.is_empty() || !valid_id_part(component) || component.starts_with("d2e-") {
            format!("d2e-{}", svg_id(component))
        } else {
            component.into()
        };
    literal_id(&format!("{prefix}{component}{suffix}"))
}
impl Id {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
pub fn local_iri(id: &Id) -> String {
    format!("url(#{})", id.0)
}
pub fn parse_local_iri(value: &str) -> Result<Id, String> {
    let id = value
        .strip_prefix("url(#")
        .and_then(|s| s.strip_suffix(')'));
    match id {
        Some(s) if !s.is_empty() && valid_id_part(s) => Ok(Id(s.into())),
        _ => Err(format!("invalid SVG local IRI {value:?}")),
    }
}
#[derive(Clone, Debug)]
pub struct Fragment(String);
impl Fragment {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
pub fn text(value: impl AsRef<[u8]>) -> Fragment {
    Fragment(escape_text(&decode_bytes(value.as_ref())))
}
pub fn trusted_fragment(value: impl AsRef<[u8]>) -> Fragment {
    let value =
        std::str::from_utf8(value.as_ref()).expect("invalid UTF-8 or character in XML value");
    assert!(
        value.chars().all(valid_xml_char),
        "invalid UTF-8 or character in XML value"
    );
    Fragment(value.into())
}
