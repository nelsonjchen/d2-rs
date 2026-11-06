// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/import.go parse/attributes.
// Structural work counters only; this is not the complete SVG importer.
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};

/// Check positive importer options and streamed UTF-8 XML structural work.
/// Limits follow original MaxBytes, MaxDepth, MaxElements, MaxAttributes,
/// MaxAttributeBytes, MaxPathCommands, MaxTransformFunctions, MaxUseDepth,
/// MaxResources order. Return source bytes/elements/attributes/attribute bytes.
/// Path/transform/resource compilation is a separate stage.
#[doc(hidden)]
pub fn structural_work(
    source: &str,
    xml: &str,
    limits: [usize; 9],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<[usize; 4], String> {
    let context = || match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => {
            Err("context deadline exceeded".to_owned())
        }
        Some(_) => Err("context canceled".to_owned()),
        None => Ok(()),
    };
    context()?;
    if limits.contains(&0) {
        return Err("d2svgimport: all import limits must be positive".to_owned());
    }
    if xml.len() > limits[0] {
        return Err(format!(
            "d2svgimport: {source} is {} bytes, exceeding limit {}",
            xml.len(),
            limits[0]
        ));
    }
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let mut reader = NsReader::from_str(xml);
    let mut counts = [xml.len(), 0, 0, 0];
    let mut depth = 0;
    loop {
        context()?;
        let event = reader.read_event().map_err(|_| {
            error(format!(
                "invalid XML near byte {}",
                reader.buffer_position()
            ))
        })?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(node) | Event::Empty(node) => {
                // A self-closing element has the same start-element depth charge.
                if depth + 1 > limits[1] {
                    return Err(error(format!("XML depth exceeds limit {}", limits[1])));
                }
                counts[1] += 1;
                if counts[1] > limits[2] {
                    return Err(error(format!("element count exceeds limit {}", limits[2])));
                }
                for attr in node.attributes().with_checks(false) {
                    context()?;
                    let attr = attr.map_err(|_| {
                        error(format!(
                            "invalid XML near byte {}",
                            reader.buffer_position()
                        ))
                    })?;
                    counts[2] += 1;
                    if counts[2] > limits[3] {
                        return Err(error(format!(
                            "attribute count exceeds limit {}",
                            limits[3]
                        )));
                    }
                    let (namespace, local) = reader.resolve_attribute(attr.key);
                    let namespace_len = if attr.key.as_ref().starts_with(b"xmlns:") {
                        5
                    } else {
                        match namespace {
                            ResolveResult::Bound(ns) => ns.as_ref().len(),
                            ResolveResult::Unbound => 0,
                            ResolveResult::Unknown(ns) => ns.len(),
                        }
                    };
                    // encoding/xml normalizes literal XML line endings before
                    // entity expansion. Character references keep their value.
                    let raw = std::str::from_utf8(attr.value.as_ref()).map_err(|_| {
                        error(format!(
                            "invalid XML near byte {}",
                            reader.buffer_position()
                        ))
                    })?;
                    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
                    let value = quick_xml::escape::unescape(&normalized).map_err(|_| {
                        error(format!(
                            "invalid XML near byte {}",
                            reader.buffer_position()
                        ))
                    })?;
                    let bytes = namespace_len + local.as_ref().len() + value.len();
                    if bytes > limits[4] - counts[3] {
                        return Err(error(format!("attribute bytes exceed limit {}", limits[4])));
                    }
                    counts[3] += bytes;
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_context_precedes_limits_and_bytes() {
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            structural_work("math", "<svg/>", [0; 9], Some(&token)).unwrap_err(),
            "context canceled"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.expire();
        assert_eq!(
            structural_work("math", "<svg/>", [1; 9], Some(&token)).unwrap_err(),
            "context deadline exceeded"
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_xml_work_controls() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-xml-budget-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let limits: [usize; 9] = serde_json::from_value(row["limits"].clone()).unwrap();
            let result = structural_work("math", row["xml"].as_str().unwrap(), limits, None);
            assert_eq!(
                result.as_ref().err().map(String::as_str),
                row["error"].as_str(),
                "{row}"
            );
            assert_eq!(
                serde_json::to_value(result.ok()).unwrap(),
                row["counts"],
                "{row}"
            );
        }
    }
}
