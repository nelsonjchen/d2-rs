// SPDX-License-Identifier: MPL-2.0
//! TALA parity regressions from the wide differential sweep. Every expected
//! value is taken from the pinned Go D2 (6a7b4837) CLI on arm64.
use std::io::Write;
use std::process::{Command, Stdio};

fn render_tala(source: &str, seed: &str) -> String {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=tala", "--tala-seeds", seed, "-", "-"])
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

struct Geometry {
    view_box: String,
    rectangles: Vec<[String; 4]>,
    routes: Vec<String>,
}

fn geometry(svg: &str) -> Geometry {
    let document = roxmltree::Document::parse(svg).unwrap();
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| {
            ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap().to_owned())
        })
        .collect();
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap().to_owned())
        .collect();
    Geometry {
        view_box: document
            .root_element()
            .attribute("viewBox")
            .unwrap()
            .to_owned(),
        rectangles,
        routes,
    }
}

fn assert_rectangles(actual: &[[String; 4]], expected: &[[&str; 4]]) {
    let actual = actual
        .iter()
        .map(|rect| rect.each_ref().map(String::as_str))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn undirected_edge_with_source_arrowhead_ranks_from_its_target() {
    // `--` keeps SrcArrow false, so the adapter does not reverse it, but the
    // explicit source arrowhead makes HasSourceArrow true. makeSimpleDAG and
    // isSource/isSink read it through DirectedEndpoints as n2 -> n0.
    let svg = render_tala(
        "n0.c0\nn0.c1 -- n2 { source-arrowhead.shape: cf-one }\nn0.c1 -> n1\nn2 -> n1\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 637 700");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["173", "156", "262", "186"],
            ["117", "0", "62", "66"],
            ["0", "432", "62", "66"],
            ["314", "216", "61", "66"],
            ["233", "216", "61", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 263 214 L 263 43 S 263 33 253 33 L 181 33",
            "M 263 284 L 263 455 S 263 465 253 465 L 66 465",
            "M 115 33 L 41 33 S 31 33 31 43 L 31 428",
        ]
    );
}

#[test]
fn prescaled_outside_label_reserves_its_scaled_margin() {
    // Prescale grows n4 to 120px and its label to the 28px face before
    // Preprocess runs ComputeNodeSpacing, so the left margin is 32 + 10.
    let svg = render_tala(
        "n4: { label.near: outside-left-center }\nn2.c2 -> n4\nn0 -> n1\nn3 -- n2.c2\n\
         n4 -> n1\nn2.c1 -> n4\nn2.c1 -> n4\nn2.c1 <-> n2.c0\n",
        "2",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 758 932");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["306", "279", "120", "120"],
            ["0", "186", "240", "544"],
            ["494", "120", "62", "66"],
            ["494", "306", "62", "66"],
            ["119", "0", "62", "66"],
            ["119", "246", "61", "66"],
            ["60", "366", "120", "120"],
            ["90", "604", "61", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 159 314 L 159 329 S 159 339 169 339 L 302 339",
            "M 525 188 L 525 302",
            "M 149 68 L 149 244",
            "M 428 339 L 490 339",
            "M 139 364 L 139 349 S 139 339 149 339 L 302 339",
            "M 182 442 L 356 442 S 366 442 366 432 L 366 403",
            "M 120 490 L 120 600",
        ]
    );
}

#[test]
fn constant_near_bounds_use_the_tala_label_dimensions() {
    // The TALA patch replaces n1's LabelDimensions with its prescaled label
    // before d2near.boundingBox places the legend below the graph.
    let svg = render_tala(
        "direction: up\nn1: { label.near: outside-bottom-center }\n\
         legend: log { near: bottom-center }\nn0 -- n1: web\nn0 <- n1\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 322 690");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "240", "120", "120"],
            ["26", "422", "67", "66"],
            ["0", "0", "120", "120"],
        ],
    );
}

#[test]
fn root_grid_crossings_route_around_placed_constant_nears() {
    // LayoutNested routes extracted root-grid crossings after d2near.Layout,
    // so the placed legend is part of the RouteEdges graph.
    let svg = render_tala(
        "grid-rows: 1\nn0\nn2\nn4.c0.c0\nlegend: cdn { near: center-left }\nn0 -> n4.c0.c0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 798 578");
    assert_eq!(
        actual.routes,
        ["M 31 308 L 31 366 S 31 376 41 376 L 344 376 S 354 376 354 366 L 354 190"]
    );
}

#[test]
fn root_grid_crossings_route_in_extraction_order() {
    // Each crossing is restored with the first extracted cell (n0, n2, n4)
    // holding an endpoint, not in authored edge order.
    let svg = render_tala(
        "grid-rows: 1\nn0: {\n  c0: {\n    shape: sql_table\n  }\n  c1: {\n    shape: sql_table\n    \
         id: int {constraint: primary_key}\n  }\n}\nn2: {\n  c1: { shape: document; label: \"admin\"; \
         tooltip: billing }\n}\nn3: {\n}\nn4: {\n  c0: {\n    c0: {\n    }\n  }\n}\n\
         n4.c1 -> n2.c0\nn4.c0.c1 -> n2.c1\nn4.c1 -> n4.c0.c1 { target-arrowhead.shape: circle }\n\
         n2.c1 -> n0.c2: user\nn0.c2 -> n4.c0.c0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1295 744");
    assert_eq!(
        actual.routes,
        [
            "M 905 93 L 1083 93 S 1093 93 1093 103 L 1093 532 S 1093 542 1083 542 L 389 542 \
             S 379 542 379 532 L 379 226",
            "M 872 344 L 872 502 S 872 512 862 512 L 434 512 S 424 512 424 502 L 424 120",
            "M 872 128 L 872 272",
            "M 348 126 L 229 126 S 219 126 219 136 L 219 231 S 219 241 209 241 L 125 241",
            "M 90 276 L 90 472 S 90 482 100 482 L 781 482 S 791 482 791 472 L 791 346",
        ]
    );
}
