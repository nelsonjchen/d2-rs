// SPDX-License-Identifier: MPL-2.0
//! Original raw target links transported into the production graph/style loops.
use crate::*;
use serde_json::{Value, json};

fn observe(input: &Value) -> Value {
    if input["op"] == "link" {
        return json!({"value": board_output_link(input["path"].as_str().unwrap())});
    }
    assert_eq!(input["op"], "relink");
    let mut diagram = input["diagram"].clone();
    for field in ["layers", "scenarios", "steps"] {
        assert!(diagram[field].is_null() || diagram[field].as_array().unwrap().is_empty());
    }
    // Original target fields stay intact. Only link storage is adapted to
    // the native compiler's node styles and graph edge-link side table.
    let mut graph = Graph::default();
    let mut styles = BTreeMap::new();
    let mut node_ids = Vec::new();
    for (i, shape) in diagram["shapes"].as_array().unwrap().iter().enumerate() {
        let id = graph.add_node(d2_rs_core::Node::new(i.to_string(), Size::default()));
        styles.insert(
            id,
            compiler::SourceNodeStyle {
                link: shape["link"].as_str().map(str::to_owned),
                ..Default::default()
            },
        );
        node_ids.push(id);
    }
    let mut edge_ids = Vec::new();
    for edge in diagram["connections"].as_array().unwrap() {
        let id = graph.add_edge(d2_rs_core::Edge {
            source: node_ids[0],
            target: node_ids[0],
        });
        graph.set_edge_link(id, edge["link"].as_str().map(str::to_owned));
        edge_ids.push(id);
    }
    let paths = input["paths"].as_object().unwrap();
    let current = Path::new(paths[input["current"].as_str().unwrap()].as_str().unwrap());
    relink_document_links(&mut graph, &mut styles, |link| {
        paths
            .get(link)
            .map(|dest| relative_board_output_link(current, Path::new(dest.as_str().unwrap())))
    });
    for (shape, id) in diagram["shapes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(node_ids)
    {
        shape["link"] = styles[&id].link.clone().unwrap().into();
    }
    for (edge, id) in diagram["connections"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(edge_ids)
    {
        edge["link"] = graph.edge_link(id).unwrap().into();
    }
    json!({"diagram": diagram, "error": ""})
}

#[test]
#[ignore = "input-only original board-link API replay"]
fn replay() {
    let input = std::env::var_os("D2RS_LINK_REQUEST").unwrap();
    let output = std::env::var_os("D2RS_LINK_RESPONSE").unwrap();
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let results: Vec<_> = requests.iter().map(observe).collect();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer(&mut file, &results).unwrap();
}
