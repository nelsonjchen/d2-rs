// SPDX-License-Identifier: MPL-2.0
//! d2renderers/d2latex/limits.go. Validate before recursive MathJax parsing.
use crate::MathError;

/// Maximum UTF-8 byte length admitted by D2's LaTeX renderer.
pub const MAX_INPUT_BYTES: usize = 4 << 10;
/// Maximum unescaped, uncommented brace-group nesting.
pub const MAX_GROUP_NESTING_DEPTH: usize = 128;

/// D2's shared LaTeX admission policy. Rust strings already guarantee UTF-8.
pub fn validate_input(input: &str) -> Result<(), MathError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(MathError::InputLimit(format!(
            "latex input is {} bytes, exceeding limit {MAX_INPUT_BYTES}",
            input.len()
        )));
    }
    let (mut depth, mut backslashes, mut in_comment) = (0usize, 0usize, false);
    for &c in input.as_bytes() {
        if in_comment {
            if c == b'\n' || c == b'\r' {
                in_comment = false;
            }
            continue;
        }
        if c == b'\\' {
            backslashes += 1;
            continue;
        }
        let escaped = backslashes % 2 != 0;
        backslashes = 0;
        if escaped {
            continue;
        }
        match c {
            b'%' => in_comment = true,
            b'{' => {
                depth += 1;
                if depth > MAX_GROUP_NESTING_DEPTH {
                    return Err(MathError::InputLimit(format!(
                        "latex group nesting depth {depth} exceeds limit {MAX_GROUP_NESTING_DEPTH}"
                    )));
                }
            }
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_and_group_limits_precede_parsing() {
        assert!(validate_input(&"x".repeat(MAX_INPUT_BYTES)).is_ok());
        assert_eq!(
            validate_input(&"x".repeat(MAX_INPUT_BYTES + 1))
                .unwrap_err()
                .to_string(),
            "latex input is 4097 bytes, exceeding limit 4096"
        );
        let nested = format!("{}x{}", "{".repeat(128), "}".repeat(128));
        assert!(validate_input(&nested).is_ok());
        let too_deep = format!("{{{nested}}}");
        for check in [crate::render, crate::render_exact] {
            assert_eq!(
                check(&too_deep).unwrap_err().to_string(),
                "latex group nesting depth 129 exceeds limit 128"
            );
        }
        assert_eq!(
            crate::measure(&too_deep).unwrap_err().to_string(),
            "latex group nesting depth 129 exceeds limit 128"
        );
        assert!(
            validate_input(&"é".repeat(2049))
                .unwrap_err()
                .to_string()
                .contains("4098 bytes")
        );
    }
    #[test]
    fn escape_parity_comments_and_unmatched_closes_follow_source() {
        assert!(validate_input(&r"\{".repeat(129)).is_ok());
        assert!(
            validate_input(&format!(
                "{}%{}\r{}",
                "{".repeat(128),
                "{".repeat(129),
                "}".repeat(128)
            ))
            .is_ok()
        );
        assert!(validate_input(&format!("{}\\%{{", "{".repeat(128))).is_err());
        assert!(
            validate_input(&format!("{}\\\\%{{\n{}", "{".repeat(128), "}".repeat(128))).is_ok()
        );
        assert!(validate_input(&format!("}}}}{}", "{".repeat(128))).is_ok());
    }
}
