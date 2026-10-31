// SPDX-License-Identifier: MPL-2.0
//! Whole GIFs from the original Render test and five complete board witnesses.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_isometric::{
    img::{
        context::RenderError,
        render::{Options, render_with_context},
    },
    target::Diagram,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn whole_gifs_match_every_source_frame_palette_rectangle_and_delay() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/gif-api.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 17);
    let mut total = 0;
    for (index, call) in calls.iter().enumerate() {
        let r = &call["request"];
        let diagram: Option<Diagram> = serde_json::from_value(r["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &r["options"];
        let options = Options {
            width: o["Width"].as_i64().unwrap_or(0),
            height: o["Height"].as_i64().unwrap_or(0),
            format: o["Format"].as_str().unwrap_or("").into(),
            timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
            ..Default::default()
        };
        let token = CancellationToken::default();
        if r["context"] == "context canceled" {
            token.cancel();
        }
        let result = render_with_context(Some(&token), diagram.as_ref(), Some(&options));
        let want = &call["want"];
        if want["error"] != "" {
            assert_eq!(
                result.unwrap_err().to_string(),
                want["error"],
                "call {index}"
            );
            continue;
        }
        let data = result.unwrap();
        assert_eq!(
            serde_json::to_value(&diagram).unwrap(),
            before,
            "source changed at {index}"
        );
        let mut decode = gif::DecodeOptions::new()
            .read_info(data.as_slice())
            .unwrap();
        assert_eq!(decode.width() as u64, want["canvas"][0]);
        assert_eq!(decode.height() as u64, want["canvas"][1]);
        assert_eq!(decode.repeat(), gif::Repeat::Infinite);
        let palette = decode.global_palette().unwrap().to_vec();
        assert_eq!(
            format!("{:x}", Sha256::digest(&palette)),
            want["palette_sha256"],
            "palette {index}"
        );
        let width = decode.width() as usize;
        let mut canvas = vec![0_u8; width * decode.height() as usize * 4];
        for (i, expected) in want["frames"].as_array().unwrap().iter().enumerate() {
            let frame = decode.read_next_frame().unwrap().unwrap();
            assert_eq!(frame.dispose, gif::DisposalMethod::Keep);
            assert_eq!(frame.delay as u64, expected["delay"]);
            assert!(frame.transparent.is_none());
            assert!(frame.palette.as_ref().is_none_or(|p| *p == palette));
            let rect = [
                frame.left,
                frame.top,
                frame.left + frame.width,
                frame.top + frame.height,
            ];
            assert_eq!(
                serde_json::to_value(rect).unwrap(),
                expected["rect"],
                "delta {index}/{i}"
            );
            for y in 0..frame.height as usize {
                for x in 0..frame.width as usize {
                    let color = frame.buffer[y * frame.width as usize + x] as usize * 3;
                    let at = ((frame.top as usize + y) * width + frame.left as usize + x) * 4;
                    canvas[at..at + 3].copy_from_slice(&palette[color..color + 3]);
                    canvas[at + 3] = 255;
                }
            }
            assert_eq!(
                format!("{:x}", Sha256::digest(&canvas)),
                expected["rgba_sha256"],
                "frame {index}/{i}"
            );
            total += 1;
        }
        assert!(
            decode.read_next_frame().unwrap().is_none(),
            "extra frame {index}"
        );
    }
    assert_eq!(total, 592);
}

#[test]
fn gif_cancellation_keeps_the_typed_cause_and_admission_order() {
    let token = CancellationToken::default();
    token.cancel();
    let options = Options {
        format: "gif".into(),
        width: 160,
        height: 100,
        ..Default::default()
    };
    assert!(matches!(
        render_with_context(Some(&token), None, Some(&options)),
        Err(RenderError::Context(LayoutError::Cancelled))
    ));
    let bad = Options {
        width: 1,
        ..options
    };
    assert!(
        render_with_context(Some(&token), None, Some(&bad))
            .unwrap_err()
            .to_string()
            .contains("dimensions 1x100")
    );
}
