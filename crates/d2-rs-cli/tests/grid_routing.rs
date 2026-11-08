// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn elk_grid_routes_against_reinjected_container_labels() {
    // d2layouts.LayoutNested reinjects ELK's selected inside labels before
    // d2grid.Layout calls TraceToShape. The measured outer-container label
    // exceeds its original box, but fits the container after child layout.
    let source =
        "grid-columns: 1\nd -> outer-container\nd: {r: {}}\nouter-container: {d: {r: {}}}\n";
    for (source, route) in [
        (source.to_owned(), "M 125.5 168 L 125.5 202"),
        (format!("grid: {{\n{source}}}\n"), "M 197.5 240 L 197.5 274"),
    ] {
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
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let svg = String::from_utf8(output.stdout).unwrap();
        // Pinned Go routes to the container border, then subtracts the four
        // pixel arrow adjustment. A stale outside label clips five pixels early
        // at the implicit root and 36 pixels early within the named grid.
        assert!(svg.contains(&format!("<path d=\"{route}\"")), "{route}");
    }
}

#[test]
fn elk_sequence_lifelines_ignore_restored_grid_crossings() {
    // LayoutNested removes crossings that touch an extracted grid actor before
    // d2sequence lays out the parent, then restores and reroutes them. Such a
    // crossing must not consume a message row or extend the actor lifelines.
    let source = "t{d{shape:sequence_diagram\n\
h->sequence diagram.x->you can\n\
you can{grid-rows:2\n\
d{d{}}\n\
n->sequence diagram.{class:n}}}}\n";
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
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let svg = String::from_utf8(output.stdout).unwrap();
    assert!(svg.contains("<path d=\"M 124 539 L 124 746\""));
    assert!(!svg.contains("<path d=\"M 124 539 L 124 816\""));
}
