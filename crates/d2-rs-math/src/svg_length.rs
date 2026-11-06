// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/style.go numeric helpers.
const NUMBER_ERROR: &str = "expected a finite unitless number or px length";
pub fn number(value: &str) -> Result<f64, String> {
    if value.len() > 512 {
        return Err("number is too long".into());
    }
    let raw = value.trim();
    if raw.is_empty() {
        return Err("empty number".into());
    }
    let data = raw.as_bytes();
    let mut offset = usize::from(matches!(data.first(), Some(b'+' | b'-')));
    let mut digits = 0;
    while data.get(offset).is_some_and(u8::is_ascii_digit) {
        offset += 1;
        digits += 1;
    }
    if data.get(offset) == Some(&b'.') {
        offset += 1;
        while data.get(offset).is_some_and(u8::is_ascii_digit) {
            offset += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Err(NUMBER_ERROR.into());
    }
    if matches!(data.get(offset), Some(b'e' | b'E')) {
        offset += 1;
        if matches!(data.get(offset), Some(b'+' | b'-')) {
            offset += 1;
        }
        let start = offset;
        while data.get(offset).is_some_and(u8::is_ascii_digit) {
            offset += 1;
        }
        if start == offset {
            return Err(NUMBER_ERROR.into());
        }
    }
    if offset != data.len() {
        return Err(NUMBER_ERROR.into());
    }
    d2_rs_core::parse_d2_float(raw)
        .filter(|v| v.is_finite())
        .ok_or_else(|| NUMBER_ERROR.into())
}
pub fn length(value: &str, allow_units: bool) -> Result<f64, String> {
    if value.len() > 512 {
        return Err("number is too long".into());
    }
    let mut raw = value.trim();
    if raw.contains('%') {
        return Err("percentages are unsupported".into());
    }
    let mut scale = 1.;
    if allow_units {
        // Go's untyped decimal ratios are exact until converted to float64.
        // Integer ratios avoid rounding the denominator before division.
        for (unit, factor) in [
            ("px", 1.),
            ("in", 96.),
            ("cm", 4800. / 127.),
            ("mm", 480. / 127.),
            ("pt", 4. / 3.),
            ("pc", 16.),
            ("q", 120. / 127.),
        ] {
            if raw
                .get(raw.len().saturating_sub(unit.len())..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(unit))
            {
                raw = &raw[..raw.len() - unit.len()];
                if raw.trim() != raw {
                    return Err("units must immediately follow their number".into());
                }
                scale = factor;
                break;
            }
        }
    }
    let result = number(raw)? * scale;
    if !result.is_finite() {
        return Err("absolute length must be finite".into());
    }
    Ok(result)
}
pub fn mathjax_ex(value: &str) -> Result<f64, String> {
    if value.len() > 512 {
        return Err("number is too long".into());
    }
    let raw = value.trim();
    if raw.len() <= 2
        || !raw
            .get(raw.len() - 2..)
            .is_some_and(|s| s.eq_ignore_ascii_case("ex"))
    {
        return Err("expected a finite ex length".into());
    }
    let raw = &raw[..raw.len() - 2];
    if raw.trim() != raw {
        return Err("units must immediately follow their number".into());
    }
    let result = number(raw).map_err(|_| "expected a finite ex length".to_owned())? * 8.;
    if !result.is_finite() {
        return Err("ex length must be finite".into());
    }
    Ok(result)
}
pub fn unit_interval(value: &str) -> Result<f64, String> {
    if value.len() > 512 {
        return Err("number is too long".into());
    }
    if value.contains('%') {
        return Err("percentages are unsupported".into());
    }
    let result = number(value)?;
    if !(0.0..=1.0).contains(&result) {
        return Err("must be in [0,1]".into());
    }
    Ok(result)
}
/// Source dash-array stage. All-zero patterns resolve to absent dashes; mixed
/// zero entries are rejected. The caller handles the CSS keyword "none".
pub fn dashes(
    value: &str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Option<Vec<f64>>, String> {
    let parts = super::svg_geometry::list(value, usize::MAX, cancel)?;
    if parts.is_empty() {
        return Err("expected at least one dash length".into());
    }
    let mut dashes = Vec::with_capacity(parts.len());
    let mut positive = false;
    let mut zero = false;
    for part in parts {
        let dash = length(part, true)?;
        if dash < 0. {
            return Err("dash lengths must not be negative".into());
        }
        positive |= dash > 0.;
        zero |= dash == 0.;
        dashes.push(dash);
    }
    if !positive {
        return Ok(None);
    }
    if zero {
        return Err("mixed zero-length dash entries are unsupported".into());
    }
    if dashes.len() % 2 == 1 {
        let original = dashes.clone();
        dashes.extend(original);
    }
    let mut total = 0.;
    for dash in &dashes {
        total += dash;
        if !total.is_finite() {
            return Err("dash total must be finite".into());
        }
    }
    Ok(Some(dashes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_stroke_dash_array_contract() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-dashes-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = dashes(row["value"].as_str().unwrap(), cancel);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(dashes) = result {
                let bits = dashes.map(|v| v.into_iter().map(f64::to_bits).collect::<Vec<_>>());
                assert_eq!(serde_json::to_value(bits).unwrap(), row["bits"], "{row}");
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_svg_length_number_interval_and_ex() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-length-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let raw = row["value"].as_str().unwrap();
            let result = match row["kind"].as_str().unwrap() {
                "number" => number(raw),
                "length" => length(raw, row["allow_units"].as_bool().unwrap()),
                "ex" => mathjax_ex(raw),
                "interval" => unit_interval(raw),
                _ => unreachable!(),
            };
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(value) = result {
                assert_eq!(value.to_bits(), row["bits"].as_u64().unwrap(), "{row}");
            }
        }
    }
}
