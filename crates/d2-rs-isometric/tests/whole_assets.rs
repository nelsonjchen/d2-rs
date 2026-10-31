// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "oracle-fixtures")]
//! Original icon/image-bearing diagrams with pinned explicit asset inputs.
#[path = "../examples/support/asset_sources.rs"]
mod asset_sources;
use base64::Engine as _;
use d2_rs_isometric::{
    img::render::{Options, render_svg},
    scene::RenderOpts,
    target::Diagram,
};
use sha2::{Digest, Sha256};
#[test]
fn original_asset_boards_match_complete_go_svg_and_default_errors() {
    let fixture: serde_json::Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/whole-assets.json")).unwrap();
    let scratch =
        std::env::temp_dir().join(format!("d2rs-isometric-assets-{}", std::process::id()));
    std::fs::create_dir(&scratch).unwrap();
    let result = std::panic::catch_unwind(|| {
        let mut sources = serde_json::Map::new();
        for (url, resource) in fixture["resources"].as_object().unwrap() {
            let data = base64::engine::general_purpose::STANDARD
                .decode(resource["data"].as_str().unwrap())
                .unwrap();
            let digest = format!("{:x}", Sha256::digest(&data));
            assert_eq!(digest, resource["sha256"]);
            let path = scratch.join(digest);
            std::fs::write(&path, data).unwrap();
            sources.insert(
                url.clone(),
                serde_json::json!({"path":path,"mime":resource["mime"]}),
            );
        }
        let request = serde_json::json!({"assetSources": sources});
        for case in fixture["cases"].as_array().unwrap() {
            let diagram: Diagram = serde_json::from_value(case["diagram"].clone()).unwrap();
            let mut opts = Options {
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
            assert_eq!(
                render_svg(&diagram, &opts).unwrap_err(),
                case["default_error"].as_str().unwrap(),
                "{}",
                case["board"]
            );
            opts.assets = asset_sources::options(&request).unwrap();
            let svg = render_svg(&diagram, &opts).unwrap();
            assert!(
                svg == case["svg"].as_str().unwrap(),
                "{}: whole SVG differs",
                case["board"]
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(svg.as_bytes())),
                case["svg_sha256"]
            );
        }
    });
    std::fs::remove_dir_all(&scratch).unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
