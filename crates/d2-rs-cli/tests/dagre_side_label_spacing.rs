// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn render_dagre(source: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=dagre", "--omit-version", "-", "-"])
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
        "{source}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn dagre_keeps_cross_rank_shifts_for_side_labels_in_containers() {
    // d2dagrelayout's adjustCrossRankSpacing shifts and grows objects by the
    // side margin and padding from Object.Spacing(): inside middle-left/right
    // labels on a child or container, and outside-left/right container
    // labels. The expected inner viewBox and shape boxes (container, then
    // child) are from the pinned Go CLI.
    for (source, view_box, boxes) in [
        (
            "a: {\n  b: { label.near: center-right }\n}\n",
            "-91 -121 333 368",
            [(10, 20, 131, 126), (40, 50, 71, 66)],
        ),
        (
            "a: {\n  label.near: outside-left-top\n  x\n}\n",
            "-85 -81 331 328",
            [(32, 20, 113, 126), (62, 50, 53, 66)],
        ),
        (
            "a: {\n  label.near: center-left\n  x: { label.near: center-right }\n}\n",
            "-69 -81 333 328",
            [(32, 20, 131, 126), (62, 50, 71, 66)],
        ),
    ] {
        let svg = render_dagre(source);
        let document = roxmltree::Document::parse(&svg).unwrap();
        let inner = document
            .descendants()
            .filter(|node| node.has_tag_name("svg"))
            .nth(1)
            .unwrap();
        assert_eq!(inner.attribute("viewBox"), Some(view_box), "{source}");
        let rects = document
            .descendants()
            .filter(|node| {
                node.has_tag_name("rect")
                    && node
                        .attribute("class")
                        .is_some_and(|classes| classes.split_whitespace().any(|c| c == "stroke-B1"))
            })
            .map(|node| {
                let number = |name| node.attribute(name).unwrap().parse::<i64>().unwrap();
                (number("x"), number("y"), number("width"), number("height"))
            })
            .collect::<Vec<_>>();
        assert_eq!(rects, boxes, "{source}");
    }
}
