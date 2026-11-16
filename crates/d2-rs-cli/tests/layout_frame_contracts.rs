// SPDX-License-Identifier: MPL-2.0
//! Frame and margin contracts for compound layouts. Every expected rectangle
//! and label below was captured from the pinned Go CLI.
use std::io::Write;
use std::process::{Command, Stdio};

fn render(engine: &str, source: &str) -> String {
    render_with(engine, source, &[])
}

fn render_with(engine: &str, source: &str, extra: &[&str]) -> String {
    let layout = format!("--layout={engine}");
    let mut args = vec![layout.as_str(), "--omit-version"];
    args.extend_from_slice(extra);
    args.extend(["-", "-"]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(&args)
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

fn stroked_rectangles(svg: &str) -> Vec<(i32, i32, i32, i32)> {
    let document = roxmltree::Document::parse(svg).unwrap();
    document
        .descendants()
        .filter(|n| n.has_tag_name("rect") && n.attribute("stroke").is_some())
        .map(|n| {
            let value = |a| n.attribute(a).unwrap().parse::<f64>().unwrap() as i32;
            (value("x"), value("y"), value("width"), value("height"))
        })
        .collect()
}

#[test]
fn dagre_compound_translation_precedes_origin_rounding() {
    // dagro layout.go translateGraph subtracts the fractional minimum
    // (here a 166 px self-loop dummy leaves the rank-0 leaf at 30.5) before
    // d2dagrelayout/layout.go:217 publishes Round(x - w/2).
    for (source, expected) in [
        (
            "direction: right\ns: {\n}\nexternal <-> external: a longer connection label\nexternal -- other.child: a longer connection label\n",
            vec![
                (27, 0, 52, 66),
                (0, 172, 105, 66),
                (581, 142, 140, 126),
                (611, 172, 80, 66),
            ],
        ),
        (
            "direction: left\no0 <-> o0\no0 -- o0: \"first line\\nsecond line\"\nc0.n1 -- o0: a longer connection label\n",
            vec![(0, 86, 62, 66), (514, 56, 122, 126), (544, 86, 62, 66)],
        ),
    ] {
        assert_eq!(
            stroked_rectangles(&render("dagre", source)),
            expected,
            "{source}"
        );
    }
}

#[test]
fn dagre_reflection_ignores_measured_vessel_boxes() {
    // RL/BT reverse Dagro's internal flow before translateGraph; the vessel
    // is rebuilt from its border nodes, so its measured label-wide box must
    // not move the pivot for its children.
    for (direction, expected) in [("left", (20, 56, 81, 85)), ("up", (10, 20, 108, 85))] {
        let source = format!(
            "direction: {direction}\ncontainer: {{\n  n2: {{shape: queue; width: 21; height: 25; label.near: center-right}}\n}}\n"
        );
        assert_eq!(
            stroked_rectangles(&render("dagre", &source)),
            vec![expected],
            "{direction}"
        );
    }
}

#[test]
fn elk_nested_label_overhang_leads_mirrored_node() {
    // elk-go's InnermostNodeMarginCalculator keeps a label taller or wider
    // than its node as a margin. After UP/LEFT mirroring that margin precedes
    // the node, and the nested component hull starts at the margin box.
    for (direction, size, expected) in [
        ("up", (83, 19), vec![(12, 12, 183, 121), (62, 62, 83, 19)]),
        ("left", (10, 66), vec![(12, 12, 157, 166), (82, 62, 10, 66)]),
    ] {
        let source = format!(
            "direction: {direction}\ncontainer: {{\n  n0: {{width: {}; height: {}; label.near: center-center}}\n}}\n",
            size.0, size.1
        );
        assert_eq!(
            stroked_rectangles(&render("elk", &source)),
            expected,
            "{direction}"
        );
    }
}

#[test]
fn sequence_group_reference_keeps_block_label_language() {
    // d2compiler compileLabel sets Language only for block strings. A group
    // reference to an actor's note compiles after the actor's own markdown
    // declaration, so its plain label is rendered as markdown.
    for source in [
        "shape: sequence_diagram\na\ng: {\n  a.n: \"tall\\nnote\"\n}\na.n: |md\n  text\n|\n",
        "shape: sequence_diagram\na\na.n: |md\n  text\n|\ng: {\n  a.n: \"tall\\nnote\"\n}\n",
    ] {
        let svg = render("dagre", source);
        assert!(
            svg.contains("class=\"md-text text fill-N1\"") && svg.contains(">tall note"),
            "{source}"
        );
    }
}

#[test]
fn dagre_plain_text_icon_stays_inside() {
    // d2dagrelayout positionLabelsIcons puts a default icon outside (top
    // left) only for SQLTable, Class or a non-empty Language. A plain
    // `shape: text` has no language, so its icon is InsideMiddleCenter and
    // adds no top margin; markdown (Language "markdown") keeps it outside.
    // Bundling is off so the icon URL is never fetched.
    const ICON: &str = "https://icons.terrastruct.com/essentials/087-menu.svg";
    for (source, expected) in [
        (
            format!("direction: down\nc0.n: {{shape: text; icon: {ICON}}}\n"),
            vec![(10, 20, 67, 81)],
        ),
        (
            format!("direction: left\nc0.n: {{shape: text; icon: {ICON}}}\nc0.n -> o\n"),
            vec![(174, 56, 67, 81), (0, 64, 54, 66)],
        ),
        (
            format!("direction: up\nc0.c1.n: hello world {{shape: text; icon: {ICON}}}\n"),
            vec![(0, 29, 194, 152), (30, 70, 134, 81)],
        ),
        (
            format!("direction: right\nc0.n: |md\n  # Title\n  some text\n| {{icon: {ICON}}}\n"),
            vec![(20, 86, 127, 195)],
        ),
    ] {
        assert_eq!(
            stroked_rectangles(&render_with("dagre", &source, &["--bundle=false"])),
            expected,
            "{source}"
        );
    }
}
