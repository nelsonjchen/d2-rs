// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original master option constructor calls.
use super::*;

#[test]
#[ignore = "executed explicitly by elk-options-scoreboard.py"]
fn replay() {
    let calls: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_ELK_OPTIONS_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<Value> = calls
        .iter()
        .map(|call| {
            let options = &call["options"];
            let value = |key: &str| options[key].as_f64().unwrap_or(0.0);
            let configured = ElkOptions {
                algorithm: options["elk.algorithm"]
                    .as_str()
                    .map(crate::ElkAlgorithm::parse)
                    .transpose()
                    .unwrap()
                    .unwrap_or_default(),
                node_spacing: value("spacing.nodeNodeBetweenLayers"),
                edge_node_spacing: value("spacing.edgeNodeBetweenLayers"),
                edge_edge_spacing: options["spacing.edgeEdgeBetweenLayers"]
                    .as_f64()
                    .unwrap_or(50.0),
                self_loop_spacing: value("elk.spacing.nodeSelfLoop"),
                ..ElkOptions::default()
            };
            match call["kind"].as_str().unwrap() {
                "root" => root_layout_options(&configured, ""),
                "container" => container_layout_options(&configured, ""),
                "leaf" => leaf_layout_options(),
                "port" => port_layout_options(options["side"].as_str().unwrap()),
                "edge-label" => inline_label_layout_options(),
                kind => panic!("unknown original constructor {kind}"),
            }
        })
        .collect();
    std::fs::write(
        std::env::var_os("D2RS_ELK_OPTIONS_RESPONSE").unwrap(),
        serde_json::to_vec(&results).unwrap(),
    )
    .unwrap();
}
