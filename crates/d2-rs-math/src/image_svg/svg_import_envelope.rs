// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// decoder_field: Copyright 2009 The Go Authors. All rights reserved.
//! D2 v0.9.0 import.go declaration/PI admission and lib/svg/doctype.go.
//! Raw source remains authoritative. The parser shadow blanks only envelope
//! tokens, preserving every byte position and line break; it resolves no DTD.
pub struct Envelope {
    pub shadow: String,
    pub errors: Vec<(usize, String)>,
    pub asset_errors: Vec<(usize, String)>,
    pub first_root_end: Option<usize>,
}
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}
fn name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.' | b'-') || c >= 128
}
// Go encoding/xml:procInst searches name= before the importer's stricter field
// parser. Preserve this lookup, including its deliberately permissive syntax.
pub(super) fn decoder_field<'a>(name: &str, mut input: &'a str) -> &'a str {
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
fn declaration(input: &str, end: usize) -> Result<(), String> {
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
    p.skip();
    let mut fields = Vec::new();
    while p.at < p.bytes.len() {
        let start = p.at;
        while p.at < p.bytes.len() && p.bytes[p.at].is_ascii_alphabetic() {
            p.at += 1;
        }
        if start == p.at {
            return Err(invalid());
        }
        let name = &p.bytes[start..p.at];
        p.skip();
        if !p.take(b"=") {
            return Err(invalid());
        }
        p.skip();
        let value = p.quoted().ok_or_else(invalid)?;
        fields.push((name, value));
        if p.at < p.bytes.len() && !space(p.bytes[p.at]) {
            return Err(invalid());
        }
        p.skip();
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
// Scan quoted tag/directive boundaries without interpreting their contents.
// Directive comments are skipped as encoding/xml does; raw DOCTYPE matching
// still refuses them, preventing normalized comments from widening admission.
pub fn markup_end(source: &str, start: usize, directive: bool) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut at = start + 2;
    let mut quote = 0;
    let mut depth = 0;
    while at < bytes.len() {
        let c = bytes[at];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if matches!(c, b'\'' | b'"') {
            quote = c;
        } else if directive && bytes[at..].starts_with(b"<!--") {
            at += 4 + source[at + 4..].find("-->")? + 3;
            continue;
        } else if directive && c == b'<' {
            depth += 1;
        } else if c == b'>' {
            if depth == 0 {
                return Some(at + 1);
            }
            depth -= 1;
        }
        at += 1;
    }
    None
}
pub fn scan(source: &str) -> Envelope {
    let mut shadow = source.as_bytes().to_vec();
    let mut errors = Vec::new();
    let mut asset_errors = Vec::new();
    let mut at = 0;
    let mut declaration_allowed = true;
    let mut seen_declaration = false;
    let mut seen_doctype = false;
    let mut seen_root = false;
    let mut depth: usize = 0;
    let mut first_root_end = None;
    while at < source.len() {
        let rest = &source[at..];
        if rest.starts_with("<!--") || rest.starts_with("<![CDATA[") {
            let (prefix, tail) = if rest.starts_with("<!--") {
                (4, "-->")
            } else {
                (9, "]]>")
            };
            let Some(end) = rest[prefix..]
                .find(tail)
                .map(|end| at + prefix + end + tail.len())
            else {
                break;
            };
            at = end;
            declaration_allowed = false;
            continue;
        }
        if rest.starts_with("<?") {
            let mut target_end = at + 2;
            while target_end < source.len() && name_byte(source.as_bytes()[target_end]) {
                target_end += 1;
            }
            let target = &source[at + 2..target_end];
            let Some(end) = source[target_end..]
                .find("?>")
                .map(|end| target_end + end + 2)
            else {
                break;
            };
            let input = source[target_end..end - 2].trim_start_matches([' ', '\t', '\r', '\n']);
            let error = if !super::go_xml_names::valid(target) {
                Some(format!("invalid XML near byte {target_end}"))
            } else if target == "xml" {
                // Decoder version/charset errors precede importer PI placement.
                let decoded = declaration(input, end);
                let decoder_error = decoded
                    .as_ref()
                    .err()
                    .filter(|error| error.starts_with("invalid XML near byte"));
                if let Some(error) = decoder_error {
                    Some(error.clone())
                } else if seen_declaration || !declaration_allowed || seen_root {
                    Some("XML processing instructions are forbidden".into())
                } else {
                    decoded.err()
                }
            } else {
                Some("XML processing instructions are forbidden".into())
            };
            if let Some(error) = error {
                if error.starts_with("invalid XML near byte") {
                    asset_errors.push((at, "malformed SVG: malformed SVG XML".into()));
                } else if error == "XML processing instructions are forbidden" {
                    asset_errors.push((
                        at,
                        "malformed SVG: processing instruction is not allowed".into(),
                    ));
                }
                errors.push((at, error));
            }
            for byte in &mut shadow[at..end] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            seen_declaration = true;
            declaration_allowed = false;
            at = end;
            continue;
        }
        if rest.starts_with("<!") {
            let Some(end) = markup_end(source, at, true) else {
                break;
            };
            if seen_root || seen_doctype || !doctype(&source[at..end], at) {
                errors.push((at, "unsupported XML directive or DTD".into()));
                asset_errors.push((at, "unsupported SVG: XML directive or DTD".into()));
            }
            for byte in &mut shadow[at..end] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            seen_doctype = true;
            declaration_allowed = false;
            at = end;
            continue;
        }
        if rest.starts_with('<') {
            let Some(end) = markup_end(source, at, false) else {
                break;
            };
            if rest.starts_with("</") {
                depth = depth.saturating_sub(1);
                if depth == 0 && first_root_end.is_none() {
                    first_root_end = Some(end);
                }
            } else {
                if depth == 0 && first_root_end.is_some() {
                    errors.push((at, format!("multiple root elements near byte {end}")));
                    asset_errors.push((at, "malformed SVG: multiple root elements".into()));
                }
                seen_root = true;
                if !source[at..end].ends_with("/>") {
                    depth += 1;
                } else if depth == 0 && first_root_end.is_none() {
                    first_root_end = Some(end);
                }
            }
            declaration_allowed = false;
            at = end;
        } else {
            at += rest.find('<').unwrap_or(rest.len());
            declaration_allowed = false;
        }
    }
    Envelope {
        shadow: String::from_utf8(shadow)
            .expect("blanking complete XML envelope tokens preserves UTF-8"),
        errors,
        asset_errors,
        first_root_end,
    }
}
