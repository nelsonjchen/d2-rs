// SPDX-License-Identifier: MPL-2.0
//! Original native_page_test.go source-root replay, independent of mesh goldens.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_isometric::img::native_page::native_remove_page_glyph;
use d2_rs_scene::{Node, Primitive, Stroke};
use serde_json::{Value, json};

fn outer_stroke(root: &Node) -> Option<&Stroke> {
    if let Some(Primitive::Path(p)) = &root.primitive {
        if let Some(stroke) = &p.stroke {
            return Some(stroke);
        }
    }
    root.children.iter().flatten().find_map(outer_stroke)
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/page-glyph-api.json").into()
        });
    let cases: Value = serde_json::from_str(&source).unwrap();
    let output: Vec<Vec<Value>> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| {
                    let req = &op["request"];
                    assert_eq!(req["op"], "remove-page-glyph");
                    let mut root: Node = serde_json::from_value(req["root"].clone()).unwrap();
                    // The mutable visitor must leave the original outer stroke in place.
                    let before = outer_stroke(&root).map(std::ptr::from_ref);
                    native_remove_page_glyph(&mut root);
                    let after = outer_stroke(&root).map(std::ptr::from_ref);
                    json!({"root":root,"outer_preserved":before.is_some() && before == after})
                })
                .collect()
        })
        .collect();
    if let Ok(path) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&output).unwrap()).unwrap();
    } else {
        let expected: Vec<Vec<Value>> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect()
            })
            .collect();
        assert_eq!(output.len(), expected.len());
        for (actual, expected) in output.iter().zip(&expected) {
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                // Compare the typed scene: Go writes 1 where serde writes 1.0.
                // Every scene field remains present in the external JSON gate.
                let actual_root: Node = serde_json::from_value(actual["root"].clone()).unwrap();
                let expected_root: Node = serde_json::from_value(expected["root"].clone()).unwrap();
                assert_eq!(actual_root, expected_root);
                assert_eq!(actual["outer_preserved"], expected["outer_preserved"]);
            }
        }
    }
}
