//! Reconstructs an ELK input graph from `d2-rs graph` JSON for oracle probes.
use d2_rs_core::{Graph, ShapeKind};
use d2_rs_elk::{ElkOptions, graph_to_elk_graph};
use std::collections::BTreeMap;
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let graph: Graph = serde_json::from_str(&input)?;
    let mut labels = BTreeMap::new();
    for (id, node) in graph.nodes() {
        if node.label_size.is_some()
            && !matches!(
                node.shape,
                ShapeKind::Text | ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code
            )
        {
            labels.insert(id, node.external_id.clone());
        }
    }
    let elk = graph_to_elk_graph(&graph, &labels, ElkOptions::default());
    serde_json::to_writer(std::io::stdout().lock(), &elk)?;
    Ok(())
}
