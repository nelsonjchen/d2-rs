// SPDX-License-Identifier: MPL-2.0
// Full painted-primitive contract for actual pinned Go cancellation outputs.
#[cfg(feature = "oracle-fixtures")]
use super::*;
use std::collections::BTreeMap;
type Matrix = [f64; 6];
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
#[derive(Debug, PartialEq)]
pub(super) struct Primitive {
    kind: String,
    attributes: BTreeMap<String, String>,
    world: Matrix,
    paint: BTreeMap<String, String>,
    text: String,
}
const PAINT: [&str; 8] = [
    "fill",
    "stroke",
    "stroke-width",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
];
pub(super) fn inventory(svg: &str) -> (BTreeMap<String, String>, Vec<Primitive>, Vec<String>) {
    fn walk(
        node: roxmltree::Node<'_, '_>,
        parent: Matrix,
        paint: &BTreeMap<String, String>,
        out: &mut Vec<Primitive>,
        errors: &mut Vec<String>,
    ) {
        let mut own = paint.clone();
        for a in node.attributes() {
            if PAINT.contains(&a.name()) {
                own.insert(a.name().into(), a.value().into());
            }
        }
        let transform = node
            .attribute("transform")
            .map(|s| s.parse::<svgtypes::Transform>().unwrap())
            .unwrap_or_default();
        let world = multiply(
            parent,
            [
                transform.a,
                transform.b,
                transform.c,
                transform.d,
                transform.e,
                transform.f,
            ],
        );
        if let Some(error) = node.attribute("data-mjx-error") {
            errors.push(error.into());
        }
        let kind = node.tag_name().name();
        if [
            "path", "rect", "line", "circle", "ellipse", "polygon", "polyline", "text",
        ]
        .contains(&kind)
            || (kind == "svg" && node.parent_element().is_some())
        {
            let attributes = node
                .attributes()
                .filter(|a| a.name() != "transform" && !PAINT.contains(&a.name()))
                .map(|a| (a.name().into(), a.value().into()))
                .collect();
            let text = if kind == "text" {
                node.descendants()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .collect()
            } else {
                String::new()
            };
            out.push(Primitive {
                kind: kind.into(),
                attributes,
                world,
                paint: own.clone(),
                text,
            });
        }
        for child in node.children().filter(roxmltree::Node::is_element) {
            walk(child, world, &own, out, errors);
        }
    }
    let doc = roxmltree::Document::parse(svg).unwrap();
    let root = doc.root_element();
    assert_eq!(
        root.tag_name().namespace(),
        Some("http://www.w3.org/2000/svg")
    );
    let attributes = root
        .attributes()
        .map(|a| (a.name().into(), a.value().into()))
        .collect();
    let mut primitives = Vec::new();
    let mut errors = Vec::new();
    walk(
        root,
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        &BTreeMap::new(),
        &mut primitives,
        &mut errors,
    );
    (attributes, primitives, errors)
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn native_cancellation_targets_options_bounds_and_paint_match_original_go() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/enclose-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 225);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let size = measure(expression);
        let svg = render(expression);
        assert_eq!(
            size.as_ref().ok().map(|s| s.width),
            row["width"].as_f64(),
            "{expression} width"
        );
        assert_eq!(
            size.as_ref().ok().map(|s| s.height),
            row["height"].as_f64(),
            "{expression} height"
        );
        assert_eq!(
            size.err().map(|e| e.to_string()),
            row["measure_error"].as_str().map(str::to_owned),
            "{expression} measure error"
        );
        match svg {
            Ok(svg) => {
                assert!(row["render_error"].is_null());
                assert_eq!(
                    inventory(&svg),
                    inventory(row["svg"].as_str().unwrap()),
                    "{expression}"
                );
            }
            Err(error) => assert_eq!(
                Some(error.to_string()),
                row["render_error"].as_str().map(str::to_owned),
                "{expression} render error"
            ),
        }
    }
}
