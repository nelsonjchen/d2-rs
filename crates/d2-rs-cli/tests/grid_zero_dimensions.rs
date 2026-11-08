// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn explicit_zero_height_grid_child_is_rendered() {
    // Pinned d2grid accepts a zero-sized cell and places it normally; it does
    // not apply the positive-size validation used by the core layout engines.
    let source = b"h: {\n  grid-rows: 2\n  d.height: 0\n}\n";
    for layout in ["dagre", "elk", "tala"] {
        let layout_arg = format!("--layout={layout}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args([layout_arg.as_str(), "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{layout}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let svg = String::from_utf8(output.stdout).unwrap();
        let rect = svg
            .split_once("<g class=\"aC5k\">")
            .expect("grid child")
            .1
            .split_once("<rect ")
            .expect("child rectangle")
            .1
            .split_once("/>")
            .unwrap()
            .0;
        assert!(
            rect.contains("width=\"54\" height=\"0\""),
            "{layout}: {rect}"
        );
    }
}
