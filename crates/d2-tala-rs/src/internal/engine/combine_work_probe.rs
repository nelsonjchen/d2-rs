// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Direct positioned-graph controls for the production component packer.
#![cfg(feature = "diagnostic-traces")]
use super::*;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PreparedNode {
    #[serde(rename = "X")]
    x: f64,
    #[serde(rename = "Y")]
    y: f64,
    #[serde(rename = "W")]
    w: f64,
    #[serde(rename = "H")]
    h: f64,
    #[serde(default)]
    fixed: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PreparedEdge {
    from: usize,
    to: usize,
    points: Vec<[f64; 2]>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Request {
    name: String,
    nodes: Vec<PreparedNode>,
    groups: Vec<Vec<u32>>,
    edges: Vec<PreparedEdge>,
    limit: Option<i64>,
    cancel_at_work: Option<i64>,
    #[serde(default)]
    obstacles: Vec<[f64; 4]>,
}
struct Background {
    cancel_at_work: Option<i64>,
    work: std::cell::Cell<i64>,
}
impl limits::Context for Background {
    fn done(&self) -> Option<bool> {
        None
    }
    fn transaction_work_used(&self, work: i64) {
        self.work.set(work);
    }
    fn error(&self) -> Option<limits::ContextError> {
        self.cancel_at_work
            .filter(|limit| self.work.get() >= *limit)
            .map(|_| limits::ContextError::Cancelled)
    }
}
fn bits(point: Point) -> [String; 2] {
    [
        format!("{:016x}", point.x.to_bits()),
        format!("{:016x}", point.y.to_bits()),
    ]
}
fn run(request: Request) -> Value {
    let mut input = Graph::default();
    for (index, node) in request.nodes.iter().enumerate() {
        input.add_node(crate::Node::new(
            index.to_string(),
            Size {
                width: node.w,
                height: node.h,
            },
        ));
    }
    for edge in &request.edges {
        input.add_edge(crate::Edge {
            source: NodeId(edge.from as u32),
            target: NodeId(edge.to as u32),
        });
    }
    let mut graph = ArenaGraph::from_input(&input);
    for (index, node) in request.nodes.iter().enumerate() {
        graph.set_position(
            NodeId(index as u32),
            Point {
                x: node.x,
                y: node.y,
            },
        );
        if node.fixed {
            graph.nodes[index].fixed_top_left = Some(Point {
                x: node.x,
                y: node.y,
            });
        }
    }
    for (index, edge) in request.edges.iter().enumerate() {
        graph.edge_records[index]
            .route
            .replace(RoutePoints::literal(
                &(edge
                    .points
                    .iter()
                    .map(|p| Point { x: p[0], y: p[1] })
                    .collect::<Vec<_>>()),
            ));
    }
    let context = Background {
        cancel_at_work: request.cancel_at_work,
        work: std::cell::Cell::new(0),
    };
    let mut guard = limits::WorkGuard::new(
        Some(&context),
        "CombineSubgraphs",
        request.limit.unwrap_or(limits::MAX_ENGINE_WORK_UNITS),
    )
    .unwrap();
    let obstacles = request
        .obstacles
        .iter()
        .map(|b| Rect {
            origin: Point { x: b[0], y: b[1] },
            size: Size {
                width: b[2],
                height: b[3],
            },
        })
        .collect::<Vec<_>>();
    // The source observer wraps only the main packing guard. Its snapshot
    // helper receives the original background context.
    let snapshot_context = Background {
        cancel_at_work: None,
        work: std::cell::Cell::new(0),
    };
    let result = graph.combine_known_placement_components_with_guard(
        request
            .groups
            .into_iter()
            .map(|g| g.into_iter().map(NodeId).collect())
            .collect(),
        true,
        false,
        true,
        &obstacles,
        &snapshot_context,
        &mut guard,
    );
    let (order, error) = match result {
        Ok(order) => (
            order.into_iter().map(|n| n.0).collect::<Vec<_>>(),
            String::new(),
        ),
        Err(error) => (vec![], error.to_string()),
    };
    json!({"name":request.name,"error":error,"work":guard.used(),"order":order,
        "positions":graph.nodes.iter().map(|n|graph.position(n.input_id).map(bits)).collect::<Vec<_>>(),
        "routes":graph.edge_records.iter().map(|e|e.route.read().iter().copied().map(bits).collect::<Vec<_>>()).collect::<Vec<_>>()})
}
/// Runs prepared geometry through the production component packer, exposing
/// its actual work use, ordered result and complete node/route coordinates.
pub fn diagnostic_combine_work_probe(value: Value) -> Value {
    match serde_json::from_value::<Vec<Request>>(value) {
        Ok(requests) => Value::Array(requests.into_iter().map(run).collect()),
        Err(error) => json!({"probe_error":error.to_string()}),
    }
}
