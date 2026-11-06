// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use serde_json::Value;
#[test]
fn original_labelled_graph_bounds() {
    let source: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/tala-labelled-graph-bounds-v090.json"
    ))
    .unwrap();
    assert_eq!(source.len(), 2412);
    let routes: &[&[(f64, f64)]] = &[
        &[],
        &[(0., 0.), (100., 0.)],
        &[(0., 0.), (0., 100.)],
        &[(-40.25, 23.75), (120.5, 140.25)],
        &[(10., 20.), (90., 20.), (90., 140.), (200., 140.)],
        &[(13.25, -7.5), (13.25, -7.5)],
    ];
    let mut actual = Vec::new();
    for row in &source {
        let mut input = crate::Graph::default();
        input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.,
                height: 8.,
            },
        ));
        input.add_node(crate::Node::new(
            "b",
            Size {
                width: 14.,
                height: 11.,
            },
        ));
        input.add_edge(crate::Edge {
            source: NodeId(0),
            target: NodeId(1),
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(
            NodeId(0),
            Point {
                x: -20.25,
                y: -17.5,
            },
        );
        graph.set_position(NodeId(1), Point { x: 40.5, y: 8.25 });
        let edge = &mut graph.edge_records[0];
        edge.route.replace(RoutePoints::literal(
            &routes[row["route"].as_u64().unwrap() as usize]
                .iter()
                .map(|&(x, y)| Point { x, y })
                .collect::<Vec<_>>(),
        ));
        let src = row["src"].as_str().unwrap();
        let dst = row["dst"].as_str().unwrap();
        edge.source_arrow = !src.is_empty() && src != "none";
        edge.target_arrow = !dst.is_empty() && dst != "none";
        edge.source_arrowhead = Some(src.into());
        edge.target_arrowhead = Some(dst.into());
        let size = Size {
            width: row["width"].as_f64().unwrap(),
            height: 31.75,
        };
        let mode = row["mode"].as_u64().unwrap();
        if mode == 0 || mode == 3 {
            edge.label = Some(crate::EdgeLabel {
                text: "label".into(),
                size,
                position: LabelPosition::from_d2_name(row["position"].as_str().unwrap()),
                percentage: row["percentage"].as_f64().unwrap(),
            });
        }
        if mode == 1 || mode == 3 {
            edge.source_arrowhead_label = Some(crate::ArrowheadLabel {
                text: "source".into(),
                size,
                color: None,
            });
        }
        if mode == 2 || mode == 3 {
            edge.target_arrowhead_label = Some(crate::ArrowheadLabel {
                text: "target".into(),
                size,
                color: None,
            });
        }
        let (low, high) = graph.fixed_node_bounds(&[NodeId(0), NodeId(1)]).unwrap();
        let (low, high) = graph.include_graph_edge_bounds(low, high);
        let mut observed = row.clone();
        observed["bbox"] = serde_json::json!(
            [low.x, low.y, high.x, high.y].map(|v| format!("{:016x}", v.round().to_bits()))
        );
        observed["area"] = format!(
            "{:016x}",
            graph
                .component_graph_area(&[NodeId(0), NodeId(1)])
                .to_bits()
        )
        .into();
        actual.push(observed);
    }
    if let Some(path) = std::env::var_os("D2RS_LABELLED_BOUNDS_RECORDS") {
        std::fs::write(path, serde_json::to_vec_pretty(&actual).unwrap()).unwrap();
    }
    let diffs = actual
        .iter()
        .zip(&source)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .collect::<Vec<_>>();
    assert!(
        diffs.is_empty(),
        "{} source bounds differences;first {:?}",
        diffs.len(),
        diffs.first()
    );
}
