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

#[cfg(target_os = "linux")]
#[test]
#[ignore = "expected hashes depend on the host's fallback fonts under /usr/share/fonts; \
            GitHub's Ubuntu runners ship different faces. Run with --ignored on the \
            qualification VM until the test pins its own font directory."]
fn paged_default_ignorable_uses_system_fallback_after_bundled_fonts() {
    // Pinned d2cli.newFontFallbackOptions chains d2fonts' bundled resolver to
    // SystemFallbackResolver for PDF/PPTX as well as GIF. ZWJ keeps no ink or
    // advance, while Arabic alef must come from the same host fallback face.
    for (source, format, expected) in [
        (
            "e\u{200d}ا\n",
            "pdf",
            "62447ca5afdb810e58ca2e693790f4a312c927d47ead9ebd783fc294f7ff7db0",
        ),
        (
            "e\u{200d}ا\n",
            "pptx",
            "96b13dcfb430877df67ea00494948cd99a061bc34bf2a728f93322c49d418fd5",
        ),
        (
            "a -> b: e\u{200d}ا\n",
            "pdf",
            "7a8a7b7b9714c6dfa7639c7fb0b8a6e024c714bf0a846891f25299a3fbecd87e",
        ),
        (
            "a -> b: e\u{200d}ا\n",
            "pptx",
            "1ea4a46434c8c6b59d3e0e7d727f0a45d47b2c5f6912ebd408f60ff0893bec24",
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args([
                "--layout=elk",
                &format!("--stdout-format={format}"),
                "-",
                "-",
            ])
            .env("SOURCE_DATE_EPOCH", "946782245")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let digest = ring::digest::digest(&ring::digest::SHA256, &output.stdout);
        let actual = digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(actual, expected, "{format}");
    }
}
