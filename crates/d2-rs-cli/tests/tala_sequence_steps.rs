// SPDX-License-Identifier: MPL-2.0
//! TALA sequence-step parity regressions from the wide differential sweep
//! (seed 12, case 279, and the step-shape cases found while fixing it).
//! Every expected value is taken from the pinned Go D2
//! (6a7b4837) CLI output for the same input and `--tala-seeds 1`.
use std::io::Write;
use std::process::{Command, Stdio};

fn render_tala(source: &str) -> String {
    let output = run_tala(source);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn run_tala(source: &str) -> std::process::Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=tala", "--tala-seeds", "1", "-", "-"])
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
    process.wait_with_output().unwrap()
}

/// `d` of every connection path.
fn routes(document: &roxmltree::Document) -> Vec<String> {
    document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap().to_owned())
        .collect()
}

fn view_box(document: &roxmltree::Document) -> String {
    document
        .root_element()
        .attribute("viewBox")
        .unwrap()
        .to_owned()
}

/// Outline paths of the `shape` groups (steps render as paths).
fn shape_paths(document: &roxmltree::Document) -> Vec<String> {
    document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("path")))
        .map(|node| node.attribute("d").unwrap().to_owned())
        .collect()
}

fn shape_rectangles(document: &roxmltree::Document) -> Vec<[String; 2]> {
    document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y"].map(|key| node.attribute(key).unwrap().to_owned()))
        .collect()
}

/// `(text, x, y)` of every edge label.
fn edge_labels(document: &roxmltree::Document) -> Vec<(String, String, String)> {
    document
        .descendants()
        .filter(|node| {
            node.has_tag_name("text")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.contains("text-italic"))
        })
        .map(|node| {
            (
                node.text().unwrap_or_default().to_owned(),
                node.attribute("x").unwrap().to_owned(),
                node.attribute("y").unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn balance_symmetry_leaves_a_sequence_vessel_in_place() {
    // placement/symmetry.go isSimple excludes Graph.IsSequenceVessel, so the
    // n3-n4 vessel is not re-centred between n2 and n1; AlignAxes keeps n3
    // above n2.
    let svg =
        render_tala("n3.shape: step\nn4.shape: step\nn1 -- n2\nn2 -- n3\nn4 -- n3\nn1 -- n4\n");
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(view_box(&document), "0 0 434 438");
    assert_eq!(
        shape_paths(&document),
        [
            "M 0 0 L 67 0 L 102 51 L 67 101 L 0 101 L 35 51 Z",
            "M 67 0 L 135 0 L 170 51 L 135 101 L 67 101 L 102 51 Z",
        ]
    );
    assert_eq!(
        shape_rectangles(&document),
        [
            ["170".to_owned(), "170".to_owned()],
            ["20".to_owned(), "170".to_owned()],
        ]
    );
}

#[test]
fn sequence_step_label_near_stays_a_label_obstacle() {
    // labeling.Initialize runs over graph.Nodes after AddSequences removed
    // the steps, so n3's explicit center label is never fixed: PlaceLabels
    // keeps it as an obstacle and "search" goes to the other side.
    let svg = render_tala(
        "n3: { shape: step; label: \"queue worker billing\"; label.near: center-center }\n\
         n4: { shape: step; label: \"Title one two\" }\n\
         n4 <- n3: admin\nn3 -- n4: search\nn4 <-> n3: billing\n",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(view_box(&document), "0 0 577 362");
    assert_eq!(
        edge_labels(&document),
        [
            ("search".to_owned(), "197.5".to_owned(), "42".to_owned()),
            ("billing".to_owned(), "231.5".to_owned(), "128".to_owned()),
        ]
    );
}

#[test]
fn step_edges_inside_a_container_project_onto_the_vessel() {
    // Node.isDescendantOf continues from a step through Sequence.Vessel,
    // whose container is `c`, so the second n3 -> n4 edge collapses onto
    // `c` at the root scope instead of failing the seed.
    let svg = render_tala("c: {\n  n3.shape: step\n  n4.shape: step\n  n3 -> n4\n  n3 -> n4\n}\n");
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(view_box(&document), "0 0 544 511");
    assert_eq!(
        shape_paths(&document),
        [
            "M 60 120 L 153 120 L 188 185 L 153 249 L 60 249 L 95 185 Z",
            "M 153 120 L 247 120 L 282 185 L 247 249 L 153 249 L 188 185 Z",
        ]
    );
    assert_eq!(
        routes(&document),
        ["M 106 118 L 106 10 S 106 0 116 0 L 225 0 S 235 0 235 10 L 235 116"]
    );
}

#[test]
fn edge_crossings_count_the_edge_between_two_steps() {
    // GraphEdgeCrossings keeps the remaining n3 -- n4 edge on the step
    // pointers, so AlignAxes rejects the last 1px shift of n2 and n1.
    let svg = render_tala(
        "direction: down\nn2: auth\nn3: { shape: step; label: \"queue worker billing\" }\n\
         n4: { shape: step }\nn2 -> n4\nn4 <- n3: admin\nn3 -- n4: search\n\
         n1 -- n2\nn2 -- n3\nn1 <- n4\n",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(view_box(&document), "0 0 588 733");
    assert_eq!(
        shape_rectangles(&document),
        [
            ["223".to_owned(), "0".to_owned()],
            ["231".to_owned(), "465".to_owned()],
        ]
    );
    assert_eq!(routes(&document)[0], "M 266 68 L 266 182");
}

#[test]
fn moved_step_label_position_fails_seed_metadata_validation() {
    // labeling.Initialize never fixes a step's explicit label position, so
    // label placement moves it and evaluateSeedResult rejects the seed.
    let output =
        run_tala("n3: { shape: step; label.near: outside-top-center }\nn4.shape: step\nn3 -> n4\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "all TALA seed attempts failed: seed 1: validate TALA seed result metadata: \
             node 104193830 fixed label position changed"
        ),
        "{stderr}"
    );
}
