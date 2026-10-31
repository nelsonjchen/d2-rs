// SPDX-License-Identifier: MPL-2.0
//! Original ordered-board GIFs: common framing, palette, ownership and timing.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_isometric::{
    img::{context::RenderError, render::Options, timeline::render_timeline},
    target::Diagram,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn ordered_boards_match_every_source_frame_palette_rectangle_and_delay() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/timeline-api.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 12);
    let mut total = 0;
    for (index, call) in calls.iter().enumerate() {
        let r = &call["request"];
        let diagram: Option<Vec<Option<Diagram>>> =
            serde_json::from_value(r["boards"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &r["options"];
        let options = Options {
            width: o["Width"].as_i64().unwrap_or(0),
            height: o["Height"].as_i64().unwrap_or(0),
            fit_content: o["FitContent"].as_bool().unwrap_or(false),
            format: o["Format"].as_str().unwrap_or("").into(),
            timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
            ..Default::default()
        };
        let token = CancellationToken::default();
        if r["context"] == "context canceled" {
            token.cancel();
        }
        let result = render_timeline(
            Some(&token),
            diagram.as_deref().unwrap_or_default(),
            Some(&options),
            r["interval_ns"].as_i64().unwrap(),
        );
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
    assert_eq!(total, 10);
}

#[test]
fn one_static_board_holds_once_without_a_loop_extension() {
    let options = Options {
        format: "gif".into(),
        width: 64,
        height: 64,
        ..Default::default()
    };
    let bytes = render_timeline(
        None,
        &[Some(Diagram::default())],
        Some(&options),
        370_000_000,
    )
    .unwrap();
    assert!(!bytes.windows(11).any(|s| s == b"NETSCAPE2.0"));
    let mut reader = gif::DecodeOptions::new()
        .read_info(bytes.as_slice())
        .unwrap();
    assert_eq!(reader.read_next_frame().unwrap().unwrap().delay, 37);
    assert!(reader.read_next_frame().unwrap().is_none());
}

#[test]
fn timeline_preserves_typed_cancellation_and_checks_admission_first() {
    let token = CancellationToken::default();
    token.cancel();
    let options = Options {
        format: "gif".into(),
        width: 64,
        height: 64,
        ..Default::default()
    };
    let board = Some(Diagram::default());
    assert!(matches!(
        render_timeline(Some(&token), &[board], Some(&options), 1_000_000_000),
        Err(RenderError::Context(LayoutError::Cancelled))
    ));
    assert_eq!(
        render_timeline(Some(&token), &[], Some(&options), 0)
            .unwrap_err()
            .to_string(),
        "isometric timeline requires GIF, 1..128 boards, and an interval in (0,10m]"
    );
}
