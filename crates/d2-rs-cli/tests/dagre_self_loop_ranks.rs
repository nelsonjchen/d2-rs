// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn compound_translation_discards_removed_self_edge_dummies() {
    // Pinned dagro layout.go: positionSelfEdges removes the wide dummy before
    // translateGraph measures the compound envelope, leaf nodes and labels.
    // The terminal rank's 166 px dummy must not leave a 34 px left margin.
    // These rectangles and route origins come from the pinned Go CLI.
    for (body, up_y, left_x, loop_index) in [
        (
            "container: {}\ncontainer.n0 -> outside\noutside <- outside: a longer connection label",
            186,
            411,
            1,
        ),
        (
            "container: {}\noutside <- outside: a longer connection label\noutside <- container.n0: x",
            207,
            420,
            0,
        ),
    ] {
        for direction in ["down", "right", "up", "left"] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
                .args(["--layout=dagre", "--omit-version", "-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(
                child.stdin.take().unwrap(),
                "direction: {direction}\n{body}"
            )
            .unwrap();
            let output = child.wait_with_output().unwrap();
            let context = format!("{direction}: {body}");
            if matches!(direction, "down" | "right") {
                assert!(!output.status.success(), "{context}");
                assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
                    "render target root.connections[{loop_index}].route[4:5] must have a finite, non-zero segment length"
                )), "{context}");
                continue;
            }
            assert!(
                output.status.success(),
                "{context}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let svg = String::from_utf8(output.stdout).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let rectangles = document
                .descendants()
                .filter(|n| n.has_tag_name("rect") && n.attribute("stroke").is_some())
                .map(|n| {
                    let value = |a| n.attribute(a).unwrap().parse::<i32>().unwrap();
                    (value("x"), value("y"), value("width"), value("height"))
                })
                .collect::<Vec<_>>();
            let expected = if direction == "up" {
                vec![
                    (10, up_y, 122, 126),
                    (22, 0, 98, 66),
                    (40, up_y + 30, 62, 66),
                ]
            } else {
                vec![
                    (left_x, 56, 122, 126),
                    (0, 86, 98, 66),
                    (left_x + 30, 86, 62, 66),
                ]
            };
            assert_eq!(rectangles, expected, "{context}");
            let routes = document
                .descendants()
                .filter(|n| {
                    n.has_tag_name("path")
                        && n.attribute("class")
                            .is_some_and(|c| c.split_whitespace().any(|c| c == "connection"))
                })
                .map(|n| n.attribute("d").unwrap())
                .collect::<Vec<_>>();
            assert_eq!(routes.len(), 2, "{context}");
            assert!(
                routes[loop_index].starts_with(if direction == "up" {
                    "M 124 33 C"
                } else {
                    "M 49 155.5 C"
                }),
                "{context}: {}",
                routes[loop_index]
            );
        }
    }
}

#[test]
fn self_loop_labels_size_ranks_and_preserve_reflected_routes() {
    // Pinned dagro layout.go: insertSelfEdges -> positionY ->
    // positionSelfEdges -> removeBorderNodes -> undoCoordinateSystem ->
    // translateGraph. The dummy sizes the rank, then only the restored
    // label box participates in translation. Geometry captured from Go.
    let directions = ["down", "right", "up", "left"];
    for (body, rectangles, starts) in [
        (
            "a.n0 -> a.n0: a longer connection label",
            [
                vec![(10, 20, 318, 126), (40, 50, 62, 66)],
                vec![(76, 56, 226, 177), (155, 86, 62, 66)],
                vec![(10, 20, 318, 126), (40, 50, 62, 66)],
                vec![(70, 56, 226, 177), (155, 86, 62, 66)],
            ],
            [
                "M 103.969952 76.654619",
                "M 176.493281 153.434744",
                "M 103.969952 89.345381",
                "M 195.506719 153.434744",
            ],
        ),
        (
            "a -> a: a longer connection label",
            std::array::from_fn(|_| vec![(0, 0, 53, 66)]),
            [
                "M 54.466038 27.63299",
                "M 18.558738 67.450715",
                "M 54.46604 38.366994",
                "M 34.441235 67.450721",
            ],
        ),
        (
            "container: {}\noutside <- outside: a longer connection label",
            [
                vec![],
                vec![],
                vec![(0, 0, 114, 66), (174, 0, 98, 66)],
                vec![(0, 0, 114, 66), (8, 126, 98, 66)],
            ],
            ["", "", "M 276 33", "M 57 195.5"],
        ),
    ] {
        for (index, direction) in directions.into_iter().enumerate() {
            let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
                .args(["--layout=dagre", "--omit-version", "-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(
                child.stdin.take().unwrap(),
                "direction: {direction}\n{body}"
            )
            .unwrap();
            let output = child.wait_with_output().unwrap();
            let context = format!("{direction}: {body}");
            if starts[index].is_empty() {
                assert!(!output.status.success(), "{context}");
                assert!(String::from_utf8_lossy(&output.stderr).contains(
                    "render target root.connections[0].route[4:5] must have a finite, non-zero segment length"
                ), "{context}");
                continue;
            }
            assert!(
                output.status.success(),
                "{context}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let svg = String::from_utf8(output.stdout).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let actual = document
                .descendants()
                .filter(|n| n.has_tag_name("rect") && n.attribute("stroke").is_some())
                .map(|n| {
                    let value = |a| n.attribute(a).unwrap().parse::<i32>().unwrap();
                    (value("x"), value("y"), value("width"), value("height"))
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, rectangles[index], "{context}");
            let route = document
                .descendants()
                .find(|n| {
                    n.has_tag_name("path")
                        && n.attribute("class")
                            .is_some_and(|c| c.split_whitespace().any(|c| c == "connection"))
                })
                .unwrap()
                .attribute("d")
                .unwrap();
            assert!(route.starts_with(starts[index]), "{context}: {route}");
        }
    }
}

#[test]
fn restored_self_labels_translate_both_axes() {
    // Pinned dagro layout.go: removeSelfEdges runs before
    // adjustCoordinateSystem and positionSelfEdges restores the label after
    // it, so undoCoordinateSystem swaps only the label. translateGraph then
    // measures label height along the flow and label width across it, in
    // every direction, for flat, compound and expanded-compound graphs.
    // Rectangles and route origins come from the pinned Go CLI.
    let long = "this self loop label is deliberately very long so that it is wider than its node";
    let tall = r#""one\ntwo\nthree\nfour""#;
    type Case = (&'static str, Vec<(i32, i32, i32, i32)>, Vec<&'static str>);
    let cases: Vec<(String, [Case; 4])> = vec![
        (
            format!("a -> a: {long}"),
            [
                ("down", vec![(0, 0, 53, 66)], vec!["M 54.990457 29.804857"]),
                (
                    "right",
                    vec![(0, 123, 53, 66)],
                    vec!["M 18.558738 190.450715"],
                ),
                ("up", vec![(0, 0, 53, 66)], vec!["M 54.990457 36.195143"]),
                (
                    "left",
                    vec![(0, 123, 53, 66)],
                    vec!["M 34.441235 190.450721"],
                ),
            ],
        ),
        (
            format!("o -> o: {tall}\no -> c.n"),
            [
                (
                    "down",
                    vec![(40, 2, 54, 66), (10, 203, 113, 126), (40, 233, 53, 66)],
                    vec!["M 95.916904 25.929493", "M 66.5 70.5"],
                ),
                (
                    "right",
                    vec![(8, 86, 54, 66), (182, 56, 113, 126), (212, 86, 53, 66)],
                    vec!["M 28.118261 153.463231", "M 64.5 119"],
                ),
                (
                    "up",
                    vec![(40, 286, 54, 66), (10, 25, 113, 126), (40, 55, 53, 66)],
                    vec!["M 95.916904 327.070507", "M 66.5 283.5"],
                ),
                (
                    "left",
                    vec![(253, 86, 54, 66), (20, 56, 113, 126), (50, 86, 53, 66)],
                    vec!["M 286.381739 153.463231", "M 251 119"],
                ),
            ],
        ),
        (
            format!("o -> o: {tall}\no -> c.d.n"),
            [
                (
                    "down",
                    vec![
                        (60, 2, 54, 66),
                        (0, 216, 173, 197),
                        (30, 257, 113, 126),
                        (60, 287, 53, 66),
                    ],
                    vec!["M 115.916904 25.929493", "M 86.5 70.5"],
                ),
                (
                    "right",
                    vec![
                        (8, 147, 54, 66),
                        (202, 76, 173, 197),
                        (232, 117, 113, 126),
                        (262, 147, 53, 66),
                    ],
                    vec!["M 28.118261 214.463231", "M 64.5 180"],
                ),
                (
                    "up",
                    vec![
                        (60, 395, 54, 66),
                        (0, 38, 173, 197),
                        (30, 79, 113, 126),
                        (60, 109, 53, 66),
                    ],
                    vec!["M 115.916904 436.070507", "M 86.5 392.5"],
                ),
                (
                    "left",
                    vec![
                        (353, 147, 54, 66),
                        (40, 76, 173, 197),
                        (70, 117, 113, 126),
                        (100, 147, 53, 66),
                    ],
                    vec!["M 386.381739 214.463231", "M 351 180"],
                ),
            ],
        ),
    ];
    for (body, directions) in cases {
        for (direction, rectangles, starts) in directions {
            let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
                .args(["--layout=dagre", "--omit-version", "-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(
                child.stdin.take().unwrap(),
                "direction: {direction}\n{body}"
            )
            .unwrap();
            let output = child.wait_with_output().unwrap();
            let context = format!("{direction}: {body}");
            assert!(
                output.status.success(),
                "{context}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let svg = String::from_utf8(output.stdout).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let actual = document
                .descendants()
                .filter(|n| n.has_tag_name("rect") && n.attribute("stroke").is_some())
                .map(|n| {
                    let value = |a| n.attribute(a).unwrap().parse::<i32>().unwrap();
                    (value("x"), value("y"), value("width"), value("height"))
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, rectangles, "{context}");
            let routes = document
                .descendants()
                .filter(|n| {
                    n.has_tag_name("path")
                        && n.attribute("class")
                            .is_some_and(|c| c.split_whitespace().any(|c| c == "connection"))
                })
                .map(|n| n.attribute("d").unwrap())
                .collect::<Vec<_>>();
            assert_eq!(routes.len(), starts.len(), "{context}");
            for (route, start) in routes.iter().zip(&starts) {
                assert!(
                    route.starts_with(&format!("{start} ")),
                    "{context}: {route}"
                );
            }
        }
    }
}

#[test]
fn vessel_border_origin_is_published_rounded() {
    // Pinned dagro layout.go removeBorderNodes centers a vessel on its left
    // border chain; d2dagrelayout/layout.go publishes Round(x - width/2)
    // before fitPadding trims it. A half-pixel border must not survive into
    // the fitted width (Go: 259 + 170, not 259 + 169.5 -> 169).
    // Rectangles come from the pinned Go CLI.
    let body = "s: {\n  shape: sequence_diagram\n  b -> a: long response label\n}\nexternal -> s\nother.child <-> other.child";
    for (direction, expected) in [
        (
            "down",
            vec![
                (0, 266, 312, 316),
                (104, 50, 105, 66),
                (259, 20, 170, 126),
                (12, 354, 100, 66),
                (200, 354, 100, 66),
                (289, 50, 80, 66),
            ],
        ),
        (
            "up",
            vec![
                (0, 0, 312, 316),
                (104, 466, 105, 66),
                (259, 436, 170, 126),
                (12, 88, 100, 66),
                (200, 88, 100, 66),
                (289, 466, 80, 66),
            ],
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=dagre", "--omit-version", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(
            child.stdin.take().unwrap(),
            "direction: {direction}\n{body}"
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{direction}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let svg = String::from_utf8(output.stdout).unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let actual = document
            .descendants()
            .filter(|n| n.has_tag_name("rect") && n.attribute("stroke").is_some())
            .map(|n| {
                let value = |a| n.attribute(a).unwrap().parse::<i32>().unwrap();
                (value("x"), value("y"), value("width"), value("height"))
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{direction}");
    }
}
