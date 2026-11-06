// SPDX-License-Identifier: MPL-2.0
//! All twenty original TestPNG boards, independently reproduced by pinned Go.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_isometric::{
    img::render::{Options, render_png_with_context},
    target::Diagram,
};
use sha2::{Digest, Sha256};

fn boards<'a>(d: &'a Diagram, name: String, result: &mut Vec<(String, &'a Diagram)>) {
    if !d.is_folder_only {
        result.push((name.clone(), d));
    }
    for (kind, children) in [
        ("layers", &d.layers),
        ("scenarios", &d.scenarios),
        ("steps", &d.steps),
    ] {
        for (i, child) in children.iter().flatten().enumerate() {
            if let Some(child) = child {
                boards(child, format!("{name}.{kind}.{i}"), result);
            }
        }
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn upstream_complete_native_png_boards() {
    let fixture: serde_json::Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/png.json")).unwrap();
    let mut count = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let diagram: Diagram = serde_json::from_value(case["diagram"].clone()).unwrap();
        let mut list = vec![];
        boards(&diagram, "isometric".into(), &mut list);
        assert_eq!(list.len(), case["expected"].as_object().unwrap().len());
        for (name, board) in list {
            let data = render_png_with_context(
                None,
                Some(board),
                Some(&Options {
                    width: 1200,
                    height: 1200,
                    fit_content: true,
                    ..Default::default()
                }),
            )
            .unwrap();
            let hash = format!("{:x}", Sha256::digest(&data));
            assert_eq!(
                hash,
                case["expected"][&name]["png_sha256"].as_str().unwrap(),
                "{}/{}",
                case["fixture"],
                name
            );
            count += 1;
        }
    }
    assert_eq!(count, 20);
}
