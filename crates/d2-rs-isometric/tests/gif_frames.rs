// SPDX-License-Identifier: MPL-2.0
//! Whole PNG frames, source ownership and callback contracts from pinned Go.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_isometric::{
    img::{
        context::RenderError,
        gif_capture::{CallbackError, Capture, CaptureError, capture_frames},
        render::Options,
    },
    target::Diagram,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn options(o: &Value) -> Options {
    Options {
        width: o["Width"].as_i64().unwrap_or(0),
        height: o["Height"].as_i64().unwrap_or(0),
        format: o["Format"].as_str().unwrap_or("").into(),
        fit_content: o["FitContent"].as_bool().unwrap_or(false),
        timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
        ..Default::default()
    }
}
fn time(v: &Value) -> f64 {
    match v.as_str().unwrap() {
        "+Inf" => f64::INFINITY,
        "-Inf" => f64::NEG_INFINITY,
        value => value.parse().unwrap(),
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_capture_sessions_and_gif_frame_boards() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/gif-frames-api.json"
    ))
    .unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 15);
    let mut frames_checked = 0;
    for (index, call) in calls.iter().enumerate() {
        let r = &call["request"];
        let d: Option<Diagram> = serde_json::from_value(r["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&d).unwrap();
        let token = CancellationToken::default();
        let mut received = vec![];
        let mut emit = |i: usize, png: Vec<u8>| -> Result<(), CallbackError> {
            assert_eq!(i, received.len());
            received.push(png);
            if r["cancelAfter"].as_i64() == Some(i as i64) {
                token.cancel();
            }
            let error = r["callbackError"].as_str().unwrap();
            if error.is_empty() {
                Ok(())
            } else {
                Err(std::io::Error::other(error).into())
            }
        };
        let seconds: Vec<_> = r["seconds"].as_array().unwrap().iter().map(time).collect();
        let result = capture_frames(
            Some(&token),
            d.as_ref(),
            Some(&options(&r["options"])),
            &seconds,
            r["traffic"].as_bool().unwrap(),
            Some(&mut emit),
        );
        assert_eq!(
            result.err().map(|e| e.to_string()).unwrap_or_default(),
            call["want"]["error"],
            "call {index}"
        );
        let expected = call["want"]["frames"].as_array().unwrap();
        assert_eq!(received.len(), expected.len(), "call {index}");
        for (png, want) in received.iter().zip(expected) {
            assert_eq!(
                format!("{:x}", Sha256::digest(png)),
                want["sha256"],
                "call {index}"
            );
            assert_eq!(png.len() as u64, want["bytes"].as_u64().unwrap());
            frames_checked += 1;
        }
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            before,
            "source mutated at {index}"
        );
    }
    assert_eq!(frames_checked, 32);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn capture_preserves_callback_identity_and_checks_last_callback_cancellation() {
    #[derive(Debug)]
    struct StopEncoder;
    impl std::fmt::Display for StopEncoder {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("encoder stopped")
        }
    }
    impl std::error::Error for StopEncoder {}
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/gif-frames-api.json"
    ))
    .unwrap();
    let r = &fixture["calls"][0]["request"];
    let d: Diagram = serde_json::from_value(r["diagram"].clone()).unwrap();
    let o = options(&r["options"]);
    let mut stopped =
        |_: usize, _: Vec<u8>| -> Result<(), CallbackError> { Err(Box::new(StopEncoder)) };
    let error = capture_frames(
        None,
        Some(&d),
        Some(&o),
        &[0.0, 1.0],
        true,
        Some(&mut stopped),
    )
    .unwrap_err();
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .is::<StopEncoder>()
    );
    let token = CancellationToken::default();
    let mut count = 0;
    let mut cancel = |_: usize, _: Vec<u8>| -> Result<(), CallbackError> {
        count += 1;
        token.cancel();
        Ok(())
    };
    let error = capture_frames(
        Some(&token),
        Some(&d),
        Some(&o),
        &[0.0],
        true,
        Some(&mut cancel),
    )
    .unwrap_err();
    assert_eq!(count, 1);
    assert!(matches!(
        error,
        CaptureError::Render(RenderError::Context(LayoutError::Cancelled))
    ));
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn capture_admission_precedes_initialization_and_child_deadline_does_not_escape() {
    let token = CancellationToken::default();
    token.cancel();
    let error = capture_frames(Some(&token), None, None, &[0.0], true, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "capture needs 1..104 times and a callback"
    );
    let mut emit = |_: usize, _: Vec<u8>| -> Result<(), CallbackError> { panic!("must not emit") };
    let error =
        capture_frames(Some(&token), None, None, &[0.0], true, Some(&mut emit)).unwrap_err();
    assert!(matches!(
        error,
        CaptureError::Render(RenderError::Context(LayoutError::Cancelled))
    ));
    let error = capture_frames(None, None, None, &[0.0; 104], true, Some(&mut emit)).unwrap_err();
    assert_eq!(error.to_string(), "capture aggregate pixels exceed limit");
    let token = CancellationToken::default();
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/gif-frames-api.json"
    ))
    .unwrap();
    let d: Diagram =
        serde_json::from_value(fixture["calls"][0]["request"]["diagram"].clone()).unwrap();
    let o = Options {
        timeout_ns: 1,
        width: 160,
        height: 100,
        ..Default::default()
    };
    let error = match Capture::open(Some(&token), Some(&d), Some(&o)) {
        Ok(_) => panic!("deadline admitted"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RenderError::Context(LayoutError::DeadlineExceeded)
    ));
    assert!(!token.is_cancelled());
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn authored_node_and_packet_clocks_close_the_same_gif_seam() {
    // The upstream GIF node-loop witness uses independent 8 s authored and
    // 25/3 s packet clocks. Sampled PNGs from this board are bound above.
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/gif-frames-api.json"
    ))
    .unwrap();
    let r = &fixture["calls"][2]["request"];
    let d: Diagram = serde_json::from_value(r["diagram"].clone()).unwrap();
    let before = serde_json::to_value(&d).unwrap();
    let o = options(&r["options"]);
    let capture = Capture::open(None, Some(&d), Some(&o)).unwrap();
    let first = capture.frame_image_at(0.0, 0.0, true).unwrap();
    let last = capture.frame_image_at(8.0, 25.0 / 3.0, true).unwrap();
    assert_eq!(first.pix, last.pix);
    assert_ne!(
        first.pix,
        capture.frame_image_at(0.5, 0.5, true).unwrap().pix
    );
    assert_eq!(
        capture.frame(0.0, false).unwrap(),
        d2_rs_isometric::img::render::render_png_with_context(None, Some(&d), Some(&o)).unwrap()
    );
    assert_eq!(serde_json::to_value(&d).unwrap(), before);
}
