// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn svg_digest(layout: &str, source: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([&format!("--layout={layout}"), "--omit-version", "-", "-"])
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
    ring::digest::digest(&ring::digest::SHA256, &output.stdout)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn grid_cell_3d_hexagon_reserves_half_depth_above() {
    // Complete SVGs from pinned D2 6a7b48378, --omit-version, for each
    // layout. d2grid sizes cells with Object.GetMargin, whose
    // GetModifierElementAdjustments gives a 3D hexagon a top margin of the
    // Go integer constant THREE_DEE_OFFSET / 2 = 7, not the full 15.
    let root = "grid-rows: 2\na\nb: {shape: hexagon; style.3d: true}\n";
    for layout in ["dagre", "elk", "tala"] {
        assert_eq!(
            svg_digest(layout, root),
            "333bc5ce9c49be3b869eb7be9437f73b73c66a37616bf985d0bb92e8ceb48520",
            "{layout}"
        );
    }
    let nested = "g: {grid-columns: 2\na\nb: {shape: hexagon; style.3d: true}\n}\n";
    assert_eq!(
        svg_digest("dagre", nested),
        "ae2de2f6862f280dda16b6237218dbb7c3576fcd356cfb121bd8ede982bd594e"
    );
    assert_eq!(
        svg_digest("elk", nested),
        "80809a342c13478033c8ea5d83e78c4187b1e901351d695b7f4c0008c80add29"
    );
}
