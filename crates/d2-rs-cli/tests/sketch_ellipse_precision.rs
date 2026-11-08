// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[test]
fn native_sketch_ellipse_uses_four_f64_quadrants() {
    // Pinned d2raster geometry.go:ellipseSubpaths constructs four f64 cubic
    // quadrants from the scene ellipse. usvg lowers the same SVG ellipse to
    // eight f32 cubics; flattening those instead changes boundary coverage.
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
            b"outer: {\n\toval: {\n\t\tshape: oval\n\t\ta\n\t\tb\n\t\tc\n\t}\n}\ntall: {height: 800}\n",
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
    assert_eq!((frame.width, frame.height), (552, 1054));
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    let offset = (105 * frame.width as usize + 201) * 3;
    assert_eq!(&pixels[offset..offset + 3], &[201, 131, 156]);
}
