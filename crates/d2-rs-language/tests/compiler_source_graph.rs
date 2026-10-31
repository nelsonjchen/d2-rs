// SPDX-License-Identifier: MPL-2.0
//! Source-graph labels before measurement text transforms. The nine pinned
//! d2compiler TestCompile goldens this file used to replay are covered at the
//! pin by the language scoreboard (docs/fixture-retirements/langlibelk.md).
use d2_rs_core::{Size, ast::Shared};
use d2_rs_language::{compiler, parse};

#[test]
fn source_labels_precede_measurement_text_transforms() {
    let parsed = parse(
        "labels.d2",
        b"x: {style.text-transform: uppercase}\nx -> y: hi {style.text-transform: capitalize}\n",
        None,
    );
    let ast = Shared::new(parsed.map);
    let (compiled, root) = compiler::compile_ir(ast.clone()).unwrap();
    let source = compiler::source_graph_json(&compiled, root, ast).unwrap();
    assert_eq!(source["objects"][0]["attributes"]["label"]["value"], "x");
    assert_eq!(source["edges"][0]["attributes"]["label"]["value"], "hi");
    let (graph, labels, _) =
        compiler::lower_graph_with_measurer_and_labels(&compiled, root, &|_: &str| Size::default())
            .unwrap();
    let object = graph.nodes().next().unwrap().0;
    let edge = graph.edges().next().unwrap().0;
    assert_eq!(labels[&object], "X");
    assert_eq!(graph.edge_label(edge).unwrap().text, "Hi");
}
