// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[test]
fn sketch_root_stroke_restarts_scanline_order_for_each_png_band() {
    // Pinned d2raster RenderBands resets the scanline rasterizer for every
    // 256-row destination. Carrying one active-edge list through the complete
    // frame changes f32 winding accumulation after expired-edge swaps.
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=dagre",
            "--sketch",
            "--theme=102",
            "--pad=20",
            "--scale=0.5",
            "--center",
            "--omit-version",
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
        .write_all(
            b"x{near:top-left\na->b}\ny{nea\nr:top-right\na->b\nc->d}\nz{near:bottom-center\nc->d}\nb{near:bottom-right\na{c{d}}}\n",
        )
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
    assert_eq!((frame.width, frame.height), (956, 1076));
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    let offset = (1024 * frame.width as usize + 787) * 3;
    assert_eq!(&pixels[offset..offset + 3], &[77, 26, 52]);
}
