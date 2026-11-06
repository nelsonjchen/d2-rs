// SPDX-License-Identifier: MPL-2.0
// Go math/big behavior translated under the Go BSD notice in THIRD_PARTY_NOTICES.md.
//! Exact numeric acceptance used by pinned d2parser.parseValue.
use d2_rs_core::ast::{BigInt, BigRational};

fn sign(bytes: &[u8]) -> (bool, &[u8]) {
    match bytes.first() {
        Some(b'-') => (true, &bytes[1..]),
        Some(b'+') => (false, &bytes[1..]),
        _ => (false, bytes),
    }
}
// Scan base-zero naturals as Go nat.scan does, returning the unread suffix.
fn scan(bytes: &[u8], fractional: bool) -> Option<(BigInt, u32, i64, &[u8])> {
    let mut base = 10;
    let mut prefix = 0;
    let mut index = 0;
    let mut count = 0i64;
    let mut prev_digit = false;
    let mut prev_separator = false;
    let mut invalid_separator = false;
    if bytes.first() == Some(&b'0') {
        index = 1;
        count = 1;
        prev_digit = true;
        if let Some(next) = bytes.get(1) {
            match next {
                b'b' | b'B' => {
                    base = 2;
                    prefix = 2;
                }
                b'o' | b'O' => {
                    base = 8;
                    prefix = 2;
                }
                b'x' | b'X' => {
                    base = 16;
                    prefix = 2;
                }
                _ if !fractional => {
                    base = 8;
                    prefix = 1;
                }
                _ => {}
            }
            if prefix != 0 {
                count = 0;
                index = prefix;
            }
        }
    }
    let mut digits = Vec::new();
    let mut point = None;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'.' && fractional && point.is_none() {
            if prev_separator {
                invalid_separator = true;
            }
            prev_digit = false;
            prev_separator = false;
            point = Some(count);
        } else if byte == b'_' {
            if !prev_digit {
                invalid_separator = true;
            }
            prev_digit = false;
            prev_separator = true;
        } else {
            let digit = match byte {
                b'0'..=b'9' => (byte - b'0') as u32,
                b'a'..=b'z' => (byte - b'a' + 10) as u32,
                b'A'..=b'Z' => (byte - b'A' + 10) as u32,
                _ => u32::MAX,
            };
            if digit >= base {
                break;
            }
            prev_digit = true;
            prev_separator = false;
            count += 1;
            digits.push(byte);
        }
        index += 1;
    }
    if invalid_separator || prev_separator {
        return None;
    }
    if count == 0 {
        if prefix == 1 {
            return Some((BigInt::from(0), 10, 0, &bytes[index..]));
        }
        return None;
    }
    let value = if digits.is_empty() {
        BigInt::from(0)
    } else {
        BigInt::parse_bytes(&digits, base)?
    };
    Some((value, base, point.map_or(0, |p| p - count), &bytes[index..]))
}
fn integer(bytes: &[u8]) -> Option<BigInt> {
    let (negative, bytes) = sign(bytes);
    let (value, _, _, rest) = scan(bytes, false)?;
    if !rest.is_empty() {
        return None;
    }
    Some(if negative { -value } else { value })
}
fn exponent(bytes: &[u8]) -> Option<(i64, u32)> {
    if bytes.is_empty() {
        return Some((0, 10));
    }
    let base = match bytes[0] {
        b'e' | b'E' => 10,
        b'p' | b'P' => 2,
        _ => return None,
    };
    let (negative, bytes) = sign(&bytes[1..]);
    let mut digits = String::new();
    let mut previous = false;
    if negative {
        digits.push('-');
    }
    for &byte in bytes {
        if byte.is_ascii_digit() {
            digits.push(byte as char);
            previous = true;
        } else if byte == b'_' && previous {
            previous = false;
        } else {
            return None;
        }
    }
    if !previous {
        return None;
    }
    Some((digits.parse().ok()?, base))
}
/// Go 1.27.0 big.Rat.SetString semantics, restricted to the string result.
pub fn parse_rational(text: &str) -> Option<BigRational> {
    let bytes = text.as_bytes();
    if let Some(slash) = bytes.iter().position(|b| *b == b'/') {
        let numerator = integer(&bytes[..slash])?;
        let (denominator, _, _, rest) = scan(&bytes[slash + 1..], false)?;
        if !rest.is_empty() || denominator == BigInt::from(0) {
            return None;
        }
        return Some(BigRational::new(numerator, denominator));
    }
    let (negative, bytes) = sign(bytes);
    let (mut numerator, base, fraction, rest) = scan(bytes, true)?;
    let (exponent, exponent_base) = exponent(rest)?;
    if numerator == BigInt::from(0) {
        return Some(BigRational::from_integer(numerator));
    }
    let mut exp5 = if base == 10 { fraction } else { 0 };
    let mut exp2 = fraction.wrapping_mul(match base {
        8 => 3,
        16 => 4,
        _ => 1,
    });
    if exponent_base == 10 {
        exp5 = exp5.wrapping_add(exponent);
    }
    exp2 = exp2.wrapping_add(exponent);
    let power5 = exp5.checked_abs()?;
    if power5 > 1_000_000 || !(-10_000_000..=10_000_000).contains(&exp2) {
        return None;
    }
    let mut denominator = BigInt::from(1);
    if power5 != 0 {
        let power = BigInt::from(5).pow(power5 as u32);
        if exp5 > 0 {
            numerator *= power;
        } else {
            denominator = power;
        }
    }
    if exp2 > 0 {
        numerator <<= exp2 as usize;
    } else if exp2 < 0 {
        denominator <<= (-exp2) as usize;
    }
    if negative {
        numerator = -numerator;
    }
    Some(BigRational::new(numerator, denominator))
}

#[cfg(test)]
#[cfg(feature = "oracle-fixtures")]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[test]
    fn pinned_go_numeric_regressions() {
        let report: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/numeric.json"
        ))
        .unwrap();
        let requests = report["requests"].as_array().unwrap();
        let expected = report["reference"].as_array().unwrap();
        assert_eq!(requests.len(), expected.len());
        for (index, (request, expected)) in requests.iter().zip(expected).enumerate() {
            let bytes: Vec<u8> = request["hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
                .collect();
            let rational = std::str::from_utf8(&bytes).ok().and_then(parse_rational);
            let actual =
                json!({"accepted":rational.is_some(),"rational":rational.map(|r|r.to_string())});
            assert_eq!(&actual, expected, "case {index}: {request}");
        }
    }
}
