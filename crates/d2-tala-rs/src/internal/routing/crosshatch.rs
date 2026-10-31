// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned coordinator.go1822–2062: actual cluster abductions and atomic routes.
use super::channel_solver::minimum;
use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError, RuntimeRouteContext};
use crate::internal::owner_groups::OwnerGroupKind;
use crate::internal::{ArenaGraph, OwnerGraphId, RoutePoints, limits};
use crate::{EdgeId, NodeId, Point, Rect};
type Result<T> = std::result::Result<T, RouteStageError>;

fn rectangular_ray_border(rect: Rect, center: Point, toward: Point) -> Point {
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    if dx == 0.0 && dy == 0.0 {
        return center;
    }
    let mut scale = f64::INFINITY;
    if dx > 0.0 {
        scale = minimum(scale, (rect.right() - center.x) / dx);
    } else if dx < 0.0 {
        scale = minimum(scale, (rect.origin.x - center.x) / dx);
    }
    if dy > 0.0 {
        scale = minimum(scale, (rect.bottom() - center.y) / dy);
    } else if dy < 0.0 {
        scale = minimum(scale, (rect.origin.y - center.y) / dy);
    }
    if scale == f64::INFINITY || scale <= 0.0 {
        return center;
    }
    // Pinned Go ARM64 uses FMADDD for these returned coordinates.
    Point {
        x: dx.mul_add(scale, center.x),
        y: dy.mul_add(scale, center.y),
    }
}
fn descendant(
    g: &ArenaGraph,
    mut current: Option<NodeId>,
    ancestor: NodeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    loop {
        guard.step()?;
        if current == Some(ancestor) {
            return Ok(true);
        }
        let Some(node) = current else {
            return Ok(false);
        };
        if let Some(container) = g.nodes[node.0 as usize].container {
            current = Some(container);
            continue;
        }
        if let Some(vessel) = g
            .owner_node_group_vessel(node, OwnerGroupKind::Cluster)
            .unwrap()
        {
            current = vessel;
            continue;
        }
        if let Some(vessel) = g
            .owner_node_group_vessel(node, OwnerGroupKind::Sequence)
            .unwrap()
        {
            current = vessel;
            continue;
        }
        return Ok(false);
    }
}
fn convert_to_straight_line(
    g: &mut ArenaGraph,
    id: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    guard.step()?;
    let e = &g.edge_records[id.0 as usize];
    let (from, to) = (e.from, e.to);
    let (Some(from_position), Some(to_position)) = (g.position(from), g.position(to)) else {
        return Err(RouteStageError::Invariant(
            "invalid edge while crosshatching routes".into(),
        ));
    };
    let from_rect = Rect {
        origin: from_position,
        size: g.nodes[from.0 as usize].rect.size,
    };
    let to_rect = Rect {
        origin: to_position,
        size: g.nodes[to.0 as usize].rect.size,
    };
    let a = rectangular_ray_border(from_rect, from_rect.center(), to_rect.center());
    let b = rectangular_ray_border(to_rect, to_rect.center(), from_rect.center());
    for node in super::nudge::nodes(g) {
        guard.step()?;
        let from_descendant = descendant(g, Some(from), node, guard)?;
        let to_descendant = descendant(g, Some(to), node, guard)?;
        if from_descendant || to_descendant || node == from || node == to {
            continue;
        }
        if let Some(position) = g.position(node) {
            if ArenaGraph::segment_intersects_box(
                a,
                b,
                position,
                g.nodes[node.0 as usize].rect.size,
            ) {
                return Ok(());
            }
        }
    }
    guard.finish()?;
    g.edge_records[id.0 as usize]
        .route
        .replace(RoutePoints::literal(&[a, b]));
    guard.finish()
}
fn run(g: &mut ArenaGraph, guard: &RouteAtomicGuard<'_>) -> Result<()> {
    let mut vessels = Vec::new();
    let map = g
        .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
        .unwrap();
    for vessel in g.owner_group_map_keys(&map).unwrap() {
        guard.step()?;
        vessels.push(vessel);
    }
    vessels.sort_by_key(|v| v.map_or(0, |n| g.nodes[n.0 as usize].tala_id));
    for vessel in vessels {
        guard.step()?;
        let cluster = g
            .owner_graph_group_entry(OwnerGraphId(0), OwnerGroupKind::Cluster, vessel)
            .unwrap()
            .flatten()
            .ok_or_else(|| {
                RouteStageError::Invariant("nil cluster while crosshatching routes".into())
            })?;
        let mut external_edges = Vec::new();
        if let Some(abductions) = g.owner_cluster_policy(cluster).unwrap().edge_abductions {
            for record in abductions.iter_slots() {
                guard.step()?;
                let edge = record
                    .and_then(|id| g.owner_edge_abduction(id))
                    .and_then(|r| r.source_edge)
                    .ok_or_else(|| {
                        RouteStageError::Invariant(
                            "nil cluster edge abduction while crosshatching routes".into(),
                        )
                    })?;
                external_edges.push(edge);
            }
        }
        if external_edges.len() < 2 {
            continue;
        }
        // Copied point keys use Go value equality, including signed zero.
        let mut ports: Vec<(Point, Vec<EdgeId>)> = Vec::new();
        for edge in external_edges {
            guard.step()?;
            let e = &g.edge_records[edge.0 as usize];
            let mut cluster_port = None;
            for node in g.owner_group_fields(cluster).unwrap().nodes.iter_slots() {
                guard.step()?;
                if node == Some(e.from) {
                    cluster_port = (!e.route.is_empty()).then(|| e.route.slot(0)).flatten();
                    break;
                } else if node == Some(e.to) {
                    cluster_port = e.route.len().checked_sub(1).and_then(|i| e.route.slot(i));
                    break;
                }
            }
            if let Some(port) = cluster_port {
                let value = port.get();
                if let Some((_, edges)) = ports.iter_mut().find(|(p, _)| *p == value) {
                    edges.push(edge);
                } else {
                    ports.push((value, vec![edge]));
                }
            }
        }
        for _ in &ports {
            guard.step()?;
        }
        ports.sort_by(|(a, _), (b, _)| {
            if a.x != b.x {
                a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal)
            }
        });
        for (_, edges) in ports {
            if edges.len() < 2 {
                continue;
            }
            for edge in edges {
                convert_to_straight_line(g, edge, guard)?;
            }
        }
    }
    Ok(())
}
pub(super) fn crosshatch_with_context(
    g: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<()> {
    route_stage_guard::run_atomic_route_stage_typed(
        g,
        Some(OwnerGraphId(0)),
        context,
        "Crosshatch",
        &[],
        budget,
        run,
    )
}
pub(in crate::internal) fn crosshatch_with_budget(
    g: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<()> {
    crosshatch_with_context(g, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn crosshatch(g: &mut ArenaGraph) {
    let _ = crosshatch_with_budget(g, &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS));
}

#[cfg(test)]
mod tests {
    use super::*;
    const MUTATION: &[u8] = br#"{"nodes":[{"id":1,"topLeft":{"x":0,"y":0},"width":100,"height":100},{"id":2,"topLeft":{"x":0,"y":200},"width":100,"height":100},{"id":3,"topLeft":{"x":400,"y":0},"width":100,"height":100},{"id":4,"topLeft":{"x":400,"y":200},"width":100,"height":100}],"edges":[{"id":11,"from":1,"to":3,"points":[{"x":100,"y":100},{"x":400,"y":50}]},{"id":12,"from":2,"to":4,"points":[{"x":100,"y":100},{"x":400,"y":250}]}],"containers":{"0":[1,2,3,4]},"clusters":{"5":{"Nodes":[1,2],"Arrangement":"Row","DesiredArrangement":"Row","Padding":0,"EdgeAbductions":[{"Edge":11,"OriginallyFromNode":1,"CurrentFromNode":1,"CurrentToNode":3},{"Edge":12,"OriginallyFromNode":2,"CurrentFromNode":2,"CurrentToNode":4}]}},"clusterVessels":{"5":{"id":5,"topLeft":{"x":-50,"y":-50},"width":250,"height":400}}}"#;
    fn fixture() -> ArenaGraph {
        ArenaGraph::from_input(&crate::graphjson::graphjson_to_graph(MUTATION).unwrap())
    }
    #[test]
    fn crosshatch_replaces_routes_but_preserves_retained_points() {
        let mut g = fixture();
        let old = g.edge_records[0].route.clone();
        let old_values = old.read();
        let point = old.slot(0).unwrap();
        let work = RouteStageWork::new(100000);
        crosshatch_with_budget(&mut g, &work).unwrap();
        assert_eq!(work.used(), 78);
        assert_eq!(
            g.edge_records[0].route.read(),
            vec![Point { x: 100.0, y: 50.0 }, Point { x: 400.0, y: 50.0 }]
        );
        assert_eq!(g.edge_records[0].route.capacity(), 2);
        assert_eq!(old.read(), old_values);
        assert!(!point.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
    }
    #[test]
    fn crosshatch_uses_explicit_abductions_instead_of_endpoint_membership() {
        let mut g = fixture();
        let group = g
            .owner_node_group_id(NodeId(0), OwnerGroupKind::Cluster)
            .unwrap()
            .unwrap();
        let mut policy = g.owner_cluster_policy(group).unwrap();
        policy.edge_abductions = None;
        g.set_owner_cluster_policy(group, policy).unwrap();
        let before = g.edge_records[0].route.clone();
        let work = RouteStageWork::new(100000);
        crosshatch_with_budget(&mut g, &work).unwrap();
        assert_eq!(work.used(), 38);
        assert!(
            before
                .header()
                .same_backing_allocation(&g.edge_records[0].route.header())
        );
        assert_eq!(g.edge_records[0].route.read(), before.read());
    }
    #[test]
    fn crosshatch_empty_route_has_no_cluster_port() {
        let mut g = fixture();
        g.edge_records[0].route.replace(RoutePoints::literal(&[]));
        let second = g.edge_records[1].route.clone();
        let work = RouteStageWork::new(100000);
        crosshatch_with_budget(&mut g, &work).unwrap();
        assert!(g.edge_records[0].route.is_empty());
        assert_eq!(work.used(), 42);
        assert!(
            second
                .header()
                .same_backing_allocation(&g.edge_records[1].route.header())
        );
    }
    #[test]
    fn crosshatch_every_work_limit_restores_retained_storage() {
        for limit in 0..78 {
            let mut g = fixture();
            let old = g
                .edge_records
                .iter()
                .map(|e| e.route.clone())
                .collect::<Vec<_>>();
            let values = old.iter().map(|h| h.read()).collect::<Vec<_>>();
            let error = crosshatch_with_budget(&mut g, &RouteStageWork::new(limit)).unwrap_err();
            assert!(error.is_route_work_limit(), "limit {limit}: {error}");
            for (i, h) in old.iter().enumerate() {
                assert_eq!(g.edge_records[i].route.read(), values[i]);
                assert!(
                    h.header()
                        .same_backing_allocation(&g.edge_records[i].route.header())
                );
                for j in 0..h.len() {
                    assert!(
                        h.slot(j)
                            .unwrap()
                            .same_allocation(&g.edge_records[i].route.slot(j).unwrap())
                    );
                }
            }
        }
    }
    #[test]
    fn crosshatch_cancel_after_first_replacement_restores_both_headers() {
        struct Cancel(std::cell::Cell<usize>);
        impl limits::Context for Cancel {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                let n = self.0.get() + 1;
                self.0.set(n);
                (n == 17).then_some(limits::ContextError::Cancelled)
            }
        }
        let mut g = fixture();
        let old = g
            .edge_records
            .iter()
            .map(|e| e.route.clone())
            .collect::<Vec<_>>();
        let context = Cancel(std::cell::Cell::new(0));
        let error = crosshatch_with_context(&mut g, Some(&context), &RouteStageWork::new(100000))
            .unwrap_err();
        assert_eq!(error.to_string(), "Crosshatch: context canceled");
        assert_eq!(context.0.get(), 17);
        for (i, h) in old.iter().enumerate() {
            assert_eq!(g.edge_records[i].route.read(), h.read());
            assert!(
                h.header()
                    .same_backing_allocation(&g.edge_records[i].route.header())
            );
        }
    }
}
