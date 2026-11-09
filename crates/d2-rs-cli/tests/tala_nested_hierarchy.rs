// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn render_tala_seed_three(source: &str) -> String {
    render_tala_seed(source, "3")
}

fn render_tala_seed(source: &str, seed: &str) -> String {
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

#[test]
fn cluster_flips_use_retained_descendant_endpoints() {
    // Pinned placement/cluster_connections.go reads the cluster abductions'
    // CurrentTo pointer, and layoutgraph/node.go:271-277 includes container
    // descendants in SetGraphReference. Complete pinned Go geometry, seed 1.
    let svg = render_tala_seed(
        "direction: right\nc1: {n3;n1}\nc0 -> c1.n1\nc0.n0 -> c0.n4\nn2 -> c1.n1\n",
        "1",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 852 862")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["386", "237", "264", "186"],
            ["0", "0", "316", "320"],
            ["0", "340", "316", "320"],
            ["528", "297", "62", "66"],
            ["446", "297", "62", "66"],
            ["61", "60", "62", "66"],
            ["60", "194", "63", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 318 278 L 341 278 S 351 278 351 288 L 351 320 S 351 330 361 330 L 442 330",
            "M 92 128 L 92 190",
            "M 318 381 L 341 381 S 351 381 351 371 L 351 340 S 351 330 361 330 L 442 330",
        ]
    );
}

#[test]
fn opposing_nested_edges_keep_the_ovg_search_port_selection() {
    // Pinned routing/ovg_edge_router.go:387-415 publishes the one OVG
    // search result. It does not re-search direct tunnels with a coordinate
    // tie-break after selecting ports. Complete pinned Go geometry, seed 1.
    let svg = render_tala_seed(
        "c0: {c1: {n4;n3};n2}\nn1 -> c0.c1\nc0.c1.n3 -> n1\nn1 -> c0.c1.n3\n",
        "1",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 645 828")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["0", "180", "443", "446"],
            ["203", "0", "120", "120"],
            ["60", "240", "323", "240"],
            ["60", "500", "62", "66"],
            ["120", "300", "63", "66"],
            ["203", "300", "120", "120"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 263 122 L 263 236",
            "M 233 298 L 233 124",
            "M 293 122 L 293 296",
        ]
    );
}

#[test]
fn compound_hierarchy_preserves_proxy_incidence_order() {
    // Pinned hierarchy/compound.go builds proxy edges in Graph.Edges order;
    // hierarchy/placement.go splits that proxy before shuffling its nodes.
    // Complete geometry and routes below are from the pinned Go CLI, seed 1.
    let svg = render_tala_seed(
        "direction: down\nc0: {c1: {n3};n2}\nn0\nn1\nn1 -> c0.c1\nc0.c1 -> c0.c1.n3\nn0 -> c0.c1\n",
        "1",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 607 664")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["10", "156", "384", "306"],
            ["343", "0", "62", "66"],
            ["0", "0", "62", "66"],
            ["70", "216", "182", "186"],
            ["272", "216", "62", "66"],
            ["130", "276", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 36 68 L 36 299 S 36 309 46 309 L 66 309",
            "M 161 400 L 161 346",
            "M 368 68 L 368 101 S 368 111 358 111 L 171 111 S 161 111 161 121 L 161 212",
        ]
    );
}

#[test]
fn transpose_resizes_cluster_vessels_after_wrapping_container_members() {
    // Pinned layoutgraph/transaction.go:1205-1233 wraps containers before
    // SyncClusters. Cluster.SyncGeometry resizes the live vessel from its
    // refitted members (cluster.go:61-92,186-200), so transpose.go:245-256
    // scores the expanded vessel before choosing a rotation. Go, seed 1.
    let svg = render_tala_seed("direction: down\nc0.n3 -> c0.n0\nn1 -> n2\nc0 -> n2\n", "1");
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 586 733")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["0", "0", "182", "318"],
            ["202", "0", "182", "318"],
            ["161", "465", "62", "66"],
            ["60", "60", "62", "66"],
            ["60", "192", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 91 128 L 91 188",
            "M 293 320 L 293 381 S 293 391 283 391 L 202 391 S 192 391 192 401 L 192 461",
            "M 91 320 L 91 381 S 91 391 101 391 L 182 391 S 192 391 192 401 L 192 461",
        ]
    );
}

#[test]
fn sized_swaps_preserve_self_loop_clearance_with_explicit_direction() {
    // Pinned placement/optimization_guard.go:439-488 uses Node.DeltaTo for
    // both swap checks. layoutgraph/node.go:846-860 includes loop clearance
    // regardless of graph direction. Complete geometry/routes from Go, seed 1.
    let svg = render_tala_seed("direction: right\nn3 -> c0\nn0 -> c0.n4\nn0 -> n0\n", "1");
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 571 577")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["0", "249", "62", "66"],
            ["186", "189", "183", "186"],
            ["246", "30", "62", "66"],
            ["246", "249", "63", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 64 282 L 182 282",
            "M 277 98 L 277 245",
            "M 244 47 L 226 47 S 216 47 216 37 L 216 10 S 216 0 226 0 L 252 0 S 262 0 262 10 L 262 26",
        ]
    );
}

#[test]
#[cfg(target_arch = "x86_64")]
fn equidistance_keeps_an_exact_zero_score_on_amd64() {
    // Pinned placementcost/graph.go:202-203 rounds the symmetry product before
    // subtracting on amd64. Equidistance treats its exact zero as no accepted
    // trial (placement/equidistance.go:391-392), retaining n3's x=264.
    // The fused arm64 score has a nonzero residual. Geometry below is Go amd64.
    let svg = render_tala_seed(
        "direction: right\nn0 -> n4\nn4 -> n3\nn4 -> n4\nn3 -> n1\n",
        "1",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 660 298")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["0", "30", "62", "66"],
            ["132", "30", "63", "66"],
            ["264", "30", "62", "66"],
            ["396", "30", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 64 71 L 128 71",
            "M 197 63 L 260 63",
            "M 130 47 L 112 47 S 102 47 102 37 L 102 10 S 102 0 112 0 L 138 0 S 148 0 148 10 L 148 26",
            "M 328 63 L 392 63",
        ]
    );
}

#[test]
fn edges_to_the_container_itself_disqualify_an_outer_hierarchy() {
    // Pinned hierarchy/eligibility.go:65-70 calls IsDescendantOf, which
    // includes the container itself (layoutgraph/layout.go:314-316).
    // An edge between a container and its child is an internal connection.
    // Complete rectangles/routes are from pinned Go, seed 3.
    let svg =
        render_tala_seed_three("direction: down\nc0 -> c1.n2\nc1 -> c1.n2\nn3 -> n0\nc1 -> n0\n");
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 540 572")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["216", "0", "61", "66"],
            ["156", "184", "182", "186"],
            ["0", "60", "62", "66"],
            ["0", "244", "62", "66"],
            ["216", "244", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 246 68 L 246 240",
            "M 336 277 L 282 277",
            "M 31 128 L 31 240",
            "M 154 277 L 66 277",
        ]
    );
}

#[test]
fn container_self_loops_do_not_disqualify_an_outer_hierarchy() {
    // Pinned hierarchy/eligibility.go:65-68 skips nonstructural self-loops
    // when checking for internal container connections. The loop still
    // reserves space and routes normally. Complete geometry/routes from
    // pinned Go, seed 3, cover both containers and every leaf and edge.
    let svg = render_tala_seed_three(
        "direction: down\nc0: {n0}\nc1: {n1}\nn2\nc0.n0 -> c0.n0\nc0.n0 -> c1.n1\nc0.n0 -> n2\nc1.n1 -> n2\n",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 531 880")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["117", "0", "212", "216"],
            ["0", "336", "182", "186"],
            ["207", "612", "62", "66"],
            ["207", "90", "62", "66"],
            ["60", "396", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 205 107 L 187 107 S 177 107 177 97 L 177 70 S 177 60 187 60 L 213 60 S 223 60 223 70 L 223 86",
            "M 205 131 L 98 131 S 88 131 88 141 L 88 392",
            "M 238 158 L 238 608",
            "M 91 464 L 91 635 S 91 645 101 645 L 203 645",
        ]
    );
}

#[test]
fn restored_container_edges_keep_their_incident_order_after_tree_copyback() {
    // Pinned layoutgraph/layout.go:105-128 restores changed endpoints after
    // recursive child placement. Its reconnect appends the outside edge
    // after the inner edge, which transpose.go:126-177 observes at a branch
    // size tie. Complete geometry and routes are from pinned Go, seed 3.
    let svg = render_tala_seed_three(
        "direction: right\nc0: {n0;n2}\nc1: {n1}\nc0.n0 -> c0.n2\nc0.n2 -> c1.n1\n",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 702 520")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["0", "0", "182", "318"],
            ["318", "132", "182", "186"],
            ["60", "60", "62", "66"],
            ["60", "192", "62", "66"],
            ["378", "192", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(routes, ["M 91 128 L 91 188", "M 124 225 L 374 225"]);
}

#[test]
fn compaction_preserves_self_loop_clearance_with_an_explicit_direction() {
    // Pinned compaction.go:267 calls Node.DoesOverlapAt for shifted subgraphs.
    // layoutgraph/node.go:846-860 includes loop offsets regardless of the
    // container direction. These complete rectangles/routes are from Go,
    // seed 3; dropping loop clearance admits a different compaction move.
    let svg = render_tala_seed_three(
        "direction: down\nn0\nn1\nn2\nn0 -> n0\nn1 -> n0\nn1 -> n2\nn2 -> n2\n",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 426 460")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["162", "192", "62", "66"],
            ["162", "30", "62", "66"],
            ["30", "30", "62", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 160 209 L 142 209 S 132 209 132 199 L 132 172 S 132 162 142 162 L 168 162 S 178 162 178 172 L 178 188",
            "M 201 98 L 201 188",
            "M 160 63 L 96 63",
            "M 28 47 L 10 47 S 0 47 0 37 L 0 10 S 0 0 10 0 L 36 0 S 46 0 46 10 L 46 26",
        ]
    );
}

#[test]
fn outside_edges_leave_nested_hierarchy_sources_and_sinks_available() {
    // Pinned hierarchy/discovery.go: assign abducts the parent edges before
    // descending. layoutgraph/layout.go reconnects their child incidences to
    // the container, so they cannot disqualify its inner source and sink.
    // These complete rectangles and routes are from the pinned Go CLI, seed 3.
    let source = "k.a -> k.b\nk.b -> k.c\np -> k.a\nk.a -> k.d\nk.e -> q\nk.b -> k.e\n";
    let svg = render_tala_seed_three(source);
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 666 988")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["0", "160", "464", "498"],
            ["240", "0", "53", "66"],
            ["240", "720", "54", "66"],
            ["240", "220", "53", "66"],
            ["147", "376", "53", "66"],
            ["60", "532", "53", "66"],
            ["350", "376", "54", "66"],
            ["234", "532", "53", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 238 253 L 183 253 S 173 253 173 263 L 173 372",
            "M 145 409 L 96 409 S 86 409 86 419 L 86 528",
            "M 266 68 L 266 216",
            "M 295 253 L 367 253 S 377 253 377 263 L 377 372",
            "M 263 600 L 263 716",
            "M 202 409 L 250 409 S 260 409 260 419 L 260 528",
        ]
    );
}

#[test]
fn cluster_vessels_do_not_obstruct_their_own_symmetry() {
    // Pinned placementcost/symmetry.go: obstructed excludes center/a/b by
    // pointer identity. The live cluster vessel also occupies a sibling slot;
    // counting its retained member as an obstacle loses the symmetry credit
    // and incorrectly accepts a 48 px Equidistance move. Geometry is from Go.
    let svg = render_tala_seed_three(include_str!("fixtures/tala_cluster_obstruction.d2"));
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 1074 934")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["736", "330", "136", "69"],
            ["0", "184", "606", "548"],
            ["736", "475", "96", "66"],
            ["417", "475", "129", "66"],
            ["420", "331", "126", "66"],
            ["60", "288", "90", "66"],
            ["60", "374", "90", "66"],
            ["170", "440", "209", "232"],
            ["299", "244", "100", "66"],
        ]
    );
}

#[test]
fn transpose_flips_cluster_vessels_in_induced_components() {
    // Pinned placement/transpose.go:233-241,273-280 runs the moved vessel's
    // onlyFlip transaction through Graph.Clusters, including in SplitSubgraphs.
    // The complete geometry below is from the pinned Go CLI with seed 3.
    let svg = render_tala_seed_three(
        "n0 -> n46\nn43 -> n60\nn43 -> n78\nn6 -> n13: link\nn0 -> n72: link\nn12 -> n46: link\nn78 -> n48: link\nn46 -> n60: link\nn13 -> n43: link\nn67 -> n46: link\n",
    );
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 880 940")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles,
        [
            ["305", "505", "62", "66"],
            ["301", "337", "70", "66"],
            ["301", "0", "70", "66"],
            ["301", "169", "70", "66"],
            ["455", "0", "70", "66"],
            ["0", "0", "62", "66"],
            ["147", "0", "70", "66"],
            ["301", "672", "70", "66"],
            ["149", "294", "70", "66"],
            ["608", "0", "70", "66"],
            ["149", "380", "70", "66"],
        ]
    );
    let routes = document
        .descendants()
        .filter(|node| {
            node.has_tag_name("path")
                && node
                    .attribute("class")
                    .is_some_and(|class| class.starts_with("connection"))
        })
        .map(|node| node.attribute("d").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "M 336 503 L 336 407",
            "M 336 68 L 336 165",
            "M 373 33 L 451 33",
            "M 64 33 L 143 33",
            "M 336 573 L 336 668",
            "M 221 327 L 250 327 S 260 327 260 337 L 260 360 S 260 370 270 370 L 297 370",
            "M 527 33 L 604 33",
            "M 336 335 L 336 239",
            "M 219 33 L 297 33",
            "M 221 413 L 250 413 S 260 413 260 403 L 260 380 S 260 370 270 370 L 297 370",
        ]
    );
}

#[test]
#[ignore = "heavy: about 16 s in a release build and over the 120 s TALA deadline in debug; run with --release -- --ignored"]
fn dense_network_recomputes_the_lattice_after_cluster_placement() {
    // Pinned placement/stages.go:268-297 calls ComputeCellSize before Swap
    // and TransposeAll. The positioned row vessel is wider than its initial
    // column, changing the owner's lattice and the later alignment scores.
    let svg = render_tala_seed_three(include_str!("fixtures/tala_dense_network.d2"));
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        document.root_element().attribute("viewBox"),
        Some("0 0 2360 3642")
    );
    let rectangles = document
        .descendants()
        .filter(|node| node.attribute("class") == Some("shape"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
        .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(rectangles.len(), 80);
    assert_eq!(
        &rectangles[..4],
        &[
            ["856", "1040", "120", "120"],
            ["1398", "1094", "89", "66"],
            ["713", "1364", "120", "120"],
            ["1169", "2374", "120", "120"],
        ]
    );
    assert_eq!(
        document
            .descendants()
            .filter(|node| {
                node.has_tag_name("path")
                    && node
                        .attribute("class")
                        .is_some_and(|class| class.starts_with("connection"))
            })
            .count(),
        99
    );
    let labels = document
        .descendants()
        .filter(|node| node.has_tag_name("text"))
        .filter_map(|node| node.text())
        .collect::<Vec<_>>();
    for index in 0..80 {
        assert!(
            labels.contains(&format!("host-{index}").as_str()),
            "missing host-{index}"
        );
    }
}
