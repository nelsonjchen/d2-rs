// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Source-order staging, route capture and seeding from packGuarded.
use super::binpack_traversal::PackingNodeRef;
use super::*;

pub(super) struct PackingSavedPosition {
    pub(super) node: PackingNodeRef,
    pub(super) value: Option<Point>,
    pub(super) point: Option<shared_topology::OwnerNodePoint>,
    snapshot: Option<shared_topology::OwnerNodePointSnapshot>,
}
#[derive(Default)]
pub(super) struct PackingStaging {
    pub(super) positions: Vec<PackingSavedPosition>,
    pub(super) routes: Vec<(EdgeId, Vec<Point>)>,
    position_index: BTreeMap<PackingNodeRef, usize>,
    route_index: BTreeSet<EdgeId>,
}
impl PackingStaging {
    pub(super) fn position_value(&self, node: PackingNodeRef) -> Point {
        self.position_index
            .get(&node)
            .and_then(|&i| self.positions[i].value)
            .unwrap_or_default()
    }
    pub(super) fn record(&mut self, graph: &ArenaGraph, node: PackingNodeRef) {
        if self.position_index.contains_key(&node) {
            return;
        }
        let point = match node {
            PackingNodeRef::Node(n) => graph.owner_node_point(n).ok(),
            _ => None,
        };
        self.position_index.insert(node, self.positions.len());
        self.positions.push(PackingSavedPosition {
            node,
            value: graph.packing_bounds_geometry(node).0,
            snapshot: point.as_ref().map(|p| p.snapshot()),
            point,
        });
    }
    pub(super) fn restore_positions_checked(
        &self,
        graph: &mut ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        for saved in &self.positions {
            guard.step()?;
            if let (PackingNodeRef::Node(node), Some(snapshot)) = (saved.node, &saved.snapshot) {
                graph
                    .restore_owner_node_point(node, snapshot)
                    .expect("packing snapshot belongs to this arena");
                if let Some(value) = saved.value {
                    graph.packing_publish_position(graph.nodes[node.0 as usize].tala_id, value);
                }
            } else {
                graph.packing_set_ref_position(saved.node, saved.value);
            }
        }
        Ok(())
    }
    pub(super) fn restore_routes_checked(
        &self,
        graph: &mut ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        for (edge, route) in &self.routes {
            guard.step()?;
            for (i, point) in route.iter().enumerate() {
                guard.step()?;
                *graph.edge_records[edge.0 as usize].route.point_mut(i) = *point;
            }
        }
        Ok(())
    }
}
impl ArenaGraph {
    /// Go writes root.TopLeft fields at the post-wrap caller boundary. Keep
    /// the allocation, and assign absolute coordinates without delta rounding.
    pub(super) fn packing_write_root_position(&mut self, node: NodeId, value: Point) {
        if self.owner_bound_point_value(node).is_some() {
            let point = self
                .owner_node_point(node)
                .expect("root point belongs to this arena");
            self.write_owner_point(&point, value)
                .expect("root point belongs to this arena");
        } else {
            *self.nodes[node.0 as usize]
                .position
                .as_mut()
                .expect("runtime error: invalid memory address or nil pointer dereference") = value;
        }
        self.nodes[node.0 as usize].rect.origin = value;
        self.nodes[node.0 as usize].unpositioned_scope_translation = None;
    }
    // The compact numerical layer still reads vessel coordinate views. Publish
    // those views from the vessel identity, never from its first member's move.
    fn packing_publish_position(&mut self, id: i64, value: Point) {
        self.set_projected_node_position(id, value);
        if let Some(index) = self.clusters.iter().position(|c| c.vessel_tala_id == id) {
            self.pending_cluster_vessel_positions.insert(index, value);
            self.serialized_vessel_positions_by_tala.insert(id, value);
        }
        if self.sequences.iter().any(|s| s.vessel_tala_id == id) {
            self.serialized_vessel_positions_by_tala.insert(id, value);
        }
    }
    pub(super) fn packing_set_ref_position(&mut self, node: PackingNodeRef, value: Option<Point>) {
        match node {
            PackingNodeRef::Node(n) => {
                if let Some(value) = value {
                    self.set_position(n, value);
                    self.packing_publish_position(self.nodes[n.0 as usize].tala_id, value);
                } else {
                    self.clear_position(n);
                }
            }
            PackingNodeRef::Cluster(i) => {
                let id = self.clusters[i].vessel_tala_id;
                if let Some(value) = value {
                    self.pending_cluster_vessel_positions.insert(i, value);
                    self.serialized_vessel_positions_by_tala.insert(id, value);
                } else {
                    self.pending_cluster_vessel_positions.remove(&i);
                    self.serialized_vessel_positions_by_tala.remove(&id);
                }
            }
            PackingNodeRef::Sequence(i) => {
                let id = self.sequences[i].vessel_tala_id;
                if let Some(value) = value {
                    self.serialized_vessel_positions_by_tala.insert(id, value);
                } else {
                    self.serialized_vessel_positions_by_tala.remove(&id);
                }
            }
        }
    }
    pub(super) fn packing_translate_ref(&mut self, node: PackingNodeRef, delta: Point) {
        match node {
            PackingNodeRef::Node(n) => {
                // Node.Translate mutates its existing coordinate allocation.
                self.translate_owner_point(n, delta.x, delta.y);
                self.packing_publish_position(
                    self.nodes[n.0 as usize].tala_id,
                    self.position(n).unwrap(),
                );
            }
            _ => {
                let position = self
                    .packing_bounds_geometry(node)
                    .0
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                self.packing_set_ref_position(
                    node,
                    Some(Point {
                        x: position.x + delta.x,
                        y: position.y + delta.y,
                    }),
                );
            }
        }
    }
    pub(super) fn packing_translate_refs_checked(
        &mut self,
        nodes: &[PackingNodeRef],
        delta: Point,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        for &node in nodes {
            guard.step()?;
            self.packing_translate_ref(node, delta);
        }
        Ok(())
    }
    pub(super) fn packing_move_routes_checked(
        &mut self,
        nodes: &[PackingNodeRef],
        saved: &PackingStaging,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let mut moved = BTreeSet::new();
        for &node in nodes {
            guard.step()?;
            for edge in self.packing_incidents(node) {
                guard.step()?;
                if !moved.insert(edge) {
                    continue;
                }
                for i in 0..self.edge_records[edge.0 as usize].route.len() {
                    guard.step()?;
                    // Source reads the owning node only after visiting the point.
                    let current = self
                        .packing_bounds_geometry(node)
                        .0
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    let original = saved.position_value(node);
                    let mut point = self.edge_records[edge.0 as usize].route.point_mut(i);
                    point.x += current.x - original.x;
                    point.y += current.y - original.y;
                }
            }
        }
        Ok(())
    }
    pub(super) fn packing_stage_checked(
        &mut self,
        groups: &[Vec<PackingNodeRef>],
        seed_first: bool,
        root_top_left: Point,
        edges_placed: bool,
        saved: &mut PackingStaging,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        // Compact vessels need an independent live coordinate before a member
        // moves. This materializes the existing representation, not a snapshot
        // of future per-node values: aliased concrete nodes are still captured
        // immediately before each source visit below.
        let mut vessels = BTreeMap::new();
        for &node in groups.iter().flatten() {
            if !matches!(node, PackingNodeRef::Node(_)) {
                vessels
                    .entry(node)
                    .or_insert_with(|| self.packing_bounds_geometry(node).0);
            }
        }
        for (node, point) in vessels {
            self.packing_set_ref_position(node, point);
        }
        for group in groups {
            guard.step()?;
            for &node in group {
                guard.step()?;
                saved.record(self, node);
                self.packing_translate_ref(
                    node,
                    Point {
                        x: 100_000.0,
                        y: 100_000.0,
                    },
                );
                if edges_placed {
                    for edge in self.packing_incidents(node) {
                        guard.step()?;
                        if saved.route_index.contains(&edge) {
                            continue;
                        }
                        let mut route = Vec::new();
                        for &point in &self.edge_records[edge.0 as usize].route.read() {
                            guard.step()?;
                            route.push(point);
                        }
                        // An interrupted copy does not publish a partial saved route.
                        saved.routes.push((edge, route));
                        saved.route_index.insert(edge);
                    }
                }
            }
        }
        if seed_first {
            let first = &groups[0];
            let nodes = first.iter().copied().map(Some).collect::<Vec<_>>();
            let (low, _) = self
                .packing_bounds_checked(&nodes, guard)
                .map_err(|failure| failure.error)?;
            self.packing_translate_refs_checked(
                first,
                Point {
                    x: root_top_left.x - low.x,
                    y: root_top_left.y - low.y,
                },
                guard,
            )?;
            if edges_placed {
                self.packing_move_routes_checked(first, saved, guard)?;
            }
        }
        guard.step()
    }
}

#[cfg(test)]
mod tests {
    use super::super::binpack_guard::PackingContext;
    use super::*;
    fn graph() -> ArenaGraph {
        let mut input = Graph::default();
        for name in ["a", "b"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ));
        }
        input.add_edge(crate::Edge {
            source: NodeId(0),
            target: NodeId(1),
        });
        let mut g = ArenaGraph::from_input(&input);
        g.initialize_shared_topology();
        g.set_position(NodeId(0), Point { x: 1.0, y: 2.0 });
        g.set_position(NodeId(1), Point { x: 11.0, y: 12.0 });
        g.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 3.0, y: 4.0 },
            Point { x: 13.0, y: 14.0 },
        ]));
        g
    }
    #[test]
    fn snapshots_are_interleaved_with_alias_writes_and_duplicate_visits() {
        let mut g = graph();
        let pointer = g.allocate_owner_node_point(Some(Point { x: 1.0, y: 2.0 }));
        for n in [NodeId(0), NodeId(1)] {
            g.bind_owner_node_point(n, pointer.clone()).unwrap();
        }
        let groups = vec![vec![
            PackingNodeRef::Node(NodeId(0)),
            PackingNodeRef::Node(NodeId(1)),
            PackingNodeRef::Node(NodeId(0)),
        ]];
        let mut saved = PackingStaging::default();
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        g.packing_stage_checked(
            &groups,
            false,
            Point::default(),
            false,
            &mut saved,
            &mut guard,
        )
        .unwrap();
        assert_eq!(guard.used(), 5);
        assert_eq!(saved.positions.len(), 2);
        assert_eq!(saved.positions[0].value, Some(Point { x: 1.0, y: 2.0 }));
        assert_eq!(
            saved.positions[1].value,
            Some(Point {
                x: 100001.0,
                y: 100002.0
            })
        );
        assert_eq!(
            pointer.get(),
            Some(Point {
                x: 300001.0,
                y: 300002.0
            })
        );
        // Numerical candidates must detach the allocation while retaining alias relations.
        let mut trial = g.clone();
        trial.packing_translate_ref(groups[0][0], Point { x: 5.0, y: 7.0 });
        assert_eq!(
            pointer.get(),
            Some(Point {
                x: 300001.0,
                y: 300002.0
            })
        );
        assert_eq!(
            trial.position(NodeId(1)),
            Some(Point {
                x: 300006.0,
                y: 300009.0
            })
        );
        saved.restore_positions_checked(&mut g, &mut guard).unwrap();
        assert_eq!(
            pointer.get(),
            Some(Point {
                x: 100001.0,
                y: 100002.0
            })
        );
        assert!(
            g.owner_node_point(NodeId(0))
                .unwrap()
                .same_allocation(&pointer)
        );
    }
    #[test]
    fn interrupted_route_copy_does_not_publish_partial_snapshot() {
        let mut g = graph();
        let mut saved = PackingStaging::default();
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 4).unwrap();
        assert!(
            g.packing_stage_checked(
                &[vec![PackingNodeRef::Node(NodeId(0))]],
                false,
                Point::default(),
                true,
                &mut saved,
                &mut guard
            )
            .is_err()
        );
        assert_eq!(guard.used(), 5);
        assert_eq!(saved.positions.len(), 1);
        assert!(saved.routes.is_empty());
        assert_eq!(
            g.position(NodeId(0)),
            Some(Point {
                x: 100001.0,
                y: 100002.0
            })
        );
        assert_eq!(g.position(NodeId(1)), Some(Point { x: 11.0, y: 12.0 }));
        assert_eq!(g.edge_records[0].route.point(0), Point { x: 3.0, y: 4.0 });
    }
    #[test]
    fn routes_move_once_by_first_incident_node_and_restore_existing_storage() {
        let mut g = graph();
        let mut saved = PackingStaging::default();
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        g.packing_stage_checked(
            &[vec![
                PackingNodeRef::Node(NodeId(1)),
                PackingNodeRef::Node(NodeId(0)),
            ]],
            true,
            Point { x: 20.0, y: 30.0 },
            true,
            &mut saved,
            &mut guard,
        )
        .unwrap();
        assert_eq!(saved.routes.len(), 1);
        assert_eq!(saved.positions[0].node, PackingNodeRef::Node(NodeId(1)));
        assert_eq!(g.edge_records[0].route.point(0), Point { x: 22.0, y: 32.0 });
        let backing = g.edge_records[0].route.backing_identity();
        saved.restore_routes_checked(&mut g, &mut guard).unwrap();
        assert_eq!(backing, g.edge_records[0].route.backing_identity());
        assert_eq!(g.edge_records[0].route.point(0), Point { x: 3.0, y: 4.0 });
    }
    #[test]
    fn nil_position_is_snapshotted_before_translate_panics() {
        let mut g = graph();
        g.clear_position(NodeId(0));
        let mut saved = PackingStaging::default();
        let mut guard = limits::WorkGuard::new(Some(&PackingContext), "BinPack", 100).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.packing_stage_checked(
                &[vec![PackingNodeRef::Node(NodeId(0))]],
                false,
                Point::default(),
                false,
                &mut saved,
                &mut guard,
            )
        }));
        assert!(result.is_err());
        assert_eq!(guard.used(), 2);
        assert_eq!(saved.positions.len(), 1);
        assert!(saved.positions[0].value.is_none());
    }
}
