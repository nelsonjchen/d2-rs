// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original PreserveCompoundRoutes calls. Placement is
//! supplied by Go as input; this observes refinement, not CompoundCandidate.
use super::*;
use serde_json::{Value, json};

pub(in crate::internal) fn decode(value: &Value) -> (Graph, ArenaGraph) {
    let input =
        crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value["wire"]).unwrap()).unwrap();
    let mut arena = ArenaGraph::from_input(&input);
    let nodes = value["view"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), arena.nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        arena.set_position(
            NodeId(index as u32),
            serde_json::from_value(node["point"].clone()).unwrap(),
        );
        arena.nodes[index].label_position_fixed = node["label"]["fixed"] == true;
        arena.nodes[index].icon_position_fixed = node["icon"]["fixed"] == true;
    }
    let edges = value["view"]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), arena.edge_records.len());
    for (index, edge) in edges.iter().enumerate() {
        let actual = &mut arena.edge_records[index];
        // graphjson assigns IDs to zero-ID edges and omits curve/fixed bits.
        // Restore those INPUT fields explicitly, including duplicate zero IDs.
        actual.tala_id = edge["id"].as_i64().unwrap();
        actual.is_curve = edge["curve"].as_bool().unwrap();
        actual.label_position_fixed = edge["label"]["fixed"] == true;
    }
    (input, arena)
}

pub(in crate::internal) fn view(graph: &ArenaGraph, metadata: &Value) -> Value {
    let nodes: Vec<_> = graph.nodes.iter().enumerate().map(|(index, n)| {
        let label = n.label_size.map(|size| json!({"text":metadata["nodes"][index]["label"]["text"],
            "position":n.label_position.d2_name(),"width":size.width,"height":size.height,"fixed":n.label_position_fixed}));
        let icon = n.has_icon.then(|| json!({"position":n.icon_position.unwrap().d2_name(),"fixed":n.icon_position_fixed}));
        json!({"id":n.tala_id,"parent":n.container.map(|id| graph.nodes[id.0 as usize].tala_id),
            "point":graph.position(NodeId(index as u32)),"width":n.rect.size.width,"height":n.rect.size.height,"label":label,"icon":icon})
    }).collect();
    let arrow = |l: &Option<ArrowheadLabel>| {
        l.as_ref()
            .map(|l| json!({"text":l.text,"width":l.size.width,"height":l.size.height}))
    };
    let edges: Vec<_> = graph.member_edge_indices().map(|index| {
        let e = &graph.edge_records[index];
        let label = e.label.as_ref().map(|l| json!({"text":l.text,"position":l.position.d2_name(),
            "width":l.size.width,"height":l.size.height,"fixed":e.label_position_fixed}));
        json!({"id":e.tala_id,"from":graph.nodes[e.from.0 as usize].tala_id,"to":graph.nodes[e.to.0 as usize].tala_id,
            // The arena stores percentage on labels. For a label-less input
            // the scalar remains opaque metadata, like node label text above.
            "points":*e.route.read(),"curve":e.is_curve,"percentage":e.label.as_ref().map_or_else(
                || metadata["edges"][index]["percentage"].as_f64().unwrap(), |l| l.percentage),
            "label":label,"source_label":arrow(&e.source_arrowhead_label),"target_label":arrow(&e.target_arrowhead_label)})
    }).collect();
    json!({"nodes":nodes,"edges":edges})
}

#[test]
#[ignore = "explicit original-call replay requires input/output paths"]
fn replay() {
    let request = std::env::var_os("D2RS_TALA_INTERIOR_REQUEST").unwrap();
    let response = std::env::var_os("D2RS_TALA_INTERIOR_RESPONSE").unwrap();
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    let results: Vec<_> = requests
        .iter()
        .map(|q| {
            let (_, before) = decode(&q["before"]);
            let (input, selected) = decode(&q["selected"]);
            // Validate input reconstruction before invoking production code.
            assert_eq!(view(&before, &q["before"]["view"]), q["before"]["view"]);
            assert_eq!(
                view(&selected, &q["selected"]["view"]),
                q["selected"]["view"]
            );
            let result = preserve_compound_routes(&input, &before, &selected, None);
            let (graph, error) = match result {
                Ok(graph) => (view(&graph, &q["selected"]["view"]), String::new()),
                Err(error) => (Value::Null, error.to_string()),
            };
            json!({"graph":graph,"error":error,"before_after":view(&before,&q["before"]["view"]),
            "selected_after":view(&selected,&q["selected"]["view"])})
        })
        .collect();
    std::fs::write(response, serde_json::to_vec(&results).unwrap()).unwrap();
}
