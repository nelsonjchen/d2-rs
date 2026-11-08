// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn dagre_keeps_parallel_outgoing_compound_children_in_distinct_lanes() {
    // Pinned Dagro's Brandes-Koepf pass places the two child-to-external
    // chains in separate cross-axis lanes. D2 publishes those coordinates
    // through adjustCrossRankSpacing and fitContainerPadding unchanged.
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=dagre", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"a.b -> c\na.h -> g\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let svg = String::from_utf8(output.stdout).unwrap();
    for (class, expected_x) in [("YS5i", "40"), ("YS5o", "154")] {
        let rect = svg
            .split_once(&format!("<g class=\"{class}\">"))
            .expect("compound child")
            .1
            .split_once("<rect ")
            .expect("child rectangle")
            .1
            .split_once("/>")
            .unwrap()
            .0;
        assert!(rect.contains(&format!("x=\"{expected_x}\"")), "{rect}");
    }
    assert!(svg.contains("<rect x=\"10\" y=\"20\" width=\"227\" height=\"126\""));
}
