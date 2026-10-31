use d2_rs_core::Graph;
use d2_rs_elk::{ElkOptions, graph_to_elk_graph};
fn main() {
    let graph: Graph =
        serde_json::from_slice(&std::fs::read("/tmp/icon-graph.json").unwrap()).unwrap();
    let elk = graph_to_elk_graph(&graph, &graph.node_labels, ElkOptions::default());
    for node in &elk.children[1].children[..12] {
        let o = node.layout_options.as_ref().unwrap();
        println!(
            "{} {} {} {} {}",
            node.id,
            o["d2-rs.marginTop"],
            o["d2-rs.marginRight"],
            o["d2-rs.marginBottom"],
            o["d2-rs.marginLeft"]
        );
    }
}
