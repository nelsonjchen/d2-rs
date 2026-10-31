// SPDX-License-Identifier: MPL-2.0
// Copyright 2009 The Go Authors. All rights reserved.
// Go-derived lexical and hexadecimal conversion portions retain BSD-3-Clause
// terms in THIRD_PARTY_NOTICES.md. Source: Go 1.27 internal/strconv/{atof,atoi}.go.

/// D2's strconv.ParseFloat(value, 64) contract. Syntax and overflow errors
/// return None; underflow, signed zero and literal infinities remain values.
/// Decimal conversion uses Rust's correctly rounded parser after Go lexing.
#[doc(hidden)]
pub fn parse_d2_float(value: &str) -> Option<f64> {
    parse_d2_float_result(value).ok()
}

/// The same conversion, retaining strconv's syntax/range distinction for CLI
/// diagnostics. Existing numeric callers continue to use the Option wrapper.
#[doc(hidden)]
pub fn parse_d2_float_result(value: &str) -> Result<f64, &'static str> {
    let mut overflow = false;
    parse_d2_float_inner(value, &mut overflow).ok_or(if overflow {
        "value out of range"
    } else {
        "invalid syntax"
    })
}

fn parse_d2_float_inner(value: &str, overflow: &mut bool) -> Option<f64> {
    if value.eq_ignore_ascii_case("nan") {
        return Some(f64::from_bits(0x7ff8_0000_0000_0001));
    }
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let negative = bytes.first() == Some(&b'-');
    let unsigned = value.get(index..)?;
    if unsigned.eq_ignore_ascii_case("inf") || unsigned.eq_ignore_ascii_case("infinity") {
        return Some(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let hex = bytes.get(index) == Some(&b'0')
        && bytes
            .get(index + 1)
            .is_some_and(|c| c.eq_ignore_ascii_case(&b'x'))
        && index + 2 < bytes.len();
    let base = if hex { 16 } else { 10 };
    let maximum_digits = if hex { 16 } else { 19 };
    if hex {
        index += 2;
    }
    let mut mantissa = 0u64;
    let mut digits = 0i64;
    let mut retained_digits = 0i64;
    let mut point = 0i64;
    let mut saw_dot = false;
    let mut saw_digits = false;
    let mut truncated = false;
    let mut underscores = false;
    while let Some(&c) = bytes.get(index) {
        if c == b'_' {
            underscores = true;
            index += 1;
            continue;
        }
        if c == b'.' {
            if saw_dot {
                break;
            }
            saw_dot = true;
            point = digits;
            index += 1;
            continue;
        }
        let digit = if c.is_ascii_digit() {
            c - b'0'
        } else if hex && (b'a'..=b'f').contains(&c.to_ascii_lowercase()) {
            c.to_ascii_lowercase() - b'a' + 10
        } else {
            break;
        };
        saw_digits = true;
        index += 1;
        if digit == 0 && digits == 0 {
            point -= 1;
            continue;
        }
        digits += 1;
        if retained_digits < maximum_digits {
            mantissa = mantissa * base + u64::from(digit);
            retained_digits += 1;
        } else if digit != 0 {
            truncated = true;
        }
    }
    if !saw_digits {
        return None;
    }
    if !saw_dot {
        point = digits;
    }
    if hex {
        point *= 4;
        retained_digits *= 4;
    }
    let exponent_marker = if hex { b'p' } else { b'e' };
    if bytes
        .get(index)
        .is_some_and(|c| c.eq_ignore_ascii_case(&exponent_marker))
    {
        index += 1;
        let exponent_negative = bytes.get(index) == Some(&b'-');
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        if !bytes.get(index).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        let mut exponent = 0i64;
        while let Some(&c) = bytes.get(index) {
            if c == b'_' {
                underscores = true;
            } else if c.is_ascii_digit() {
                if exponent < 10000 {
                    exponent = exponent * 10 + i64::from(c - b'0');
                }
            } else {
                break;
            }
            index += 1;
        }
        point += if exponent_negative {
            -exponent
        } else {
            exponent
        };
    } else if hex {
        return None;
    }
    if index != bytes.len() || (underscores && !underscore_ok(bytes)) {
        return None;
    }
    if hex {
        let exponent = if mantissa != 0 {
            point - retained_digits
        } else {
            0
        };
        let number = from_hex(mantissa, exponent, negative, truncated);
        *overflow = number.is_none();
        number
    } else {
        let number = if underscores {
            value.replace('_', "").parse::<f64>().ok()?
        } else {
            value.parse::<f64>().ok()?
        };
        *overflow = !number.is_finite();
        number.is_finite().then_some(number)
    }
}

fn underscore_ok(mut bytes: &[u8]) -> bool {
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        bytes = &bytes[1..];
    }
    let hex = bytes.len() >= 2 && bytes[0] == b'0' && bytes[1].eq_ignore_ascii_case(&b'x');
    let mut previous = if hex { b'0' } else { b'^' };
    if hex {
        bytes = &bytes[2..];
    }
    for &c in bytes {
        if c.is_ascii_digit() || (hex && (b'a'..=b'f').contains(&c.to_ascii_lowercase())) {
            previous = b'0';
        } else if c == b'_' {
            if previous != b'0' {
                return false;
            }
            previous = b'_';
        } else {
            if previous == b'_' {
                return false;
            }
            previous = b'!';
        }
    }
    previous != b'_'
}

fn from_hex(mut mantissa: u64, mut exponent: i64, negative: bool, truncated: bool) -> Option<f64> {
    exponent += 52;
    while mantissa != 0 && mantissa >> 54 == 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    if truncated {
        mantissa |= 1;
    }
    while mantissa >> 55 != 0 {
        mantissa = mantissa >> 1 | mantissa & 1;
        exponent += 1;
    }
    while mantissa > 1 && exponent < -1024 {
        mantissa = mantissa >> 1 | mantissa & 1;
        exponent += 1;
    }
    let mut round = mantissa & 3;
    mantissa >>= 2;
    round |= mantissa & 1;
    exponent += 2;
    if round == 3 {
        mantissa += 1;
        if mantissa == 1 << 53 {
            mantissa >>= 1;
            exponent += 1;
        }
    }
    if mantissa >> 52 == 0 {
        exponent = -1023;
    }
    if exponent > 1023 {
        return None;
    }
    let bits = (mantissa & ((1 << 52) - 1))
        | (((exponent + 1023) as u64 & 2047) << 52)
        | (u64::from(negative) << 63);
    Some(f64::from_bits(bits))
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_float_syntax_and_ieee_bits() {
        let cases: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/go-float-v127.json")).unwrap();
        let cases = cases.as_array().unwrap();
        assert!(cases.len() > 1000);
        for case in cases {
            let input = case["input"].as_str().unwrap();
            if let Err(reason) = super::parse_d2_float_result(input) {
                assert_eq!(
                    format!(
                        "strconv.ParseFloat: parsing {}: {reason}",
                        crate::quote_go_bytes(input.as_bytes())
                    ),
                    case["error"].as_str().unwrap(),
                    "{input:?}"
                );
            }
            let actual = super::parse_d2_float(input);
            assert_eq!(
                actual.is_some(),
                case["valid"].as_bool().unwrap(),
                "{input:?}"
            );
            if let Some(actual) = actual {
                assert_eq!(
                    format!("{:016x}", actual.to_bits()),
                    case["bits"].as_str().unwrap(),
                    "{input:?}"
                );
            }
        }
    }
}

/// strconv.FormatFloat(value, 'e', -1, 64) digits: the shortest decimal
/// that round-trips, choosing the correctly rounded (ties-to-even) candidate
/// when two of that length round-trip. Rust's shortest `{:e}` breaks such
/// exact ties upward (e.g. float32 132.6 as f64 needs 17 digits and is
/// exactly halfway), so recompute at the same length with Rust's exact,
/// ties-to-even `{:.Ne}` and keep it when it round-trips.
fn go_shortest_e(value: f64) -> String {
    let shortest = format!("{value:e}");
    let (mantissa, _) = shortest.split_once('e').expect("scientific float");
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).count();
    let exact = format!("{value:.*e}", digits.saturating_sub(1));
    if exact.parse::<f64>().ok() == Some(value) {
        exact
    } else {
        shortest
    }
}

/// encoding/json's float64 encoder (floatEncoder with bits 64): 'f' format
/// in [1e-6, 1e21), otherwise 'e' with a two-digit-minimum exponent cleaned
/// back to `e-7`, as Go 1.27 writes JSON numbers.
pub fn go_json_float(value: f64) -> String {
    let abs = value.abs();
    let e = go_shortest_e(value);
    let (mantissa, exponent) = e.split_once('e').expect("scientific float");
    let exponent: i32 = exponent.parse().expect("float exponent");
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if abs != 0.0 && (abs < 1e-6 || abs >= 1e21) {
        let mut out = String::new();
        if negative {
            out.push('-');
        }
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let sign = if exponent < 0 { '-' } else { '+' };
        // strconv writes at least two exponent digits; json trims e-0X.
        let magnitude = exponent.unsigned_abs();
        if sign == '-' && magnitude < 10 {
            out.push_str(&format!("e-{magnitude}"));
        } else {
            out.push_str(&format!("e{sign}{magnitude:02}"));
        }
        return out;
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    // 'f' format with the shortest digits.
    let point = exponent + 1;
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if point <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-point) as usize));
        out.push_str(&digits);
    } else if point as usize >= digits.len() {
        out.push_str(&digits);
        out.push_str(&"0".repeat(point as usize - digits.len()));
    } else {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    }
    out
}

#[cfg(test)]
mod json_float_tests {
    use super::go_json_float;

    #[test]
    fn matches_go_encoding_json() {
        for (value, expected) in [
            (f64::from(132.6f32), "132.60000610351562"),
            (f64::from(121.8f32), "121.80000305175781"),
            (26.5, "26.5"),
            (0.0, "0"),
            (-0.0, "-0"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (1.5e-10, "1.5e-10"),
            (123456789.0, "123456789"),
            (0.1, "0.1"),
            (1e20, "100000000000000000000"),
            (0.000001, "0.000001"),
            (-2.5e-7, "-2.5e-7"),
        ] {
            assert_eq!(go_json_float(value), expected, "{value:e}");
        }
    }
}
