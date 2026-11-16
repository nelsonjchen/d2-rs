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
fn sized_swaps_preserve_prescaled_outside_label_clearance() {
    // Prescale grows n4's label before ComputeNodeSpacing. Go's shared
    // childrenGraph nodes retain the resulting margins in bestSwapCandidate's
    // overlap checks (placement/sized_optimizer.go), requiring a 78px gap
    // between n4's bottom label and n2's top label during sized swaps.
    let svg = render_tala(
        "n0: { label: \"auth queue log\" }\n\
         n2: { label.near: outside-top-right; width: 80 }\n\
         n3: { shape: class; \"+field\": int; \"-method(a int)\": bool }\n\
         n4: { label.near: outside-bottom-center }\n\
         n0 -- n2\nn3 <- n4\nn4 -> n2\nn0 <- n2\nn2 -> n3\nn4 <-> n2\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 690 690");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["5", "368", "151", "120"],
            ["40", "214", "80", "61"],
            ["186", "91", "302", "184"],
            ["186", "91", "302", "92"],
            ["0", "0", "120", "120"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 66 366 L 66 277",
            "M 182 167 L 110 167 S 100 167 100 157 L 100 122",
            "M 60 122 L 60 210",
            "M 92 364 L 92 277",
            "M 122 244 L 182 244",
            "M 80 124 L 80 210",
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

#[test]
fn consumed_step_sequence_edges_stay_out_of_root_cross_graph_routing() {
    // TALA's buildLayoutPatch drops the step-sequence edge `a.c1 -> a.c2`
    // from g.Edges, so LayoutNested's RouteEdges for the extracted grid
    // crossing never sees it (sweep seed 9 case 6).
    let svg = render_tala(
        "a: {\n  c1: { shape: step }\n  c2: { shape: step }\n}\n\
         b: {\n  grid-columns: 1\n  c0\n}\na.c1 -> a.c2\na.c2 -> b.c0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 489 629");
    assert_eq!(
        actual.routes,
        ["M 171 163 L 171 324 S 171 334 161 334 L 125 334"]
    );
}

#[test]
fn hierarchy_container_child_keeps_its_hierarchy_offset() {
    // n1 is placed by PlaceHierarchies, so placeNodes skips
    // placeNodesOrthogonally and its SyncNestedGeometry: n1.c0 keeps the
    // centred 61px offset and lines up with n4 instead of sitting at the
    // 60px container padding.
    let svg = render_tala(
        "direction: right\nn0: {\n  c0: {\n  }\n}\nn1: {\n  c0: { label: \"mail auth\"; \
         style.italic: true }\n}\nn4 -> n2\nn4 <-> n1.c0\nn0.c0 -> n2\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 857 695");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["322", "307", "181", "186"],
            ["0", "0", "232", "188"],
            ["382", "61", "63", "66"],
            ["593", "61", "62", "66"],
            ["382", "367", "61", "66"],
            ["60", "61", "112", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 447 94 L 589 94",
            "M 378 94 L 176 94",
            "M 445 400 L 614 400 S 624 400 624 390 L 624 131",
        ]
    );
}

#[test]
fn consumed_step_sequence_edges_stay_out_of_grid_scope_routing() {
    // The same removal holds when the step sequence lies in a cell of a
    // nested grid whose crossings are routed in the grid's own scope.
    let svg = render_tala(
        "g: {\n  grid-rows: 1\n  a: {\n    c1: { shape: step }\n    c2: { shape: step }\n    \
         c1 -> c2\n  }\n  b: {\n    c0\n  }\n  a.c2 -> b.c0\n}\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 830 543");
    assert_eq!(actual.routes, ["M 277 153 L 443 153"]);
}

#[test]
fn hierarchy_container_child_keeps_its_offset_for_multiple_shapes() {
    // The multiple n1.c0 is 10px taller than its drawn box; re-centring it
    // in n1 would move it, and through packing n0 and n2, by 10px.
    let svg = render_tala(
        "direction: up\nn0: {\n  c1: {\n    c0: {\n    }\n    c1: {\n    }\n  }\n}\nn1: {\n  \
         c0: { style.multiple: true }\n}\nn0.c0 -> n2: search { target-arrowhead.shape: cf-one }\n\
         n0.c0 -> n1.c0\nn2 -> n1.c0: queue\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 944 971");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["80", "463", "663", "306"],
            ["80", "0", "191", "196"],
            ["23", "307", "62", "66"],
            ["381", "523", "262", "186"],
            ["150", "60", "61", "66"],
            ["140", "70", "61", "66"],
            ["140", "643", "61", "66"],
            ["441", "583", "61", "66"],
            ["522", "583", "61", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 138 676 L 61 676 S 51 676 51 666 L 51 377",
            "M 170 641 L 170 140",
            "M 51 305 L 51 113 S 51 103 61 103 L 136 103",
        ]
    );
}

#[test]
fn root_grid_cell_keeps_tala_routes_around_its_nested_grid() {
    // n4 is a root-grid cell container. Its own LayoutNested call lays out
    // the nested grid n4.c1 and routes the crossing n4.c1.c0 -> n4.c0.c2 with
    // TALA's RouteEdges; the root grid then only moves the cell graph, so
    // the route keeps TALA's integral shape borders.
    let svg = render_tala(
        "grid-rows: 2\nn0\nn1\nn4: {\n  c0\n  c1: {\n    grid-columns: 3\n    c0\n  }\n}\n\
         n4.c1.c0 -> n4.c0.c2\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 704 614");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "231", "66"],
            ["271", "0", "231", "66"],
            ["0", "106", "502", "306"],
            ["60", "166", "181", "186"],
            ["261", "166", "181", "186"],
            ["321", "226", "61", "66"],
            ["120", "226", "61", "66"],
        ],
    );
    assert_eq!(actual.routes, ["M 319 259 L 185 259"]);
}

#[test]
fn object_grid_cell_keeps_tala_routes_around_its_nested_grid() {
    // The same cell below an object grid: the enclosing TALA pass must not
    // re-apply n4.c1 after n4's own layout has routed the crossing.
    let svg = render_tala(
        "g: {\n  grid-rows: 2\n  n0\n  n1\n  n4: {\n    c0\n    c1: {\n      grid-columns: 3\n      \
         c0\n    }\n  }\n}\ng.n4.c1.c0 -> g.n4.c0.c2\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 824 734");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "622", "532"],
            ["60", "60", "231", "66"],
            ["331", "60", "231", "66"],
            ["60", "166", "502", "306"],
            ["120", "226", "181", "186"],
            ["321", "226", "181", "186"],
            ["381", "286", "61", "66"],
            ["180", "286", "61", "66"],
        ],
    );
    assert_eq!(actual.routes, ["M 379 319 L 245 319"]);
}

fn run_tala_without_bundling(source: &str, seeds: &str) -> (bool, String) {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=tala",
            "--bundle=false",
            "--tala-seeds",
            seeds,
            "-",
            "-",
        ])
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
    (
        output.status.success(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn zero_sized_text_icon_makes_every_seed_score_non_finite() {
    // A one-letter `shape: text` node is too small for an inside icon, so
    // Node.IconSize is 0. quality/labels.go scores the icon box against the
    // placed boxes including itself and subtracts one; a zero-sized box does
    // not overlap itself, the label total is -1 and 1 / (1 + total) is +Inf.
    // result.go evaluateSeedResult rejects each seed's non-finite score.
    for source in [
        "a: {shape: text; icon: https://example.com/icon.svg}\n",
        "a: {shape: text; icon: https://example.com/icon.svg}\nb -> a\n",
        "a: {shape: text; icon: https://example.com/icon.svg; icon.near: top-left}\n",
    ] {
        let (success, stderr) = run_tala_without_bundling(source, "1,2");
        assert!(!success, "{source}");
        assert_eq!(
            stderr,
            "err: failed to compile -: all TALA seed attempts failed: \
             seed 1: TALA seed result has a non-finite score\n\
             err: seed 2: TALA seed result has a non-finite score\n",
            "{source}"
        );
    }
    // A wider text label leaves room for a nonzero icon; an outside icon is
    // not clipped to the inner box. Both lay out normally in Go.
    for source in [
        "a: hello world {shape: text; icon: https://example.com/icon.svg}\n",
        "a: {shape: text; icon: https://example.com/icon.svg; icon.near: outside-top-left}\n",
    ] {
        let (success, stderr) = run_tala_without_bundling(source, "1,2");
        assert!(success, "{source}: {stderr}");
    }
}

#[test]
fn rejected_transpose_after_a_cluster_flip_restores_the_step_container() {
    // x and y form a cluster beside l0's step sequence. transpose.go runs
    // optimizeCluster(onlyFlip) inside each rotation's transaction, and
    // Transaction.Rollback restores the original TopLeft pointers; the
    // rejected rotations leave this layout. Previously every seed failed.
    let svg = render_tala(
        "l0: {\n  a: {shape: step}\n  b: {shape: step}\n  a -> b\n}\n\
         x -> l0\ny -> l0\nsrc -> l0.a\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 473 565");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "142", "271", "221"],
            ["99", "0", "54", "66"],
            ["173", "0", "54", "66"],
            ["0", "0", "67", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 126 68 L 126 99 C 126 109 136 99 136 109 L 136 138",
            "M 200 68 L 200 94 S 200 104 190 104 L 146 104 S 136 104 136 114 L 136 138",
            "M 30 68 L 30 242 S 30 252 40 252 L 90 252",
        ]
    );
}

#[test]
fn container_holding_a_step_sequence_is_not_a_hierarchy_member() {
    // AddSequences runs before hierarchy assignment, so l0's only child is
    // the sequence vessel. hierarchy/eligibility.go isSimpleCandidate rejects
    // Graph.IsSequenceVessel, so x, y -> l0 -> src stays in ordinary
    // placement. Treating l0 as a hierarchy member left its steps without
    // positions and every seed failed ("UnpositionedNode").
    let svg = render_tala(
        "l0: {\na: {shape: step}\nb: {shape: step}\na -> b\n}\n\
         x -> l0\ny -> l0\nl0.a -> src\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 473 708");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "142", "271", "221"],
            ["72", "0", "54", "66"],
            ["146", "0", "54", "66"],
            ["73", "440", "67", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 99 68 L 99 94 S 99 104 109 104 L 126 104 S 136 104 136 114 L 136 138",
            "M 173 68 L 173 94 S 173 104 163 104 L 146 104 S 136 104 136 114 L 136 138",
            "M 95 305 L 95 436",
        ]
    );
}

#[test]
fn swap_stage_direct_skips_mirroring_while_a_sequence_has_abducted_edges() {
    // placement/direct.go returns before computing transforms when any node
    // in Graph.Nodes is a sequence vessel with edge abductions; the vessel
    // itself is a Graph.Nodes member. SwapStuff's direct(true) must not
    // mirror these boards.
    let svg = render_tala(
        "l0: {\n  a: {shape: step}\n  b: {shape: step}\n  a -> b\n}\n\
         x -> l0\ny -> l0\nsrc -> l0.a\n",
        "3",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 648 625");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "142", "271", "221"],
            ["71", "0", "54", "66"],
            ["145", "0", "54", "66"],
            ["379", "220", "67", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 98 68 L 98 94 S 98 104 108 104 L 125 104 S 135 104 135 114 L 135 138",
            "M 172 68 L 172 94 S 172 104 162 104 L 145 104 S 135 104 135 114 L 135 138",
            "M 412 288 L 412 413 S 412 423 402 423 L 99 423 S 89 423 89 413 L 89 307",
        ]
    );
    let svg = render_tala(
        "c: {\ndirection: left\nn0: {shape: step}\nn1: {shape: step}\nn0 -> n1\n}\n\
         outside -> c.n0\nc.n1 -> done\ndone -> c\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 813 553");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["153", "70", "289", "221"],
            ["513", "148", "98", "66"],
            ["0", "147", "81", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 562 216 L 562 341 S 562 351 552 351 L 256 351 S 246 351 246 341 L 246 235",
            "M 297 128 L 297 10 S 297 0 287 0 L 50 0 S 40 0 40 10 L 40 143",
            "M 83 180 L 149 180",
        ]
    );
}

#[test]
fn root_mirror_flips_trees_restored_inside_a_nested_container() {
    // hub -> l1, l2 is a tree placed inside l0 (direction left), then the
    // root direct() mirrors x. placement/direct.go mirrorNode flips the
    // shared NodeToTree orientation of every mirrored member, including tree
    // members restored below the nested container, so the leaves are entered
    // from the side facing the hub.
    let svg = render_tala(
        "l0: {\ndirection: left\nhub -> l1\nhub -> l2\n}\nsrc -> l0.s\next -- l0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 693 704");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["142", "134", "349", "368"],
            ["194", "0", "67", "66"],
            ["0", "252", "68", "66"],
            ["202", "318", "72", "66"],
            ["374", "376", "57", "66"],
            ["374", "260", "57", "66"],
            ["202", "194", "52", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 276 351 L 314 351 S 324 351 324 361 L 324 399 S 324 409 334 409 L 370 409",
            "M 276 351 L 314 351 S 324 351 324 341 L 324 303 S 324 293 334 293 L 370 293",
            "M 228 68 L 228 190",
            "M 70 285 L 140 285",
        ]
    );
}
