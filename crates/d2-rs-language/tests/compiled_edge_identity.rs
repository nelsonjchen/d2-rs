// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![cfg(feature = "oracle-fixtures")]
use d2_rs_core::{Graph, ast::Shared, edge_abs_id};
use d2_rs_language::{compiler, parse};

#[test]
fn compiled_edge_identity_matches_original_go_v090() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/compiled-edge-identity-v090.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 40);
    let mut differences = Vec::new();
    for row in cases {
        let name = row["input"]["name"].as_str().unwrap();
        let parsed = parse(
            "-",
            row["input"]["source"].as_str().unwrap().as_bytes(),
            None,
        );
        assert!(parsed.error.is_none(), "{name}: {:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        let roundtrip: Graph =
            serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
        for (phase, graph) in [("native", &graph), ("serialized", &roundtrip)] {
            let actual = graph
                .edges()
                .map(|(id, edge)| {
                    let arrows = graph.edge_arrows(id);
                    serde_json::json!({
                        "src": graph.node(edge.source).unwrap().external_id,
                        "dst": graph.node(edge.target).unwrap().external_id,
                        "src_arrow": arrows.source, "dst_arrow": arrows.target,
                        "abs_id": edge_abs_id(graph, id).unwrap(),
                        "index": graph.edge_index(id).unwrap(),
                    })
                })
                .collect::<Vec<_>>();
            let expected = row["edges"].as_array().unwrap().clone();
            if actual != expected {
                differences.push(format!("{name}/{phase}: {actual:?} != {expected:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
