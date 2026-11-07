// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/stylesheet.go.
use super::{
    svg_declarations::{PROPERTIES, Rule},
    svg_geometry::trim_space,
};
use std::collections::{BTreeMap, HashSet};
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn space(byte: u8) -> bool {
    b" \t\n\r\x0c".contains(&byte)
}
pub fn classes(
    raw: Option<&str>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(Vec<String>, HashSet<String>), String> {
    let Some(raw) = raw else {
        return Ok((Vec::new(), HashSet::new()));
    };
    let bytes = raw.as_bytes();
    let mut start = None;
    let mut classes = Vec::new();
    let mut seen = HashSet::new();
    for offset in 0..=bytes.len() {
        if offset & 4095 == 0 {
            context(cancel)?;
        }
        if offset < bytes.len() && !space(bytes[offset]) {
            if start.is_none() {
                start = Some(offset);
            }
            continue;
        }
        if let Some(from) = start.take() {
            let class = &raw[from..offset];
            if seen.insert(class.into()) {
                classes.push(class.into());
            }
        }
    }
    context(cancel)?;
    Ok((classes, seen))
}
pub fn validate_element(
    source: &str,
    attrs: &BTreeMap<String, String>,
    order: &[String],
    children: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    for name in order {
        context(cancel)?;
        if name != "type" {
            return Err(error(format!(
                "element <style> has unsupported attribute {:?}",
                if name.len() > 64 {
                    "<oversized-name>"
                } else {
                    name
                }
            )));
        }
    }
    if children != 0 {
        return Err(error(
            "element <style> cannot contain child elements".into(),
        ));
    }
    if let Some(raw) = attrs.get("type") {
        if !trim_space(raw, cancel)?.eq_ignore_ascii_case("text/css") {
            return Err(error("element <style> has unsupported type".into()));
        }
    }
    Ok(())
}
pub fn parse(
    source: &str,
    input: &str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<Rule>, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let bytes = input.as_bytes();
    for index in 0..bytes.len() {
        if index & 4095 == 0 {
            context(cancel)?;
        }
        if bytes
            .get(index..index + 2)
            .is_some_and(|s| s == b"/*" || s == b"*/")
        {
            return Err(error("CSS comments in <style> are unsupported".into()));
        }
    }
    let mut rules = Vec::new();
    let mut offset = 0;
    loop {
        while offset < bytes.len() && space(bytes[offset]) {
            offset += 1;
            if offset & 4095 == 0 {
                context(cancel)?;
            }
        }
        context(cancel)?;
        if offset == bytes.len() {
            return Ok(rules);
        }
        if bytes[offset] == b'}' {
            return Err(error(
                "malformed stylesheet: unexpected closing brace".into(),
            ));
        }
        let start = offset;
        while offset < bytes.len() && bytes[offset] != b'{' {
            if offset & 4095 == 0 {
                context(cancel)?;
            }
            if bytes[offset] == b'}' {
                return Err(error("malformed stylesheet selector".into()));
            }
            offset += 1;
        }
        if offset == bytes.len() {
            return Err(error(
                "malformed stylesheet: missing declaration block".into(),
            ));
        }
        let selector = trim_space(&input[start..offset], cancel)?;
        if selector.len() < 2 || !selector.starts_with('.') {
            return Err(error(
                if selector.starts_with('@') {
                    "CSS at-rules in <style> are unsupported"
                } else {
                    "unsupported stylesheet selector"
                }
                .into(),
            ));
        }
        let class = &selector[1..];
        for (index, byte) in class.bytes().enumerate() {
            if index & 4095 == 0 {
                context(cancel)?;
            }
            if index == 0 {
                if !byte.is_ascii_alphabetic() && byte != b'_' {
                    return Err(error("unsupported stylesheet selector".into()));
                }
            } else if !byte.is_ascii_alphanumeric() && !b"_-".contains(&byte) {
                return Err(error("unsupported stylesheet selector".into()));
            }
        }
        context(cancel)?;
        offset += 1;
        let start = offset;
        while offset < bytes.len() && bytes[offset] != b'}' {
            if offset & 4095 == 0 {
                context(cancel)?;
            }
            if bytes[offset] == b'{' {
                return Err(error("malformed stylesheet declaration block".into()));
            }
            offset += 1;
        }
        if offset == bytes.len() {
            return Err(error(
                "malformed stylesheet: unterminated declaration block".into(),
            ));
        }
        let body = &input[start..offset];
        let data = body.as_bytes();
        let mut declarations = BTreeMap::new();
        let mut start = 0;
        while start <= data.len() {
            context(cancel)?;
            let mut end = start;
            while end < data.len() && data[end] != b';' {
                end += 1;
                if end & 4095 == 0 {
                    context(cancel)?;
                }
            }
            let raw = trim_space(&body[start..end], cancel)?;
            start = if end == data.len() {
                data.len() + 1
            } else {
                end + 1
            };
            if raw.is_empty() {
                continue;
            }
            let mut colon = None;
            for (index, byte) in raw.bytes().enumerate() {
                if index & 4095 == 0 {
                    context(cancel)?;
                }
                if byte == b':' {
                    colon = Some(index);
                    break;
                }
            }
            let malformed = || error("malformed stylesheet declaration".into());
            let colon = colon.filter(|&n| n > 0).ok_or_else(malformed)?;
            let property = trim_space(&raw[..colon], cancel)?;
            let value = trim_space(&raw[colon + 1..], cancel)?;
            if property.len() > 64 {
                return Err(malformed());
            }
            let property =
                String::from_utf8(d2_rs_core::ir::go_lowercase(property.as_bytes())).unwrap();
            if property.is_empty()
                || !property
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'-')
                || value.is_empty()
            {
                return Err(malformed());
            }
            if !PROPERTIES.contains(&property.as_str()) {
                return Err(error(format!(
                    "unsupported stylesheet property {property:?}"
                )));
            }
            if property == "transform" {
                return Err(error(
                    "unsupported stylesheet transform; use the transform presentation attribute"
                        .into(),
                ));
            }
            for (index, byte) in value.bytes().enumerate() {
                if index & 4095 == 0 {
                    context(cancel)?;
                }
                if byte == b'!' {
                    return Err(error("unsupported !important in <style>".into()));
                }
            }
            declarations.insert(property, value.into());
        }
        context(cancel)?;
        rules.push(Rule {
            class: class.into(),
            declarations,
        });
        offset += 1;
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_stylesheet_rules_classes_and_style_element() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-stylesheet-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap_or("") {
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
            let value = row["value"].as_str().unwrap_or("");
            let operation = row["operation"].as_str().unwrap();
            let result: Result<serde_json::Value, String> = match operation {
                "parse" => parse("math", value, cancel).map(|v| serde_json::to_value(v).unwrap()),
                "classes" => classes(Some(value), cancel).map(|(v, set)| {
                    assert_eq!(v.len(), set.len());
                    serde_json::to_value(v).unwrap()
                }),
                "element" => validate_element(
                    "math",
                    &BTreeMap::from([("type".into(), value.into())]),
                    &["type".into()],
                    0,
                    cancel,
                )
                .map(|_| serde_json::Value::Null),
                _ => {
                    let order: Vec<String> = serde_json::from_value(row["order"].clone()).unwrap();
                    validate_element(
                        "math",
                        &BTreeMap::from([("type".into(), "text/css".into())]),
                        &order,
                        row["children"].as_u64().unwrap() as usize,
                        cancel,
                    )
                    .map(|_| serde_json::Value::Null)
                }
            };
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(output) = result {
                if operation != "order" {
                    assert_eq!(output, row["output"], "{row}");
                }
            }
        }
    }
}
