// SPDX-License-Identifier: MPL-2.0
// CSS stop acceptance translates csscolorparser v0.1.8 (MIT), Copyright (c)
// 2020 Nor Khasyatillah. See THIRD_PARTY_NOTICES.md for the preserved license.
//! D2 lib/color.ValidColor, including its distinct ordinary/gradient grammars.
use d2_rs_core::{ir::go_lowercase, parse_d2_float};

fn lower(value: &str) -> String {
    String::from_utf8(go_lowercase(value.as_bytes())).expect("Unicode lowercase is UTF-8")
}

fn named(value: &str) -> bool {
    super::config::D2_NAMED_COLORS
        .split_ascii_whitespace()
        .any(|name| name == value)
}

fn hex(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len()) && value.bytes().all(|c| c.is_ascii_hexdigit())
}

// Go unicode.IsSpace/strings.Fields/TrimSpace. In particular, neither BOM nor
// zero-width space is whitespace; CSS FieldsFunc below has narrower separators.
fn space(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}')
}

pub(super) fn valid_color(value: &str) -> bool {
    let gradient = value
        .strip_prefix("linear-gradient(")
        .map(|params| (false, params))
        .or_else(|| {
            value
                .strip_prefix("radial-gradient(")
                .map(|params| (true, params))
        })
        .and_then(|(radial, params)| params.strip_suffix(')').map(|params| (radial, params)))
        .filter(|(_, params)| !params.is_empty() && !params.contains('\n'));
    let Some((radial, params)) = gradient else {
        return named(&lower(value))
            || value
                .strip_prefix('#')
                .is_some_and(|digits| hex(digits, &[3, 6]));
    };
    // D2 splitParams keeps empty comma-separated entries but omits a final
    // empty buffer, and never rejects an unmatched inner parenthesis itself.
    let mut list = Vec::new();
    let mut nesting = 0usize;
    let mut start = 0;
    for (index, c) in params.char_indices() {
        match c {
            ',' if nesting == 0 => {
                list.push(&params[start..index]);
                start = index + 1;
            }
            '(' => nesting += 1,
            ')' if nesting > 0 => nesting -= 1,
            _ => {}
        }
    }
    if start < params.len() {
        list.push(&params[start..]);
    }
    let Some(first) = list.first() else {
        return false;
    };
    let first = first.trim_matches(space);
    let has_direction = if radial {
        matches!(first, "circle" | "ellipse")
    } else {
        first.ends_with("deg") || first.starts_with("to ")
    };
    let stops = if has_direction {
        if list.len() < 2 {
            return false;
        }
        &list[1..]
    } else {
        &list[..]
    };
    stops.iter().all(|stop| {
        let fields: Vec<_> = stop
            .split(space)
            .filter(|field| !field.is_empty())
            .collect();
        // Master parseColorStops rejects malformed entries and requires a
        // finite numeric position before canonicalizing it for SVG output.
        matches!(fields.len(), 1 | 2)
            && css_stop(fields[0])
            && (fields.len() == 1
                || parse_d2_float(fields[1].strip_suffix('%').unwrap_or(fields[1]))
                    .is_some_and(f64::is_finite))
    })
}

fn number(value: &str) -> bool {
    parse_d2_float(value.trim_matches(space)).is_some()
}

fn percentage_or_number(value: &str) -> bool {
    number(value.strip_suffix('%').unwrap_or(value))
}

fn angle(value: &str) -> bool {
    for suffix in ["deg", "grad", "rad", "turn"] {
        if let Some(value) = value.strip_suffix(suffix) {
            return number(value);
        }
    }
    number(value)
}

fn css_stop(raw: &str) -> bool {
    let value = lower(raw);
    let value = value.trim_matches(space);
    if value != "currentcolor" && named(value) {
        return true;
    }
    if let Some(digits) = value.strip_prefix('#') {
        return hex(digits, &[3, 4, 6, 8]);
    }
    if let Some(open) = value.find('(')
        && let Some(inner) = value[open + 1..].strip_suffix(')')
    {
        let function = value[..open].trim_matches(space);
        let parts: Vec<_> = inner
            .split([',', '/', ' '])
            .filter(|p| !p.is_empty())
            .collect();
        if !matches!(parts.len(), 3 | 4) {
            return false;
        }
        if parts.len() == 4 && !percentage_or_number(parts[3]) {
            return false;
        }
        return match function {
            "rgb" | "rgba" | "oklab" | "lab" => parts[..3].iter().all(|v| percentage_or_number(v)),
            "hsl" | "hsla" | "hwb" | "hwba" | "hsv" | "hsva" => {
                angle(parts[0]) && percentage_or_number(parts[1]) && percentage_or_number(parts[2])
            }
            "oklch" | "lch" => {
                percentage_or_number(parts[0]) && percentage_or_number(parts[1]) && angle(parts[2])
            }
            _ => false,
        };
    }
    hex(value, &[3, 4, 6, 8])
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn original_go_ordinary_colors_and_gradient_stop_acceptance() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/style-color-go-master.json"
        ))
        .unwrap();
        let cases = fixture.as_array().unwrap();
        assert_eq!(cases.len(), 807);
        for case in cases {
            let input = case["input"].as_str().unwrap();
            assert_eq!(
                super::valid_color(input),
                case["valid"].as_bool().unwrap(),
                "{input:?}"
            );
        }
    }
}
