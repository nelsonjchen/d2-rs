// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/gradient.go
// stopFromDeclarations and validateStylesheetRules.
use super::{
    svg_color, svg_declarations::Rule, svg_length, svg_primitive::Stop, svg_resources::Resource,
    svg_style,
};
use std::collections::BTreeMap;
pub fn from_declarations(source: &str, values: &BTreeMap<String, String>) -> Result<Stop, String> {
    let error = |name: &str, reason: String| {
        format!("d2svgimport: {source}: element <stop> has invalid {name}: {reason}")
    };
    let mut color = [0, 0, 0, 255];
    if let Some(raw) = values.get("stop-color") {
        let lower = String::from_utf8(d2_rs_core::ir::go_lowercase(raw.as_bytes())).unwrap();
        if raw.eq_ignore_ascii_case("inherit")
            || raw.eq_ignore_ascii_case("currentColor")
            || lower.contains("url(")
        {
            return Err(error(
                "stop-color",
                "only an explicit solid color is supported".into(),
            ));
        }
        color = svg_color::solid(raw).map_err(|e| error("stop-color", e))?;
    }
    let mut opacity = 1.;
    if let Some(raw) = values.get("stop-opacity") {
        if raw.eq_ignore_ascii_case("inherit") {
            return Err(error("stop-opacity", "inheritance is unsupported".into()));
        }
        opacity = svg_length::unit_interval(raw).map_err(|e| error("stop-opacity", e))?;
    }
    color[3] = (f64::from(color[3]) * opacity).round() as u8;
    Ok(Stop { offset: 0., color })
}
pub fn validate_rules(
    source: &str,
    rules: &[Rule],
    resources: &BTreeMap<String, Resource>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    for rule in rules {
        if let Some(cause) = cancel.and_then(d2_rs_core::CancellationToken::error) {
            return Err(
                if matches!(cause, d2_rs_core::LayoutError::DeadlineExceeded) {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                }
                .into(),
            );
        }
        let mut regular = BTreeMap::new();
        let mut stops = BTreeMap::new();
        for (property, value) in &rule.declarations {
            if matches!(property.as_str(), "stop-color" | "stop-opacity") {
                stops.insert(property.clone(), value.clone());
            } else {
                regular.insert(property.clone(), value.clone());
            }
        }
        if !regular.is_empty() && !stops.is_empty() {
            return Err(format!(
                "d2svgimport: {source}: stylesheet rule .{} mixes stop-only and rendering properties",
                rule.class
            ));
        }
        if !stops.is_empty() {
            from_declarations(source, &stops)?;
        } else {
            svg_style::compute(
                source,
                "style",
                &svg_style::Style::default(),
                &regular,
                resources,
                cancel,
            )?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_stop_values_and_unmatched_rule_validation() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-stop-rules-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let values = serde_json::from_value(row["values"].clone()).unwrap();
            let result = if row["operation"] == "stop" {
                from_declarations("math", &values).map(|s| serde_json::to_value(s).unwrap())
            } else {
                validate_rules(
                    "math",
                    &[Rule {
                        class: "unused".into(),
                        declarations: values,
                    }],
                    &BTreeMap::new(),
                    None,
                )
                .map(|_| serde_json::Value::Null)
            };
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(output) = result {
                if row["operation"] == "stop" {
                    let expected: Stop = serde_json::from_value(row["output"].clone()).unwrap();
                    assert_eq!(output, serde_json::to_value(expected).unwrap(), "{row}");
                }
            }
        }
    }
}
