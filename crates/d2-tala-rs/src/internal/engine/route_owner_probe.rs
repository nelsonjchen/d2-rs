// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![cfg(feature = "diagnostic-traces")]
use super::*;
use serde_json::{Value, json};
use shared_topology::{OwnerNodePoint, OwnerRouteHeader, OwnerRouteSnapshot};
fn bits(point: Option<Point>) -> Value {
    point
        .map(|p| {
            json!([
                format!("{:016x}", p.x.to_bits()),
                format!("{:016x}", p.y.to_bits())
            ])
        })
        .unwrap_or(Value::Null)
}
fn point_record(point: &OwnerNodePoint, identities: &mut Vec<OwnerNodePoint>) -> Value {
    let id = if point.get().is_none() {
        0
    } else {
        match identities.iter().position(|p| p.same_allocation(point)) {
            Some(i) => i + 1,
            None => {
                identities.push(point.clone());
                identities.len()
            }
        }
    };
    json!({"id":id,"value":bits(point.get())})
}
fn observe(
    graph: &ArenaGraph,
    points: &BTreeMap<String, OwnerNodePoint>,
    headers: &BTreeMap<String, OwnerRouteHeader>,
) -> Value {
    let mut identities = Vec::new();
    let point_records = points
        .iter()
        .map(|(name, p)| (name.clone(), point_record(p, &mut identities)))
        .collect::<BTreeMap<_, _>>();
    let nodes = (0..graph.nodes.len())
        .map(|i| {
            point_record(
                &graph.owner_node_point(NodeId(i as u32)).unwrap(),
                &mut identities,
            )
        })
        .collect::<Vec<_>>();
    let mut all = headers.clone();
    for i in 0..graph.edge_records.len() {
        all.insert(
            format!("@edge{i}"),
            graph.owner_edge_route(EdgeId(i as u32)).unwrap(),
        );
    }
    let mut slots = BTreeMap::<usize, usize>::new();
    let mut routes = BTreeMap::new();
    for (name, h) in all {
        let values=(0..h.capacity()).map(|i|{
            let key=h.observed_slot_identity(i).unwrap();let next=slots.len()+1;let id=*slots.entry(key).or_insert(next);
            json!({"slot":id,"point":point_record(&h.capacity_point(i).unwrap(),&mut identities)})
        }).collect::<Vec<_>>();
        routes.insert(
            name,
            json!({"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":values}),
        );
    }
    json!({"points":point_records,"nodes":nodes,"routes":routes})
}
pub(super) fn run(graph: &mut ArenaGraph, name: &str, actions: &Value) -> Value {
    graph.initialize_shared_topology();
    for i in 0..graph.nodes.len() {
        graph
            .materialize_owner_node_point(NodeId(i as u32))
            .unwrap();
    }
    for i in 0..graph.edge_records.len() {
        // layoutgraph.NewEdge initializes Points to a nonnil empty slice.
        let nil = graph.allocate_owner_route_header(&[], 0, true).unwrap();
        graph.bind_owner_edge_route(EdgeId(i as u32), nil).unwrap();
    }
    let mut points = BTreeMap::<String, OwnerNodePoint>::new();
    let mut headers = BTreeMap::<String, OwnerRouteHeader>::new();
    let mut snapshots = BTreeMap::<String, OwnerRouteSnapshot>::new();
    let mut trace = vec![observe(graph, &points, &headers)];
    for action in actions.as_array().unwrap() {
        let string = |key: &str| action[key].as_str().unwrap();
        let number = |key: &str| action[key].as_u64().unwrap() as usize;
        let get_points = |graph: &mut ArenaGraph, points: &BTreeMap<String, OwnerNodePoint>| {
            action["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    if p.is_null() {
                        graph.allocate_owner_node_point(None)
                    } else {
                        points[p.as_str().unwrap()].clone()
                    }
                })
                .collect::<Vec<_>>()
        };
        match string("op") {
            "point" => {
                let value = action["value"].as_array().map(|v| Point {
                    x: v[0].as_f64().unwrap(),
                    y: v[1].as_f64().unwrap(),
                });
                points.insert(
                    string("name").into(),
                    graph.allocate_owner_node_point(value),
                );
            }
            "node" => {
                graph
                    .bind_owner_node_point(
                        NodeId(number("node") as u32),
                        points[string("point")].clone(),
                    )
                    .unwrap();
            }
            "make" => {
                let ps = get_points(graph, &points);
                let h = graph
                    .allocate_owner_route_header(
                        &ps,
                        number("cap"),
                        action["present"].as_bool().unwrap(),
                    )
                    .unwrap();
                headers.insert(string("name").into(), h);
            }
            "append" => {
                let ps = get_points(graph, &points);
                let h = graph
                    .append_owner_route(&headers[string("from")], &ps)
                    .unwrap();
                headers.insert(string("name").into(), h);
            }
            "slice" => {
                let h = headers[string("from")]
                    .reslice(number("low"), number("high"), number("max"))
                    .unwrap();
                headers.insert(string("name").into(), h);
            }
            "slot" => {
                let p = if action["point"].is_null() {
                    graph.allocate_owner_node_point(None)
                } else {
                    points[string("point")].clone()
                };
                graph
                    .write_owner_route_slot(&headers[string("header")], number("index"), &p)
                    .unwrap();
            }
            "keep" => {
                let p = headers[string("header")]
                    .capacity_point(number("index"))
                    .unwrap();
                points.insert(string("name").into(), p);
            }
            "write" => {
                let v = action["value"].as_array().unwrap();
                graph
                    .write_owner_point(
                        &points[string("point")],
                        Point {
                            x: v[0].as_f64().unwrap(),
                            y: v[1].as_f64().unwrap(),
                        },
                    )
                    .unwrap();
            }
            "bind" => {
                graph
                    .bind_owner_edge_route(
                        EdgeId(number("edge") as u32),
                        headers[string("header")].clone(),
                    )
                    .unwrap();
            }
            "snapshot" => {
                snapshots.insert(
                    string("name").into(),
                    graph
                        .snapshot_owner_edge_route(EdgeId(number("edge") as u32))
                        .unwrap(),
                );
            }
            "restore" => {
                graph
                    .restore_owner_edge_route(
                        EdgeId(number("edge") as u32),
                        &snapshots[string("name")],
                    )
                    .unwrap();
            }
            _ => panic!("unknown route operation"),
        }
        trace.push(observe(graph, &points, &headers));
    }
    json!({"name":name,"trace":trace})
}
