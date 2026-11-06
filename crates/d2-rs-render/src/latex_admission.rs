// SPDX-License-Identifier: MPL-2.0
// D2 d2scenebuild/latex.go validateLatexInput and d2latex/limits.go.
use super::RenderError;
pub(super) fn validate(object: &str, label: &[u8]) -> Result<(), RenderError> {
    if label.len() > 4096 {
        return Err(RenderError::LatexInput {
            object: object.to_owned(),
            bytes: label.len(),
        });
    }
    if std::str::from_utf8(label).is_err() {
        return Err(RenderError::LatexInvalidUtf8 {
            object: object.to_owned(),
        });
    }
    let (mut depth, mut backslashes, mut in_comment) = (0usize, 0usize, false);
    for &c in label {
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
                if depth > 128 {
                    return Err(RenderError::LatexNesting {
                        object: object.to_owned(),
                        depth,
                    });
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
    fn nesting_respects_escapes_and_comments() {
        assert!(validate("shape", "{".repeat(128).as_bytes()).is_ok());
        assert_eq!(
            validate("shape", "{".repeat(129).as_bytes())
                .unwrap_err()
                .to_string(),
            "scene: shape latex group nesting depth 129 exceeds limit 128"
        );
        assert!(validate("shape", format!("\\{{%{}\n}}", "{".repeat(129)).as_bytes()).is_ok());
        assert!(validate("shape", format!("\\\\{}", "{".repeat(129)).as_bytes()).is_err());
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_latex_input_utf8_and_byte_boundaries() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/latex-input-admission-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 30);
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let hex = r["unit_hex"].as_str().unwrap();
            let unit: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let label = unit.repeat(r["repeat"].as_u64().unwrap() as usize);
            let error = validate(r["object"].as_str().unwrap(), &label)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert_eq!(error, row["error"].as_str().unwrap(), "{}", r["name"]);
        }
    }
}
