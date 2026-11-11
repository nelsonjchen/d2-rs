// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn elk_svg_digest(source: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=elk", "--omit-version", "-", "-"])
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
fn elk_label_dummy_keeps_label_node_spacing_from_north_south_port_dummy() {
    // Complete SVG from pinned D2 6a7b48378, --layout=elk --omit-version.
    // elk-go v0.2.0 layered/spacings.go separates a north/south-port dummy
    // (the sql_table's port) from the edge-label dummy in its layer with
    // spacing.labelNode, not spacing.edgeNode.
    assert_eq!(
        elk_svg_digest(
            "direction: left\nn2: {shape: sql_table}\nn2 -> n4\nn1 -> n2\nn3 <- n4: x\n"
        ),
        "986e83fc4aa651d425b8aac1d25fa08a404dea82c9ab6efc583c1017426f3c4c"
    );
}

#[test]
fn elk_edge_into_grid_cell_traces_against_grid_label_position() {
    // Complete SVG from pinned D2 6a7b48378, --layout=elk --omit-version.
    // d2layouts.DefaultRouter's TraceToShape reads the cell's LabelPosition
    // set by d2grid (InsideMiddleCenter), so the overflowing label of the
    // narrow cell does not clip the edge as an outside-bottom label would.
    assert_eq!(
        elk_svg_digest(
            "g: {grid-columns: 1\nc0: {label: \"billing queue\"; width: 40}\nc1\n}\n\
             a -> b\na -> g.c0\n"
        ),
        "de39d5808536a1b50eb3ffc872207d3e1159ff1231055bd013e44ecd7be2fe0a"
    );
}
