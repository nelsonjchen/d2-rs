// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned ovg_hierarchy.go: construct each hierarchy before ordinary OVG ports.
use super::channel_solver::{maximum, minimum};
use super::ovg_intersections::{AxisMap, Budget, check, step};
use super::ovg_ports::PortBuild;
use super::{ArenaGraph, NodeId, Point, PortSide};
use crate::internal::shared_topology::OwnerHierarchy;
use std::collections::BTreeMap;

pub(super) struct HierarchyOvg {
    pub(super) nodes: Vec<NodeId>,
    pub(super) ports: PortBuild,
}
fn work(budget: Budget<'_, '_>, count: usize) -> Result<(), ()> {
    budget.map_or(Ok(()), |b| b.reserve_work(count as u64))
}
fn sort_work(budget: Budget<'_, '_>, count: usize) -> Result<(), ()> {
    budget.map_or(Ok(()), |b| b.reserve_sort_work(count))
}
fn levels(graph: &ArenaGraph, hierarchy: &OwnerHierarchy) -> Vec<(Option<NodeId>, i64)> {
    graph
        .owner_hierarchy_fields(hierarchy)
        .expect("live hierarchy record")
        .levels
        .entries()
        .into_iter()
        .map(|(key, level)| {
            let key = key.map(|canonical| {
                graph
                    .nodes
                    .iter()
                    .find(|node| {
                        node.hierarchy.as_ref().is_some_and(|h| {
                            h.record.same_allocation(hierarchy) && h.node == canonical
                        })
                    })
                    .map_or(canonical, |node| node.input_id)
            });
            (key, level)
        })
        .collect()
}
fn node(key: Option<NodeId>) -> NodeId {
    key.expect("runtime error: invalid memory address or nil pointer dereference")
}
fn direction(graph: &ArenaGraph, id: NodeId) -> Option<crate::Direction> {
    graph
        .directions
        .get(&graph.nodes[id.0 as usize].container)
        .copied()
}
fn write_point(graph: &mut ArenaGraph, id: NodeId, point: Point) {
    // Node.Mirror/Transpose mutate the existing Point; set_position replaces it.
    if let Ok(header) = graph.owner_node_point(id) {
        graph.write_owner_point(&header, point).unwrap();
    } else {
        graph.nodes[id.0 as usize].position = Some(point);
    }
    graph.nodes[id.0 as usize].rect.origin = point;
}
fn mirror_nodes(graph: &mut ArenaGraph, nodes: &[NodeId], x: bool, y: bool) {
    for &id in nodes {
        let n = &graph.nodes[id.0 as usize];
        let mut p = graph
            .position(id)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let offset = |side: i64| {
            n.loop_offsets
                .as_ref()
                .and_then(|o| o.get(&side))
                .copied()
                .unwrap_or(0.0)
        };
        if x {
            p.x = -p.x - n.rect.size.width + offset(7) - offset(5);
        }
        if y {
            p.y = -p.y - n.rect.size.height + offset(4) - offset(6);
        }
        write_point(graph, id, p);
    }
}
fn transpose_nodes(graph: &mut ArenaGraph, nodes: &[NodeId]) {
    for &id in nodes {
        let n = &mut graph.nodes[id.0 as usize];
        std::mem::swap(&mut n.rect.size.width, &mut n.rect.size.height);
        let mut p = graph
            .position(id)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        std::mem::swap(&mut p.x, &mut p.y);
        write_point(graph, id, p);
        let n = &mut graph.nodes[id.0 as usize];
        std::mem::swap(&mut n.node_padding.left, &mut n.node_padding.top);
        std::mem::swap(&mut n.node_padding.right, &mut n.node_padding.bottom);
        std::mem::swap(&mut n.layout_margins.left, &mut n.layout_margins.top);
        std::mem::swap(&mut n.layout_margins.right, &mut n.layout_margins.bottom);
    }
}
fn transform_work(ports: &PortBuild, count: usize, budget: Budget<'_, '_>) -> Result<(), ()> {
    work(budget, count)?;
    for &id in &ports.members {
        step(budget)?;
        work(budget, 2)?;
        for _ in ports.owner_directions(id) {
            step(budget)?;
            work(budget, 1)?;
        }
    }
    check(budget)
}
pub(super) fn build(
    graph: &mut ArenaGraph,
    hierarchy: &OwnerHierarchy,
    budget: Budget<'_, '_>,
) -> Result<HierarchyOvg, ()> {
    check(budget)?;
    let mut nodes = Vec::new();
    for (key, _) in levels(graph, hierarchy) {
        step(budget)?;
        nodes.push(key);
    }
    sort_work(budget, nodes.len())?;
    nodes.sort_by_key(|key| graph.nodes[node(*key).0 as usize].tala_id);
    check(budget)?;
    let nodes = nodes.into_iter().map(node).collect::<Vec<_>>();
    let mut ports = PortBuild::default();
    ports.add_ports(graph, &nodes, budget)?;
    let (mut mirror_x, mut mirror_y, mut transposed) = (false, false, false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match direction(graph, nodes[0]) {
            Some(crate::Direction::Left) => {
                transform_work(&ports, nodes.len(), budget)?;
                mirror_nodes(graph, &nodes, true, false);
                ports.transform_hierarchy(false, true, false);
                mirror_x = true;
                check(budget)?;
            }
            Some(crate::Direction::Up) => {
                transform_work(&ports, nodes.len(), budget)?;
                mirror_nodes(graph, &nodes, false, true);
                ports.transform_hierarchy(false, false, true);
                mirror_y = true;
                check(budget)?;
            }
            _ => {}
        }
        work(budget, nodes.len())?;
        let horizontal = matches!(
            direction(graph, nodes[0]),
            Some(crate::Direction::Left | crate::Direction::Right)
        ) || nodes
            .iter()
            .any(|id| graph.nodes[id.0 as usize].shape == crate::ShapeKind::SqlTable);
        if horizontal {
            transform_work(&ports, nodes.len(), budget)?;
            ports.transform_hierarchy(true, false, false);
            transpose_nodes(graph, &nodes);
            transposed = true;
            check(budget)?;
        }
        let graph_nodes = graph.graph_node_order();
        ports.add_nodes_intersections(graph, &graph_nodes, budget)?;
        add_level_nodes(&mut ports, graph, hierarchy, budget)?;
        let bounds = bounds(&ports, budget)?;
        ports.add_boundary_layers(graph, &graph_nodes, bounds, budget)?;
        ports.add_corners(graph, &graph_nodes, bounds, budget)?;
        check(budget)
    }));
    // Source installs inverse defers only after each forward transform succeeds.
    if transposed {
        ports.transform_hierarchy(true, false, false);
        transpose_nodes(graph, &nodes);
    }
    if mirror_x || mirror_y {
        mirror_nodes(graph, &nodes, mirror_x, mirror_y);
        ports.transform_hierarchy(false, mirror_x, mirror_y);
    }
    match result {
        Ok(result) => result?,
        Err(payload) => std::panic::resume_unwind(payload),
    }
    Ok(HierarchyOvg { nodes, ports })
}
fn bounds(ports: &PortBuild, budget: Budget<'_, '_>) -> Result<super::ovg_boundaries::Bounds, ()> {
    let Some(&first) = ports.members.first() else {
        check(budget)?;
        return Ok(Some((
            Point {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
            },
            Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            },
        )));
    };
    let mut lo = ports.nodes[first].point();
    let mut hi = lo;
    for n in ports.member_nodes() {
        step(budget)?;
        let p = n.point();
        lo.x = minimum(lo.x, p.x);
        lo.y = minimum(lo.y, p.y);
        hi.x = maximum(hi.x, p.x);
        hi.y = maximum(hi.y, p.y);
    }
    check(budget)?;
    Ok(Some((lo, hi)))
}
fn add_level_nodes(
    ports: &mut PortBuild,
    graph: &ArenaGraph,
    hierarchy: &OwnerHierarchy,
    budget: Budget<'_, '_>,
) -> Result<(), ()> {
    check(budget)?;
    let mut by_level = BTreeMap::<i64, Vec<NodeId>>::new();
    for (key, level) in levels(graph, hierarchy) {
        step(budget)?;
        by_level.entry(level).or_default().push(node(key));
    }
    for level in 0..by_level.len() as i64 {
        step(budget)?;
        let Some(nodes) = by_level.get_mut(&level) else {
            sort_work(budget, 0)?;
            continue;
        };
        sort_work(budget, nodes.len())?;
        crate::internal::go_sort::sort_by(nodes, |a, b| {
            graph.position(*a).unwrap().x < graph.position(*b).unwrap().x
        });
    }
    let (mut lo, mut hi) = bounds(ports, budget)?.unwrap();
    lo.x -= 20.0;
    lo.x = (lo.x / 20.0).floor() * 20.0;
    let width = ((hi.x - lo.x) / 20.0).ceil() * 20.0;
    hi.x = lo.x + width + 20.0;
    let mut ys = AxisMap::<()>::default();
    for level in 1..by_level.len() as i64 {
        step(budget)?;
        let above = by_level
            .get(&(level - 1))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let below = by_level.get(&level).map(Vec::as_slice).unwrap_or_default();
        let level_ys = boundary_above(ports, graph, above, below, lo.x, hi.x, budget)?;
        aligned(ports, above, below, &level_ys, budget)?;
        for y in level_ys {
            ys.entry(y);
        }
    }
    let xs = sibling_lanes(ports, graph, &by_level, lo.x, hi.x, budget)?;
    ports.add_intersections(graph, &graph.graph_node_order(), xs, ys, budget)
}
fn boundary_above(
    ports: &mut PortBuild,
    graph: &ArenaGraph,
    above: &[NodeId],
    below: &[NodeId],
    left: f64,
    right: f64,
    budget: Budget<'_, '_>,
) -> Result<Vec<f64>, ()> {
    check(budget)?;
    let mut min_y = f64::INFINITY;
    let mut edges_above = 1.0;
    for &id in below {
        step(budget)?;
        min_y = minimum(min_y, graph.position(id).unwrap().y);
        for edge in graph.nodes[id.0 as usize].edges.header().iter_slots() {
            step(budget)?;
            let e = &graph.edge_records[edge
                .expect("runtime error: invalid memory address or nil pointer dereference")
                .0 as usize];
            let adjacent = if e.from == id { e.to } else { e.from };
            if e.from == e.to || graph.nodes[adjacent.0 as usize].hierarchy.is_none() {
                continue;
            }
            let level = |id: NodeId| {
                graph.nodes[id.0 as usize]
                    .hierarchy
                    .as_ref()
                    .map_or(0, |h| h.level())
            };
            if level(adjacent) < level(id) {
                edges_above += 1.0;
            }
        }
    }
    let mut max_y = f64::NEG_INFINITY;
    for &id in above {
        step(budget)?;
        let n = &graph.nodes[id.0 as usize];
        max_y = maximum(max_y, graph.position(id).unwrap().y + n.rect.size.height);
    }
    let distance = min_y - max_y;
    let count = minimum(edges_above, distance / 20.0);
    let pad = maximum((distance / count).ceil(), 20.0);
    let mut ys = Vec::new();
    let mut y = min_y;
    for _ in 0..(count as i64).wrapping_sub(1) {
        step(budget)?;
        y -= pad;
        ys.push(y);
        ports.add_point(Point { x: left, y }, budget)?;
        ports.add_point(Point { x: right, y }, budget)?;
    }
    check(budget)?;
    Ok(ys)
}
fn aligned(
    ports: &mut PortBuild,
    above: &[NodeId],
    below: &[NodeId],
    ys: &[f64],
    budget: Budget<'_, '_>,
) -> Result<(), ()> {
    check(budget)?;
    for &y in ys {
        step(budget)?;
        for (nodes, side) in [(above, PortSide::Bottom), (below, PortSide::Top)] {
            for &node in nodes {
                step(budget)?;
                for index in ports.ports_by_orientation(node, side, budget)? {
                    ports.add_point(
                        Point {
                            x: ports.nodes[index].point().x,
                            y,
                        },
                        budget,
                    )?;
                }
            }
        }
    }
    check(budget)
}
fn sibling_lanes(
    ports: &mut PortBuild,
    graph: &ArenaGraph,
    levels: &BTreeMap<i64, Vec<NodeId>>,
    left: f64,
    right: f64,
    budget: Budget<'_, '_>,
) -> Result<AxisMap<()>, ()> {
    check(budget)?;
    let mut xs = AxisMap::<()>::default();
    for level in 0..levels.len() as i64 {
        step(budget)?;
        let nodes = levels.get(&level).map(Vec::as_slice).unwrap_or_default();
        let mut last = false;
        let mut i = 0;
        let mut id = nodes[i];
        let mut selected = ports.ports_by_orientation(id, PortSide::Left, budget)?;
        let mut x = left;
        while x <= right {
            step(budget)?;
            xs.entry(x);
            for &port in &selected {
                ports.add_point(
                    Point {
                        x,
                        y: ports.nodes[port].point().y,
                    },
                    budget,
                )?;
            }
            let rect = super::ovg_intersections::rect(graph, id);
            if !last && x >= rect.origin.x - 20.0 {
                x = ((rect.origin.x + rect.size.width) / 20.0).ceil() * 20.0;
                selected = ports.ports_by_orientation(id, PortSide::Right, budget)?;
                last = i == nodes.len() - 1;
                if !last {
                    i += 1;
                    id = nodes[i];
                    selected.extend(ports.ports_by_orientation(id, PortSide::Left, budget)?);
                }
            }
            x += 20.0;
        }
    }
    check(budget)?;
    Ok(xs)
}

#[cfg(feature = "diagnostic-traces")]
pub(in crate::internal) fn diagnostic(
    value: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use super::route_search_guard::OvgBuildBudget;
    use crate::internal::{
        limits::{Context, ContextError},
        owner_points::SharedOwnerPoint,
    };
    use serde_json::{Value, json};
    struct Caller<'a> {
        config: &'a Value,
        point: SharedOwnerPoint,
        polls: std::cell::Cell<u64>,
        fired: std::cell::Cell<bool>,
    }
    impl Context for Caller<'_> {
        fn done(&self) -> Option<bool> {
            match self.config["context"].as_str() {
                Some("open") => Some(false),
                Some("closed") => Some(true),
                _ => None,
            }
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            let mut point = self.point.get();
            if self.config["negative"].as_bool().unwrap_or(false)
                && !self.fired.get()
                && point.x < 0.0
            {
                self.fired.set(true);
                point.x += 7.0;
                self.point.set(point);
                if self.config["panic"].as_bool().unwrap_or(false) {
                    panic!("hierarchy transformed context panic");
                }
                return Some(ContextError::Cancelled);
            }
            if self.config["mutate_at"].as_u64() == Some(poll) {
                point.x += 7.0;
                self.point.set(point);
            }
            if self.config["panic_at"]
                .as_u64()
                .is_some_and(|at| at != 0 && poll >= at)
            {
                panic!("hierarchy context panic");
            }
            if self.config["cancel_at"]
                .as_u64()
                .is_some_and(|at| at != 0 && poll >= at)
            {
                return Some(if self.config["deadline"].as_bool().unwrap_or(false) {
                    ContextError::DeadlineExceeded
                } else {
                    ContextError::Cancelled
                });
            }
            None
        }
    }
    fn observe(graph: &ArenaGraph) -> Value {
        let mut ids = BTreeMap::new();
        json!(graph.graph_node_order().iter().map(|&id|{
            let node=&graph.nodes[id.0 as usize];let point=graph.owner_node_point(id).unwrap().shared_point();
            let next=ids.len()+1;let identity=point.as_ref().map_or(0,|p|*ids.entry(p.allocation_key()).or_insert(next));
            json!({"id":node.tala_id,"point_id":identity,"point":point.map(|p|{let p=p.get();[format!("{:016x}",p.x.to_bits()),format!("{:016x}",p.y.to_bits())]}),"size":[format!("{:016x}",node.rect.size.width.to_bits()),format!("{:016x}",node.rect.size.height.to_bits())]})
        }).collect::<Vec<_>>())
    }
    let mut output = Vec::new();
    for row in value.as_array().ok_or("expected hierarchy requests")? {
        let input = crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&row["graph"])?)?;
        let mut graph = ArenaGraph::from_input(&input);
        for id in graph.graph_node_order() {
            graph.nodes[id.0 as usize].position = input.initial_positions[id.0 as usize];
            graph.materialize_owner_node_point(id).unwrap();
        }
        let config = &row["hierarchy_ovg"];
        let before = observe(&graph);
        let caller = Caller {
            config,
            point: graph
                .owner_node_point(NodeId(0))
                .unwrap()
                .shared_point()
                .unwrap(),
            polls: std::cell::Cell::new(0),
            fired: std::cell::Cell::new(false),
        };
        let runtime = super::route_stage_guard::RuntimeRouteContext;
        let context: &dyn Context = if config["context"].as_str().is_some_and(|c| !c.is_empty())
            || config["negative"].as_bool().unwrap_or(false)
        {
            &caller
        } else {
            &runtime
        };
        let budget = OvgBuildBudget::with_context(context);
        let mut initialized = false;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            budget.check()?;
            initialized = true;
            let hierarchy = graph.nodes[0]
                .hierarchy
                .as_ref()
                .expect("runtime error: invalid memory address or nil pointer dereference")
                .record
                .clone();
            build(&mut graph, &hierarchy, Some(&budget))
        }));
        let mut error = None;
        let mut panic = None;
        let mut ovg = Value::Null;
        match outcome {
            Ok(Ok(h)) => ovg = h.ports.observe_lifecycle_stage(&graph, true),
            Ok(Err(())) => {
                error = Some(
                    budget
                        .take_context_error()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "TALA OVG resource limit exceeded".into()),
                )
            }
            Err(payload) => {
                panic = Some(
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "non-string panic".into()),
                )
            }
        }
        let resources = initialized.then(|| {
            let [work, nodes, edges, candidates] = budget.observed_resources();
            json!({"work":work,"nodes":nodes,"edges":edges,"candidates":candidates})
        });
        output.push(json!({"name":row["name"],"before":before,"after":observe(&graph),"resources":resources,"polls":caller.polls.get(),"error":error,"panic":panic,"ovg":ovg}));
    }
    Ok(json!(output))
}

#[cfg(test)]
mod tests {
    use super::super::route_search_guard::OvgBuildBudget;
    use super::*;
    use crate::internal::{
        limits::{Context, ContextError},
        owner_points::SharedOwnerPoint,
    };
    struct Interrupt {
        point: SharedOwnerPoint,
        panic: bool,
        fired: std::cell::Cell<bool>,
    }
    impl Context for Interrupt {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let mut point = self.point.get();
            if point.x >= 0.0 || self.fired.replace(true) {
                return None;
            }
            point.x += 7.0;
            self.point.set(point);
            if self.panic {
                panic!("hierarchy transformed context panic");
            }
            Some(ContextError::Cancelled)
        }
    }
    #[test]
    fn hierarchy_inverse_preserves_callback_mutation_on_error_and_panic() {
        for panic in [false, true] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "nodes":[{"id":1,"width":40,"height":60,"topLeft":{"x":0,"y":0}},
                    {"id":2,"width":40,"height":60,"topLeft":{"x":-160,"y":0}}],
                "edges":[{"id":11,"from":1,"to":2,"points":[]}],
                "containers":{"0":[1,2]},"directions":{"0":"left"},
                "hierarchies":[{"Levels":{"1":0,"2":1}}]
            }))
            .unwrap();
            let input = crate::graphjson::graphjson_to_graph(&bytes).unwrap();
            let mut graph = ArenaGraph::from_input(&input);
            graph.materialize_owner_node_point(NodeId(0)).unwrap();
            let point = graph
                .owner_node_point(NodeId(0))
                .unwrap()
                .shared_point()
                .unwrap();
            let context = Interrupt {
                point: point.clone(),
                panic,
                fired: std::cell::Cell::new(false),
            };
            let budget = OvgBuildBudget::with_context(&context);
            let hierarchy = graph.nodes[0].hierarchy.as_ref().unwrap().record.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                build(&mut graph, &hierarchy, Some(&budget))
            }));
            assert!(context.fired.get());
            if panic {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert_eq!(point.get(), Point { x: -7.0, y: 0.0 });
            assert_eq!(graph.position(NodeId(0)).unwrap(), point.get());
            assert_eq!(
                graph.position(NodeId(1)).unwrap(),
                Point { x: -160.0, y: 0.0 }
            );
            assert_eq!(
                graph.nodes[0].rect.size,
                crate::Size {
                    width: 40.0,
                    height: 60.0
                }
            );
            assert!(
                graph
                    .owner_node_point(NodeId(0))
                    .unwrap()
                    .shared_point()
                    .unwrap()
                    .same_allocation(&point)
            );
        }
    }
}
