// SPDX-License-Identifier: MPL-2.0
//! Full original PNG frames, not text-size-only implementation assertions.
use super::*;
#[cfg(feature = "oracle-fixtures")]
fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgb);
    (info.width, info.height, pixels)
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn committed_tala_structured_fonts_match_complete_original_frames() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/tala-structured-font-v090/provenance.json"
    ))
    .unwrap();
    let expected: [&[u8]; 3] = [
        d2_rs_testdata::oracle_fixture_bytes!(
            "tests/fixtures/tala-structured-font-v090/class-default.png"
        ),
        d2_rs_testdata::oracle_fixture_bytes!(
            "tests/fixtures/tala-structured-font-v090/sql-default.png"
        ),
        d2_rs_testdata::oracle_fixture_bytes!(
            "tests/fixtures/tala-structured-font-v090/structured-edge.png"
        ),
    ];
    for (case, expected) in fixture["cases"].as_array().unwrap().iter().zip(expected) {
        let actual = render_source_format(
            case["source"].as_str().unwrap().as_bytes(),
            &["--engine=tala".into(), "--seeds=0".into()],
            ExportFormat::Png,
        )
        .unwrap();
        assert_eq!(decode(&actual), decode(expected), "{}", case["name"]);
    }
}
#[test]
fn quoted_board_components_preserve_source_output_names_and_navigation_ancestry() {
    let targets = vec![
        "layers.\"a.b\"".to_owned(),
        "layers.\"a.b\".scenarios.\"x.y\"".to_owned(),
    ];
    // Exact original Go output inventory from quoted-board-grid under all engines.
    assert_eq!(
        board_output_path(Some(&targets[0]), ExportFormat::Png, &targets),
        PathBuf::from("a.b/index.png")
    );
    assert_eq!(
        board_output_path(Some(&targets[1]), ExportFormat::Png, &targets),
        PathBuf::from("a.b/x.y.png")
    );
    let set = targets.iter().cloned().collect();
    assert_eq!(
        rendered_board_ancestors(&targets[1], &set),
        vec![
            (targets[0].clone(), "a.b".into()),
            (targets[1].clone(), "x.y".into())
        ]
    );
    assert_eq!(
        board_output_path(
            Some("layers.'a.b'"),
            ExportFormat::Svg,
            &["layers.'a.b'".into()]
        ),
        PathBuf::from("a.b.svg")
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn paged_frames_preserve_complete_original_powerpoint_alpha() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/paged-rgba-v090/provenance.json"
    ))
    .unwrap();
    let frames: [&[u8]; 3] = [
        d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/paged-rgba-v090/board-1.png"),
        d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/paged-rgba-v090/board-2.png"),
        d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/paged-rgba-v090/board-3.png"),
    ];
    fn rgba(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        pixels.truncate(info.buffer_size());
        assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
        (info.width, info.height, pixels)
    }
    let mut frame_budget = d2_rs_export::PagedFrameBudget::default();
    for (case, expected) in fixture["frames"].as_array().unwrap().iter().zip(frames) {
        let args = vec![
            "--engine=dagre".into(),
            format!("--target={}", case["target"].as_str().unwrap()),
        ];
        let (actual, _, _) = render_source_png_with_links(
            fixture["source"].as_str().unwrap().as_bytes(),
            &args,
            &mut frame_budget,
            true,
        )
        .unwrap();
        assert_eq!(rgba(&actual), rgba(expected), "{}", case["target"]);
    }
}
