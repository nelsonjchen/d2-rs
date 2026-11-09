// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn render_tala_seed_three(source: &str) -> String {
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
    String::from_utf8(output.stdout).unwrap()
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
