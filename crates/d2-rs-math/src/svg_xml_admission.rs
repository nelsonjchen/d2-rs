// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/import.go and boilerplate.go.
use super::svg_geometry::trim_space;
use std::collections::{BTreeMap, HashSet};
pub const SVG: &str = "http://www.w3.org/2000/svg";
pub const XML: &str = "http://www.w3.org/XML/1998/namespace";
pub const XLINK: &str = "http://www.w3.org/1999/xlink";
const XMLNS_URI: &str = "http://www.w3.org/2000/xmlns/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const CC: &str = "http://creativecommons.org/ns#";
const DC: &str = "http://purl.org/dc/elements/1.1/";
const SODIPODI: &str = "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd";
const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";
#[derive(Default, Debug)]
pub struct State {
    pub namespace: Option<String>,
    pub attributes: usize,
    pub attribute_bytes: usize,
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Attribute {
    pub namespace: String,
    pub name: String,
    pub value: String,
}
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attributes {
    pub values: BTreeMap<String, String>,
    pub order: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub id: String,
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn display(name: &str) -> &str {
    if name.len() > 64 {
        "<oversized-name>"
    } else {
        name
    }
}
fn prefix_fold(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}
impl State {
    pub fn element(&mut self, source: &str, namespace: &str, name: &str) -> Result<String, String> {
        let error = |reason: String| format!("d2svgimport: {source}: {reason}");
        match namespace {
            "" | SVG => {
                if let Some(previous) = &self.namespace {
                    if previous != namespace {
                        return Err(error(format!(
                            "element <{}> mixes SVG and empty namespaces",
                            display(name)
                        )));
                    }
                } else {
                    self.namespace = Some(namespace.into());
                }
                match name {
                    "svg" | "g" | "defs" | "style" | "path" | "rect" | "circle" | "ellipse"
                    | "line" | "polyline" | "polygon" | "image" | "text" | "use" | "clipPath"
                    | "linearGradient" | "stop" | "title" | "metadata" => return Ok(name.into()),
                    "script" | "foreignObject" => {
                        return Err(error(format!("forbidden element <{name}>")));
                    }
                    "radialGradient" | "pattern" => {
                        return Err(error(format!("unsupported paint server <{name}>")));
                    }
                    _ => return Err(error(format!("unknown element <{}>", display(name)))),
                }
            }
            RDF if name == "RDF" => return Ok("rdf:RDF".into()),
            CC if name == "Work" => return Ok("cc:Work".into()),
            DC if name == "format" || name == "type" => return Ok(format!("dc:{name}")),
            SODIPODI if name == "namedview" => return Ok("sodipodi:namedview".into()),
            _ => {}
        }
        Err(error(format!(
            "element <{}> uses unsupported namespace or metadata name",
            display(name)
        )))
    }
    /// Resolved XML attributes in token order. Counter charges survive failures;
    /// values/order/metadata are returned only on complete success.
    pub fn attributes(
        &mut self,
        source: &str,
        element: &str,
        input: &[Attribute],
        max_attributes: usize,
        max_bytes: usize,
        cancel: Option<&d2_rs_core::CancellationToken>,
    ) -> Result<Attributes, String> {
        let error = |reason: String| format!("d2svgimport: {source}: {reason}");
        let mut output = Attributes::default();
        let mut expanded = HashSet::new();
        for attribute in input {
            context(cancel)?;
            self.attributes += 1;
            if self.attributes > max_attributes {
                return Err(error(format!(
                    "attribute count exceeds limit {max_attributes}"
                )));
            }
            let bytes = attribute.namespace.len() + attribute.name.len() + attribute.value.len();
            if self.attribute_bytes > max_bytes || bytes > max_bytes - self.attribute_bytes {
                return Err(error(format!("attribute bytes exceed limit {max_bytes}")));
            }
            self.attribute_bytes += bytes;
            if !expanded.insert((&attribute.namespace, &attribute.name)) {
                return Err(error(format!(
                    "element <{element}> has duplicate XML attribute {:?}",
                    display(&attribute.name)
                )));
            }
            let name = attribute.name.as_str();
            let value = attribute.value.as_str();
            if attribute.namespace == "xmlns" || (attribute.namespace.is_empty() && name == "xmlns")
            {
                namespace_declaration(source, element, attribute)?;
                continue;
            }
            if ignored_namespaced(source, element, attribute, cancel)? {
                continue;
            }
            match attribute.namespace.as_str() {
                "" => {}
                XLINK => {
                    if name != "href" {
                        return Err(error(format!(
                            "element <{element}> has unsupported xlink attribute {:?}",
                            display(name)
                        )));
                    }
                }
                _ => {
                    return Err(error(format!(
                        "element <{element}> attribute {:?} uses unsupported namespace",
                        display(name)
                    )));
                }
            }
            if prefix_fold(name, "on") {
                return Err(error(format!(
                    "element <{element}> has forbidden event attribute {:?}",
                    display(name)
                )));
            }
            if ignored_plain(source, element, name, value, cancel)? {
                output.metadata.insert(name.into(), value.into());
                continue;
            }
            if output.values.contains_key(name) {
                return Err(error(format!(
                    "element <{element}> has duplicate attribute {:?}",
                    display(name)
                )));
            }
            output.values.insert(name.into(), value.into());
            output.order.push(name.into());
            if name == "id" {
                output.id = value.into();
                if !valid_id(value, cancel)? {
                    return Err(error(format!("element <{element}> has invalid id")));
                }
            }
        }
        Ok(output)
    }
}
fn namespace_declaration(source: &str, element: &str, attribute: &Attribute) -> Result<(), String> {
    let prefix = if attribute.namespace == "xmlns" {
        attribute.name.as_str()
    } else {
        ""
    };
    let value = &attribute.value;
    let error = |reason: &str| {
        format!("d2svgimport: {source}: element <{element}> has {reason} namespace binding")
    };
    if !prefix.is_empty() && value.is_empty() {
        return Err(error("an empty prefixed"));
    }
    if prefix.eq_ignore_ascii_case("xmlns") || value == XMLNS_URI {
        return Err(error("a forbidden xmlns"));
    }
    if prefix == "xml" {
        return if value == XML {
            Ok(())
        } else {
            Err(error("an invalid xml"))
        };
    }
    if prefix_fold(prefix, "xml") || value == XML {
        return Err(error("a reserved xml"));
    }
    Ok(())
}
fn ignored_namespaced(
    source: &str,
    element: &str,
    a: &Attribute,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<bool, String> {
    let name = a.name.as_str();
    let error = |kind: &str| {
        format!(
            "d2svgimport: {source}: element <{element}> has unsupported {kind} attribute {:?}",
            display(name)
        )
    };
    match a.namespace.as_str() {
        XML => {
            if name != "space" {
                return Err(error("xml"));
            }
            let value = trim_space(&a.value, cancel)?;
            if value != "default" && value != "preserve" {
                return Err(format!(
                    "d2svgimport: {source}: element <{element}> has invalid xml:space"
                ));
            }
            Ok(true)
        }
        INKSCAPE => {
            let allowed = match element {
                "svg" => matches!(
                    name,
                    "export-filename" | "export-xdpi" | "export-ydpi" | "version"
                ),
                "g" => matches!(name, "groupmode" | "label"),
                "path" => matches!(
                    name,
                    "connector-curvature" | "export-filename" | "export-xdpi" | "export-ydpi"
                ),
                "sodipodi:namedview" => matches!(
                    name,
                    "current-layer"
                        | "cx"
                        | "cy"
                        | "document-units"
                        | "pageopacity"
                        | "pageshadow"
                        | "snap-global"
                        | "window-height"
                        | "window-maximized"
                        | "window-width"
                        | "window-x"
                        | "window-y"
                        | "zoom"
                ),
                _ => false,
            };
            if allowed {
                Ok(true)
            } else {
                Err(error("Inkscape metadata"))
            }
        }
        SODIPODI => {
            if (element == "svg" && name == "docname") || (element == "path" && name == "nodetypes")
            {
                Ok(true)
            } else {
                Err(error("Sodipodi metadata"))
            }
        }
        RDF => {
            if (element == "cc:Work" && name == "about" && a.value.is_empty())
                || (element == "dc:type"
                    && name == "resource"
                    && a.value == "http://purl.org/dc/dcmitype/StillImage")
            {
                Ok(true)
            } else {
                Err(error("RDF metadata"))
            }
        }
        _ => Ok(false),
    }
}
fn ignored_plain(
    source: &str,
    element: &str,
    name: &str,
    value: &str,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<bool, String> {
    let error = |reason: &str| format!("d2svgimport: {source}: element <{element}> {reason}");
    if name.starts_with("data-") {
        if name.len() == 5 {
            return Err(error("has invalid data-* metadata attribute"));
        }
        for (index, byte) in name.bytes().enumerate().skip(5) {
            if index & 4095 == 0 {
                context(cancel)?;
            }
            if !byte.is_ascii_alphanumeric() && byte != b'-' && byte != b'_' {
                return Err(error("has invalid data-* metadata attribute"));
            }
        }
        return Ok(true);
    }
    if name == "role" {
        if !matches!(trim_space(value, cancel)?, "img" | "none" | "presentation") {
            return Err(error("has unsupported non-rendering role"));
        }
        return Ok(true);
    }
    if name == "focusable" {
        if trim_space(value, cancel)? != "false" {
            return Err(error("focusable metadata must be false"));
        }
        return Ok(true);
    }
    Ok(false)
}
pub fn valid_id(id: &str, cancel: Option<&d2_rs_core::CancellationToken>) -> Result<bool, String> {
    if id.is_empty() {
        return Ok(false);
    }
    for (offset, c) in id.char_indices() {
        if offset & 4095 == 0 {
            context(cancel)?;
        }
        if c == '#' || c.is_whitespace() || c.is_control() {
            return Ok(false);
        }
    }
    context(cancel)?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_namespace_attribute_admission_and_charges() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-xml-admission-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            if row["operation"] == "element" {
                let prior = row["prior"].as_str().unwrap();
                let mut state = State {
                    namespace: if prior == "unset" {
                        None
                    } else {
                        Some(prior.into())
                    },
                    ..Default::default()
                };
                let result = state.element(
                    "math",
                    row["namespace"].as_str().unwrap(),
                    row["name"].as_str().unwrap(),
                );
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                assert_eq!(state.namespace.as_deref(), row["after"].as_str(), "{row}");
                if let Ok(value) = result {
                    assert_eq!(value, row["output"].as_str().unwrap(), "{row}");
                }
            } else {
                let input: Vec<Attribute> = serde_json::from_value(row["input"].clone()).unwrap();
                let mut state = State::default();
                let result = state.attributes(
                    "math",
                    row["element"].as_str().unwrap(),
                    &input,
                    row["maximum_attributes"].as_u64().unwrap() as usize,
                    row["maximum_bytes"].as_u64().unwrap() as usize,
                    None,
                );
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    row["error"].as_str().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    state.attributes as u64,
                    row["after_attributes"].as_u64().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    state.attribute_bytes as u64,
                    row["after_bytes"].as_u64().unwrap(),
                    "{row}"
                );
                if let Ok(value) = result {
                    let expected: Attributes =
                        serde_json::from_value(row["output"].clone()).unwrap();
                    assert_eq!(value, expected, "{row}");
                }
            }
        }
    }
    #[test]
    fn context_and_cumulative_attribute_budget() {
        let input = [Attribute {
            namespace: String::new(),
            name: "x".into(),
            value: "1".into(),
        }];
        let mut state = State::default();
        state.attributes("math", "svg", &input, 1, 2, None).unwrap();
        assert_eq!(
            state
                .attributes("math", "path", &input, 1, 2, None)
                .unwrap_err(),
            "d2svgimport: math: attribute count exceeds limit 1"
        );
        assert_eq!((state.attributes, state.attribute_bytes), (2, 2));
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            state
                .attributes("math", "path", &input, 1, 2, Some(&token))
                .unwrap_err(),
            "context canceled"
        );
        assert_eq!(state.attributes, 2);
        // The source checks context inside the attribute loop; an empty list
        // performs no work or context check at this private stage.
        assert!(
            state
                .attributes("math", "path", &[], 1, 2, Some(&token))
                .is_ok()
        );
    }
}
