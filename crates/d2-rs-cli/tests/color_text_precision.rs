// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[test]
fn small_scale_colrv1_uses_the_exact_canvas_transform() {
    // Pinned d2raster composes the scene's f64 canvas transform with the
    // COLRv1 glyph transform. At this coverage boundary, rebuilding that
    // transform from the SVG renderer's f32 scale changes gray 82 to 83.
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=dagre",
            "--theme=100",
            "--pad=0",
            "--scale=0.25",
            "--stdout-format=png",
            "-",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("a: 👩‍👩‍👧‍👶 {style.font-size: 100}\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut reader = png::Decoder::new(Cursor::new(output.stdout))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((frame.width, frame.height), (258, 88));
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    let offset = (43 * frame.width as usize + 115) * 3;
    assert_eq!(&pixels[offset..offset + 3], &[82, 82, 82]);
}

#[test]
fn default_ignorable_space_glyph_keeps_zero_advance() {
    // Pinned go-text/harfbuzz hideDefaultIgnorables substitutes the font's
    // space glyph for ZWJ while retaining zero advance. The following `s`
    // must therefore start on the same fixed-point pen position.
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=dagre", "--stdout-format=png", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("b.\"\u{200d}s\"\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut reader = png::Decoder::new(Cursor::new(output.stdout))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((frame.width, frame.height), (650, 736));
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    let offset = (403 * frame.width as usize + 321) * 3;
    assert_eq!(&pixels[offset..offset + 3], &[211, 214, 228]);
}
