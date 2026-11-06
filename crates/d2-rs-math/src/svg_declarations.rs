// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/style.go declarationsFor;
// boilerplate.go validateEnableBackground.
use super::{svg_geometry::trim_space, svg_length};
use std::collections::{BTreeMap, HashSet};
pub(crate) const PROPERTIES: &[&str] = &[
    "fill",
    "stroke",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "fill-rule",
    "clip-rule",
    "clip-path",
    "color",
    "display",
    "visibility",
    "transform",
    "stop-color",
    "stop-opacity",
];
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Rule {
    pub class: String,
    pub declarations: BTreeMap<String, String>,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub(crate) fn background(
    raw: &str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let raw = trim_space(raw, cancel)?;
    let data = raw.as_bytes();
    let space = |b: u8| b" \t\n\r\x0c".contains(&b);
    let mut end = 0;
    while end < data.len() && !space(data[end]) {
        end += 1;
        if end & 4095 == 0 {
            context(cancel)?;
        }
    }
    let invalid = || "only a finite new x y width height region is supported".to_owned();
    if !raw[..end].eq_ignore_ascii_case("new") || end == data.len() {
        return Err(invalid());
    }
    let tail = &raw[end..];
    let bytes = tail.as_bytes();
    let mut start = None;
    let mut fields = Vec::with_capacity(4);
    for offset in 0..=bytes.len() {
        if offset & 4095 == 0 {
            context(cancel)?;
        }
        if offset < bytes.len() && !space(bytes[offset]) {
            if bytes[offset] == b',' {
                return Err(
                    "only whitespace-separated enable-background coordinates are supported".into(),
                );
            }
            if start.is_none() {
                start = Some(offset);
            }
            continue;
        }
        if let Some(from) = start.take() {
            if fields.len() == 4 {
                return Err(invalid());
            }
            fields.push(&tail[from..offset]);
        }
    }
    context(cancel)?;
    if fields.len() != 4 {
        return Err(invalid());
    }
    let mut numbers = [0.; 4];
    for (index, field) in fields.iter().enumerate() {
        context(cancel)?;
        numbers[index] = svg_length::number(field).map_err(|_| invalid())?;
    }
    if numbers[2] <= 0. || numbers[3] <= 0. {
        return Err("enable-background width and height must be positive".into());
    }
    context(cancel)
}
/// Apply presentation, ordered class rules, then inline declarations. Rules
/// must already have passed stylesheet validation; mathjax tracks root state.
pub fn collect(
    source: &str,
    name: &str,
    root: bool,
    attrs: &BTreeMap<String, String>,
    classes: &HashSet<String>,
    rules: &[Rule],
    mathjax: &mut bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<BTreeMap<String, String>, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let mut declarations = BTreeMap::new();
    for &property in PROPERTIES {
        if let Some(raw) = attrs.get(property) {
            declarations.insert(property.into(), trim_space(raw, cancel)?.into());
        }
    }
    for (index, rule) in rules.iter().enumerate() {
        if index & 255 == 0 {
            context(cancel)?;
        }
        if classes.contains(&rule.class) {
            declarations.extend(rule.declarations.clone());
        }
    }
    let Some(inline) = attrs.get("style") else {
        return Ok(declarations);
    };
    let data = inline.as_bytes();
    for index in 0..data.len() {
        if index & 4095 == 0 {
            context(cancel)?;
        }
        if matches!(data[index], b'{' | b'}')
            || data
                .get(index..index + 2)
                .is_some_and(|s| s == b"/*" || s == b"*/")
        {
            return Err(error(format!(
                "element <{name}> has unsupported inline CSS syntax"
            )));
        }
    }
    let malformed = || error(format!("element <{name}> has malformed inline style"));
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
        let raw = trim_space(&inline[start..end], cancel)?;
        start = if end == data.len() {
            data.len() + 1
        } else {
            end + 1
        };
        if raw.is_empty() {
            continue;
        }
        let mut colon = None;
        for (index, byte) in raw.as_bytes().iter().enumerate() {
            if index & 4095 == 0 {
                context(cancel)?;
            }
            if *byte == b':' {
                colon = Some(index);
                break;
            }
        }
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
        if property == "enable-background" {
            if !root {
                return Err(error(format!(
                    "element <{name}> has unsupported enable-background outside the root <svg>"
                )));
            }
            background(value, cancel).map_err(|e| {
                context(cancel).err().unwrap_or_else(|| {
                    error(format!("root <svg> has invalid enable-background: {e}"))
                })
            })?;
            continue;
        }
        if property == "vertical-align" {
            if !root {
                return Err(error(format!(
                    "element <{name}> has unsupported vertical-align outside the root <svg>"
                )));
            }
            svg_length::mathjax_ex(value).map_err(|e| {
                error(format!(
                    "root <svg> has invalid MathJax vertical-align: {e}"
                ))
            })?;
            *mathjax = true;
            continue;
        }
        if !PROPERTIES.contains(&property.as_str()) {
            return Err(error(format!(
                "element <{name}> has unsupported style property {property:?}"
            )));
        }
        if property == "transform" {
            return Err(error(format!(
                "element <{name}> has unsupported CSS transform; use the transform presentation attribute"
            )));
        }
        let pattern = b"!important";
        if let Some(maximum) = value.len().checked_sub(pattern.len()) {
            for offset in 0..=maximum {
                if offset & 4095 == 0 {
                    context(cancel)?;
                }
                if value
                    .as_bytes()
                    .get(offset..offset + pattern.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(pattern))
                {
                    return Err(error(format!(
                        "element <{name}> uses unsupported !important"
                    )));
                }
            }
        }
        context(cancel)?;
        declarations.insert(property, value.into());
    }
    Ok(declarations)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_cascade_inline_errors_and_root_metadata() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-declarations-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let attrs = serde_json::from_value(row["attrs"].clone()).unwrap();
            let classes: HashSet<String> = serde_json::from_value(row["classes"].clone()).unwrap();
            let rules: Vec<_> = row["rules"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| Rule {
                    class: r["class"].as_str().unwrap().into(),
                    declarations: serde_json::from_value(r["declarations"].clone()).unwrap(),
                })
                .collect();
            let mut mathjax = false;
            let result = collect(
                "math",
                row["name"].as_str().unwrap(),
                row["root"].as_bool().unwrap(),
                &attrs,
                &classes,
                &rules,
                &mut mathjax,
                None,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(mathjax, row["mathjax"].as_bool().unwrap(), "{row}");
            assert_eq!(
                serde_json::to_value(result.ok()).unwrap(),
                row["declarations"],
                "{row}"
            );
        }
    }
}
