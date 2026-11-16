// SPDX-License-Identifier: MPL-2.0
//! Multi-board TXT: d2ascii sizes each board's canvas from
//! Diagram.NestedBoundingBox, which includes the descendant boards the
//! multi-board render keeps attached (d2ascii.go:134, d2target.go:424-448).
//! Expected text is the pinned Go CLI's output.
use std::{fs, process::Command};

#[test]
fn root_board_canvas_includes_attached_layer_bounds() {
    let root = std::env::temp_dir().join(format!("d2-ascii-nested-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("input.d2"),
        "a: {width: 300; b}\nlayers: {l: {c: {near: top-left}}}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(&root)
        .env_remove("D2_LAYOUT")
        .args(["input.d2", "out.txt"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let expected = [
        "┌──────────────────────────────┐",
        "│              a               │",
        "│                              │",
        "│           ┌────┐             │",
        "│           │ b  │             │",
        "│           │    │             │",
        "│           └────┘             │",
        "│                              │",
        "└──────────────────────────────┘",
    ]
    .map(|line| format!("{line}\n"))
    .concat();
    assert_eq!(
        fs::read_to_string(root.join("out/index.txt")).unwrap(),
        expected
    );
    // A single-board render detaches the layers (noChildren): b stays centred.
    let output = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .current_dir(&root)
        .env_remove("D2_LAYOUT")
        .args(["--target=", "input.d2", "root.txt"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        fs::read_to_string(root.join("root.txt"))
            .unwrap()
            .contains("│            ┌────┐            │")
    );
    fs::remove_dir_all(root).unwrap();
}
