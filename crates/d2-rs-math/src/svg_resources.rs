// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2svgimport/style.go parseSVGPaint;
// gradient.go localLinearGradientPaint,validLocalPaintID; clip.go localClipPathID.
use super::{svg_color, svg_primitive::Paint};
use std::collections::BTreeMap;
pub struct Resource {
    pub name: String,
    pub gradient: Option<Paint>,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn paint(
    raw: &str,
    resources: &BTreeMap<String, Resource>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(Option<Paint>, bool), String> {
    if raw.len() > 4096 {
        return Err("paint value is too long".into());
    }
    let value = raw.trim();
    if value.eq_ignore_ascii_case("none") {
        return Ok((None, false));
    }
    if value.eq_ignore_ascii_case("currentColor") {
        return Ok((None, true));
    }
    let lowered = String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes())).unwrap();
    if !lowered.contains("url(") {
        return Ok((
            Some(Paint::Solid {
                color: svg_color::solid(value)?,
            }),
            false,
        ));
    }
    let invalid = || {
        "paint servers require one local url(#id) reference; external references and fallbacks are forbidden".to_owned()
    };
    let id = value
        .strip_prefix("url(#")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(invalid)?;
    if id.is_empty() {
        return Err(invalid());
    }
    for (index, byte) in id.bytes().enumerate() {
        if index & 4095 == 0 {
            context(cancel)?;
        }
        if index == 0 {
            if !byte.is_ascii_alphabetic() && byte != b'_' {
                return Err(invalid());
            }
        } else if !byte.is_ascii_alphanumeric() && !b"_-.:".contains(&byte) {
            return Err(invalid());
        }
    }
    context(cancel)?;
    let resource = resources
        .get(id)
        .ok_or_else(|| "paint server references an unknown local id".to_owned())?;
    let Some(Paint::Linear {
        start,
        end,
        stops,
        units,
        transform,
        spread,
    }) = resource
        .gradient
        .as_ref()
        .filter(|_| resource.name == "linearGradient")
    else {
        return Err("paint server local id does not name a supported linear gradient".into());
    };
    context(cancel)?;
    let mut owned = Vec::with_capacity(stops.len());
    for (index, stop) in stops.iter().enumerate() {
        if index & 255 == 0 {
            context(cancel)?;
        }
        owned.push(stop.clone());
    }
    Ok((
        Some(Paint::Linear {
            start: *start,
            end: *end,
            stops: owned,
            units: *units,
            transform: *transform,
            spread: *spread,
        }),
        false,
    ))
}
pub fn clip(
    value: &str,
    resources: &BTreeMap<String, Resource>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<String, String> {
    context(cancel)?;
    let invalid = || {
        "requires one local url(#id) reference; external, quoted, and fallback URLs are forbidden"
            .to_owned()
    };
    let id = value
        .strip_prefix("url(#")
        .and_then(|s| s.strip_suffix(')'))
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)?;
    for (offset, c) in id.char_indices() {
        if offset & 4095 == 0 {
            context(cancel)?;
        }
        if c == '#' || c.is_whitespace() || c.is_control() {
            return Err(invalid());
        }
    }
    context(cancel)?;
    let resource = resources
        .get(id)
        .ok_or_else(|| "references an unknown local id".to_owned())?;
    if resource.name != "clipPath" {
        return Err("local id does not name a <clipPath> resource".into());
    }
    Ok(id.into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_resource_syntax_lookup_and_context() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-resources-v090.json"))
                .unwrap();
        for row in rows.as_array().unwrap() {
            let kind = row["kind"].as_str().unwrap();
            let mut resources = BTreeMap::new();
            for id in [
                "g".into(),
                "1g".into(),
                "_g".into(),
                "g.x:y".into(),
                "🙂".into(),
                "g)".into(),
                "a".repeat(300),
            ] {
                resources.insert(
                    id,
                    Resource {
                        name: if kind == "uncompiled" {
                            "linearGradient"
                        } else {
                            kind
                        }
                        .into(),
                        gradient: if kind == "linearGradient" {
                            Some(Paint::Linear {
                                start: [0.; 2],
                                end: [0.; 2],
                                stops: Vec::new(),
                                units: 0,
                                transform: [1., 0., 0., 1., 0., 0.],
                                spread: 0,
                            })
                        } else {
                            None
                        },
                    },
                );
            }
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
            let value = row["value"].as_str().unwrap();
            if row["operation"] == "paint" {
                let result = paint(value, &resources, cancel);
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                if let Ok((p, current)) = result {
                    let expected: Option<Paint> =
                        serde_json::from_value(row["paint"].clone()).unwrap();
                    assert_eq!(p, expected, "{row}");
                    assert_eq!(current, row["current"].as_bool().unwrap(), "{row}");
                }
            } else {
                let result = clip(value, &resources, cancel);
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                if let Ok(id) = result {
                    assert_eq!(id, row["id"].as_str().unwrap(), "{row}");
                }
            }
        }
    }
}
