// Copyright 2010 The Go Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause
// Native Go1.27 mime/type.go and type_unix.go registration/lookup boundary.
// Media parameter handling follows mime/mediatype.go. See THIRD_PARTY_NOTICES.md.
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn go_lower(value: &str) -> String {
    String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes()))
        .expect("Go Unicode lowercase preserves UTF-8")
}

#[derive(Default)]
struct Registry {
    exact: BTreeMap<String, String>,
    lower: BTreeMap<String, String>,
}

pub(super) fn lookup(extension: &str) -> Option<String> {
    static TYPES: OnceLock<Registry> = OnceLock::new();
    TYPES
        .get_or_init(Registry::system)
        .get(extension)
        .map(str::to_owned)
}

impl Registry {
    fn builtins() -> Self {
        let mut types = Self::default();
        for (extension, mime) in super::data::EXTENSIONS {
            types
                .exact
                .insert((*extension).to_owned(), (*mime).to_owned());
            types
                .lower
                .insert((*extension).to_owned(), (*mime).to_owned());
        }
        types
    }

    fn system() -> Self {
        let mut types = Self::builtins();
        #[cfg(unix)]
        {
            for path in ["/usr/local/share/mime/globs2", "/usr/share/mime/globs2"] {
                if types.load_file(path, true) {
                    return types;
                }
            }
            for path in [
                "/etc/mime.types",
                "/etc/apache2/mime.types",
                "/etc/apache/mime.types",
                "/etc/httpd/conf/mime.types",
            ] {
                types.load_file(path, false);
            }
        }
        types
    }

    fn get(&self, extension: &str) -> Option<&str> {
        self.exact
            .get(extension)
            .or_else(|| self.lower.get(&go_lower(extension)))
            .map(String::as_str)
    }

    fn set(&mut self, extension: &str, raw: &str) -> bool {
        let Some((base, mut params)) = parse_media_type(raw) else {
            return false;
        };
        let mime = if raw.starts_with("text/") && params.get("charset").is_none_or(Vec::is_empty) {
            params.insert("charset".to_owned(), b"utf-8".to_vec());
            format_media_type(&base, &params)
        } else {
            raw.to_owned()
        };
        self.exact.insert(extension.to_owned(), mime.clone());
        self.lower.insert(go_lower(extension), mime);
        true
    }

    #[cfg(any(unix, test))]
    fn load_file(&mut self, path: &str, globs: bool) -> bool {
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        self.load_reader(std::io::BufReader::new(file), globs)
            .expect("MIME database scan failed");
        true
    }

    #[cfg(any(unix, test))]
    fn load_reader(
        &mut self,
        mut reader: impl std::io::BufRead,
        globs: bool,
    ) -> std::io::Result<()> {
        use std::io::{BufRead, Read};
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            // bufio.Scanner's ScanLines has a 64 KiB token buffer limit.
            let count = reader.by_ref().take(65536).read_until(b'\n', &mut bytes)?;
            if count == 0 {
                return Ok(());
            }
            if count == 65536 && bytes.last() != Some(&b'\n') {
                return Err(std::io::Error::other("bufio.Scanner: token too long"));
            }
            if bytes.last() == Some(&b'\n') {
                bytes.pop();
            }
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            let line = std::str::from_utf8(&bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            if globs {
                self.globs_line(line)
            } else {
                self.types_line(line)
            }
        }
    }

    #[cfg(any(unix, test))]
    fn globs_line(&mut self, line: &str) {
        let fields: Vec<_> = line.split(':').collect();
        if fields.len() < 3
            || fields[0].is_empty()
            || fields[0].starts_with('#')
            || fields[2].len() < 3
        {
            return;
        }
        let Some(extension) = fields[2]
            .strip_prefix('*')
            .filter(|ext| ext.starts_with('.'))
        else {
            return;
        };
        if extension.contains(['?', '*', '[']) || self.exact.contains_key(extension) {
            return;
        }
        // Source preserves the first exact-case entry, including a builtin.
        // Different spellings may still update the case-insensitive map.
        self.set(extension, fields[1]);
    }

    #[cfg(any(unix, test))]
    fn types_line(&mut self, line: &str) {
        let mut fields = line.split_whitespace();
        let Some(mime) = fields.next().filter(|mime| !mime.starts_with('#')) else {
            return;
        };
        for ext in fields {
            if ext.starts_with('#') {
                break;
            }
            // Unlike globs2, later mime.types entries overwrite earlier ones.
            self.set(&format!(".{ext}"), mime);
        }
    }
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`{|}~".contains(&byte)
}

fn token(value: &[u8]) -> bool {
    !value.is_empty() && value.iter().all(|byte| token_byte(*byte))
}

fn consume_token(value: &str) -> (&str, &str) {
    let end = value
        .bytes()
        .position(|byte| !token_byte(byte))
        .unwrap_or(value.len());
    value.split_at(end)
}

fn special(byte: u8) -> bool {
    b"()<>@,;:\\\"/[]?=".contains(&byte)
}

fn consume_value(value: &str) -> Option<(Vec<u8>, &str)> {
    if !value.starts_with('"') {
        let (part, rest) = consume_token(value);
        return (!part.is_empty()).then(|| (part.as_bytes().to_vec(), rest));
    }
    let bytes = value.as_bytes();
    let mut output = Vec::new();
    let mut index = 1;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            return Some((output, &value[index + 1..]));
        }
        if matches!(byte, b'\r' | b'\n') {
            return None;
        }
        if byte == b'\\' && bytes.get(index + 1).is_some_and(|next| special(*next)) {
            index += 1;
            output.push(bytes[index]);
        } else {
            output.push(byte);
        }
        index += 1;
    }
    None
}

fn percent_decode(value: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < value.len() {
        if value[index] == b'%' {
            let high = (*value.get(index + 1)? as char).to_digit(16)?;
            let low = (*value.get(index + 2)? as char).to_digit(16)?;
            output.push((high * 16 + low) as u8);
            index += 3;
        } else {
            output.push(value[index]);
            index += 1;
        }
    }
    Some(output)
}

fn decode_2231(value: &[u8]) -> Option<Vec<u8>> {
    let mut fields = value.splitn(3, |byte| *byte == b'\'');
    let charset = fields.next()?;
    fields.next()?;
    let value = fields.next()?;
    if !charset.eq_ignore_ascii_case(b"utf-8") && !charset.eq_ignore_ascii_case(b"us-ascii") {
        return None;
    }
    percent_decode(value)
}

fn parse_media_type(raw: &str) -> Option<(String, BTreeMap<String, Vec<u8>>)> {
    let (base, mut rest) = raw
        .split_once(';')
        .map_or((raw, ""), |(base, _)| (base, &raw[base.len()..]));
    let base = go_lower(base.trim());
    if !base.split('/').all(|part| token(part.as_bytes())) || base.matches('/').count() > 1 {
        return None;
    }
    let mut params = BTreeMap::new();
    let mut continuations: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        if rest.trim() == ";" {
            break;
        }
        rest = rest.strip_prefix(';')?.trim_start();
        let (key, tail) = consume_token(rest);
        if key.is_empty() {
            return None;
        }
        let key = go_lower(key);
        rest = tail.trim_start().strip_prefix('=')?.trim_start();
        let (value, tail) = consume_value(rest)?;
        let map = if let Some((root, _)) = key.split_once('*') {
            continuations.entry(root.to_owned()).or_default()
        } else {
            &mut params
        };
        if map.get(&key).is_some_and(|old| *old != value) {
            return None;
        }
        map.insert(key, value);
        rest = tail;
    }
    for (key, pieces) in continuations {
        if let Some(value) = pieces.get(&format!("{key}*")) {
            if let Some(decoded) = decode_2231(value) {
                params.insert(key, decoded);
            }
            continue;
        }
        let mut value = Vec::new();
        let mut found = false;
        for index in 0.. {
            if let Some(part) = pieces.get(&format!("{key}*{index}")) {
                found = true;
                value.extend_from_slice(part);
            } else if let Some(part) = pieces.get(&format!("{key}*{index}*")) {
                found = true;
                value.extend(
                    if index == 0 {
                        decode_2231(part)
                    } else {
                        percent_decode(part)
                    }
                    .unwrap_or_default(),
                );
            } else {
                break;
            }
        }
        if found {
            params.insert(key, value);
        }
    }
    Some((base, params))
}

fn format_media_type(base: &str, params: &BTreeMap<String, Vec<u8>>) -> String {
    use std::fmt::Write;
    let mut output = base.to_owned();
    for (key, value) in params {
        output.push_str("; ");
        output.push_str(key);
        if value
            .iter()
            .any(|byte| (*byte < b' ' && *byte != b'\t') || *byte >= 0x7f)
        {
            output.push_str("*=utf-8''");
            for byte in value {
                if *byte <= b' '
                    || *byte >= 0x7f
                    || matches!(byte, b'*' | b'\'' | b'%')
                    || special(*byte)
                {
                    write!(output, "%{byte:02X}").unwrap();
                } else {
                    output.push(*byte as char);
                }
            }
        } else if token(value) {
            output.push('=');
            output.push_str(std::str::from_utf8(value).unwrap());
        } else {
            output.push_str("=\"");
            for byte in value {
                if matches!(byte, b'"' | b'\\') {
                    output.push('\\');
                }
                output.push(*byte as char);
            }
            output.push('"');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn registration_media_parameters_match_pinned_go() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/mime-registration-source-go127.json"))
        .unwrap();
        let mut types = Registry::builtins();
        for case in cases.as_array().unwrap() {
            let extension = case["extension"].as_str().unwrap();
            let registered = types.set(extension, case["mime"].as_str().unwrap());
            assert_eq!(
                registered,
                case["accepted"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            for (query, expected) in case["queries"].as_object().unwrap() {
                assert_eq!(
                    types.get(query).unwrap_or_default(),
                    expected.as_str().unwrap(),
                    "{}: {query}",
                    case["name"]
                );
            }
        }
    }

    #[test]
    fn globs_first_exact_case_wins_and_lowercase_map_keeps_registration_order() {
        let mut types = Registry::builtins();
        types.load_reader(&b"50:wrong/type:*.png\n50:application/a:*.d2Case\n40:application/b:*.d2case\n30:application/c:*.d2Case\n50:text/x-other:*.d2[0-9]\n"[..], true).unwrap();
        assert_eq!(types.get(".png"), Some("image/png"));
        assert_eq!(types.get(".d2Case"), Some("application/a"));
        assert_eq!(types.get(".d2case"), Some("application/b"));
        assert_eq!(types.get(".D2CASE"), Some("application/b"));
        assert!(types.get(".d2[0-9]").is_none());
    }

    #[test]
    fn types_files_overwrite_builtins_and_stop_at_inline_comments() {
        let mut types = Registry::builtins();
        types.load_reader(&b"# comment\napplication/custom png d2a # d2ignored\ntext/x-Later d2a\nnot/a/type d2invalid\n"[..], false).unwrap();
        assert_eq!(types.get(".png"), Some("application/custom"));
        assert_eq!(types.get(".d2a"), Some("text/x-later; charset=utf-8"));
        assert!(types.get(".d2ignored").is_none());
        assert!(types.get(".d2invalid").is_none());
    }

    #[test]
    fn scanner_rejects_overlong_tokens_and_keeps_crlf_and_final_lines() {
        let mut types = Registry::builtins();
        types
            .load_reader(
                &b"application/one d2one\r\napplication/two d2two\r"[..],
                false,
            )
            .unwrap();
        assert_eq!(types.get(".d2two"), Some("application/two"));
        assert!(
            types
                .load_reader(&vec![b'a'; 65536][..], true)
                .unwrap_err()
                .to_string()
                .contains("token too long")
        );
    }
}
