// SPDX-License-Identifier: MPL-2.0
// Source: original D2 v0.9.0 d2renderers/d2svgimport/mathjax_text.go.
// This is the generated-math scene boundary, not a general SVG importer.
use d2_rs_core::stable_sort::StableSort;
use roxmltree::Node;

// Original independently authored scene geometry, not a platform font glyph.
const MICRO_OUTLINE: &str = "M56 -360L136 -360L136 -92C136 -35 172 0 224 0C276 0 316 -35 316 -92L316 -360L396 -360L396 -84C396 -38 416 -16 456 -16L456 58C407 58 371 39 346 9C316 53 267 76 210 76C181 76 156 68 136 52L136 188L56 188Z";
fn children<'a, 'i>(n: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    n.children().filter(Node::is_element).collect()
}
fn attrs<'a, 'i: 'a>(n: Node<'a, 'i>) -> Vec<&'a str> {
    n.attributes()
        .filter(|a| !a.name().starts_with("data-") && !matches!(a.name(), "role" | "focusable"))
        .map(|a| a.name())
        .collect()
}
fn only(n: Node<'_, '_>, expected: &[&str]) -> bool {
    let names = attrs(n);
    names.len() == expected.len() && expected.iter().all(|name| names.contains(name))
}
fn metadata(n: Node<'_, '_>, expected: &[(&str, &str)]) -> bool {
    let actual: Vec<_> = n
        .attributes()
        .filter(|a| a.name().starts_with("data-") || matches!(a.name(), "role" | "focusable"))
        .collect();
    actual.len() == expected.len() && expected.iter().all(|(k, v)| n.attribute(*k) == Some(*v))
}
fn ex(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let number = raw.get(..raw.len().checked_sub(2)?)?;
    if !raw.get(raw.len() - 2..)?.eq_ignore_ascii_case("ex") || number.trim() != number {
        return None;
    }
    let n = number.parse::<f64>().ok()? * 8.0;
    n.is_finite().then_some(n)
}
fn align(root: Node<'_, '_>) -> bool {
    let Some(style) = root.attribute("style") else {
        return false;
    };
    let Some(value) = style.trim().strip_prefix("vertical-align:") else {
        return false;
    };
    let value = value
        .trim()
        .strip_suffix(';')
        .unwrap_or(value.trim())
        .trim();
    !value.contains(';') && ex(value).is_some()
}
fn frozen_root(root: Node<'_, '_>) -> bool {
    if !root.has_tag_name(("http://www.w3.org/2000/svg", "svg"))
        || !only(root, &["style", "width", "height", "viewBox"])
        || !metadata(root, &[("role", "img"), ("focusable", "false")])
        || !align(root)
    {
        return false;
    }
    if !["width", "height"]
        .iter()
        .all(|k| root.attribute(*k).and_then(ex).is_some_and(|n| n > 0.0))
    {
        return false;
    }
    let values: Vec<_> = svgtypes::NumberListParser::from(root.attribute("viewBox").unwrap_or(""))
        .collect::<Result<_, _>>()
        .unwrap_or_default();
    if values.len() != 4
        || values[0] != 0.0
        || !values.iter().all(|n| n.is_finite())
        || values[2] <= 0.0
        || values[3] <= 0.0
    {
        return false;
    }
    let nodes = children(root);
    if nodes.len() != 1 {
        return false;
    };
    let wrapper = nodes[0];
    if !wrapper.has_tag_name("g")
        || !only(wrapper, &["stroke", "fill", "stroke-width", "transform"])
        || !metadata(wrapper, &[])
        || wrapper.attribute("stroke") != Some("currentColor")
        || wrapper.attribute("fill") != Some("currentColor")
        || wrapper.attribute("stroke-width") != Some("0")
        || wrapper.attribute("transform") != Some("scale(1,-1)")
    {
        return false;
    }
    let nodes = children(wrapper);
    if nodes.len() != 1 {
        return false;
    };
    let math = nodes[0];
    math.has_tag_name("g") && only(math, &[]) && metadata(math, &[("data-mml-node", "math")])
}
fn declarations<'a, 'i: 'a>(n: Node<'a, 'i>) -> Vec<(&'a str, &'a str)> {
    let mut props: Vec<_> = n.attributes().map(|a| (a.name(), a.value())).collect();
    if let Some(style) = n.attribute("style") {
        for part in style.split(';') {
            if let Some((k, v)) = part.split_once(':') {
                props.push((k.trim(), v.trim()));
            }
        }
    }
    props
}
fn inherited(n: Node<'_, '_>, key: &str, default: &str) -> String {
    for node in n.ancestors().filter(Node::is_element) {
        if let Some((_, value)) = declarations(node)
            .into_iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            if !value.eq_ignore_ascii_case("inherit") {
                return value.to_owned();
            }
        }
    }
    default.to_owned()
}
fn local(n: Node<'_, '_>, key: &str, default: &str) -> String {
    let value = declarations(n)
        .into_iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v);
    match value {
        Some(v) if v.eq_ignore_ascii_case("inherit") => n
            .parent_element()
            .map_or_else(|| default.to_owned(), |p| local(p, key, default)),
        Some(v) => v.to_owned(),
        None => default.to_owned(),
    }
}
fn validate_text(text: Node<'_, '_>, root: Node<'_, '_>) -> Result<(), String> {
    let fail = |reason: &str| Err(reason.to_owned());
    // Original compile.go compileElement checks source-order attributes before
    // geometry.go enters the frozen MathJax content validator. Metadata has
    // already been removed by the source XML adaptor.
    const TEXT_ATTRIBUTES: &[&str] = &[
        "font-size",
        "font-family",
        "id",
        "class",
        "style",
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
    for name in attrs(text) {
        if !TEXT_ATTRIBUTES.contains(&name) {
            return Err(format!("element <text> has unsupported attribute {name:?}"));
        }
    }
    if !children(text).is_empty() {
        return fail("frozen MathJax <text> fallback cannot contain child elements");
    }
    let content: String = text.children().filter_map(|n| n.text()).collect();
    if content != "µ" {
        return fail("frozen MathJax <text> fallback supports only one U+00B5 micro sign");
    }
    if !only(text, &["transform", "font-size", "font-family"]) {
        return fail(
            "frozen MathJax <text> fallback has unsupported layout or presentation attributes",
        );
    }
    if text.attribute("transform") != Some("scale(1,-1)")
        || text.attribute("font-size") != Some("884px")
        || text.attribute("font-family") != Some("serif")
    {
        return fail("frozen MathJax <text> fallback must use scale(1,-1), 884px, and serif");
    }
    if !metadata(text, &[("data-variant", "normal")]) {
        return fail("frozen MathJax <text> fallback must declare data-variant=\"normal\"");
    }
    if !frozen_root(root) {
        return fail("painted <text> is supported only inside a verified frozen MathJax root");
    }
    let Some(unit) = text.parent_element() else {
        return fail(
            "frozen MathJax <text> fallback must be the sole child of an mi.MathML-Unit group",
        );
    };
    let mut classes: Vec<_> = unit
        .attribute("class")
        .unwrap_or("")
        .split_whitespace()
        .collect();
    classes.sort_unstable();
    classes.dedup();
    if !unit.has_tag_name("g")
        || children(unit).len() != 1
        || classes != ["MathML-Unit"]
        || !metadata(unit, &[("data-mml-node", "mi")])
        || !(only(unit, &["class"]) || only(unit, &["class", "transform"]))
    {
        return fail(
            "frozen MathJax <text> fallback must be the sole child of an mi.MathML-Unit group",
        );
    }
    if let Some(raw) = unit.attribute("transform") {
        let m = raw.parse::<svgtypes::Transform>().ok();
        if !raw.starts_with("translate(")
            || !m.is_some_and(|m| {
                m.a == 1.0
                    && m.b == 0.0
                    && m.c == 0.0
                    && m.d == 1.0
                    && m.e >= 0.0
                    && m.f == 0.0
                    && [m.a, m.b, m.c, m.d, m.e, m.f].iter().all(|n| n.is_finite())
            })
        {
            return fail("frozen MathJax unit group has unsupported placement transform");
        }
    }
    let math = children(children(root)[0])[0];
    if !unit.ancestors().any(|n| n == math) {
        return fail("frozen MathJax <text> fallback is outside the root math node");
    }
    // Source style.go resets display, opacity and clip-path at each node;
    // paint, visibility, fill-opacity, stroke-width and fill-rule inherit.
    let fill = inherited(text, "fill", "black");
    if local(text, "display", "inline") == "none"
        || inherited(text, "visibility", "visible") != "visible"
        || fill == "none"
        || fill.starts_with("url(")
        || inherited(text, "fill-opacity", "1").parse::<f64>().ok() != Some(1.0)
        || inherited(text, "stroke-width", "1").parse::<f64>().ok() != Some(0.0)
        || inherited(text, "fill-rule", "nonzero") != "nonzero"
        || !local(text, "clip-path", "").is_empty()
        || local(text, "opacity", "1").parse::<f64>().ok() != Some(1.0)
    {
        return fail(
            "frozen MathJax <text> fallback has unsupported inherited text paint or visibility",
        );
    }
    Ok(())
}

/// Compile the text fallback in a generated MathJax SVG into original D2
/// scene geometry. Ordinary SVG output keeps the unmodified MathJax SVG.
/// General SVG resources, stylesheets, cancellation and caller limits are
/// separate importer work; this function does not cover that API.
#[doc(hidden)]
pub fn import_scene_svg(source: &str) -> Result<String, String> {
    let doc = roxmltree::Document::parse(source).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    // Pinned d2svgimport/import.go elementName admits a strict SVG subset.
    // Math Render can emit links which the diagram scene importer rejects.
    for node in root.descendants().filter(Node::is_element) {
        let name = node.tag_name().name();
        match name {
            "svg" | "g" | "defs" | "style" | "path" | "rect" | "circle" | "ellipse" | "line"
            | "polyline" | "polygon" | "image" | "text" | "use" | "clipPath" | "linearGradient"
            | "stop" | "title" | "metadata" => {}
            "script" | "foreignObject" => return Err(format!("forbidden element <{name}>")),
            "radialGradient" | "pattern" => {
                return Err(format!("unsupported paint server <{name}>"));
            }
            _ => return Err(format!("unknown element <{name}>")),
        }
    }
    // Original compile.go/style.go validates root vertical-align even without text.
    if root.attribute("style").is_some_and(|s| {
        s.split(';')
            .filter_map(|p| p.split_once(':'))
            .any(|(k, v)| k.trim().eq_ignore_ascii_case("vertical-align") && ex(v).is_none())
    }) {
        return Err(
            "root <svg> has invalid MathJax vertical-align: expected a finite ex length".to_owned(),
        );
    }
    // Source compileElement/declarationsFor validates inline properties before
    // frozen text conversion. Reuse the general declaration contract here;
    // the generated-math raster adapter must not bypass source CSS admission.
    let mut mathjax = false;
    for node in root.descendants().filter(Node::is_element) {
        if node.attribute("style").is_some() {
            let attributes = node
                .attributes()
                .map(|a| (a.name().to_owned(), a.value().to_owned()))
                .collect();
            super::svg_declarations::collect(
                "generated math",
                node.tag_name().name(),
                node == root,
                &attributes,
                &std::collections::HashSet::new(),
                &[],
                &mut mathjax,
                None,
            )
            .map_err(|error| {
                error
                    .strip_prefix("d2svgimport: generated math: ")
                    .unwrap_or(&error)
                    .to_owned()
            })?;
        }
    }
    // Original style.go rejects negative stroke widths even on an
    // unpainted primitive. MathJax emits numeric local stroke-width values.
    for node in root.descendants().filter(Node::is_element) {
        if node
            .attribute("stroke-width")
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value < 0.0)
        {
            return Err(format!(
                "element <{}> has invalid stroke-width: must not be negative",
                node.tag_name().name()
            ));
        }
    }
    let mut replacements = Vec::new();
    for text in root.descendants().filter(|n| n.has_tag_name("text")) {
        validate_text(text, root)?;
        replacements.push((
            text.range(),
            format!("<path transform=\"scale(1,-1)\" d=\"{MICRO_OUTLINE}\"></path>"),
        ));
    }
    // compile.go paints line geometry only with a strictly positive stroke.
    // usvg's converted-line fallback otherwise supplies its default stroke.
    for line in root.descendants().filter(|n| n.has_tag_name("line")) {
        let width = line
            .ancestors()
            .find_map(|node| {
                node.attribute("stroke-width")
                    .filter(|value| !value.eq_ignore_ascii_case("inherit"))
            })
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        if width == 0.0 {
            replacements.push((line.range(), String::new()));
        }
    }
    replacements.stable_sort_by_key(|(range, _)| range.start);
    let mut result = source.to_owned();
    for (range, value) in replacements.into_iter().rev() {
        result.replace_range(range, &value);
    }
    Ok(result)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod contracts {
    use super::*;
    #[test]
    fn original_generated_math_scene_acceptance_and_diagnostics() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mathjax-scene-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 37);
        for row in rows {
            let result = import_scene_svg(row["svg"].as_str().unwrap());
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{}",
                row["expression"]
            );
            if let Ok(svg) = result {
                assert!(!svg.contains("<text"));
            }
        }
    }
    #[test]
    fn original_micro_outline_is_retained_in_the_local_text_frame() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mathjax-scene-source-v090.json"
        ))
        .unwrap();
        let row = &rows[1];
        assert_eq!(row["expression"], r"\micro");
        let scene =
            &row["scene"]["Root"]["Children"][0]["Children"][0]["Children"][0]["Children"][0];
        assert_eq!(scene["Transform"]["D"], -1);
        let commands = scene["Primitive"]["Commands"].as_array().unwrap();
        let segments = svgtypes::PathParser::from(MICRO_OUTLINE)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(segments.len(), commands.len());
        for (segment, command) in segments.iter().zip(commands) {
            let point = |key: &str, x: f64, y: f64| {
                assert_eq!(command[key]["X"].as_f64(), Some(x));
                assert_eq!(command[key]["Y"].as_f64(), Some(y));
            };
            match *segment {
                svgtypes::PathSegment::MoveTo { abs: true, x, y } => {
                    assert_eq!(command["Kind"], 0);
                    point("P1", x, y);
                }
                svgtypes::PathSegment::LineTo { abs: true, x, y } => {
                    assert_eq!(command["Kind"], 1);
                    point("P1", x, y);
                }
                svgtypes::PathSegment::CurveTo {
                    abs: true,
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    assert_eq!(command["Kind"], 3);
                    point("P1", x1, y1);
                    point("P2", x2, y2);
                    point("P3", x, y);
                }
                svgtypes::PathSegment::ClosePath { .. } => assert_eq!(command["Kind"], 5),
                _ => panic!("unexpected micro outline segment"),
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_literal_font_scene_contract {
    use super::*;
    #[test]
    fn source_generated_letter_svg_validation_precedes_frozen_text_geometry() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/literal-font-scene-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 1012);
        for row in rows {
            let result = import_scene_svg(row["svg"].as_str().unwrap());
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{}",
                row["expression"]
            );
        }
    }
}

#[cfg(test)]
mod enclose_stroke_contract {
    use super::*;
    #[test]
    fn original_generated_line_zero_stroke_is_unpainted_and_negative_stroke_is_rejected() {
        for command in ["cancel", "bcancel", "xcancel"] {
            let zero = crate::render(&format!("\\{command}[data-thickness=0]{{x}}")).unwrap();
            let imported = import_scene_svg(&zero).unwrap();
            assert!(!imported.contains("<line"));
            let negative =
                crate::render(&format!("\\{command}[data-thickness=-.1em]{{x}}")).unwrap();
            assert_eq!(
                import_scene_svg(&negative).unwrap_err(),
                "element <line> has invalid stroke-width: must not be negative"
            );
        }
    }
}

#[cfg(test)]
mod reference_admission_contract {
    use super::*;
    #[test]
    fn generated_reference_svg_is_valid_math_but_rejected_by_source_diagram_importer() {
        for expression in [
            r"\begin{equation}x=y\end{equation}\eqref{missing}",
            r"\ref{missing}",
        ] {
            let svg = crate::render(expression).unwrap();
            assert!(svg.contains("<a href=\"#\">"));
            assert_eq!(import_scene_svg(&svg).unwrap_err(), "unknown element <a>");
        }
    }
}
