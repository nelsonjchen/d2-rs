// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn assert_go_geometry(body: &str, edges: usize, expected: [(u32, u32, Option<&str>); 4]) {
    for (direction, (width, height, start)) in
        ["down", "right", "up", "left"].into_iter().zip(expected)
    {
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
        let svg = String::from_utf8(output.stdout).unwrap();
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
