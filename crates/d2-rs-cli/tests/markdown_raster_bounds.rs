// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[test]
fn unset_markdown_position_keeps_target_canvas_bounds() {
    // Pinned d2target.Diagram.BoundingBox passes an unset label position to
    // GetPointOnBox, which leaves the label box at the shape's top-left. The
    // Markdown painter centers its viewport independently; it must not change
    // the native raster canvas bounds.
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=dagre",
            "--sketch",
            "--theme=102",
            "--dark-theme=200",
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
        .write_all(b"e->company Warehouse.1\ncompany Warehouse:|md\nhouse.Regional-1|\n")
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
    assert_eq!((frame.width, frame.height), (178, 374));
}
