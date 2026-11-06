// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Byte-exact translations of pinned d2parser indentation helpers.
use super::{decode_rune, is_space};

/// The upstream slice offset counts runes, despite indexing a byte string.
/// Keep the remainder as bytes: non-ASCII whitespace can split a UTF-8 rune.
pub fn split_leading_indent(s: &[u8], max_spaces: i64) -> (Vec<u8>, &[u8]) {
    let mut indent = Vec::new();
    let mut byte_offset = 0;
    let mut rune_count = 0;
    while byte_offset < s.len() {
        let (r, width) = decode_rune(&s[byte_offset..]);
        if !is_space(r) {
            break;
        }
        byte_offset += width;
        rune_count += 1;
        if r == '\t' {
            indent.extend_from_slice(b"  ");
        } else {
            let mut buffer = [0; 4];
            indent.extend_from_slice(r.encode_utf8(&mut buffer).as_bytes());
        }
        if max_spaces > -1 && indent.len() as i64 == max_spaces {
            break;
        }
    }
    (indent, &s[rune_count..])
}

pub fn trim_indent(s: &[u8], indent: &[u8]) -> Vec<u8> {
    let lines: Vec<_> = s
        .split(|b| *b == b'\n')
        .map(|line| {
            if line.is_empty() {
                line
            } else {
                split_leading_indent(line, indent.len() as i64).1
            }
        })
        .collect();
    lines.join(&b'\n')
}

pub fn trim_common_indent(s: &[u8]) -> Vec<u8> {
    let mut common = Vec::new();
    for line in s.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let (indent, remainder) = split_leading_indent(line, -1);
        if indent.is_empty() {
            return s.to_vec();
        }
        if remainder.is_empty() {
            continue;
        }
        if common.is_empty() || indent.len() < common.len() {
            common = indent;
        }
    }
    if common.is_empty() {
        s.to_vec()
    } else {
        trim_indent(s, &common)
    }
}

fn trim_right_space(mut s: &[u8]) -> &[u8] {
    while !s.is_empty() {
        let mut last = ('\u{fffd}', 1);
        for width in 1..=s.len().min(4) {
            let tail = &s[s.len() - width..];
            if let Ok(text) = std::str::from_utf8(tail) {
                let mut chars = text.chars();
                let r = chars.next().unwrap();
                if chars.next().is_none() {
                    last = (r, width);
                    break;
                }
            }
        }
        if !is_space(last.0) {
            break;
        }
        s = &s[..s.len() - last.1];
    }
    s
}

pub fn trim_space_after_last_newline(s: &[u8]) -> Vec<u8> {
    let Some(last) = s.iter().rposition(|b| *b == b'\n') else {
        return trim_right_space(s).to_vec();
    };
    let tail = trim_right_space(&s[last + 1..]);
    if tail.is_empty() {
        s[..last].to_vec()
    } else {
        [&s[..last + 1], tail].concat()
    }
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    #[test]
    fn pinned_go_indentation_bytes() {
        let report: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/indent.json"
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
            let maximum = r["max"].as_i64().unwrap();
            let (indent, rest) = split_leading_indent(&bytes, maximum);
            let actual = json!({"indent":hex(&indent),"remainder":hex(rest),
                "trim":hex(&trim_indent(&bytes,&vec![b' ';(maximum+1) as usize])),
                "common":hex(&trim_common_indent(&bytes)),"last":hex(&trim_space_after_last_newline(&bytes))});
            assert_eq!(&actual, expected, "case {index}: {r}");
        }
    }
}
