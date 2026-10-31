// SPDX-License-Identifier: MPL-2.0
// Whole original-Go diagram PNGs; exact pixels exercise scene viewport bounds,
// per-layer float32 origins, band transport and final compositing together.
use super::*;
fn rgb(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert!(info.width > 0 && info.height > 0);
    buffer.truncate(info.buffer_size());
    reader.finish().unwrap();
    (info.width, info.height, buffer)
}
#[test]
fn clipped_braket_scene_origins_match_complete_original_go_pixels() {
    let cases: [(&str, &[u8]); 4] = [
        (
            r"\scriptstyle \braket{\frac{a+b}{c}}",
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/braket-fraction-script-source-v090.png"),
        ),
        (
            r"x+\braket{\sqrt{\frac{x}{y}}}y",
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/braket-nested-root-inner-product-source-v090.png"),
        ),
        (
            r"\Bra{\sqrt{\frac{x}{y}}}_i^2",
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/braket-nested-root-bra-scripts-source-v090.png"),
        ),
        (
            r"\sqrt\Huge x",
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/size-empty-radical-source-v090.png"),
        ),
    ];
    for (expression, source) in cases {
        let input = format!("x: |latex\n  {expression}\n|\n");
        let native = render_source_format_with_pixel_scale(
            input.as_bytes(),
            &[
                "--engine=tala-rs".into(),
                "--seeds=1".into(),
                "--theme=0".into(),
                "--scale=2".into(),
            ],
            ExportFormat::Png,
            Some(2),
        )
        .unwrap();
        let (nw, nh, np) = rgb(&native);
        let (sw, sh, sp) = rgb(source);
        assert_eq!((nw, nh), (sw, sh), "{expression} dimensions");
        assert!(
            np == sp,
            "{expression} first differing byte {:?}",
            np.iter().zip(&sp).enumerate().find(|(_, (a, b))| a != b)
        );
    }
}
