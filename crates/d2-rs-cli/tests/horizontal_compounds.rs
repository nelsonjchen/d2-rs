// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn dagre_horizontal_compound_edges_preserve_cross_rank_positions() {
    // Pinned d2dagrelayout/layout.go applies adjustCrossRankSpacing followed
    // immediately by fitContainerPadding. Direct vessel edges do not align
    // an external node with the first child after that spacing pass.
    for (source, class, y) in [
        ("1{}\na->l1.a\nc->l1\n", "Yw==", "172"),
        (
            "e{style{underline:false}}\nOEM Factory->e\nOEM Factory->Gos Warehouse\nGos Warehouse.s\n",
            "T0VNIEZhY3Rvcnk=",
            "159",
        ),
    ] {
        for direction in ["right", "left"] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
                .args(["--layout=dagre", "-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            write!(
                child.stdin.take().unwrap(),
                "direction:{direction}\n{source}"
            )
            .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let svg = String::from_utf8(output.stdout).unwrap();
            let object = svg
                .split_once(&format!("<g class=\"{class}\">"))
                .expect("external node")
                .1;
            let rect = object
                .split_once("<rect ")
                .expect("node rectangle")
                .1
                .split_once("/>")
                .unwrap()
                .0;
            assert!(rect.contains(&format!(" y=\"{y}\"")), "{direction}: {rect}");
        }
    }
}
