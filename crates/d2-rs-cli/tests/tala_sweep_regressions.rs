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

#[test]
fn nan_edge_label_on_a_zero_length_route_fails_result_validation() {
    // The retry edge between two overlapping steps routes with zero length,
    // so its outside label's top-left is NaN. placement/stages.go Normalize
    // folds it into the origin with math.Min, which propagates NaN to every
    // node; result_validate.go validateCompletedGraph then rejects the seed.
    let (success, stderr) = run_tala_without_bundling(
        "c: {\ndirection: left\nn0: {shape: step; width: 30}\nn1: {shape: step}\nn0 -> n1\n\
         n1 -> n0: retry\n}\noutside -> c.n0\nc.n1 -> done\n",
        "1,2",
    );
    assert!(!success);
    assert_eq!(
        stderr,
        "err: failed to compile -: all TALA seed attempts failed: seed 1: validate TALA seed \
         result: node 3859557458: node x is outside the finite supported range: NaN\n\
         err: seed 2: validate TALA seed result: node 3859557458: node x is outside the finite \
         supported range: NaN\n"
    );
}

#[test]
fn sibling_component_direct_ignores_another_components_sequence_abduction() {
    // l0 splits into the abducting s0_0 -> sequence component and the hub
    // tree. direct.go:279-284 tests only the sequences of the nodes being
    // directed, so the hub component still mirrors toward its edges.
    let svg = render_tala(
        "l0: {\ns0_1: {shape: step}\ns0_2: {shape: step}\ns0_0 -> s0_1\ns0_1 -> s0_2\n\
         hub0 -> leaf0_0\nhub0 -> leaf0_1\nhub0 -> leaf0_2\n}\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 817 620");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "615", "418"],
            ["376", "60", "77", "66"],
            ["60", "176", "80", "66"],
            ["240", "292", "96", "66"],
            ["240", "176", "96", "66"],
            ["240", "60", "96", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 407 128 L 407 184",
            "M 142 209 L 180 209 S 190 209 190 219 L 190 315 S 190 325 200 325 L 236 325",
            "M 142 209 L 236 209",
            "M 142 209 L 180 209 S 190 209 190 199 L 190 103 S 190 93 200 93 L 236 93",
        ]
    );
}

#[test]
fn a_node_joined_to_a_container_and_its_step_is_not_a_tree_leaf() {
    // dst has two Node.Edges after AddSequences: l0 -> dst and the vessel's
    // abducted l0.s2 -> dst. trees/preprocess_helpers.go treeFringeNodes
    // requires exactly one, so dst is never peeled and Graph.Edges keeps its
    // order for AlignAxes.
    let svg = render_tala(
        "l0: {\ns1: {shape: step}\ns2: {shape: step}\ns1 -> s2\n}\nl0 -> dst\nl0.s2 -> dst\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 489 565");
    assert_rectangles(
        &actual.rectangles,
        &[["0", "0", "287", "221"], ["110", "297", "68", "66"]],
    );
    assert_eq!(
        actual.routes,
        ["M 131 223 L 131 293", "M 152 163 L 152 293"]
    );
}

#[test]
fn swap_stage_direct_mirrors_an_allocated_sequence_vessel() {
    // SwapStuff's direct(checkEdgeLength) mirrors y. The live step vessel
    // keeps its TopLeft only in its owner-bound point; mirrorNode reflects it
    // like every other Graph.Nodes entry, so l0 does not stretch to the
    // unmirrored vessel and the transaction is accepted.
    let svg = render_tala(
        "l0: {\ns0: {shape: step}\ns1: {shape: step}\ns0 -> s1\nc\n}\nx1 <- l0\nx2 -> l0\n\
         l0 -> dst\nl0 -> dst\n",
        "2",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 489 826");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "131", "287", "307"],
            ["186", "0", "62", "66"],
            ["0", "0", "62", "66"],
            ["83", "504", "120", "120"],
            ["60", "191", "53", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 217 70 L 217 129",
            "M 31 68 L 31 127",
            "M 123 440 L 123 500",
            "M 163 440 L 163 500",
        ]
    );
}

#[test]
fn grid_internal_crossing_is_routed_by_its_grid_not_after_constant_nears() {
    // r.y.z -> r.u.v crosses into r.u's sequence diagram, but both endpoints
    // are inside grid r. r's own LayoutNested call restores and routes it;
    // the root call (which also places the legend k) never sees it.
    let svg = render_tala(
        "k: {near: top-right; m.n; o; m.p}\n\
         r: {grid-rows: 1; u: {shape: sequence_diagram; v -> x}; y.z}\nr.y.z -> r.u.v\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1194 1035");
    assert_eq!(
        actual.routes[0],
        "M 460 118 L 460 10 S 460 0 450 0 L 132 0 S 122 0 122 10 L 122 139"
    );
}

#[test]
fn nested_grid_crossings_route_in_extraction_order() {
    // LayoutNested restores crossings per extraction in queue order (grid n1
    // before grid n3.c0), and RouteEdges routes them in that order, so the
    // earlier routes are obstacles for the later ones.
    let svg = render_tala(
        "n1: {\n  grid-columns: 3\n  c0: {\n  }\n  c1: {\n  }\n}\nn3: {\n  c0: {\n    \
         grid-columns: 1\n  }\n}\nn3.c1 <- n2: api\n\
         n3.c1 -> n3.c0.c1 { style.double-border: true }\nn3.c0.c1 <- n1.c0.c2\n\
         n3.c0.c2 -> n4: log\nn3.c1 -> n1.c0.c1: queue { label.near: top-center }\n\
         n3.c1 <- n1.c0.c0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1087 819");
    assert_eq!(
        actual.routes,
        [
            "M 593 201 L 593 68",
            "M 626 235 L 724 235 S 734 235 734 245 L 734 261",
            "M 734 261 L 734 -50 S 734 -60 724 -60 L 160 -60 S 150 -60 150 -50 L 150 118",
            "M 734 439 L 734 447 S 734 457 724 457 L 67 457",
            "M 586 273 L 586 342 S 586 352 576 352 L 241 352 S 231 352 231 342 L 231 190",
            "M 609 275 L 609 362 S 609 372 599 372 L 161 372 S 151 372 151 362 L 151 274",
        ]
    );
}

#[test]
fn obstruction_filter_follows_a_cluster_member_to_its_vessel() {
    // c.n.o -> c.m: c is the second member of the [d, c] cluster, whose vessel
    // the arena carries in d's slot. layout.go:314 isDescendantOf walks c to
    // the vessel, so the vessel is not an obstruction of c's own edge; the
    // swap commit then resizes the vessel from the refitted c.
    let svg = render_tala("w -> d\nw -> c\nc.extra\nc.n.o -> c.m\nd.x\n", "1");
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 964 790");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["352", "0", "58", "66"],
            ["0", "144", "371", "444"],
            ["391", "144", "371", "444"],
            ["451", "204", "82", "66"],
            ["451", "290", "174", "186"],
            ["645", "290", "57", "66"],
            ["60", "204", "53", "66"],
            ["511", "350", "54", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 381 68 L 381 95 S 381 105 371 105 L 186 105 S 176 105 176 115 L 176 140",
            "M 381 68 L 381 95 S 381 105 391 105 L 576 105 S 586 105 586 115 L 586 140",
            "M 538 348 L 538 330 S 538 320 548 320 L 641 320",
        ]
    );
}

#[test]
fn sequence_step_direction_penalty_reads_the_vessel_container() {
    // A step's Container is nil while its sequence is active, so
    // edge_length.go's outerNode stays the step and ContainerDirection
    // reaches l0's `direction: down` through the vessel.
    let svg = render_tala(
        "l0: {\ndirection: down\ns0_0: {shape: step; label: \"processing step 0\"}\n\
         s0_1: {shape: step; label: \"s 1\"}\ns0_0 -> s0_1\nl1: {\nhub1 -> leaf1_0\n\
         hub1 -> leaf1_1\nhub1 -> leaf1_2\nhub1 -> leaf1_3\n}\n}\nsrc <-> l0.s0_0\n\
         sink -> l0.l1.hub1\nsrc -- l0.l1.s1_0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1181 994");
    assert_eq!(
        actual.rectangles[0..4]
            .iter()
            .map(|rect| rect.each_ref().map(String::as_str))
            .collect::<Vec<_>>(),
        [
            ["0", "216", "979", "576"],
            ["130", "0", "67", "66"],
            ["599", "18", "75", "66"],
            ["355", "276", "564", "456"],
        ]
    );
    assert_eq!(
        actual.routes[4..],
        [
            "M 163 70 L 163 272",
            "M 636 86 L 636 332",
            "M 199 33 L 443 33 S 453 33 453 43 L 453 334",
        ]
    );
}

#[test]
fn tree_route_starts_at_a_centre_port_that_shares_the_node_centre() {
    // A 10x10 stored_data node's right-centre port is its centre point.
    // routeInSShape starts from the port node itself, so both isolated tree
    // edges route as trees from that shared port.
    let svg = render_tala(
        "n0: { shape: person; height: 52 }\nn1: { shape: stored_data; width: 10; height: 10 }\n\
         n2: { shape: oval; width: 20; height: 27 }\nn1 -> n0\nn1 -> n2\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 349 381");
    assert_eq!(
        actual.routes,
        [
            "M 7 95 L 111 95",
            "M 7 95 L 50 95 S 60 95 60 85 L 60 24 S 60 14 70 14 L 114 14",
        ]
    );
}

#[test]
fn node_placement_mirror_leaves_sequence_steps_reflected_until_a_sync() {
    // The root direct() mirrors l0 and reflects s1 and s2 inside their
    // vessel (mirrorNode over Sequence.Nodes). AlignAxes scores the
    // abducted l0.s1 -> dst edge from the reflected step, as Go does.
    let svg = render_tala(
        "l0: {\ns1: {shape: step}\ns2: {shape: step}\ns0 -> s1\ns1 -> s2\n}\nx1 -> l0\n\
         x2 <- l0\nl0.s1 -> dst\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 563 867");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["39", "157", "287", "353"],
            ["152", "0", "62", "66"],
            ["299", "599", "62", "66"],
            ["0", "599", "68", "66"],
            ["205", "384", "61", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 203 417 L 142 417 S 132 417 132 407 L 132 322",
            "M 183 68 L 183 153",
            "M 295 632 L 193 632 S 183 632 183 622 L 183 512",
            "M 131 267 L 63 267 S 53 267 53 277 L 53 595",
        ]
    );
}

#[test]
fn trees_hung_on_a_sequence_vessel_return_to_the_owner_graph() {
    // n2, n3 and peer are tree nodes whose root sentinel is the sequence
    // vessel. node_placement.go:183-191 restores them (and their edges) into
    // the owner graph via g.Trees[vessel]; the tree route then starts at the
    // reconnected step n1 (Tree.sentinelNode). Previously every seed failed
    // the BinPack edge-inventory invariant.
    let svg = render_tala(
        "c: {\nn0: {shape: step; label: \"processing step 0\"}\nn1: {shape: step; label: \"s 1\"}\n\
         n0 -> n1: next\nn1 -> n2\nn2 -> n3\nn2 -> peer\n}\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 597 755");
    assert_eq!(
        actual.routes,
        [
            "M 265 163 L 265 201 S 265 211 255 211 L 208 211 S 198 211 198 221 L 198 257",
            "M 198 329 L 198 367 S 198 377 188 377 L 148 377 S 138 377 138 387 L 138 423",
            "M 198 329 L 198 367 S 198 377 208 377 L 248 377 S 258 377 258 387 L 258 423",
        ]
    );
}

#[test]
fn sequence_vessel_edges_keep_graph_edge_order_before_restored_tree_edges() {
    // addSequence reconnects member edges while walking Graph.Edges and
    // putBackNonBranchingTrees appends the restored peer edge, so
    // InitializeNodes' BFS reaches n3 and n4 before peer.
    let svg = render_tala(
        "direction: right\nc: {\ndirection: left\n\
         n0: {shape: rectangle; label: \"processing step 0\"; width: 50}\n\
         n1: {shape: step; label: \"processing step 1\"}\nn2: {shape: step; label: \"stage 2\"}\n\
         n3: {shape: rectangle; label: \"stage 3\"}\nn4: {shape: step; label: \"processing step 4\"}\n\
         n0 -> n1\nn1 -> n2: next\nn2 -> n3\nn3 -> n4: next\nn4 -> n1: retry\npeer\nn2 -> peer\n}\n\
         outside -> c.n0\nc.n4 -> done\noutside -> c.n2\ndone -> c\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1122 683");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["189", "0", "731", "441"],
            ["0", "1", "98", "66"],
            ["9", "374", "81", "66"],
            ["774", "80", "50", "61"],
            ["590", "298", "95", "66"],
            ["249", "77", "77", "66"],
        ],
    );
}

#[test]
fn split_component_direct_sees_an_abducting_sequence_vessel() {
    // The l0 component [hub0, leaf cluster, step vessel] is an induced graph
    // without sequence records. Go's subgraph shares Graph.Sequences, and the
    // vessel's ext <-> s0_0 abduction stops direct() (direct.go:279-284).
    let svg = render_tala(
        "l0: {\ndirection: down\ns0_0: {shape: step}\ns0_1: {shape: step}\ns0_2: {shape: step}\n\
         s0_0 -> s0_1\ns0_1 -> s0_2\nhub0\nhub0 -> leaf0_0\nhub0 -> leaf0_1\nhub0 -> leaf0_2\n\
         l1: {\ndirection: right\ns1_0: {shape: rectangle}\ns1_1: {shape: step}\ns1_0 -> s1_1\n}\n}\n\
         ext <-> l0.s0_0\next <-> l0.hub0\nsrc <-> l0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 963 1047");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "599", "647"],
            ["693", "185", "68", "66"],
            ["266", "779", "67", "66"],
            ["258", "146", "80", "66"],
            ["60", "60", "96", "66"],
            ["60", "146", "96", "66"],
            ["60", "232", "96", "66"],
            ["60", "366", "464", "221"],
            ["120", "444", "77", "66"],
        ],
    );
}

#[test]
fn equidistance_moves_a_step_vessel_hung_with_a_tree() {
    // Equidistance and AlignAxes see the vessel with its restored tree; the
    // vessel settles at Go y=163 (it moved to 159 when the tree root was not
    // recognised as a sentinel of the vessel).
    let svg = render_tala(
        "c: {\nn0: {shape: step; label: \"processing step 0\"}\nn1: {shape: step; label: \"s 1\"}\nn0 -> n1: next\nn1 -> n2\nn2 -> n3\nn2 -> peer\n}\noutside -> c.n0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 597 854");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "99", "395", "553"],
            ["114", "0", "98", "66"],
            ["167", "360", "62", "66"],
            ["107", "526", "62", "66"],
            ["219", "526", "77", "66"],
        ],
    );
}

#[test]
fn root_grid_cell_edges_route_inside_their_own_cell() {
    // n2.c0 -> n2.c1.c0 has both endpoints inside root grid cell n2, so it is
    // routed with that cell rather than deferred to the root (y 156, not 146).
    let svg = render_tala(
        "grid-rows: 2\nn1: {\n}\nn2: {\n  c1: {\n    grid-columns: 1\n  }\n}\nn3: {\n}\nlegend: log { near: center-left }\nn2.c1.c0 -> n4\nn2.c0 -> n2.c1.c0: admin { target-arrowhead.shape: circle }\nn0 -> n2.c1.c0: store { target-arrowhead.shape: diamond }\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 773 614");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "62", "306"],
            ["102", "0", "382", "306"],
            ["0", "346", "134", "66"],
            ["-87", "173", "67", "66"],
            ["174", "346", "134", "66"],
            ["349", "346", "134", "66"],
            ["162", "60", "181", "186"],
            ["363", "60", "61", "66"],
            ["222", "120", "61", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 252 188 L 252 342",
            "M 393 128 L 393 146 S 393 156 383 156 L 287 156",
            "M 370.333008 344 L 370.333008 181 S 370.333008 171 360.333008 171 L 287 171",
        ]
    );
}

#[test]
fn align_axes_refits_a_container_whose_child_escaped_after_placement() {
    // NodePlacement leaves n1.c1 outside n1; Go Align commits every trial with
    // AffectContainers, so the first trial refits n1 instead of rejecting all moves.
    let svg = render_tala(
        "n1: {\n  c0: { label: \"api admin queue\"; label.near: outside-left-bottom; style.3d: true; width: 80 }\n}\nn2: {\n}\nn3: { shape: person; label: \"auth mail admin\" }\nn4: {\n}\nn5: { shape: oval; label: \"worker user\" }\nn3 -> n5\nn3 -- n4.c0: web { target-arrowhead.shape: cf-many-required }\nn4.c0 <- n1.c0\nn5 -> n1.c1: api { target-arrowhead.shape: arrow; label.near: bottom-right; style.shadow: true }\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 808 654");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["204", "160", "342", "267"],
            ["544", "13", "62", "66"],
            ["0", "237", "181", "186"],
            ["391", "306", "80", "61"],
            ["60", "297", "61", "66"],
            ["401", "220", "61", "66"],
        ],
    );
}

#[test]
fn gap_normalization_visits_a_sequence_vessel_once_per_pass() {
    // A container descendant walk returned both the step vessel and its owning
    // step; processing both moved the vessel twice (everything 36px higher).
    let svg = render_tala(
        "direction: down\nw: {\ndirection: down\nc: {\ndirection: right\nn0: {shape: step}\nn1: {shape: rectangle}\nn2: {shape: step; width: 150}\nn3: {shape: step}\nn0 -> n1\nn1 -> n2\nn2 -> n3\nn3 -> n2: retry\n}\n}\noutside -> w.c.n0\nw.c.n3 -> done\noutside -> w.c.n2\ndone -> w.c\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1057 928");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "164", "855", "368"],
            ["146", "0", "98", "66"],
            ["259", "660", "81", "66"],
            ["60", "224", "735", "248"],
            ["467", "315", "62", "66"],
        ],
    );
}

#[test]
fn mirrored_container_children_keep_label_overhang() {
    // Graph.syncNested positions children with expandForLabels: s0 keeps the
    // 46px overhang of its wide label after the root mirror (x 106, not 60).
    let svg = render_tala(
        "l0: {\ndirection: right\ns0: {shape: rectangle; label: \"processing step 0\"; width: 30}\ns1: {shape: step; label: \"processing step 1\"}\ns2: {shape: step; label: \"s 2\"}\ns3: {shape: step; label: \"s 3\"}\ns0 -> s1\ns1 -> s2: next\ns2 -> s3\n}\nx0 -> l0\nx1 <- l0\nx2 -> l0\nl0.s3 -> dst\n",
        "2",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 899 589");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "600", "221"],
            ["228", "321", "62", "66"],
            ["465", "321", "62", "66"],
            ["310", "321", "62", "66"],
            ["629", "78", "68", "66"],
            ["106", "80", "30", "61"],
        ],
    );
}

#[test]
fn swap_local_score_counts_restored_tree_obstacles() {
    // Swap NodeEdgeLength sees the restored hub leaves between src and s1_0, like
    // the global score; without them the s1_0 <-> l2 swap was not taken.
    let svg = render_tala(
        "direction: right\nl0: {\ndirection: left\ns0_0: {shape: step}\ns0_1: {shape: step}\ns0_0 -> s0_1\nl1: {\nhub1\nhub1 -> leaf1_0\nhub1 -> leaf1_1\nhub1 -> leaf1_2\nhub1 -> leaf1_3\nl2: {\ndirection: left\ns2_0: {shape: step; label: \"processing step 0\"}\ns2_1: {shape: step; label: \"stage 1\"}\ns2_2: {shape: step; label: \"processing step 2\"}\ns2_0 -> s2_1\ns2_1 -> s2_2: next\n}\n}\n}\nsrc -> l0.l1.s1_0\nsink -> l0.l1\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1533 1218");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["198", "0", "1133", "1016"],
            ["0", "830", "67", "66"],
            ["0", "535", "75", "66"],
            ["258", "181", "1013", "775"],
            ["318", "415", "80", "66"],
            ["498", "241", "96", "66"],
            ["498", "357", "96", "66"],
            ["498", "473", "96", "66"],
            ["498", "589", "96", "66"],
            ["614", "434", "597", "221"],
            ["318", "830", "77", "66"],
        ],
    );
}

#[test]
fn common_uncle_siblings_include_a_sequence_vessel() {
    // src connects to hub0 and (through the step vessel) to s0_0, so both are
    // common-uncle siblings; the vessel edges are exposed through the owner step.
    let svg = render_tala(
        "l0: {\ns0_0: {shape: step}\ns0_1: {shape: step}\ns0_2: {shape: step}\ns0_0 -> s0_1\ns0_1 -> s0_2\nhub0\nhub0 -> leaf0_0\nhub0 -> leaf0_1\nhub0 -> leaf0_2\n}\nsrc -> l0.hub0\nsrc <- l0.s0_0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1012 608");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["198", "0", "612", "406"],
            ["0", "185", "67", "66"],
            ["456", "146", "80", "66"],
            ["654", "60", "96", "66"],
            ["654", "146", "96", "66"],
            ["654", "232", "96", "66"],
        ],
    );
}

fn render_tala_unbundled(source: &str, seed: &str) -> String {
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=tala",
            "--bundle=false",
            "--tala-seeds",
            seed,
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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn align_axes_moves_a_step_vessel_whose_tree_is_not_isolated() {
    // The tree n2 -> n3 -> {n4, peer} hangs on the step vessel, which also
    // carries the abducted `outside -> c.n0` edge. isIsolatedTree reads the
    // vessel's Node.Edges, so the vessel -> n2 sentinel edge is not a tree
    // edge and AlignAxes moves the vessel together with `outside`.
    let svg = render_tala(
        "direction: right\nc: {\nn0: {shape: step; label: \"s 0\"}\nn1: {shape: step; label: \"s 1\"}\n\
         n0 -> n1: next\nn1 -> n2\nn2 -> n3: next\nn3 -> n4\nn3 -> peer\n}\noutside -> c.n0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 739 899");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["198", "0", "339", "697"],
            ["0", "77", "98", "66"],
            ["348", "239", "62", "66"],
            ["348", "405", "62", "66"],
            ["287", "571", "63", "66"],
            ["400", "571", "77", "66"],
        ],
    );
}

#[test]
fn cluster_vessel_spacing_ignores_member_label_margins() {
    // n2 and n5 form a row cluster. The vessel is a fresh NewNode without a
    // label, so deltaToGuarded reads zero margins for it; n2's outside label
    // margin must not block the smart swap of the vessel with n6.
    let svg = render_tala_unbundled(
        "n0: { shape: sql_table; id: int {constraint: primary_key}; name: string }\n\
         n1: { label: \"auth queue log\" }\n\
         n2: { label: \"x\"; label.near: outside-bottom-right; icon: https://example.com/icon.svg; icon.near: center-center; width: 80 }\n\
         n3: { shape: class; \"+field\": int; \"-method(a int)\": bool }\n\
         n4: { label: \"mail\"; label.near: outside-top-left; height: 100; shape: hexagon }\n\
         n5: { label: \"a much longer label here\"; label.near: outside-right-bottom; width: 80 }\n\
         n6: { width: 40 }\nn0 <- n1: mail\nn4 -> n3\nn3 -> n0: db\n\
         n5 <- n6: a much longer label here\nn6 -> n2\nn1 <- n0\n",
        "2",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 893 1125");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["57", "558", "188", "108"],
            ["57", "558", "188", "36"],
            ["76", "778", "151", "120"],
            ["322", "425", "80", "87"],
            ["0", "237", "302", "184"],
            ["0", "237", "302", "92"],
            ["433", "425", "80", "87"],
            ["398", "237", "40", "61"],
        ],
    );
}

#[test]
fn cluster_discovery_estimates_a_step_container_by_its_vessel() {
    // l2's only child is its step vessel (363 wide), so its discovery estimate
    // exceeds 4x hub1's width and l2 and hub1 are not clustered. Estimating
    // the vessel by its owner step (117 wide) clustered them.
    let svg = render_tala(
        "l0: {\nl1: {\nl2: {\ns2_0: {shape: step}\ns2_1: {shape: step}\ns2_2: {shape: step}\n\
         s2_3: {shape: step}\ns2_0 -> s2_1\ns2_1 -> s2_2\ns2_2 -> s2_3\n}\n}\n}\n\
         src -> l0.l1.l2\nsrc -> l0.l1.hub1\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1123 749");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["198", "0", "723", "547"],
            ["0", "297", "67", "66"],
            ["258", "60", "603", "427"],
            ["318", "120", "483", "221"],
            ["318", "361", "80", "66"],
        ],
    );
    assert_eq!(
        actual.routes,
        [
            "M 69 319 L 314 319",
            "M 33 365 L 33 385 S 33 395 43 395 L 314 395"
        ]
    );
}

#[test]
fn edge_direction_reads_a_cluster_member_container_as_nil() {
    // box.n0 and box.n2 form a cluster. AddCluster clears their Container,
    // so edgeLength's outer replacement for `box.n2 -- n1` is box.n2 and its
    // ContainerDirection is box's (unset): the undirected edges carry no
    // direction penalty and n1 is placed above box.
    let svg = render_tala(
        "direction: down\nn1: { shape: c4-person; width: 52 }\nbox: {\n}\n\
         box.n2 -- n1: e45\nbox.n0 -- n1\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 462 474");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["78", "0", "182", "272"],
            ["138", "60", "62", "66"],
            ["138", "146", "62", "66"],
        ],
    );
}

#[test]
fn diamond_border_trace_keeps_the_rounded_top_corner() {
    // The routed segment ends at the diamond's top corner (91,686). The
    // rounded corner is a flat cubic; Go's line coefficient fuses its first
    // product on arm64, which keeps the double root at t=1 inside the curve.
    let svg = render_tala(
        "grid-rows: 3\nn0: { label: \"cdn auth\"; label.near: outside-right-bottom }\n\
         n1: {\n  c0: {\n    c0: { shape: queue; label: \"cdn\" }\n  }\n}\nn2: {\n}\n\
         n3: {\n  c0\n}\nn4: {\n  c0: { shape: diamond }\n  c2: {\n  }\n}\n\
         n0 -> n4.c1\nn4.c2.c0 -> n1.c0.c0: user\n\
         n3.c1 <-> n4.c0: store { source-arrowhead.shape: box; label.near: center-center }\n\
         n1.c0.c0 <- n1.c0.c1 { source-arrowhead.shape: box }\n\
         n1.c0.c0 -> n1.c0.c1: cache { source-arrowhead.shape: box; label.near: bottom-right }\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 1105 1134");
    assert!(
        actual
            .routes
            .iter()
            .any(|route| route == "M 599.681749 527.192039 L 94.818251 684.807961"),
        "{:?}",
        actual.routes
    );
}

#[test]
fn tree_copyback_keeps_the_container_child_frame_under_a_label_overhang() {
    // s2_0's label is wider than its 30px box, so expandForLabels widens the
    // fit bounds to x=-1. The restored tree leaves share the ordinary
    // children's frame; positionContainerChildren then moves all of them by
    // one delta, keeping the leaves centred under hub2.
    let svg = render_tala(
        "l2: {\ns2_0: {shape: rectangle; width: 30}\ns2_0 -> s2_1\nhub2 -> leaf2_1\nhub2 -> leaf2_2\n}\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 588 635");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "0", "386", "433"],
            ["84", "60", "30", "61"],
            ["60", "188", "77", "66"],
            ["165", "141", "80", "66"],
            ["84", "307", "96", "66"],
            ["230", "307", "96", "66"],
        ],
    );
}

#[test]
fn container_orientation_skips_an_interior_with_a_step_vessel() {
    // l0's three boundary edges leave it toward two destinations under
    // `direction: up`, but its interior holds the s0 step vessel, so
    // orientSourceInterior (g.IsSequenceVessel) leaves the interior unturned.
    let svg = render_tala(
        "direction: up\nl0: {\ns0_0: {shape: step}\ns0_1: {shape: step}\ns0_0 -> s0_1\n\
         s0_2 -> s0_3\n}\nsink <- l0.s0_0\next <- l0.s0_0\next <- l0.hub0\n",
        "1",
    );
    let actual = geometry(&svg);
    assert_eq!(actual.view_box, "0 0 553 806");
    assert_rectangles(
        &actual.rectangles,
        &[
            ["0", "198", "351", "406"],
            ["0", "0", "75", "66"],
            ["99", "0", "68", "66"],
            ["60", "478", "77", "66"],
            ["214", "478", "77", "66"],
            ["159", "258", "80", "66"],
        ],
    );
}
