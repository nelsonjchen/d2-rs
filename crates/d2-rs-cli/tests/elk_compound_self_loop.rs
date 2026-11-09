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

#[test]
fn elk_compound_self_loop_follows_transferred_parent_port() {
    // Reduced from structure-fuzz seed 2026110897 job 821. Pinned elk-go's
    // bottom-up hierarchy scheduler transfers the nested child's external
    // port before the owning scope runs LabelAndNodeSizeProcessor.
    // PortRestorer puts the opposing loop target after that fixed port, at
    // the default 10 px port spacing, instead of leaving it centered on b.
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
        .write_all(b"a.b -> a.b\na.b.c.d -> a.b\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let svg = String::from_utf8(output.stdout).unwrap();
    assert!(
        svg.contains("M 214 465 L 214 503") && svg.contains("L 326 62 S 316 62 316 72 L 316 108"),
        "self-loop target must follow the transferred hierarchy port: {svg}"
    );
    assert!(
        svg.contains("M 189 280 L 189 363") && svg.contains("L 306 116"),
        "hierarchy edge: {svg}"
    );
}
