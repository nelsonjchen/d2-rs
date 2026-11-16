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

fn connection_paths(svg: &str) -> Vec<String> {
    let document = roxmltree::Document::parse(svg).unwrap();
    document
        .descendants()
        .filter(|n| {
            n.has_tag_name("path")
                && n.attribute("class")
                    .is_some_and(|class| class.split(' ').any(|c| c == "connection"))
        })
        .map(|n| n.attribute("d").unwrap().to_owned())
        .collect()
}

fn render_error(engine: &str, source: &str) -> String {
    let layout = format!("--layout={engine}");
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([layout.as_str(), "--omit-version", "-", "-"])
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
    assert!(!output.status.success(), "{engine}: {source}");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn dagre_container_self_loop_sentinel_is_the_container_center() {
    // d2dagrelayout layout.go:249-255 brackets a self-loop with
    // edge.Src.Center(). Dagro routes a container's loop around a descendant
    // leaf, but the sentinels stay at the container's center. Dagre rejects
    // container self-loops only on the root board, so this is a child board.
    let svg = render_with(
        "dagre",
        "layers: {s: {b: {q -> _.c}; b -> b}}\n",
        &["--target=s"],
    );
    assert_eq!(
        connection_paths(&svg),
        [
            "M 67 118 C 67 156 67 226 67 262",
            "M 152.8906 54.664101 C 138 77 128.600006 89.599998 120.5 99.5 C 112.400002 109.400002 122.599998 83 81 83",
        ]
    );
}

#[test]
fn elk_model_order_sort_follows_elk_go_comparators() {
    // preserveorder/node_comparator.go handleHelperDummyNodes: the layer of a
    // normal/dummy pair comes from the normal node, and two dummies sharing a
    // reference node are ordered by feedback side or by that node's ports.
    // port_comparator.go decides every pair of ports with outgoing edges,
    // ties included.
    for (source, expected) in [
        (
            "p -> c\nb.q -> p\nc.k -> b\nz.k -> a\np.m -> a\nc -> p\nc.k -> p\n",
            vec![
                (133, 654, 158, 166),
                (98, 52, 180, 166),
                (53, 353, 154, 166),
                (311, 654, 153, 166),
                (326, 905, 80, 66),
                (103, 403, 54, 66),
                (148, 102, 80, 66),
                (361, 704, 53, 66),
                (184, 704, 57, 66),
            ],
        ),
        (
            "a: {\n}\nb: {\n}\nb.n0 -> a\noutside <- b.n0: x\nb.n0 <- outside: x\na.n0 -> outside: a longer connection label\na <- outside\n",
            vec![
                (64, 52, 162, 166),
                (319, 308, 180, 166),
                (285, 710, 120, 66),
                (369, 358, 80, 66),
                (114, 102, 62, 66),
            ],
        ),
        (
            "direction: left\ndb: {shape: sql_table; id: int {constraint: primary_key}; owner: int; name: text}\ndb.id <- api: a longer connection label\ndb.name -> host\napi <-> db.id: a longer connection label\nhost.n -> db.id\n",
            vec![
                (455, 12, 180, 144),
                (12, 172, 67, 66),
                (222, 92, 153, 166),
                (272, 142, 53, 66),
            ],
        ),
    ] {
        assert_eq!(
            stroked_rectangles(&render("elk", source)),
            expected,
            "{source}"
        );
    }
}

#[test]
fn elk_nested_left_self_loop_mirrors_against_the_nested_graph() {
    // graph_transformer.go mirrorAllX reflects a nested graph against its
    // own extent, which includes external-port dummies (first case) and is
    // narrower than a container that centers its content (second case).
    for (source, expected) in [
        (
            "direction: left\ncontainer: {\n  n0: {shape: document; width: 41; height: 25; label.near: center-left}\n}\noutside -> container.n0\noutside -> container.n0\ncontainer.n1 -> container.n1\n",
            "M 103.333 110 L 103.333 72 S 103.333 62 93.333 62 L 92.666 62 S 82.666 62 82.666 72 L 82.666 108",
        ),
        (
            "direction: left\ncontainer: {\n  n0: {shape: step; width: 41; height: 19; label.near: top-center}\n  n1: {shape: document; width: 21; height: 31; label.near: top-right}\n}\ncontainer.n0 <-> container.n0: x\ncontainer.n1 -- outside\n",
            "M 270.023786 137.000071 L 270.292373 91.833391 C 270.373643 78.166609 256.622092 78.16663 256.709878 91.83337 L 256.974307 133.000083",
        ),
    ] {
        let paths = connection_paths(&render("elk", source));
        assert!(
            paths.iter().any(|path| path == expected),
            "{source}: {paths:?}"
        );
    }
}

#[test]
fn invalid_position_names_the_first_object_in_layout_nested_order() {
    // d2layouts.go:368 validateObjectPositions walks g.Objects in compiled
    // (AST) order. TALA's adapter keeps that order even though it traverses
    // the hierarchy depth-first itself.
    let source = "classes: {red: {style.fill: red}; blue: {style.fill: blue; style.bold: true}; big: {style.font-size: 30}; ov: {shape: oval}; e: {label: \"\"}}\nshape: sequence_diagram\na\nb\nc: {style.fill: green}\nc: {g: {c.class: [blue; red]; c -> a}; class: red}\ngroup 1: {b: {class: red}; c -> b}\nb: {g: {a: {class: red}; a -> a; b -> a}; class: red}\na -> b\n";
    for engine in ["dagre", "elk", "tala"] {
        let stderr = render_error(engine, source);
        assert!(
            stderr.contains(
                "object \"c.g\" has invalid position with infinity value: x=356.000000, y=+Inf"
            ),
            "{engine}: {stderr}"
        );
    }
}

#[test]
fn dagre_short_self_loop_endpoint_extends_into_the_container() {
    // d2dagrelayout layout.go:290-325: an endpoint within MIN_SEGMENT_LEN of
    // its neighbor (here c0.d's center sentinel, half a pixel from Dagro's
    // first control) is replaced by extending the next segment into the box.
    let svg = render_with(
        "dagre",
        "layers: {s: {c0 <-> c0: \"two\\nlines\"; c0.d <- c0.d: x; c0.d.k0 <- c0.d: x}}\n",
        &["--target=s"],
    );
    assert_eq!(
        connection_paths(&svg),
        [
            "M 119.729273 193.075779 C 139.399994 172 151.100006 159.399994 160.25 149.5 C 169.399994 139.600006 199.800003 106.599998 176.369351 131.542304",
            "M 221.785032 135.937507 C 200.100006 159.399994 230.600006 126.400002 253.642516 101.468754",
            "M 178.893304 132.082293 C 287 106.599998 308.899994 106.599998 299.75 116.5 C 290.600006 126.400002 278.399994 139.600006 269.25 149.5 C 260.100006 159.399994 262.600006 133 179 133",
        ]
    );
}

#[test]
fn elk_one_sided_side_loop_label_margin_starts_at_its_top_port() {
    // label_placer.go assignOneSidedSimple aligns a one-sided West/East
    // label to its topmost port (LabelTop) and clears inline placement;
    // orthogonal_router.go updateMargins then reserves the whole label box.
    let source = "c0.n0 -> c0.n0: x\nc0.n0 -> c0.n0: x\nc0.n0 -> c0.n0: this self loop label is deliberately very long so that it is wider than its node\n";
    assert_eq!(
        stroked_rectangles(&render("elk", source)),
        vec![(12, 12, 874, 433), (317, 62, 120, 66)]
    );
}

#[test]
fn nested_sequence_lifelines_follow_children_array_order() {
    // LayoutNested queues ChildrenArray (d2layouts.go:92-93). Re-declaring an
    // import moves its field after its siblings, so `c` is extracted, laid out
    // and gets its lifelines before `b` even though g.Objects lists `b` first.
    let directory =
        std::env::temp_dir().join(format!("d2rs-lifeline-order-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("lib")).unwrap();
    std::fs::write(
        directory.join("input.d2"),
        "w -> b\nc: @lib/model\nc: {t -> _.w}\nb: @lib/model\n",
    )
    .unwrap();
    std::fs::write(
        directory.join("lib/model.d2"),
        "shape: sequence_diagram\ng1: {\n}\ng2: {\n}\na -> a\nb -> c\ng3: {\nc -> c\n}\n",
    )
    .unwrap();
    for engine in ["dagre", "elk", "tala"] {
        let status = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .current_dir(&directory)
            .args([
                &format!("--layout={engine}"),
                "--omit-version",
                "input.d2",
                "out.svg",
            ])
            .output()
            .unwrap();
        assert!(status.status.success(), "{engine}");
        let svg = std::fs::read_to_string(directory.join("out.svg")).unwrap();
        // base64 classes of (c.g1 -- )[0] and (b.g1 -- )[0]
        let c = svg.find("KGMuZzEgLS0gKVswXQ==").unwrap();
        let b = svg.find("KGIuZzEgLS0gKVswXQ==").unwrap();
        assert!(c < b, "{engine}");
    }
    std::fs::remove_dir_all(&directory).unwrap();
}
