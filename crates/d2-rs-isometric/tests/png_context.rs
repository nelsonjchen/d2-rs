// SPDX-License-Identifier: MPL-2.0
//! Public PNG admission/framing and deadlines on original golden-board inputs.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_isometric::{
    img::{
        context::RenderError,
        page::{PageError, render_page_with_context},
        render::{Options, normalize, render_png_with_context},
    },
    target::Diagram,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_png_calls_and_deadline_boards() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/png-api.json"
    ))
    .unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 18);
    for (index, call) in calls.iter().enumerate() {
        let request = &call["request"];
        let diagram: Option<Diagram> = serde_json::from_value(request["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &request["options"];
        let options = Options {
            format: o["Format"].as_str().unwrap_or("").into(),
            width: o["Width"].as_i64().unwrap_or(0),
            height: o["Height"].as_i64().unwrap_or(0),
            fit_content: o["FitContent"].as_bool().unwrap_or(false),
            timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
            ..Default::default()
        };
        let token = CancellationToken::default();
        if request["context"] == "context canceled" {
            token.cancel();
        }
        let result = if request["api"] == "page" {
            render_page_with_context(Some(&token), diagram.as_ref(), Some(&options))
                .map(|p| p.png)
                .map_err(|e| e.to_string())
        } else {
            // Render dispatches only after the original format admission.
            normalize(Some(&options)).and_then(|o| {
                render_png_with_context(Some(&token), diagram.as_ref(), Some(&o))
                    .map_err(|e| e.to_string())
            })
        };
        if let Some(hash) = call["want"]["sha256"].as_str() {
            let png = result.unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&png)), hash, "call {index}");
            assert_eq!(png.len() as u64, call["want"]["bytes"].as_u64().unwrap());
        } else {
            assert_eq!(result.unwrap_err(), call["want"]["error"], "call {index}");
        }
        assert_eq!(serde_json::to_value(&diagram).unwrap(), before);
        if options.timeout_ns == 1 {
            assert!(!token.is_cancelled(), "child deadline escaped to caller");
        }
    }
}

#[test]
fn png_caller_causes_and_admission_follow_open_capture_order() {
    let token = CancellationToken::default();
    token.cancel();
    assert!(matches!(
        render_png_with_context(Some(&token), None, None).unwrap_err(),
        RenderError::Context(LayoutError::Cancelled)
    ));
    assert!(matches!(
        render_png_with_context(
            Some(&token),
            None,
            Some(&Options {
                width: 1,
                height: 1,
                ..Default::default()
            })
        )
        .unwrap_err(),
        RenderError::Message(_)
    ));
    let options = Options {
        timeout_ns: 1,
        ..Default::default()
    };
    assert_eq!(
        render_png_with_context(None, None, Some(&options))
            .unwrap_err()
            .to_string(),
        "isometric: nil diagram",
        "BuildScene's nil-diagram failure precedes the first child deadline poll"
    );
    let token = CancellationToken::default();
    token.expire();
    let error = render_png_with_context(Some(&token), None, None).unwrap_err();
    assert!(matches!(
        error,
        RenderError::Context(LayoutError::DeadlineExceeded)
    ));
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .is::<LayoutError>()
    );
}

#[derive(Debug)]
struct CancelAsset(CancellationToken);
impl d2_rs_scenebuild::ImageResolver for CancelAsset {
    fn resolve(
        &self,
        _: &str,
    ) -> Result<Option<d2_rs_scenebuild::ImageResource>, d2_rs_scenebuild::ImageResolveError> {
        self.0.cancel();
        // Context must win before these deliberately invalid encoded bytes are
        // handed to a foundation decoder, on the existing post-resolver poll.
        Ok(Some(d2_rs_scenebuild::ImageResource {
            kind: d2_rs_scenebuild::ImageKind::Raster,
            mime_type: "image/png".into(),
            data: std::sync::Arc::from([]),
            pixel_width: 1,
            pixel_height: 1,
            decoded_bytes: 4,
        }))
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn png_and_page_share_typed_mesh_cancellation() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/whole-assets.json"
    ))
    .unwrap();
    let diagram: Diagram = serde_json::from_value(fixture["cases"][0]["diagram"].clone()).unwrap();
    for page in [false, true] {
        let token = CancellationToken::default();
        let options = Options {
            width: 320,
            height: 200,
            assets: Some(d2_rs_scenebuild::AssetOptions {
                resolver_impl: Some(std::sync::Arc::new(CancelAsset(token.clone()))),
                ..Default::default()
            }),
            ..Default::default()
        };
        let message = if page {
            let error =
                render_page_with_context(Some(&token), Some(&diagram), Some(&options)).unwrap_err();
            assert!(matches!(
                error,
                PageError::StageContext {
                    cause: LayoutError::Cancelled,
                    ..
                }
            ));
            assert!(
                std::error::Error::source(&error)
                    .unwrap()
                    .is::<LayoutError>()
            );
            error.to_string()
        } else {
            let error =
                render_png_with_context(Some(&token), Some(&diagram), Some(&options)).unwrap_err();
            assert!(matches!(
                error,
                RenderError::StageContext {
                    cause: LayoutError::Cancelled,
                    ..
                }
            ));
            error.to_string()
        };
        assert!(
            message.starts_with("initialize native isometric renderer: "),
            "{message}"
        );
        assert!(message.ends_with("context canceled"), "{message}");
    }
}
