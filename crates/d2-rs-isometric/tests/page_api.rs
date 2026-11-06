// SPDX-License-Identifier: MPL-2.0
//! Complete original RenderPage calls, independently reproduced by pinned Go.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use base64::Engine;
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_isometric::{
    img::{
        page::{PageError, render_page_with_context},
        render::{Options, normalize},
    },
    target::Diagram,
};
use serde_json::{Value, json};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_public_page_calls() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/page-api.json"
    ))
    .unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 29);
    for (index, call) in calls.iter().enumerate() {
        let request = &call["request"];
        let diagram: Diagram = serde_json::from_value(request["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &request["options"];
        let options = Options {
            width: o["Width"].as_i64().unwrap_or(0),
            height: o["Height"].as_i64().unwrap_or(0),
            fit_content: o["FitContent"].as_bool().unwrap_or(false),
            link_budget: o.get("LinkBudget").filter(|v| !v.is_null()).map(|b| {
                d2_rs_scenebuild::LinkBudget {
                    max_regions: b["MaxRegions"].as_i64().unwrap(),
                    max_string_bytes: b["MaxStringBytes"].as_i64().unwrap(),
                }
            }),
            ..Default::default()
        };
        let token = CancellationToken::default();
        if request["context"] == "context canceled" {
            token.cancel();
        }
        let got = match render_page_with_context(Some(&token), Some(&diagram), Some(&options)) {
            Ok(page) => {
                json!({"result":{"PNG":base64::engine::general_purpose::STANDARD.encode(page.png),"Links":page.links,"Width":page.width,"Height":page.height},"error":""})
            }
            Err(error) => {
                if request["context"] == "context canceled" {
                    assert!(matches!(error, PageError::Context(LayoutError::Cancelled)));
                }
                json!({"result":null,"error":error.to_string()})
            }
        };
        assert_eq!(got, call["want"], "original call {index}");
        assert_eq!(
            serde_json::to_value(&diagram).unwrap(),
            before,
            "source mutated at {index}"
        );
    }
}
#[test]
fn page_admission_precedes_context_and_keeps_deadline_cause() {
    let token = CancellationToken::default();
    token.expire();
    let d = Diagram::default();
    let error = render_page_with_context(Some(&token), Some(&d), None).unwrap_err();
    assert!(matches!(
        error,
        PageError::Context(LayoutError::DeadlineExceeded)
    ));
    let error = render_page_with_context(
        Some(&token),
        Some(&d),
        Some(&Options {
            format: "svg".into(),
            ..Default::default()
        }),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "isometric page export requires PNG rendering"
    );
    let error = render_page_with_context(Some(&token), None, None).unwrap_err();
    assert_eq!(error.to_string(), "isometric page requires a diagram");
}
#[test]
fn original_option_admission_boundaries() {
    for (width, height) in [
        (1, 64),
        (63, 64),
        (64, 0),
        (-1, 100),
        (4097, 100),
        (4096, 4096),
        (i64::MAX, i64::MAX),
    ] {
        assert!(
            normalize(Some(&Options {
                width,
                height,
                ..Default::default()
            }))
            .is_err()
        );
    }
    for timeout_ns in [-1, 180_000_000_000] {
        assert!(
            normalize(Some(&Options {
                timeout_ns,
                ..Default::default()
            }))
            .is_err()
        );
    }
    assert!(
        normalize(Some(&Options {
            format: "unknown".into(),
            ..Default::default()
        }))
        .is_err()
    );
    assert!(
        normalize(Some(&Options {
            format: "gif".into(),
            width: 1600,
            height: 1000,
            ..Default::default()
        }))
        .is_err()
    );
    let o = normalize(None).unwrap();
    assert_eq!(
        (o.width, o.height, o.timeout_ns, o.format.as_str()),
        (1600, 1000, 120_000_000_000, "png")
    );
    let o = normalize(Some(&Options {
        format: "gif".into(),
        ..Default::default()
    }))
    .unwrap();
    assert_eq!((o.width, o.height), (1000, 625));
    assert_eq!(
        normalize(Some(&Options {
            width: 64,
            height: 64,
            ..Default::default()
        }))
        .unwrap()
        .width,
        64
    );
}
