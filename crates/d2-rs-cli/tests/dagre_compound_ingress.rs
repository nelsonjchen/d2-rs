// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn render_dagre(body: &str, direction: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args(["--layout=dagre", "--omit-version", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        child.stdin.take().unwrap(),
        "direction: {direction}\n{body}"
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{direction}: {body}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn assert_go_geometry(body: &str, edges: usize, expected: [(u32, u32, Option<&str>); 4]) {
    for (direction, (width, height, start)) in
        ["down", "right", "up", "left"].into_iter().zip(expected)
    {
        let svg = render_dagre(body, direction);
        let document = roxmltree::Document::parse(&svg).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some(format!("0 0 {width} {height}").as_str()),
            "{direction}: {body}: canvas differs from pinned Go"
        );
        let routes = document
            .descendants()
            .filter(|node| {
                node.has_tag_name("path")
                    && node.attribute("class").is_some_and(|classes| {
                        classes
                            .split_whitespace()
                            .any(|class| class == "connection")
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(routes.len(), edges, "{direction}: {body}: missing edge");
        if let Some(start) = start {
            let actual = routes[0]
                .attribute("d")
                .unwrap()
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(actual, start, "{direction}: {body}: route origin");
        }
    }
}

#[test]
fn dagre_preserves_container_child_lanes_and_routes_from_external_sources() {
    // Pinned d2dagrelayout/layout.go publishes Dagro's coordinates through
    // adjustRankSpacing, adjustCrossRankSpacing and fitContainerPadding.
    // The route origins also protect connected children whose canvas width
    // alone would not expose the misplaced lanes. All values are from Go.
    for (body, edges, expected) in [
        (
            "p -> k.a\nq -> k.b\n",
            2,
            [
                (429, 514, Some("M 66.5 68")),
                (489, 494, Some("M 56.5 119")),
                (429, 554, Some("M 66.5 264")),
                (489, 494, Some("M 251.5 119")),
            ],
        ),
        (
            "p -> k.a\nq -> k.b\nk.a -> k.b\n",
            3,
            [
                (429, 680, Some("M 66.5 68")),
                (642, 494, Some("M 55 119")),
                (429, 720, Some("M 66.5 430")),
                (642, 494, Some("M 405 119")),
            ],
        ),
        (
            "a.b -> c\na.h -> g\n",
            2,
            [
                (429, 554, Some("M 66.5 118")),
                (489, 494, Some("M 105 119")),
                (429, 514, Some("M 66.5 214")),
                (489, 494, Some("M 202 119")),
            ],
        ),
    ] {
        assert_go_geometry(body, edges, expected);
    }
}

#[test]
fn dagre_compound_parallel_edges_and_self_loops_share_the_translated_flow_frame() {
    // dagro/layout.go::translateGraph normalizes the reflected BT/RL ranks
    // together with restored self-edge labels before D2 publishes routes.
    assert_go_geometry(
        "c: { a -> b: one; a -> b: two; b -> b: self }\nz -> c.a\n",
        4,
        [
            (391, 701, None),
            (665, 428, None),
            (391, 741, None),
            (665, 428, None),
        ],
    );
    assert_go_geometry(
        "s: {o -> p: charge; o -> p: refund; o -> p: void; p -> p: retry}\ng -> s.o\ng -> s.a\n",
        6,
        [
            (488, 701, None),
            (689, 519, None),
            (488, 741, None),
            (689, 519, None),
        ],
    );
}

#[test]
fn dagre_nested_compound_feedback_routes_keep_dagro_cross_coordinates() {
    // d2dagrelayout/layout.go smooths the fitted Dagro routes directly. Its
    // opposing container edges receive no additional quarter-pixel shift.
    assert_go_geometry(
        "n0.n1.n2\nn0 -> n3 -> n0\n",
        2,
        [
            (384, 645, Some("M 75.5 228")),
            (586, 439, Some("M 224 122.5")),
            (384, 594, Some("M 75.5 152")),
            (586, 439, Some("M 200 122.5")),
        ],
    );
}

#[test]
fn dagre_reciprocal_container_edges_keep_the_dagro_external_node_lane() {
    // Pinned d2dagrelayout/layout.go:287–289 carries the border-model
    // coordinates through spacing and fitting without another outside-node shift.
    let body = "n0\nn1\nn0.n2\nn0.n3\nn0.n4\nn0 -> n1\nn1 -> n0\n";
    for (direction, expected) in [
        (
            "down",
            [
                "M 116.5 184 C 116.5 130 125.699997 106 159.791774 68.943724",
                "M 225.452587 64.37477 C 268.799988 105.400002 280 130 280 182",
            ],
        ),
        (
            "right",
            [
                "M 180 165.5 C 126 165.5 102 175.300003 64.856851 211.700286",
                "M 62.33359 278.990483 C 101.800003 323.100006 126 334.5 178 334.5",
            ],
        ),
        (
            "up",
            [
                "M 116.5 148 C 116.5 202 125.699997 226 159.791774 263.056276",
                "M 225.452587 267.62523 C 268.799988 226.600006 280 202 280 150",
            ],
        ),
        (
            "left",
            [
                "M 145 165.5 C 199 165.5 223 175.300003 260.143149 211.700286",
                "M 262.66641 278.990483 C 223.199997 323.100006 199 334.5 147 334.5",
            ],
        ),
    ] {
        let svg = render_dagre(body, direction);
        let document = roxmltree::Document::parse(&svg).unwrap();
        let paths = document
            .descendants()
            .filter_map(|node| {
                node.attribute("class")
                    .is_some_and(|classes| {
                        classes
                            .split_whitespace()
                            .any(|class| class == "connection")
                    })
                    .then(|| node.attribute("d"))
                    .flatten()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paths, expected,
            "{direction}: {body}: fitted routes differ from pinned Go"
        );
    }
}

#[test]
fn dagre_final_edge_trace_preserves_fractional_fitted_container_bounds() {
    // Pinned d2dagrelayout/layout.go:289–358 traces fitted TopLeft and Width
    // directly; fitPadding can set them to 20.5 and 122.5 after initial rounding.
    let body = "n0\nn1\nn2\nn0.n3\nn4\nn0 -> n1\nn0.n3 -> n0.n3\n";
    for (direction, expected) in [
        (
            "down",
            [
                "M 71 148 C 71 202 71 226 71 262",
                "M 103.902648 72.383616 C 158.800003 54.598999 166.800003 56.599998 157.5 66.5 C 148.199997 76.400002 135.800003 89.599998 126.5 99.5 C 117.199997 109.400002 134 83 106 83",
            ],
        ),
        (
            "right",
            [
                "M 145 119 C 199 119 223 119 259 119",
                "M 71.922898 153.914929 C 54.900002 210.399994 56.700001 218.399994 66 208.5 C 75.300003 198.600006 87.699997 185.399994 97 175.5 C 106.300003 165.600006 81.5 184 81.5 156",
            ],
        ),
        (
            "up",
            [
                "M 71 143 C 71 121.800003 71 106 71 70",
                "M 103.902658 259.616354 C 158.800003 277.399994 166.800003 275.399994 157.5 265.5 C 148.199997 255.600006 135.800003 242.399994 126.5 232.5 C 117.199997 222.600006 134 249 106 249",
            ],
        ),
        (
            "left",
            [
                "M 181 119 C 126.198997 119 102 119 66 119",
                "M 254.052919 153.922051 C 270.299988 210.399994 268.299988 218.399994 259 208.5 C 249.699997 198.600006 237.300003 185.399994 228 175.5 C 218.699997 165.600006 243.5 184 243.5 156",
            ],
        ),
    ] {
        let svg = render_dagre(body, direction);
        let document = roxmltree::Document::parse(&svg).unwrap();
        let paths = document
            .descendants()
            .filter_map(|node| {
                node.attribute("class")
                    .is_some_and(|classes| {
                        classes
                            .split_whitespace()
                            .any(|class| class == "connection")
                    })
                    .then(|| node.attribute("d"))
                    .flatten()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paths, expected,
            "{direction}: {body}: fitted routes differ from pinned Go"
        );
    }
}
