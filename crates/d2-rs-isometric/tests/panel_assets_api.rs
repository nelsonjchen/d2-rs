// SPDX-License-Identifier: MPL-2.0
//! Public SVG/PNG/Page results on the three original legend-mono boards.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
#[path = "../examples/support/api_options.rs"]
mod api_options;

use d2_rs_isometric::{
    img::{
        page::render_page,
        render::{Options, render_png_with_context, render_svg_with_context},
    },
    target::Diagram,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn whole_board_legend_assets_and_caller_limits() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/panel-assets-api.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 27);
    for (index, call) in calls.iter().enumerate() {
        let request = &call["request"];
        let diagram: Diagram = serde_json::from_value(request["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &request["options"];
        let options = Options {
            format: o["Format"].as_str().unwrap().into(),
            width: o["Width"].as_i64().unwrap(),
            height: o["Height"].as_i64().unwrap(),
            fit_content: o["FitContent"].as_bool().unwrap(),
            assets: api_options::assets(o),
            ..Default::default()
        };
        let mut annotations = None;
        let result = if request["api"] == "page" {
            render_page(Some(&diagram), Some(&options))
                .map(|p| {
                    annotations = Some(json!({"Links":p.links,"Width":p.width,"Height":p.height}));
                    p.png
                })
                .map_err(|e| e.to_string())
        } else if options.format == "svg" {
            render_svg_with_context(None, Some(&diagram), Some(&options))
                .map(String::into_bytes)
                .map_err(|e| e.to_string())
        } else {
            render_png_with_context(None, Some(&diagram), Some(&options)).map_err(|e| e.to_string())
        };
        if call["want"]["error"] != "" {
            assert_eq!(result.unwrap_err(), call["want"]["error"], "call {index}");
        } else {
            let data = result.unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&data)),
                call["want"]["sha256"],
                "call {index}"
            );
            assert_eq!(data.len() as u64, call["want"]["bytes"].as_u64().unwrap());
            if let Some(annotations) = annotations {
                assert_eq!(annotations, call["want"]["page"], "call {index}");
            }
        }
        assert_eq!(
            serde_json::to_value(&diagram).unwrap(),
            before,
            "source mutated at {index}"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn explicit_resolver_is_shared_with_the_legend_and_remains_bounded() {
    use base64::Engine;
    use d2_rs_scenebuild::{
        AssetOptions, ImageKind, ImageResolveError, ImageResolver, ImageResource, SvgImportLimits,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Debug)]
    struct Resolver {
        resource: ImageResource,
        calls: AtomicUsize,
    }
    impl ImageResolver for Resolver {
        fn resolve(&self, source: &str) -> Result<Option<ImageResource>, ImageResolveError> {
            assert_eq!(source, "https://example.invalid/legend.svg");
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Some(self.resource.clone()))
        }
    }
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/panel-assets-api.json")).unwrap();
    let call = &fixture["calls"][1]; // Dagre embedded PNG.
    let mut source = call["request"]["diagram"].clone();
    let uri = source["legend"]["shapes"][0]["icon"]["Opaque"]
        .as_str()
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(uri.split_once(',').unwrap().1)
        .unwrap();
    let resolver = Arc::new(Resolver {
        resource: ImageResource {
            kind: ImageKind::Svg,
            mime_type: "image/svg+xml".into(),
            data: bytes.into(),
            pixel_width: 40,
            pixel_height: 20,
            decoded_bytes: 0,
        },
        calls: AtomicUsize::new(0),
    });
    source["legend"]["shapes"][0]["icon"] =
        json!({"Scheme":"https","Host":"example.invalid","Path":"/legend.svg"});
    let diagram: Diagram = serde_json::from_value(source).unwrap();
    let mut options = Options {
        width: 320,
        height: 200,
        fit_content: true,
        assets: Some(AssetOptions {
            resolver_impl: Some(resolver.clone()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let png = render_png_with_context(None, Some(&diagram), Some(&options)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(png)), call["want"]["sha256"]);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    options.assets.as_mut().unwrap().svg_import_limits = SvgImportLimits {
        max_elements: 1,
        ..Default::default()
    };
    let error = render_png_with_context(None, Some(&diagram), Some(&options))
        .unwrap_err()
        .to_string();
    assert!(error.contains("element count exceeds limit 1"), "{error}");
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 2);
}
