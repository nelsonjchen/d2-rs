// SPDX-License-Identifier: MPL-2.0
use super::*;

fn graph() -> ArenaGraph {
    let mut input = Graph::default();
    for name in ["first", "second", "unrelated"] {
        input.add_node(crate::Node::new(
            name,
            Size {
                width: 10.,
                height: 10.,
            },
        ));
    }
    input.add_edge(crate::Edge {
        source: NodeId(0),
        target: NodeId(1),
    });
    let mut graph = ArenaGraph::from_input(&input);
    for (index, x) in [0., 50., 2000.].into_iter().enumerate() {
        graph.set_position(NodeId(index as u32), Point { x, y: 0. });
    }
    // A raw source edge can occur in only one Node.Edges slice. The first
    // receiver then requires 60 units of spacing and the second only 20.
    graph.nodes[1].edges.clear();
    graph
}

#[test]
fn sparse_overlap_keeps_clean_receiver_and_earlier_spacing_operand() {
    let graph = graph();
    let empty = BTreeSet::new();
    let late_moved = BTreeSet::from([NodeId(1)]);
    assert_eq!(
        graph.spacing_delta_with_loops(NodeId(0), NodeId(1), graph.position(NodeId(0)).unwrap()),
        60.
    );
    assert_eq!(
        graph.spacing_delta_with_loops(NodeId(1), NodeId(0), graph.position(NodeId(1)).unwrap()),
        20.
    );
    assert!(graph.transaction_has_new_overlap_for_nodes(&empty, &late_moved));
    assert!(
        graph.transaction_has_bad_state_overlap_for_nodes(&empty, &late_moved),
        "the clean earlier receiver detects the directional collision"
    );
    let prior = BTreeSet::from([(NodeId(0), NodeId(1))]);
    assert!(!graph.transaction_has_new_overlap_for_nodes(&prior, &late_moved));
    assert!(!graph.transaction_has_bad_state_overlap_for_nodes(&prior, &late_moved));
    let unrelated = BTreeSet::from([NodeId(2)]);
    assert!(!graph.transaction_has_new_overlap_for_nodes(&empty, &unrelated));
    assert!(!graph.transaction_has_bad_state_overlap_for_nodes(&empty, &unrelated));
}

#[test]
fn repeated_graph_pointer_preserves_the_first_spacing_operand() {
    let mut graph = graph();
    graph.node_order = vec![NodeId(1), NodeId(0), NodeId(1), NodeId(2), NodeId(0)];
    graph.node_order_membership_valid = false;
    let moved = BTreeSet::from([NodeId(0), NodeId(1)]);
    let empty = BTreeSet::new();
    // The first occurrence of the second node precedes the first node. Its
    // 20-unit spacing does not collide, even though the later reverse does.
    assert!(!graph.transaction_has_new_overlap_for_nodes(&empty, &moved));
    assert!(graph.transaction_has_bad_state_overlap_for_nodes(&empty, &moved));
    graph.set_position(NodeId(0), Point { x: 25., y: 0. });
    assert!(graph.transaction_has_new_overlap_for_nodes(&empty, &moved));
    graph.set_position(NodeId(0), Point { x: f64::NAN, y: 0. });
    assert!(!graph.transaction_has_new_overlap_for_nodes(&empty, &moved));
    assert!(!graph.transaction_has_bad_state_overlap_for_nodes(&empty, &moved));
}
