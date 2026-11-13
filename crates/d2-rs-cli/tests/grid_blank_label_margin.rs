// SPDX-License-Identifier: MPL-2.0
//! d2grid sizes cells with `Object.GetMargin`, whose label branch is gated on
//! `HasLabel` (a non-empty label value). A container labelled `" "` measures
//! 0x0, so its outside-top label still reserves `label.PADDING` (5) above the
//! cell. Expected values are from the pinned Go D2 (6a7b4837) CLI.
use std::io::Write;
use std::process::{Command, Stdio};

fn render_dagre(source: &str) -> String {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=dagre", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn shape_rectangles(svg: &str) -> Vec<[String; 4]> {
    let document = roxmltree::Document::parse(svg).unwrap();
    document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| {
            ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap().to_owned())
        })
        .collect()
}

fn background_y(svg: &str) -> String {
    let document = roxmltree::Document::parse(svg).unwrap();
    document
        .descendants()
        .find(|node| node.has_tag_name("rect"))
        .and_then(|node| node.attribute("y"))
        .unwrap()
        .to_owned()
}

fn assert_rectangles(svg: &str, expected: &[[&str; 4]]) {
    let actual = shape_rectangles(svg);
    let actual = actual
        .iter()
        .map(|rect| rect.each_ref().map(String::as_str))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn blank_container_label_reserves_label_padding_in_a_grid_cell() {
    let svg = render_dagre("grid-columns: 1\na: \" \" {\n  b\n}\n");
    assert_rectangles(&svg, &[["0", "5", "113", "126"], ["30", "35", "53", "66"]]);
    // The 0x0 label sits at y=0, so the diagram's top bound stays 0.
    assert_eq!(background_y(&svg), "-100");
}

#[test]
fn blank_label_margin_sets_the_row_height() {
    let svg = render_dagre("grid-columns: 2\na: \" \" {\n  b\n}\nc\n");
    assert_rectangles(
        &svg,
        &[
            ["0", "5", "113", "126"],
            ["153", "0", "53", "131"],
            ["30", "35", "53", "66"],
        ],
    );
}
