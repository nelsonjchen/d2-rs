// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/compile.go validateUseGraph.
// This stage consumes already compiled element references; XML/reference
// admission and scene instantiation remain separate importer stages.
#[derive(Clone, Debug)]
pub struct Element {
    pub children: Vec<usize>,
    pub is_use: bool,
    pub target: Option<usize>,
}

/// Resolve the local-reference admission portion of compileUse. Placement and
/// overflow validation belong to the subsequent source length/style stages.
pub fn resolve(
    source: &str,
    raw: Option<&str>,
    ids: &std::collections::BTreeMap<String, (usize, String)>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<usize, String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let raw = raw.ok_or_else(|| error("element <use> is missing href".into()))?;
    let invalid = || {
        error(
            "element <use> href must be one local #id reference; external references are forbidden"
                .into(),
        )
    };
    let id = raw
        .strip_prefix('#')
        .filter(|id| !id.is_empty())
        .ok_or_else(invalid)?;
    for (offset, value) in id.char_indices() {
        if offset & 4095 == 0 {
            context(cancel)?;
        }
        if value == '#' || value.is_whitespace() || value.is_control() {
            return Err(invalid());
        }
    }
    context(cancel)?;
    let (index, name) = ids
        .get(id)
        .ok_or_else(|| error("element <use> references an unknown local id".into()))?;
    if matches!(
        name.as_str(),
        "svg"
            | "defs"
            | "title"
            | "metadata"
            | "rdf:RDF"
            | "cc:Work"
            | "dc:format"
            | "dc:type"
            | "sodipodi:namedview"
            | "linearGradient"
            | "stop"
            | "clipPath"
    ) {
        return Err(error(format!(
            "element <use> references unsupported <{name}> resource"
        )));
    }
    Ok(*index)
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}

/// Complete compileUse's reference, placement and overflow admission. XML and
/// namespaces must already have been validated by the importer.
pub fn compile(
    source: &str,
    attrs: &std::collections::BTreeMap<String, String>,
    ids: &std::collections::BTreeMap<String, (usize, String)>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(usize, [f64; 2]), String> {
    let target = resolve(source, attrs.get("href").map(String::as_str), ids, cancel)?;
    let mut position = [0.; 2];
    for (index, name) in ["x", "y"].iter().enumerate() {
        if let Some(raw) = attrs.get(*name) {
            position[index] = super::svg_length::length(raw, true).map_err(|e| {
                format!("d2svgimport: {source}: element <use> has invalid {name}: {e}")
            })?;
        }
    }
    if let Some(raw) = attrs.get("overflow") {
        let data = raw.as_bytes();
        let space = |b: &u8| b" \t\n\r\x0c".contains(b);
        let mut start = 0;
        let mut end = data.len();
        while start < end && space(&data[start]) {
            start += 1;
            if start & 4095 == 0 {
                context(cancel)?;
            }
        }
        while end > start && space(&data[end - 1]) {
            end -= 1;
            if (data.len() - end) & 4095 == 0 {
                context(cancel)?;
            }
        }
        context(cancel)?;
        if &raw[start..end] != "visible" {
            return Err(format!(
                "d2svgimport: {source}: element <use> supports only overflow=\"visible\" inside a clipping resource"
            ));
        }
    }
    Ok((target, position))
}

/// Validate local-use cycles and expansion depth in source DFS order.
/// Elements/child indices must come from an owned parsed tree. The target can
/// be absent (an unresolved reference), which produces the source error.
pub fn validate(
    source: &str,
    elements: &[Element],
    root: usize,
    maximum_depth: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(), String> {
    let error = |reason: String| format!("d2svgimport: {source}: {reason}");
    let mut state = vec![0_u8; elements.len()];
    let mut finish = Vec::with_capacity(elements.len());
    let mut stack = vec![(root, 0)];
    state[root] = 1;
    while let Some((index, next)) = stack.last_mut() {
        if let Some(cause) = cancel.and_then(d2_rs_core::CancellationToken::error) {
            return Err(
                if matches!(cause, d2_rs_core::LayoutError::DeadlineExceeded) {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                }
                .to_owned(),
            );
        }
        let element = &elements[*index];
        let edge_count = element.children.len() + usize::from(element.is_use);
        if *next == edge_count {
            state[*index] = 2;
            finish.push(*index);
            stack.pop();
            continue;
        }
        let target = if *next < element.children.len() {
            Some(element.children[*next])
        } else {
            element.target
        };
        *next += 1;
        let target =
            target.ok_or_else(|| error("element <use> references an unknown local id".into()))?;
        match state[target] {
            1 => return Err(error("local <use> reference cycle detected".into())),
            2 => {}
            _ => {
                state[target] = 1;
                stack.push((target, 0));
            }
        }
    }
    let mut depths = vec![0_usize; elements.len()];
    for index in finish {
        let element = &elements[index];
        let depth = if element.is_use {
            1 + depths[element.target.unwrap()]
        } else {
            element
                .children
                .iter()
                .map(|&child| depths[child])
                .max()
                .unwrap_or(0)
        };
        if depth > maximum_depth {
            return Err(error(format!(
                "local <use> expansion depth exceeds limit {maximum_depth}"
            )));
        }
        depths[index] = depth;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_use_placement_and_overflow() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-use-placement-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let attrs: std::collections::BTreeMap<String, String> =
                serde_json::from_value(row["attrs"].clone()).unwrap();
            let ids = std::collections::BTreeMap::from([("a".into(), (0, "path".into()))]);
            let actual = compile("math", &attrs, &ids, None);
            assert_eq!(
                actual.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok((target, position)) = actual {
                assert_eq!(target, 0);
                assert_eq!(
                    serde_json::to_value(position.map(f64::to_bits)).unwrap(),
                    row["bits"],
                    "{row}"
                );
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_reference_admission() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-use-reference-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let ids = std::collections::BTreeMap::from([(
                row["id"].as_str().unwrap().into(),
                (0, row["name"].as_str().unwrap().into()),
            )]);
            let actual = resolve("math", row["raw"].as_str(), &ids, None);
            let expected = if row["error"].as_str().unwrap().is_empty() {
                Ok(0)
            } else {
                Err(row["error"].as_str().unwrap().to_owned())
            };
            assert_eq!(actual, expected, "{row}");
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_local_use_graph_order_and_depth() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-use-graph-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let elements: Vec<_> = row["elements"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| Element {
                    children: serde_json::from_value(e["children"].clone()).unwrap(),
                    is_use: e["is_use"].as_bool().unwrap(),
                    target: e["target"].as_u64().map(|n| n as usize),
                })
                .collect();
            let result = validate(
                "math",
                &elements,
                0,
                row["maximum_depth"].as_u64().unwrap() as usize,
                None,
            );
            assert_eq!(
                result.err().unwrap_or_default(),
                row["error"].as_str().unwrap(),
                "{row}"
            );
        }
    }
    #[test]
    fn graph_context_precedes_reference_checks() {
        let elements = [Element {
            children: Vec::new(),
            is_use: true,
            target: None,
        }];
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            validate("math", &elements, 0, 0, Some(&token)).unwrap_err(),
            "context canceled"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.expire();
        assert_eq!(
            validate("math", &elements, 0, 0, Some(&token)).unwrap_err(),
            "context deadline exceeded"
        );
    }
}
