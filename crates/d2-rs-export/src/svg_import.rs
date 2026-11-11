// SPDX-License-Identifier: MPL-2.0
//! Early SVG import contracts from D2 v0.9.0 d2svgimport/import.go,
//! compile.go, gradient.go and boilerplate.go. Geometry/style/resource
//! compilation remains in the native raster pipeline.
#[cfg(all(test, feature = "oracle-fixtures"))]
use std::collections::{HashMap, HashSet};
#[cfg(all(test, feature = "oracle-fixtures"))]
const SVG: &str = "http://www.w3.org/2000/svg";
#[cfg(all(test, feature = "oracle-fixtures"))]
const XLINK: &str = "http://www.w3.org/1999/xlink";
#[cfg(all(test, feature = "oracle-fixtures"))]
const XML: &str = "http://www.w3.org/XML/1998/namespace";
#[cfg(all(test, feature = "oracle-fixtures"))]
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
#[cfg(all(test, feature = "oracle-fixtures"))]
const CC: &str = "http://creativecommons.org/ns#";
#[cfg(all(test, feature = "oracle-fixtures"))]
const DC: &str = "http://purl.org/dc/elements/1.1/";
#[cfg(all(test, feature = "oracle-fixtures"))]
const SODIPODI: &str = "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd";
#[cfg(all(test, feature = "oracle-fixtures"))]
const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";
#[cfg(all(test, feature = "oracle-fixtures"))]
const PRESENTATION: &[&str] = &[
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
#[cfg(all(test, feature = "oracle-fixtures"))]
fn shown(name: &str) -> &str {
    if name.len() > 64 {
        "<oversized-name>"
    } else {
        name
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn specific(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "svg" => &[
            "width",
            "height",
            "viewBox",
            "preserveAspectRatio",
            "version",
            "x",
            "y",
            "enable-background",
        ],
        "g" | "defs" | "rdf:RDF" | "cc:Work" | "dc:format" | "dc:type" => &[],
        "style" => &["type"],
        "title" | "metadata" => &["id"],
        "sodipodi:namedview" => &[
            "id",
            "pagecolor",
            "bordercolor",
            "borderopacity",
            "objecttolerance",
            "gridtolerance",
            "guidetolerance",
            "fit-margin-bottom",
            "fit-margin-left",
            "fit-margin-right",
            "fit-margin-top",
            "showgrid",
        ],
        "path" => &["d"],
        "rect" => &["x", "y", "width", "height", "rx", "ry"],
        "circle" => &["cx", "cy", "r"],
        "ellipse" => &["cx", "cy", "rx", "ry"],
        "line" => &["x1", "y1", "x2", "y2"],
        "polyline" | "polygon" => &["points"],
        "image" => &["x", "y", "width", "height", "href", "preserveAspectRatio"],
        "text" => &["font-size", "font-family"],
        "use" => &["href", "x", "y", "overflow"],
        "clipPath" => &["id", "clipPathUnits"],
        "linearGradient" => &[
            "id",
            "x1",
            "y1",
            "x2",
            "y2",
            "gradientUnits",
            "gradientTransform",
            "spreadMethod",
        ],
        "stop" => &["offset", "class", "style", "stop-color", "stop-opacity"],
        _ => return None,
    })
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn nonrendering(name: &str) -> bool {
    matches!(
        name,
        "title"
            | "metadata"
            | "rdf:RDF"
            | "cc:Work"
            | "dc:format"
            | "dc:type"
            | "sodipodi:namedview"
    )
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn element_name(
    node: roxmltree::Node<'_, '_>,
    namespace: &mut Option<String>,
) -> Result<String, String> {
    element_name_value(
        node.tag_name().name(),
        node.tag_name().namespace().unwrap_or(""),
        namespace,
    )
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn element_name_value(
    local: &str,
    ns: &str,
    namespace: &mut Option<String>,
) -> Result<String, String> {
    let name = match ns {
        "" | SVG => {
            if let Some(previous) = namespace.as_deref() {
                if previous != ns {
                    return Err(format!(
                        "element <{}> mixes SVG and empty namespaces",
                        shown(local)
                    ));
                }
            } else {
                *namespace = Some(ns.into());
            }
            match local {
                "script" | "foreignObject" => return Err(format!("forbidden element <{local}>")),
                "radialGradient" | "pattern" => {
                    return Err(format!("unsupported paint server <{local}>"));
                }
                _ if specific(local).is_some() && !local.contains(':') => local.into(),
                _ => return Err(format!("unknown element <{}>", shown(local))),
            }
        }
        RDF if local == "RDF" => "rdf:RDF".into(),
        CC if local == "Work" => "cc:Work".into(),
        DC if matches!(local, "format" | "type") => format!("dc:{local}"),
        SODIPODI if local == "namedview" => "sodipodi:namedview".into(),
        _ => {
            return Err(format!(
                "element <{}> uses unsupported namespace or metadata name",
                shown(local)
            ));
        }
    };
    Ok(name)
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn trim(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\r', '\n'])
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn metadata_attribute(
    element: &str,
    attribute: roxmltree::Attribute<'_, '_>,
) -> Result<bool, String> {
    metadata_attribute_value(
        element,
        attribute.name(),
        attribute.namespace().unwrap_or(""),
        attribute.value(),
    )
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn metadata_attribute_value(
    element: &str,
    name: &str,
    ns: &str,
    value: &str,
) -> Result<bool, String> {
    let error = |kind: &str| {
        format!(
            "element <{element}> has unsupported {kind} metadata attribute {:?}",
            shown(name)
        )
    };
    match ns {
        XML => {
            if name != "space" {
                return Err(format!(
                    "element <{element}> has unsupported xml attribute {:?}",
                    shown(name)
                ));
            }
            if !matches!(trim(value), "default" | "preserve") {
                return Err(format!("element <{element}> has invalid xml:space"));
            }
            return Ok(true);
        }
        INKSCAPE => {
            let allowed: &[&str] = match element {
                "svg" => &["export-filename", "export-xdpi", "export-ydpi", "version"],
                "g" => &["groupmode", "label"],
                "path" => &[
                    "connector-curvature",
                    "export-filename",
                    "export-xdpi",
                    "export-ydpi",
                ],
                "sodipodi:namedview" => &[
                    "current-layer",
                    "cx",
                    "cy",
                    "document-units",
                    "pageopacity",
                    "pageshadow",
                    "snap-global",
                    "window-height",
                    "window-maximized",
                    "window-width",
                    "window-x",
                    "window-y",
                    "zoom",
                ],
                _ => &[],
            };
            if !allowed.contains(&name) {
                return Err(error("Inkscape"));
            }
            return Ok(true);
        }
        SODIPODI => {
            if !matches!((element, name), ("svg", "docname") | ("path", "nodetypes")) {
                return Err(error("Sodipodi"));
            }
            return Ok(true);
        }
        RDF => {
            if (element == "cc:Work" && name == "about" && value.is_empty())
                || (element == "dc:type"
                    && name == "resource"
                    && value == "http://purl.org/dc/dcmitype/StillImage")
            {
                return Ok(true);
            }
            return Err(error("RDF"));
        }
        _ => {}
    }
    if !ns.is_empty() {
        return Ok(false);
    }
    if let Some(suffix) = name.strip_prefix("data-") {
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(format!(
                "element <{element}> has invalid data-* metadata attribute"
            ));
        }
        return Ok(true);
    }
    if name == "role" {
        if !matches!(trim(value), "img" | "none" | "presentation") {
            return Err(format!(
                "element <{element}> has unsupported non-rendering role"
            ));
        }
        return Ok(true);
    }
    if name == "focusable" {
        if trim(value) != "false" {
            return Err(format!(
                "element <{element}> focusable metadata must be false"
            ));
        }
        return Ok(true);
    }
    Ok(false)
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn name_byte(byte: u8) -> bool {
    byte >= 128 || byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'-')
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn check_name_at(source: &str, start: usize) -> Result<(), String> {
    let mut end = start;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| name_byte(*byte))
    {
        end += 1;
    }
    let Some(name) = source.get(start..end) else {
        return Ok(());
    };
    if !crate::go_xml_names::valid(name) || name.matches(':').count() > 1 {
        return Err(format!("invalid XML near byte {end}"));
    }
    Ok(())
}
#[cfg(all(test, feature = "oracle-fixtures"))]
fn name_parse_error(source: &str, error: &roxmltree::Error) -> Option<String> {
    if !matches!(
        error,
        roxmltree::Error::InvalidName(_)
            | roxmltree::Error::InvalidChar(b'=', 128..=255, _)
            | roxmltree::Error::InvalidChar2("a whitespace", 128..=255, _)
    ) {
        return None;
    }
    let position = error.pos();
    let mut row = 1;
    let mut column = 1;
    let mut offset = source.len();
    for (at, c) in source.char_indices() {
        if row == position.row && column == position.col {
            offset = at;
            break;
        }
        if c == '\n' {
            row += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    let mut start = offset;
    while start > 0 && name_byte(source.as_bytes()[start - 1]) {
        start -= 1;
    }
    check_name_at(source, start).err()
}

// roxmltree joins adjacent text/CDATA but retains only the first token's
// range. Source encoding/xml reports the end of the first non-space token.
// Recover those lexical boundaries from already validated XML, including
// numeric references to the four XML whitespace characters.
#[cfg(all(test, feature = "oracle-fixtures"))]
fn nonspace_text_end(source: &str, mut offset: usize) -> Option<usize> {
    while offset < source.len() {
        let rest = &source[offset..];
        if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let length = cdata.find("]]>")?;
            let end = offset + 9 + length + 3;
            if !trim(&cdata[..length]).is_empty() {
                return Some(end);
            }
            offset = end;
            continue;
        }
        if rest.starts_with('<') {
            return None;
        }
        let length = rest.find('<').unwrap_or(rest.len());
        let end = offset + length;
        let mut text = &source[offset..end];
        while !text.is_empty() {
            if let Some(entity) = text.strip_prefix('&') {
                let finish = entity.find(';')?;
                let name = &entity[..finish];
                let value = name
                    .strip_prefix("#x")
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| {
                        name.strip_prefix('#')
                            .and_then(|number| number.parse::<u32>().ok())
                    });
                if !matches!(value, Some(9 | 10 | 13 | 32)) {
                    return Some(end);
                }
                text = &entity[finish + 1..];
            } else {
                let character = text.chars().next()?;
                if !matches!(character, ' ' | '\t' | '\r' | '\n') {
                    return Some(end);
                }
                text = &text[character.len_utf8()..];
            }
        }
        offset = end;
    }
    None
}

// Source boilerplate.go:validateNonRenderingTree runs after all parse-level
// checks, before any element compilation. Its cardinality pass deliberately
// visits siblings in reverse order, independently of the topology pass.
#[cfg(all(test, feature = "oracle-fixtures"))]
fn validate_metadata_tree(
    document: &roxmltree::Document<'_>,
    names: &HashMap<roxmltree::NodeId, String>,
) -> Result<(), String> {
    let root = document.root_element();
    for node in document.descendants().filter(|node| node.is_element()) {
        let name = names[&node.id()].as_str();
        let parent = node.parent().filter(|node| node.is_element());
        let parent_name = parent.map(|node| names[&node.id()].as_str());
        if let Some(parent_name) = parent_name {
            match parent_name {
                "metadata" if name != "rdf:RDF" => {
                    return Err(format!(
                        "element <metadata> has unsupported child <{}>",
                        shown(name)
                    ));
                }
                "rdf:RDF" if name != "cc:Work" => {
                    return Err(format!(
                        "element <rdf:RDF> has unsupported child <{}>",
                        shown(name)
                    ));
                }
                "cc:Work" if !matches!(name, "dc:format" | "dc:type") => {
                    return Err(format!(
                        "element <cc:Work> has unsupported child <{}>",
                        shown(name)
                    ));
                }
                "title" | "dc:format" | "dc:type" | "sodipodi:namedview" => {
                    return Err(format!(
                        "non-rendering element <{parent_name}> cannot contain child elements"
                    ));
                }
                _ => {}
            }
        }
        match name {
            "metadata" | "sodipodi:namedview" if parent != Some(root) => {
                return Err(format!(
                    "non-rendering element <{name}> must be a direct child of the root <svg>"
                ));
            }
            "rdf:RDF" if parent_name != Some("metadata") => {
                return Err("element <rdf:RDF> is only supported inside <metadata>".into());
            }
            "cc:Work" if parent_name != Some("rdf:RDF") => {
                return Err("element <cc:Work> is only supported inside <rdf:RDF>".into());
            }
            "dc:format" | "dc:type" if parent_name != Some("cc:Work") => {
                return Err(format!(
                    "element <{name}> is only supported inside <cc:Work>"
                ));
            }
            _ => {}
        }
        if name == "dc:format" {
            let text: String = node
                .children()
                .filter_map(|child| child.text().filter(|_| child.is_text()))
                .collect();
            if trim(&text) != "image/svg+xml" {
                return Err("element <dc:format> has unsupported metadata text".into());
            }
        }
    }
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let name = names[&node.id()].as_str();
        let children: Vec<_> = node.children().filter(|node| node.is_element()).collect();
        match name {
            "metadata" if children.len() != 1 || names[&children[0].id()] != "rdf:RDF" => {
                return Err("element <metadata> must contain exactly one <rdf:RDF>".into());
            }
            "rdf:RDF" if children.len() != 1 || names[&children[0].id()] != "cc:Work" => {
                return Err("element <rdf:RDF> must contain exactly one <cc:Work>".into());
            }
            "cc:Work"
                if children.len() != 2
                    || children
                        .iter()
                        .filter(|child| names[&child.id()] == "dc:format")
                        .count()
                        != 1
                    || children
                        .iter()
                        .filter(|child| names[&child.id()] == "dc:type")
                        .count()
                        != 1 =>
            {
                return Err(
                    "element <cc:Work> must contain exactly one <dc:format> and one <dc:type>"
                        .into(),
                );
            }
            _ => {}
        }
        stack.extend(children);
    }
    Ok(())
}

/// Historical envelope fixtures exercise the shared lexical implementation.
#[cfg(all(test, feature = "oracle-fixtures"))]
fn validate_asset_envelope(source: &str) -> Result<(), String> {
    super::svg_asset_xml::validate(source)
}

// Source parse/attributes contracts run on Go-compatible expanded token
// names before the strict native document parser can discard duplicates or
// refuse namespace declarations. Compilation remains a later stage.
#[cfg(all(test, feature = "oracle-fixtures"))]
fn validate_token_attributes(source: &str) -> Result<(), String> {
    use super::svg_asset_xml::Event;
    let Some(events) = super::svg_asset_xml::events(source) else {
        return Ok(());
    };
    let mut stack: Vec<String> = Vec::new();
    let mut namespace = None;
    let mut ids = HashSet::new();
    let mut seen_root = false;
    let mut envelope_errors = super::svg_import_envelope::scan(source)
        .errors
        .into_iter()
        .peekable();
    for event in events {
        match event {
            Event::Envelope { start } => {
                if envelope_errors
                    .peek()
                    .is_some_and(|(offset, _)| *offset <= start)
                {
                    return Err(envelope_errors.next().unwrap().1);
                }
            }
            Event::Comment if stack.last().is_some_and(|name| name == "style") => {
                return Err("CSS comments in <style> are unsupported".into());
            }
            Event::Text { value, end } => {
                if !matches!(
                    stack.last().map(String::as_str),
                    Some("style" | "title" | "dc:format" | "text")
                ) && !trim(&value).is_empty()
                {
                    return Err(format!("text content is unsupported near byte {end}"));
                }
            }
            Event::End => {
                stack.pop();
            }
            Event::Start {
                namespace: ns,
                name: local,
                attributes,
                end,
            } => {
                if stack.is_empty() && seen_root {
                    return Err(format!("multiple root elements near byte {end}"));
                }
                let name = element_name_value(&local, &ns, &mut namespace)?;
                if !seen_root && name != "svg" {
                    return Err(format!("root element must be <svg>, got <{name}>"));
                }
                seen_root = true;
                let mut expanded = HashSet::new();
                let mut names = HashSet::new();
                let mut node_id = None;
                for attribute in attributes {
                    let key = attribute.name.as_str();
                    let ns = attribute.namespace.as_str();
                    let value = attribute.value.as_str();
                    if !expanded.insert((attribute.namespace.clone(), attribute.name.clone())) {
                        return Err(format!(
                            "element <{name}> has duplicate XML attribute {:?}",
                            shown(key)
                        ));
                    }
                    if ns == "xmlns" || (ns.is_empty() && key == "xmlns") {
                        let prefix = if ns == "xmlns" { key } else { "" };
                        if !prefix.is_empty() && value.is_empty() {
                            return Err(format!(
                                "element <{name}> has an empty prefixed namespace binding"
                            ));
                        }
                        if prefix.eq_ignore_ascii_case("xmlns")
                            || value == "http://www.w3.org/2000/xmlns/"
                        {
                            return Err(format!(
                                "element <{name}> has a forbidden xmlns namespace binding"
                            ));
                        }
                        if prefix == "xml" {
                            if value != XML {
                                return Err(format!(
                                    "element <{name}> has an invalid xml namespace binding"
                                ));
                            }
                        } else if prefix
                            .get(..3)
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("xml"))
                            || value == XML
                        {
                            return Err(format!(
                                "element <{name}> has a reserved xml namespace binding"
                            ));
                        }
                        continue;
                    }
                    if !ns.is_empty() && metadata_attribute_value(&name, key, ns, value)? {
                        continue;
                    }
                    if ns == XLINK && key != "href" {
                        return Err(format!(
                            "element <{name}> has unsupported xlink attribute {:?}",
                            shown(key)
                        ));
                    }
                    if !matches!(ns, "" | XLINK) {
                        return Err(format!(
                            "element <{name}> attribute {:?} uses unsupported namespace",
                            shown(key)
                        ));
                    }
                    if key
                        .get(..2)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("on"))
                    {
                        return Err(format!(
                            "element <{name}> has forbidden event attribute {:?}",
                            shown(key)
                        ));
                    }
                    if metadata_attribute_value(&name, key, ns, value)? {
                        continue;
                    }
                    if !names.insert(attribute.name.clone()) {
                        return Err(format!(
                            "element <{name}> has duplicate attribute {:?}",
                            shown(key)
                        ));
                    }
                    if key == "id" {
                        if value.is_empty()
                            || value
                                .chars()
                                .any(|c| c == '#' || c.is_whitespace() || c.is_control())
                        {
                            return Err(format!("element <{name}> has invalid id"));
                        }
                        node_id = Some(attribute.value.clone());
                    }
                }
                if node_id.is_some_and(|id| !ids.insert(id)) {
                    return Err("duplicate id".into());
                }
                stack.push(name);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
fn validate_compiled_attributes(source: &str) -> Result<(), String> {
    use super::{svg_asset_xml::Event, svg_import_geometry};
    let Some(events) = super::svg_asset_xml::events(source) else {
        return Ok(());
    };
    let mut depth = 0usize;
    let mut namespace = None;
    let mut root_attrs = None;
    for event in events {
        match event {
            Event::End => {
                depth = depth.saturating_sub(1);
            }
            Event::Start {
                namespace: ns,
                name: local,
                attributes,
                ..
            } => {
                let name = element_name_value(&local, &ns, &mut namespace)?;
                let root = depth == 0;
                depth += 1;
                let mut attrs = HashMap::new();
                let allowed = specific(&name).expect("supported element");
                for attribute in attributes {
                    let key = attribute.name.as_str();
                    let ns = attribute.namespace.as_str();
                    if ns == "xmlns"
                        || (ns.is_empty() && key == "xmlns")
                        || metadata_attribute_value(&name, key, ns, &attribute.value)?
                    {
                        continue;
                    }
                    let common = !nonrendering(&name)
                        && !matches!(name.as_str(), "style" | "linearGradient" | "stop")
                        && (matches!(key, "id" | "class" | "style") || PRESENTATION.contains(&key));
                    if !common && !allowed.contains(&key) {
                        return Err(format!(
                            "element <{name}> has unsupported attribute {:?}",
                            shown(key)
                        ));
                    }
                    attrs.insert(attribute.name, attribute.value);
                }
                svg_import_geometry::compile(&name, &attrs, root)?;
                if root {
                    root_attrs = Some(attrs);
                }
            }
            _ => {}
        }
    }
    if let Some(attrs) = root_attrs {
        svg_import_geometry::root_viewport(&attrs)?;
    }
    Ok(())
}

/// Validate parsed element/attribute contracts. XML-token limits and malformed
/// XML diagnostics require the complete importer and are not inferred here.
#[cfg(all(test, feature = "oracle-fixtures"))]
pub(crate) fn validate_attributes(source: &str) -> Result<(), String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    validate_token_attributes(source)?;
    let envelope = super::svg_import_envelope::scan(source);
    let mut envelope_errors = envelope.errors.into_iter().peekable();
    // If the native document parser refuses trailing content, still inspect
    // the completed first root. Go emits its semantic errors before reading
    // a second root token; envelope errors retain the latter's source offset.
    let parsed = roxmltree::Document::parse(&envelope.shadow);
    let parsed = if parsed.is_err() {
        envelope
            .first_root_end
            .filter(|end| *end < envelope.shadow.len())
            .map_or(parsed, |end| {
                roxmltree::Document::parse(&envelope.shadow[..end])
            })
    } else {
        parsed
    };
    let document = match parsed {
        Ok(document) => document,
        Err(error) => {
            if let Some((_, error)) = envelope_errors.next() {
                return Err(error);
            }
            if let Some(error) = name_parse_error(source, &error) {
                return Err(error);
            }
            return validate_compiled_attributes(source);
        }
    };
    let mut namespace = None;
    let mut ids = HashSet::new();
    let mut element_names = HashMap::new();
    for node in document.descendants() {
        if envelope_errors
            .peek()
            .is_some_and(|(offset, _)| *offset <= node.range().start)
        {
            return Err(envelope_errors.next().unwrap().1);
        }
        if !node.is_element() {
            let parent_name = node
                .parent()
                .and_then(|parent| element_names.get(&parent.id()))
                .map(String::as_str);
            if node.is_comment() && parent_name == Some("style") {
                return Err("CSS comments in <style> are unsupported".into());
            }
            if node.is_text()
                && !matches!(parent_name, Some("style" | "title" | "dc:format" | "text"))
                && !trim(node.text().unwrap_or_default()).is_empty()
            {
                let end = match nonspace_text_end(source, node.range().start) {
                    Some(end) => end,
                    None => {
                        // Shadow whitespace may join text on either side of
                        // a rejected envelope token. The original token fails
                        // before text that follows it.
                        if let Some((_, error)) = envelope_errors.next() {
                            return Err(error);
                        }
                        continue;
                    }
                };
                return Err(format!("text content is unsupported near byte {end}"));
            }
            continue;
        }
        // encoding/xml validates the full lexical token before the importer
        // sees element/attribute namespaces and semantic presentation values.
        check_name_at(source, node.range().start + 1)?;
        for attribute in node.attributes() {
            check_name_at(source, attribute.range_qname().start)?;
        }
        let name = element_name(node, &mut namespace)?;
        if node == document.root_element() && name != "svg" {
            return Err(format!("root element must be <svg>, got <{name}>"));
        }
        let mut names = HashMap::new();
        let mut node_id = None;
        for attribute in node.attributes() {
            if metadata_attribute(&name, attribute)? {
                continue;
            }
            let key = attribute.name();
            let ns = attribute.namespace().unwrap_or("");
            if ns == XLINK && key != "href" {
                return Err(format!(
                    "element <{name}> has unsupported xlink attribute {:?}",
                    shown(key)
                ));
            }
            if !matches!(ns, "" | XLINK) {
                return Err(format!(
                    "element <{name}> attribute {:?} uses unsupported namespace",
                    shown(key)
                ));
            }
            if key
                .get(..2)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("on"))
            {
                return Err(format!(
                    "element <{name}> has forbidden event attribute {:?}",
                    shown(key)
                ));
            }
            if names.insert(key, ()).is_some() {
                return Err(format!(
                    "element <{name}> has duplicate attribute {:?}",
                    shown(key)
                ));
            }
            if key == "id" {
                let id = attribute.value();
                if id.is_empty()
                    || id
                        .chars()
                        .any(|c| c == '#' || c.is_whitespace() || c.is_control())
                {
                    return Err(format!("element <{name}> has invalid id"));
                }
                node_id = Some(id);
            }
        }
        if node_id.is_some_and(|id| !ids.insert(id)) {
            return Err("duplicate id".into());
        }
        element_names.insert(node.id(), name.clone());
    }
    if let Some((_, error)) = envelope_errors.next() {
        return Err(error);
    }
    validate_metadata_tree(&document, &element_names)?;
    validate_compiled_attributes(source)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn namespace_and_duplicate_outcomes_match_pristine_go_importer() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-import-namespace-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 51);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_attributes(row["svg"].as_str().unwrap())
                .err()
                .map(|error| format!("d2svgimport: probe: {error}"))
                .unwrap_or_default();
            if actual != row["error"].as_str().unwrap() {
                failures.push(format!("{}: {actual:?} != {:?}", row["name"], row["error"]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn raw_xml_outcomes_match_pristine_go_asset_validator() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-asset-xml-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 75);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_asset_envelope(row["svg"].as_str().unwrap())
                .err()
                .unwrap_or_default();
            let expected = row["raw_error"].as_str().unwrap();
            let expected = if expected.is_empty() || expected.starts_with("unsupported SVG:") {
                expected.to_owned()
            } else {
                format!("malformed SVG: {expected}")
            };
            if actual != expected {
                failures.push(format!("{}: {actual:?} != {expected:?}", row["name"]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn asset_envelope_outcomes_match_pristine_go_probe_resource() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-asset-envelope-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 82);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_asset_envelope(row["svg"].as_str().unwrap())
                .err()
                .unwrap_or_default();
            if actual != row["error"].as_str().unwrap() {
                failures.push(format!("{}: {actual:?} != {:?}", row["name"], row["error"]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn envelope_outcomes_match_pristine_go_importer() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-import-envelope-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 82);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_attributes(row["svg"].as_str().unwrap())
                .err()
                .map(|error| format!("d2svgimport: probe: {error}"))
                .unwrap_or_default();
            if actual != row["error"].as_str().unwrap() {
                failures.push(format!("{}: {actual:?} != {:?}", row["name"], row["error"]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn metadata_topology_and_parse_precedence_match_pristine_go_importer() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-import-topology-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 48);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_attributes(row["svg"].as_str().unwrap())
                .err()
                .map(|error| format!("d2svgimport: probe: {error}"))
                .unwrap_or_default();
            if actual != row["error"].as_str().unwrap() {
                failures.push(format!("{}: {actual:?} != {:?}", row["name"], row["error"]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn element_attribute_outcomes_match_pristine_go_importer() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-import-attribute-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 62);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_attributes(row["svg"].as_str().unwrap())
                .err()
                .map(|error| format!("d2svgimport: probe: {error}"))
                .unwrap_or_default();
            let expected = row["error"].as_str().unwrap();
            if actual != expected {
                failures.push(format!(
                    "{}: actual={actual:?} expected={expected:?} XML={:?}",
                    row["name"].as_str().unwrap(),
                    roxmltree::Document::parse(row["svg"].as_str().unwrap()).err()
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn geometry_and_viewport_outcomes_match_pristine_go_importer() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-import-viewport-outcomes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 152);
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let actual = super::validate_attributes(row["svg"].as_str().unwrap())
                .err()
                .map(|error| format!("d2svgimport: probe: {error}"))
                .unwrap_or_default();
            if actual != row["error"].as_str().unwrap() {
                failures.push(format!("{}: {actual:?} != {:?}", row["name"], row["error"]));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
