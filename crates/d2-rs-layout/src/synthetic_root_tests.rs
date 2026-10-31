// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
fn empty_object_root_connections_bypass_compiled_engine_and_context() {
    let mut graph = Graph::default();
    let mut root = d2_rs_core::Node::new("", d2_rs_core::Size::default());
    root.shape = d2_rs_core::ShapeKind::SqlTable;
    graph.root_object = Some(Box::new(root));
    graph.add_edge(d2_rs_core::Edge {
        source: NodeId::ROOT,
        target: NodeId::ROOT,
    });
    graph.tala_seed_data = Some(Err("invalid source seed data".into()));
    let token = CancellationToken::new();
    token.cancel();
    let options = LayoutOptions {
        seeds: vec![],
        max_concurrency: usize::MAX,
        ..Default::default()
    };
    for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
        let result = LayoutDispatcher
            .layout_compiled_with_cancellation(&graph, engine, &options, Some(&token))
            .unwrap();
        assert!(result.boxes.is_empty());
        assert!(result.routes.is_empty());
    }
}
