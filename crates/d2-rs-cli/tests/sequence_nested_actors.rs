// SPDX-License-Identifier: MPL-2.0
//! Sequence diagrams whose actors hold nested diagrams or message-less spans.
//! Every expected value is taken from the pinned Go D2 (6a7b4837) CLI on
//! arm64 for the same input.
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(layout: &str, source: &str) -> Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([&format!("--layout={layout}"), "-", "-"])
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

/// `d` of every connection path, in document order.
fn routes(svg: &str) -> Vec<String> {
    let document = roxmltree::Document::parse(svg).unwrap();
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

#[test]
fn grid_actor_keeps_the_nested_dagre_edge_curve() {
    // LayoutNested extracts the grid actor `b`, lays out its cell container
    // `g` with dagre (which sets Edge.IsCurve), and InjectNested appends the
    // nested edges unchanged after d2sequence places the actors.
    let output = run(
        "dagre",
        "shape: sequence_diagram\na -> b\nb: {grid-rows: 2; x.a.b.c -> y; g: {c -> d}}\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let svg = String::from_utf8(output.stdout).unwrap();
    assert!(svg.contains(r#"viewBox="-89 -49 668 886""#));
    assert_eq!(
        routes(&svg),
        [
            "M 355.474289 179.71695 L 266.551421 328.5661",
            "M 361.5 342 C 361.5 380 361.5 400 361.5 436",
            "M 62 598 L 62 735",
            "M 315 598 L 315 735",
            "M 64 666 L 311 666",
        ]
    );
}

/// `e.a.b.e` inside the grid actor resolves to the top-level actor `a`, so
/// `a.b.e` is a span without messages (top-left y = +Inf) and its parent span
/// `a.b` gets height math.Max(Inf-Inf, MIN_SPAN_HEIGHT) = NaN.
const MESSAGELESS_SPANS: &str =
    "shape:sequence_diagram\nh->a->m->you can\nyou can{grid-rows:2;e.a.b.e->r;d{e-}}\n";

#[test]
fn messageless_nested_span_height_is_nan_for_tala_routing() {
    // The cross-graph edge is routed by TALA RouteEdges, whose
    // validateD2Objects checks height before the infinite top-left.
    let output = run("tala", MESSAGELESS_SPANS);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "err: failed to compile -: D2 object \"b\": height is outside the supported range: NaN\n"
    );
}

#[test]
fn messageless_nested_span_position_is_infinite() {
    for layout in ["dagre", "elk"] {
        let output = run(layout, MESSAGELESS_SPANS);
        assert_eq!(output.status.code(), Some(1), "{layout}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "err: failed to compile -: object \"a.b\" has invalid position with infinity \
             value: x=206.000000, y=+Inf\n",
            "{layout}"
        );
    }
}
