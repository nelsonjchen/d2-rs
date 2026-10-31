use d2_rs_core::{Direction, Edge, Graph, Node, Size};

fn main() {
    let mut graph = Graph::with_direction(Direction::Right);
    let a = graph.add_node(Node::new(
        "A",
        Size {
            width: 56.0,
            height: 80.0,
        },
    ));
    let b = graph.add_node(Node::new(
        "B",
        Size {
            width: 54.0,
            height: 66.0,
        },
    ));
    let c = graph.add_node(Node::new(
        "C",
        Size {
            width: 54.0,
            height: 66.0,
        },
    ));
    let d = graph.add_node(Node::new(
        "D",
        Size {
            width: 56.0,
            height: 80.0,
        },
    ));
    for (source, target) in [(a, c), (a, d), (b, c), (b, d)] {
        graph.add_edge(Edge { source, target });
    }
    let elk =
        d2_rs_elk::graph_to_elk_graph(&graph, &graph.node_labels, d2_rs_elk::ElkOptions::default());
    println!("{}", serde_json::to_string_pretty(&elk).unwrap());
}
