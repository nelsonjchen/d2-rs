// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[test]
fn translated_sketch_shadow_pattern_uses_prepared_device_matrix() {
    // Pinned d2raster prepares the user-space streak pattern with the
    // primitive's unshifted f64 device matrix. A translated second shadow
    // must resolve that same paint after its filter crop is applied.
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
        .write_all(b"a: {style.shadow: true}\nb: {style.shadow: true}\na -> b\n")
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
    assert_eq!((frame.width, frame.height), (100, 280));
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    let pixel = |x, y| {
        let offset = (y * frame.width as usize + x) * 3;
        &pixels[offset..offset + 3]
    };
    assert_eq!(pixel(50, 190), [252, 237, 242]);
    assert_eq!(pixel(50, 250), [244, 230, 234]);
    assert_eq!(pixel(50, 255), [177, 181, 199]);
}
