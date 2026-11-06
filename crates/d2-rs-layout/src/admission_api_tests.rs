// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original graph-admission calls.
use d2_rs_core::{CancellationToken, Edge, Graph, LayoutOptions, Node, NodeId, Size};
use serde_json::{Value, json};

#[test]
#[ignore = "executed explicitly by layout-admission-scoreboard.py"]
fn replay() {
    let input: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_LAYOUT_ADMISSION_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let mut results = Vec::new();
    for call in input {
        let engine = call["engine"].as_str().unwrap();
        let count = call["objects"].as_u64().unwrap() as usize;
        let token = CancellationToken::new();
        if call["canceled"].as_bool().unwrap() {
            token.cancel();
        }
        let mut graph = Graph::default();
        for i in 0..count {
            graph.add_node(Node::new(
                i.to_string(),
                Size {
                    width: 100.,
                    height: 60.,
                },
            ));
        }
        for edge in call["edges"].as_array().unwrap() {
            if edge.is_null() {
                continue;
            }
            let endpoints: Option<Vec<u32>> = edge
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().map(|v| v as u32))
                .collect();
            if let Some(pair) = endpoints {
                graph.add_edge(Edge {
                    source: NodeId(pair[0]),
                    target: NodeId(pair[1]),
                });
            }
        }
        let options = LayoutOptions::default();
        let result: Result<(), String> = match engine {
            "dagre" => d2_rs_dagre::layout_with_cancellation(&graph, &options, Some(&token))
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "ELK" => d2_rs_elk::layout_with_cancellation(&graph, &options, Some(&token))
                .map(|_| ())
                .map_err(|e| e.to_string()),
            "test" => {
                let edges = call["edges"].as_array().unwrap().iter().map(|edge| {
                    let pair = edge.as_array()?;
                    Some((
                        NodeId(pair[0].as_u64()? as u32),
                        NodeId(pair[1].as_u64()? as u32),
                    ))
                });
                d2_rs_core::layout_guard::check_raw("test", count, edges, Some(&token))
                    .map_err(|e| e.to_string())
            }
            _ => panic!("unknown engine"),
        };
        results.push(json!({"error":result.err().unwrap_or_default()}));
    }
    std::fs::write(
        std::env::var_os("D2RS_LAYOUT_ADMISSION_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}
