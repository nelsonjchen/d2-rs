// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn elk_compound_self_loop_keeps_imported_opposite_port_sides() {
    // Pinned elk-go runs CompoundGraphPreprocessor before SelfLoopPreprocessor.
    // The hierarchy boundary therefore fixes the default output/input sides;
    // model-order sorting leaves the detached loop target ahead of the later
    // hierarchy input. This is the minimized D414 ELK fuzz witness.
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=elk", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"a -> a: e\nb -> a.1\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let svg = String::from_utf8(output.stdout).unwrap();
    assert!(
        svg.contains("M 80 371 L 80 409") && svg.contains("L 98 199"),
        "compound loop must route around the right side: {svg}"
    );
    assert!(svg.contains("M 88 80 L 88 249"), "hierarchy edge: {svg}");
}
