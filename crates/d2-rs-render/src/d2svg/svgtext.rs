// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 lib/svg/number.go and lib/svg/text.go.

/// svg.FormatFloat: integral values print as integers (`-0` for negative
/// zero); others use strconv.FormatFloat(v, 'f', 6, 64) with trailing zeros
/// and a trailing '.' trimmed. Rust's `{:.6}` matches Go's exact
/// round-half-to-even decimal conversion.
pub(crate) fn format_float(value: f64) -> String {
    // 2^63; float64(math.MaxInt64) rounds up to it, so the bound is exclusive.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if (-LIMIT..LIMIT).contains(&value) {
        let integer = value as i64;
        if value == integer as f64 {
            if integer == 0 && value.is_sign_negative() {
                return "-0".to_owned();
            }
            return integer.to_string();
        }
    }
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "+Inf" } else { "-Inf" }.to_owned();
    }
    let mut s = format!("{value:.6}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.').len();
        s.truncate(trimmed);
    }
    s
}

/// encoding/xml.EscapeText, as used by svg.EscapeText.
pub(crate) fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c if !is_in_character_range(c) => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// encoding/xml.isInCharacterRange.
fn is_in_character_range(c: char) -> bool {
    let r = c as u32;
    r == 0x09
        || r == 0x0A
        || r == 0x0D
        || (0x20..=0xD7FF).contains(&r)
        || (0xE000..=0xFFFD).contains(&r)
        || (0x10000..=0x10FFFF).contains(&r)
}

/// svg.ScopedID, shared with the public typed markup API.
pub(crate) fn scoped_id(prefix: &str, component: &str, suffix: &str) -> String {
    super::markup::scoped_id(prefix, component, suffix)
        .as_str()
        .to_owned()
}

/// svg.SVGID: unpadded standard base32 of the text.
pub(crate) fn svg_id(text: &str) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    for chunk in bytes.chunks(5) {
        let mut buffer = [0u8; 5];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let bits = buffer
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
        let symbols = (chunk.len() * 8).div_ceil(5);
        for index in 0..symbols {
            let shift = 35 - index * 5;
            out.push(ALPHABET[((bits >> shift) & 31) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_float_matches_go() {
        for (value, expected) in [
            (0.0, "0"),
            (-0.0, "-0"),
            (26.5, "26.5"),
            (121.80000305175781, "121.800003"),
            (0.0078125, "0.007812"),
            (-1e-9, "-0"),
            (1e21, "1000000000000000000000"),
            (1.0 / 3.0, "0.333333"),
        ] {
            assert_eq!(format_float(value), expected, "{value}");
        }
    }

    #[test]
    fn escape_text_matches_go_xml() {
        assert_eq!(
            escape_text("a\"b'c&<>\t\n\r"),
            "a&#34;b&#39;c&amp;&lt;&gt;&#x9;&#xA;&#xD;"
        );
        assert_eq!(escape_text("\u{1}x"), "\u{FFFD}x");
    }

    #[test]
    fn svg_id_is_unpadded_base32() {
        assert_eq!(svg_id("a"), "ME");
        assert_eq!(svg_id("hello"), "NBSWY3DP");
        assert_eq!(svg_id("x.y"), "PAXHS");
    }
}
