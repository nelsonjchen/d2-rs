// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
fn synthetic_root_endpoints_round_trip_without_becoming_objects() {
    let mut graph = Graph::default();
    let mut root = Node::new("", Size::default());
    root.shape = ShapeKind::Class;
    graph.root_object = Some(Box::new(root));
    let edge = graph.add_edge(Edge {
        source: NodeId::ROOT,
        target: NodeId::ROOT,
    });
    graph.edge_arrows[edge.0 as usize].target = true;
    let encoded = serde_json::to_string(&graph).unwrap();
    let restored: Graph = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.nodes().len(), 0);
    assert!(restored.node(NodeId::ROOT).is_none());
    assert_eq!(
        restored.endpoint_node(NodeId::ROOT).unwrap().shape,
        ShapeKind::Class
    );
    assert_eq!(edge_abs_id(&restored, edge).as_deref(), Some("( -> )[0]"));
    assert!(restored.endpoint_node(NodeId(0)).is_none());
}
