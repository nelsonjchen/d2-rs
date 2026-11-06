// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "oracle-fixtures")]
//! Whole-board contracts independently reproduced by pinned Go Render.
use d2_rs_isometric::{
    img::render::{Options, render_svg},
    scene::RenderOpts,
    target::Diagram,
};
#[test]
fn upstream_whole_board_svg_bytes() {
    check_boards(false);
}
#[test]
fn upstream_routed_whole_board_svg_bytes() {
    check_boards(true);
}
fn check_boards(routed: bool) {
    let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/whole-svg.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let has_routes = case["diagram"]["connections"]
            .as_array()
            .is_some_and(|c| !c.is_empty());
        if has_routes != routed {
            continue;
        }
        let diagram: Diagram = serde_json::from_value(case["diagram"].clone()).unwrap();
        let opts = Options {
            render: RenderOpts {
                theme_id: case["themeID"]
                    .as_i64()
                    .or_else(|| diagram.config.as_ref().and_then(|c| c.theme_id)),
                theme_overrides: diagram
                    .config
                    .as_ref()
                    .and_then(|c| c.theme_overrides.clone()),
            },
            width: 1200,
            height: 1200,
            fit_content: true,
            ..Default::default()
        };
        let actual = render_svg(&diagram, &opts).unwrap();
        let expected = case["svg"].as_str().unwrap();
        assert!(
            actual == expected,
            "{}: whole SVG differs at byte {:?}",
            case["board"],
            actual
                .bytes()
                .zip(expected.bytes())
                .position(|(a, b)| a != b)
        );
    }
}
