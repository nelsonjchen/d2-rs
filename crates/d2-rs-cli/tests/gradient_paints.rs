// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

fn png(source: &str) -> (u32, u32, Vec<u8>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=dagre",
            "--theme=200",
            "--sketch",
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
        .write_all(source.as_bytes())
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
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    pixels.truncate(frame.buffer_size());
    (frame.width, frame.height, pixels)
}

fn pixel(pixels: &[u8], width: u32, x: usize, y: usize) -> &[u8] {
    let offset = (y * width as usize + x) * 3;
    &pixels[offset..offset + 3]
}

#[test]
fn sketch_gradient_connection_retains_route_and_local_arrow_paint() {
    // Pinned d2scenebuild/sketch_connection.go retains the typed paint on
    // the route and each arrow path, in the arrow's local coordinate frame.
    let (width, height, pixels) =
        png("a -> b: edge {style.stroke: \"linear-gradient(red, blue)\"}\n");
    assert_eq!((width, height), (514, 910));
    for (x, y, expected) in [
        (256, 360, [229, 0, 26]),
        (255, 540, [31, 0, 224]),
        (253, 563, [82, 0, 173]),
    ] {
        assert_eq!(pixel(&pixels, width, x, y), expected, "at {x},{y}");
    }
}

#[test]
fn gradient_connection_label_uses_the_scene_ink_box() {
    // Pinned d2scenebuild/labels.go centeredTextRuns: Ink is the measured
    // label box, independent of the actual glyph outline bounds.
    let (width, height, pixels) =
        png("a -> b: edge {style.font-color: \"linear-gradient(red, blue)\"}\n");
    assert_eq!((width, height), (514, 910));
    assert_eq!(pixel(&pixels, width, 255, 441), [207, 0, 46]);
}

#[test]
fn multiline_gradient_label_restarts_its_paint_on_each_line() {
    // Pinned d2scenebuild/build.go gives every node label line its own
    // TextRun.Ink, with the full label width and one line's height.
    let (width, height, pixels) =
        png("a: \"one\\ntwo\" {style.font-color: \"linear-gradient(red, blue)\"}\n");
    assert_eq!((width, height), (556, 568));
    assert_eq!(pixel(&pixels, width, 300, 273), [72, 0, 183]);
    assert_eq!(pixel(&pixels, width, 306, 310), [72, 0, 183]);
}
