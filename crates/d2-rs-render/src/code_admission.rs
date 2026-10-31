// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2scenebuild/code_shape.go validateCodeText.
use super::RenderError;

pub(super) fn validate(
    object: &str,
    label: &[u8],
    language: &str,
    font_size: i64,
) -> Result<(), RenderError> {
    if label.is_empty() || language.is_empty() {
        return Ok(());
    }
    let label_text = std::str::from_utf8(label).map_err(|_| RenderError::InvalidLabelUtf8 {
        object: object.to_owned(),
    })?;
    let invalid = |field, value, requirement| RenderError::CodeInput {
        object: object.to_owned(),
        field,
        value,
        requirement,
    };
    if label.len() > 1 << 20 {
        return Err(invalid(
            "label",
            label.len() as i64,
            "code source bytes must not exceed 1048576",
        ));
    }
    let runes = label_text.chars().count();
    if runes > 100_000 {
        return Err(invalid(
            "label",
            runes as i64,
            "code source runes must not exceed 100000",
        ));
    }
    if font_size <= 0 {
        return Err(invalid(
            "fontSize",
            font_size,
            "must be positive for a syntax-highlighted label",
        ));
    }
    if font_size > i64::from(i32::MAX) / 64 {
        return Err(invalid(
            "fontSize",
            font_size,
            "must fit raster font scaling",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_code_input_errors_and_boundaries() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/code-input-admission-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 61);
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let hex = r["unit_hex"].as_str().unwrap();
            let unit: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let label = unit.repeat(r["repeat"].as_u64().unwrap() as usize);
            let error = validate(
                r["object"].as_str().unwrap(),
                &label,
                r["language"].as_str().unwrap(),
                r["font_size"].as_i64().unwrap(),
            )
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
            assert_eq!(error, row["error"].as_str().unwrap(), "{}", r["name"]);
        }
    }
}
