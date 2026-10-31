// SPDX-License-Identifier: MPL-2.0
// D2 v0.9.0 d2scenebuild/latex.go validateLatexInput.
use super::RenderError;
pub(super) fn validate(object: &str, label: &[u8]) -> Result<(), RenderError> {
    if std::str::from_utf8(label).is_err() {
        return Err(RenderError::InvalidLabelUtf8 {
            object: object.to_owned(),
        });
    }
    if label.len() > 4096 {
        return Err(RenderError::LatexInput {
            object: object.to_owned(),
            bytes: label.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_latex_input_utf8_and_byte_boundaries() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/latex-input-admission-v090.json"))
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
