// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn fitted_cluster_vessels_keep_their_size_between_split_components() {
    // D48 job 402's enclosing-wrapper variant. Pinned node_placement.go:227-234
    // resizes the retained vessel after each member fit, before later split
    // components call ComputeCellSize. Geometry and routes are from the pin.
    let source = "wrapper: {\n*.shape:circle\nb->e\nc->f\nc->g\nd->h\nd->i\ne->j\ne->k\n*.ha\n}";
    for seed in [1, 2, 3] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        let (viewbox, wrapper_size, centers, expected_routes) = if seed == 2 {
            (
                "0 0 1082 1071",
                ["880", "869"],
                [
                    ["206", "169"],
                    ["206", "455"],
                    ["440", "210"],
                    ["362", "496"],
                    ["518", "496"],
                    ["674", "387"],
                    ["596", "673"],
                    ["752", "673"],
                    ["128", "741"],
                    ["284", "741"],
                ],
                [
                    "M 206 239 L 206 383",
                    "M 440 280 L 440 343 S 440 353 430 353 L 372 353 S 362 353 362 363 L 362 424",
                    "M 440 280 L 440 343 S 440 353 450 353 L 508 353 S 518 353 518 363 L 518 424",
                    "M 674 457 L 674 499.498993 S 674 509.498993 664 509.498993 L 606 509.498993 S 596 509.498993 596 519.498993 L 596 601",
                    "M 674 457 L 674 499.498993 S 674 509.498993 684 509.498993 L 742 509.498993 S 752 509.498993 752 519.498993 L 752 601",
                    "M 206 525 L 206 608.499023 S 206 618.499023 196 618.499023 L 138 618.499023 S 128 618.499023 128 628.499023 L 128 669",
                    "M 206 525 L 206 608.499023 S 206 618.499023 216 618.499023 L 274 618.499023 S 284 618.499023 284 628.499023 L 284 669",
                ],
            )
        } else {
            (
                "0 0 1192 869",
                ["990", "667"],
                [
                    ["134", "206"],
                    ["420", "206"],
                    ["128", "461"],
                    ["414", "383"],
                    ["414", "539"],
                    ["576", "461"],
                    ["862", "383"],
                    ["862", "539"],
                    ["706", "128"],
                    ["706", "284"],
                ],
                [
                    "M 204 206 L 348 206",
                    "M 198 461 L 261 461 S 271 461 271 451 L 271 393 S 271 383 281 383 L 342 383",
                    "M 198 461 L 261 461 S 271 461 271 471 L 271 529 S 271 539 281 539 L 342 539",
                    "M 646 461 L 699 461 S 709 461 709 451 L 709 394 S 709 384 719 384 L 790 384",
                    "M 646 461 L 699 461 S 709 461 709 471 L 709 529 S 709 539 719 539 L 790 539",
                    "M 490 206 L 553 206 S 563 206 563 196 L 563 138 S 563 128 573 128 L 634 128",
                    "M 490 206 L 553 206 S 563 206 563 216 L 563 274 S 563 284 573 284 L 634 284",
                ],
            )
        };
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some(viewbox),
            "seed {seed}"
        );
        let wrapper = document
            .descendants()
            .find(|node| node.attribute("class") == Some("shape"))
            .unwrap()
            .children()
            .find(|node| node.has_tag_name("rect"))
            .unwrap();
        assert_eq!(
            ["width", "height"].map(|key| wrapper.attribute(key).unwrap()),
            wrapper_size
        );
        let ellipses = document
            .descendants()
            .filter(|node| node.has_tag_name("ellipse"))
            .map(|node| ["cx", "cy", "rx", "ry"].map(|key| node.attribute(key).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            ellipses,
            centers.map(|[x, y]| [x, y, "68", "68"]),
            "seed {seed}"
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
        assert_eq!(routes, expected_routes, "seed {seed}");
    }
}

#[test]
fn nested_cluster_flips_preserve_ancestor_overlap_transitions() {
    // D46 job 664's rectangle-wrapper variant. Pinned transaction.go:1306-1337
    // rejects a cluster flip that makes a saved ancestor spacing overlap
    // exact. Canvas, rectangles and complete routes are from the pinned CLI.
    let source = r#"wrapper: {
        b.shape: rectangle
        e -> b.c: {style.stroke-width: 8; target-arrowhead.shape: diamond; target-arrowhead.style.filled: true}
        f -> b: {target-arrowhead.shape: diamond; target-arrowhead.style.filled: true}
        g -> b: {style.stroke-width: 8; target-arrowhead.shape: diamond; target-arrowhead.style.filled: true}
        b: containerlabel
    }"#;
    for seed in [1, 2, 3] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some("0 0 495 788")
        );
        let rectangles = document
            .descendants()
            .filter(|node| node.attribute("class") == Some("shape"))
            .map(|node| {
                let rect = node
                    .children()
                    .find(|child| child.has_tag_name("rect"))
                    .unwrap();
                ["x", "y", "width", "height"]
                    .map(|attribute| rect.attribute(attribute).unwrap().parse::<f64>().unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rectangles,
            [
                [0.0, 0.0, 293.0, 586.0],
                [60.0, 220.0, 173.0, 186.0],
                [120.0, 460.0, 53.0, 66.0],
                [82.0, 60.0, 54.0, 66.0],
                [156.0, 60.0, 54.0, 66.0],
                [120.0, 280.0, 53.0, 66.0],
            ],
            "seed {seed}"
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
                "M 146 455 L 146 359",
                "M 109 128 L 109 163 S 109 173 99 173 L 94 173 S 84 173 84 183 L 84 216",
                "M 183 131 L 183 163 S 183 173 193 173 L 198 173 S 208 173 208 183 L 208 207",
            ],
            "seed {seed}"
        );
    }
}

#[test]
fn branching_tree_dimension_error_keeps_the_source_stage_and_seed_order() {
    // scale-sweep.py fanout/400. Pinned Go trees/geometry.go layoutTree
    // completes tree placement before engine/pipeline.go checks the bounds.
    let mut source = String::new();
    for i in 0..400 {
        source.push_str(&format!("n{i}: Node {i}\n"));
    }
    for i in 1..400 {
        source.push_str(&format!("n{} -> n{i}\n", (i - 1) / 8));
    }
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=tala", "-", "-"])
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
    let result = process.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let stderr = String::from_utf8(result.stderr).unwrap();
    let errors = stderr
        .lines()
        .filter_map(|line| line.rsplit_once("seed ").map(|(_, error)| error))
        .collect::<Vec<_>>();
    assert_eq!(
        errors,
        [
            "1: layout invariant violated: Dimensions w:859, h:39517 reached after stage NodePlacement",
            "2: layout invariant violated: Dimensions w:859, h:39517 reached after stage NodePlacement",
            "3: layout invariant violated: Dimensions w:859, h:39517 reached after stage NodePlacement",
        ],
        "{stderr}"
    );
}

#[test]
fn enclosing_containers_fit_the_committed_cluster_rotation() {
    // D4 nested-placement witness. Pinned node_placement.go:184-237 keeps
    // optimizeCluster's accepted vessel geometry through direct and FitToGraph;
    // layoutgraph/node.go fits the combined descendants with 60px padding.
    for depth in [1, 2, 6] {
        let mut source = String::new();
        for level in (0..depth).rev() {
            source.push_str(&format!("diff_depth_{level}: {{\n"));
        }
        source.push_str("r.gg -> r.s.t\nr.s.u.v -> r.z\nr.aa -> r.s.t\nr.s.t -> g\nr.s.t -> h\n");
        source.push_str(&"}\n".repeat(depth));
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", "--tala-seeds=1", "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        let rectangles = document
            .descendants()
            .filter(|node| node.attribute("class") == Some("shape"))
            .map(|node| {
                let rect = node
                    .children()
                    .find(|child| child.has_tag_name("rect"))
                    .unwrap();
                ["x", "y", "width", "height"]
                    .map(|attribute| rect.attribute(attribute).unwrap().parse::<f64>().unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(rectangles.len(), depth + 10);
        for (level, &[x, y, width, height]) in rectangles[..depth].iter().enumerate() {
            let right = rectangles[depth..]
                .iter()
                .map(|rect| rect[0] + rect[2])
                .fold(f64::NEG_INFINITY, f64::max);
            let bottom = rectangles[depth..]
                .iter()
                .map(|rect| rect[1] + rect[3])
                .fold(f64::NEG_INFINITY, f64::max);
            let padding = 60.0 * (depth - level) as f64;
            assert_eq!(x + width, right + padding, "depth {depth}, level {level}");
            assert_eq!(y + height, bottom + padding, "depth {depth}, level {level}");
        }
    }
}

#[test]
fn ancestor_edges_preserve_child_sizes_during_recursive_placement() {
    // D46/D47 witnesses. Pinned grouping/clusters.go:300-316 rejects these
    // clusters, and layoutgraph/layout.go:197-214 detaches ancestor edges
    // before node_placement.go recurses. Dimensions are from the pinned CLI.
    let cases: [(&str, &[(f64, f64)]); 3] = [
        (
            "container.rst->container-\ncontainer->container.d-\ncontainer->container.d-2\n",
            &[
                (264.0, 272.0),
                (119.0, 66.0),
                (64.0, 66.0),
                (60.0, 66.0),
                (68.0, 66.0),
            ],
        ),
        (
            "container->container.d-\ncontainer->container.s-2\n",
            &[(266.0, 186.0), (60.0, 66.0), (66.0, 66.0)],
        ),
        (
            "container->container.d-\nr-\ncontainer->container.d-2\n",
            &[(268.0, 186.0), (56.0, 66.0), (60.0, 66.0), (68.0, 66.0)],
        ),
    ];
    for (source, expected) in cases {
        for seed in [1, 2, 3] {
            let seed_argument = format!("--tala-seeds={seed}");
            let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
                .args(["--layout=tala", &seed_argument, "-", "-"])
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
            let svg = std::str::from_utf8(&output.stdout).unwrap();
            let document = roxmltree::Document::parse(svg).unwrap();
            let dimensions = document
                .descendants()
                .filter(|node| node.attribute("class") == Some("shape"))
                .map(|node| {
                    let rect = node
                        .children()
                        .find(|child| child.has_tag_name("rect"))
                        .unwrap();
                    (
                        rect.attribute("width").unwrap().parse::<f64>().unwrap(),
                        rect.attribute("height").unwrap().parse::<f64>().unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(dimensions, expected, "seed {seed}: {source}");
        }
    }
}

#[test]
fn nested_ancestor_routes_keep_source_center_predecessors() {
    // D46's ancestor-edge witness with two enclosing containers. Pinned
    // routing/ovg_edge_router.go bestRoute follows predecessors through the
    // source center until nil or a cycle; route.go publishes slots 1 and len-2.
    let source = "wrapper: {\nwrapper: {\ncontainer.rst->container-\ncontainer->container.d-\ncontainer->container.d-2\n}\n}\n";
    for seed in [1, 2, 3] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        let routes = document
            .descendants()
            .filter(|node| {
                node.has_tag_name("path") && node.attribute("class") == Some("connection stroke-B1")
            })
            .map(|node| node.attribute("d").unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            routes,
            [
                "M 246 224 L 424 224",
                "M 252 256 L 40 256 S 30 256 30 266 L 30 289 S 30 299 40 299 L 176 299",
                "M 386 202 L 304 202 S 294 202 294 212 L 294 262",
            ],
            "seed {seed}"
        );
    }
}

#[test]
fn clustered_circle_containers_keep_the_source_alignment_choice() {
    // D48 job 402. Pinned grouping/clusters.go CreateVessel leaves
    // IsContainer false; placementcost/graph.go ContainerAlignmentCost must
    // score the ordinary containers without inheriting a vessel member's
    // flag. Dimensions are captured from the pinned Go CLI for each seed.
    let source = "*.shape:circle\nb->e\nc->f\nc->g\nd->h\nd->i\ne->j\ne->k\n*.ha";
    for (seed, expected_viewbox) in [(1, "0 0 1072 834"), (2, "0 0 962 950"), (3, "0 0 1072 834")] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some(expected_viewbox),
            "seed {seed}"
        );
    }
}

#[test]
fn rejected_transposes_restore_cluster_policy_before_alignment() {
    // D46 job 664. Pinned placement/transpose.go:231-257 rolls back each
    // trial; layoutgraph/transaction.go:1401-1405 restores Cluster policy.
    // These positions and complete routes come from the pinned Go CLI.
    let source = "b.shape:cloud\ne->b.c{style.stroke-width:8;target-arrowhead.shape:diamond;target-arrowhead.style.filled:true}\nf->b{style{}\ntarget-arrowhead.shape:diamond\ntarget-arrowhead.style.filled:true}\ng->b{style.stroke-width:8;target-arrowhead.shape:diamond;target-arrowhead.style.filled:true}\nb:containerlabel";
    for seed in [1, 2, 3] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        let rectangles = document
            .descendants()
            .filter(|node| node.attribute("class") == Some("shape"))
            .flat_map(|node| node.children().filter(|child| child.has_tag_name("rect")))
            .map(|rect| ["x", "y", "width", "height"].map(|key| rect.attribute(key).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            rectangles,
            [
                ["108", "560", "53", "66"],
                ["80", "0", "54", "66"],
                ["154", "0", "54", "66"],
                ["108", "315", "53", "66"],
            ],
            "seed {seed}"
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
                "M 134 555 L 134 394",
                "M 107 68 L 107 112.5 C 107 113.5 108 112.5 108 113.5 L 108 200",
                "M 181 71 L 181 103 S 181 113 191 113 L 224 113 S 234 113 234 123 L 234 239",
            ],
            "seed {seed}"
        );
    }
}

#[test]
fn hierarchy_shuffle_appends_abducted_edges_after_existing_incidences() {
    // D413 job 198. Pinned layoutgraph/layout.go abductEdges reconnects each
    // descendant endpoint after the ancestor's existing Edges. graph.go's
    // SplitSubgraphs BFS consumes that order before hierarchy/placement.go's
    // seeded shuffle. Rectangles and complete routes are from the pinned CLI.
    let source = "a->b.c->d{style.stroke-width:8;target-arrowhead.shape:diamond;target-arrowhead.style.filled:true}\nf->b{style{}}\ng->b{style.stroke-width:8;target-arrowhead.shape:diamond;target-arrowhead.style.filled:true}";
    for seed in [1, 2, 3] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        let rectangles = document
            .descendants()
            .filter(|node| {
                node.has_tag_name("rect")
                    && node
                        .attribute("class")
                        .is_some_and(|class| class.contains("stroke-B1"))
            })
            .map(|node| ["x", "y", "width", "height"].map(|key| node.attribute(key).unwrap()))
            .collect::<Vec<_>>();
        let expected_rectangles = if seed == 2 {
            [
                ["346", "0", "53", "66"],
                ["114", "156", "173", "186"],
                ["173", "432", "54", "66"],
                ["175", "0", "51", "66"],
                ["0", "0", "54", "66"],
                ["174", "216", "53", "66"],
            ]
        } else {
            [
                ["346", "0", "53", "66"],
                ["111", "156", "173", "186"],
                ["172", "432", "54", "66"],
                ["0", "0", "51", "66"],
                ["171", "0", "54", "66"],
                ["171", "216", "53", "66"],
            ]
        };
        assert_eq!(rectangles, expected_rectangles, "seed {seed}");
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
        let expected_routes = if seed == 2 {
            [
                "M 372 71 L 372 239 S 372 249 362 249 L 240 249",
                "M 200 287 L 200 419",
                "M 200 68 L 200 152",
                "M 27 71 L 27 239 S 27 249 37 249 L 101 249",
            ]
        } else {
            [
                "M 372 71 L 372 239 S 372 249 362 249 L 237 249",
                "M 198 287 L 198 419",
                "M 25 68 L 25 239 S 25 249 35 249 L 107 249",
                "M 198 71 L 198 143",
            ]
        };
        assert_eq!(routes, expected_routes, "seed {seed}");
    }
}

#[test]
fn hierarchy_components_keep_direct_container_edges_before_descendant_edges() {
    // D413 job 686 has both native and abducted source incidences on top2.
    // Pinned layoutgraph/edge.go reconnect appends only the changed endpoint;
    // hierarchy/placement.go shuffles the resulting SplitSubgraphs order.
    let source = "top2.t->a\ntop2.->b\ntop2.r-\na->bottom.end\nb->m\nc->bottom.d";
    for (seed, viewbox, expected_routes) in [
        (
            1,
            "0 0 625 906",
            [
                "M 327 214 L 327 358",
                "M 55 136 L 38 136 S 28 136 28 146 L 28 358",
                "M 327 430 L 327 574",
                "M 28 430 L 28 574",
                "M 204 430 L 204 574",
            ],
        ),
        (
            2,
            "0 0 931 820",
            [
                "M 567 128 L 567 272",
                "M 480 93 L 428 93 S 418 93 418 103 L 418 272",
                "M 567 344 L 567 377 S 567 387 557 387 L 336 387 S 326 387 326 397 L 326 488",
                "M 433 344 L 433 515 S 433 525 443 525 L 568 525",
                "M 86 344 L 86 488",
            ],
        ),
        (
            3,
            "0 0 685 906",
            [
                "M 156 214 L 156 358",
                "M 244 136 L 444 136 S 454 136 454 146 L 454 358",
                "M 156 430 L 156 574",
                "M 454 430 L 454 574",
                "M 279 430 L 279 574",
            ],
        ),
    ] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=tala", &format!("--tala-seeds={seed}"), "-", "-"])
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
        let svg = std::str::from_utf8(&output.stdout).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some(viewbox),
            "seed {seed}"
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
        assert_eq!(routes, expected_routes, "seed {seed}");
    }
}

#[test]
fn ordinary_parallel_edges_use_mirrored_center_ports() {
    // F16 job 919. Pinned Go's
    // d2layouts/d2talalayout/internal/routing/ovg_edge_router.go:centerSymmetricalPorts
    // uses the ordinary nodes' complete port inventories when marking mirrored
    // center ports, not only the saved inventories used for cluster routing.
    let source = "a.b-> c\na.h-> g\ne-> c\n*.shape: circle\nf-> d\na-> e\ng-> f\na.h-> g\nd-> c";
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=tala", "--tala-seeds=1", "-", "-"])
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
    let svg = std::str::from_utf8(&output.stdout).unwrap();
    let document = roxmltree::Document::parse(svg).unwrap();
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
            "M 228 393 L 228 611 S 228 621 238 621 L 344 621",
            "M 148 233 L 148 118",
            "M 196 646 L 344 646",
            "M 384 100 L 384 305",
            "M 157 473 L 157 592",
            "M 236 59 L 342 59",
            "M 88 233 L 88 70 S 88 60 98 60 L 110 60",
            "M 384 388 L 384 592",
        ]
    );
}

#[test]
fn alignment_symmetry_uses_active_cluster_vessel_geometry() {
    // F11 job 508. Pinned Go's placementcost/symmetry.go replaces an
    // adjacent cluster member with Cluster.Vessel, and obstruction checks
    // use that vessel's TopLeft and size together. Seed 3 exposes the case:
    // combining the retained member position with the vessel size rejects a
    // mirrored pair and changes the later routes.
    let source = "a: |md\n# h\n|\n\na-> b\nc -> b\nd-> e\nf-> e\nb-> f\nb-> g\ng-> f\nb-> h\nb-> i\nb-> d\nj-> c\nj-> a\nb-> j\ni-> k\nd-> l\nl-> e\nm-> l\nm-> n\nn-> i\nd-> n\nf-> n\nb-> o\np-> l\ne-> q";
    let mut process = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=tala", "--tala-seeds=3", "-", "-"])
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
    let svg = std::str::from_utf8(&output.stdout).unwrap();
    let document = roxmltree::Document::parse(svg).unwrap();
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
    assert_eq!(routes[5], "M 232 356 L 247 356 S 257 356 257 366 L 257 509");
    assert_eq!(routes[21], "M 176 439 L 176 509");
}
