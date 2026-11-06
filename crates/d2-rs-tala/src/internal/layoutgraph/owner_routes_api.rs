// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Explicit Edge.Points storage and transaction_snapshot.go::captureEdge.
// Included in shared_topology so point and slice ownership use one namespace.

#[derive(Clone, Debug)]
pub(super) struct OwnerRouteHeader {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    slice: SharedPointSlice,
}
impl OwnerRouteHeader {
    pub(super) fn is_nil(&self) -> bool {
        self.slice.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.slice.len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.slice.capacity()
    }
    pub(super) fn offset(&self) -> usize {
        self.slice.offset()
    }
    pub(super) fn get(&self, index: usize) -> Option<OwnerNodePoint> {
        self.slice.get(index).map(|point| OwnerNodePoint {
            arena: self.arena,
            scope: self.scope,
            point,
        })
    }
    pub(super) fn capacity_point(&self, index: usize) -> Option<OwnerNodePoint> {
        self.slice
            .read_capacity_slot(index)
            .map(|point| OwnerNodePoint {
                arena: self.arena,
                scope: self.scope,
                point,
            })
    }
    pub(super) fn reslice(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Result<Self, OwnerTopologyError> {
        Ok(Self {
            arena: self.arena,
            scope: self.scope,
            slice: self
                .slice
                .reslice_with_capacity(low, high, max)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?,
        })
    }
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observed_slot_identity(&self, index: usize) -> Option<usize> {
        self.slice.slot_address(index)
    }
    pub(super) fn same_backing(&self, other: &Self) -> bool {
        self.slice.same_backing_allocation(&other.slice)
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerRouteSnapshot {
    header: OwnerRouteHeader,
    backing: SharedPointSliceSnapshot,
    points: Vec<OwnerNodePointSnapshot>,
}

impl ArenaGraph {
    fn validate_owner_route(&self, header: &OwnerRouteHeader) -> Result<(), OwnerTopologyError> {
        if Some(header.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(header.arena),
            });
        }
        if !MemberScopeId::same_scope(&header.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignRouteScope);
        }
        Ok(())
    }
    fn wrap_owner_route(&self, slice: SharedPointSlice) -> OwnerRouteHeader {
        OwnerRouteHeader {
            arena: self.topology_arena_id().expect("initialized owner"),
            scope: self.shared_topology.member_scope,
            slice,
        }
    }
    pub(super) fn allocate_owner_route_header(
        &mut self,
        points: &[OwnerNodePoint],
        capacity: usize,
        present: bool,
    ) -> Result<OwnerRouteHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        for point in points {
            self.validate_owner_point(point)?;
        }
        let slice = if present {
            let slice = SharedPointSlice::with_len_cap(points.len(), capacity)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
            for (index, point) in points.iter().enumerate() {
                slice
                    .write_capacity_slot(index, point.point.clone())
                    .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
            }
            slice
        } else {
            if !points.is_empty() || capacity != 0 {
                return Err(OwnerTopologyError::ContainerSlice(
                    "nil route bounds".into(),
                ));
            }
            SharedPointSlice::nil()
        };
        Ok(self.wrap_owner_route(slice))
    }
    /// Actual repeated append, as used by graphjson/codec.go deserializeEdge.
    /// Every append allocates/updates real initialized backing slots; no route
    /// length is converted into a synthetic snapshot work charge.
    pub(super) fn append_owner_route(
        &self,
        header: &OwnerRouteHeader,
        points: &[OwnerNodePoint],
    ) -> Result<OwnerRouteHeader, OwnerTopologyError> {
        self.validate_owner_route(header)?;
        for point in points {
            self.validate_owner_point(point)?;
        }
        let values = points
            .iter()
            .map(|point| point.point.clone())
            .collect::<Vec<_>>();
        Ok(self.wrap_owner_route(
            header
                .slice
                .append(&values)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?,
        ))
    }
    pub(super) fn write_owner_route_slot(
        &self,
        header: &OwnerRouteHeader,
        index: usize,
        point: &OwnerNodePoint,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_route(header)?;
        self.validate_owner_point(point)?;
        header
            .slice
            .write_capacity_slot(index, point.point.clone())
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))
    }
    pub(super) fn bind_owner_edge_route(
        &mut self,
        edge: EdgeId,
        header: OwnerRouteHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.require_topology_edge(edge)?;
        self.validate_owner_route(&header)?;
        self.edge_records[edge.0 as usize].route.replace(RoutePoints::from_header(header.slice));
        Ok(())
    }
    pub(super) fn owner_edge_route(
        &self,
        edge: EdgeId,
    ) -> Result<OwnerRouteHeader, OwnerTopologyError> {
        self.require_topology_edge(edge)?;
        Ok(self.wrap_owner_route(
            self.edge_records[edge.0 as usize].route.header().clone(),
        ))
    }
    pub(super) fn snapshot_owner_edge_route(
        &self,
        edge: EdgeId,
    ) -> Result<OwnerRouteSnapshot, OwnerTopologyError> {
        let header = self.owner_edge_route(edge)?;
        let points = (0..header.capacity())
            .map(|i| {
                header
                    .capacity_point(i)
                    .expect("route capacity bound")
                    .snapshot()
            })
            .collect();
        let backing = header.slice.snapshot();
        Ok(OwnerRouteSnapshot {
            header,
            backing,
            points,
        })
    }
    pub(super) fn restore_owner_edge_route(
        &mut self,
        edge: EdgeId,
        snapshot: &OwnerRouteSnapshot,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_route(&snapshot.header)?;
        // Restore point objects before repairing the original backing window.
        // Aliases to an old allocation survive field replacement and append.
        for point in &snapshot.points {
            if let Some(value) = point.value {
                point.header.point.as_ref().unwrap().set(value);
            }
        }
        self.bind_owner_edge_route(edge, self.wrap_owner_route(snapshot.backing.restore()))
    }
}

#[cfg(test)]
mod route_storage_tests {
    use super::*;
    fn graph() -> ArenaGraph {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 20.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 10.0,
                height: 20.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        input.add_edge(crate::Edge {
            source: b,
            target: a,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.initialize_shared_topology();
        graph
    }
    fn point(graph: &mut ArenaGraph, n: f64) -> OwnerNodePoint {
        graph.allocate_owner_node_point(Some(Point { x: n, y: -n }))
    }
    #[test]
    fn route_snapshot_repairs_only_its_capacity_window_and_original_point_allocations() {
        let mut graph = graph();
        let points = (0..5)
            .map(|i| point(&mut graph, f64::from(i)))
            .collect::<Vec<_>>();
        let base = graph.allocate_owner_route_header(&points, 5, true).unwrap();
        let window = base.reslice(1, 3, 4).unwrap();
        graph
            .bind_owner_edge_route(EdgeId(0), window.clone())
            .unwrap();
        graph
            .bind_owner_node_point(NodeId(0), points[2].clone())
            .unwrap();
        let saved = graph.snapshot_owner_edge_route(EdgeId(0)).unwrap();
        for p in &points {
            graph
                .write_owner_point(p, Point { x: 900.0, y: 901.0 })
                .unwrap();
        }
        let replacement = point(&mut graph, 77.0);
        for i in 0..5 {
            graph
                .write_owner_route_slot(&base, i, &replacement)
                .unwrap();
        }
        let grown = graph
            .append_owner_route(&window, &[replacement.clone(), replacement.clone()])
            .unwrap();
        graph
            .bind_owner_edge_route(EdgeId(0), grown.clone())
            .unwrap();
        graph.restore_owner_edge_route(EdgeId(0), &saved).unwrap();
        let restored = graph.owner_edge_route(EdgeId(0)).unwrap();
        assert!(restored.same_backing(&base));
        assert!(!restored.same_backing(&grown));
        assert_eq!(
            (restored.offset(), restored.len(), restored.capacity()),
            (1, 2, 3)
        );
        for (i, p) in points.iter().enumerate().take(4).skip(1) {
            assert!(base.capacity_point(i).unwrap().same_allocation(p));
            assert_eq!(
                p.get(),
                Some(Point {
                    x: i as f64,
                    y: -(i as f64)
                })
            );
        }
        for i in [0, 4] {
            assert!(
                base.capacity_point(i)
                    .unwrap()
                    .same_allocation(&replacement)
            );
        }
        assert_eq!(points[0].get().unwrap().x, 900.0);
        assert_eq!(points[4].get().unwrap().x, 900.0);
        assert!(
            graph
                .owner_node_point(NodeId(0))
                .unwrap()
                .same_allocation(&points[2])
        );
        assert_eq!(graph.position(NodeId(0)), points[2].get());
        assert_eq!(replacement.get().unwrap().x, 77.0);
    }
    #[test]
    fn numerical_route_clone_detaches_storage_but_preserves_node_and_route_alias_classes() {
        let mut graph = graph();
        let shared = point(&mut graph, 3.0);
        let nil = graph.allocate_owner_node_point(None);
        let base = graph
            .allocate_owner_route_header(&[shared.clone(), nil, shared.clone()], 5, true)
            .unwrap();
        graph
            .bind_owner_edge_route(EdgeId(0), base.clone())
            .unwrap();
        graph
            .bind_owner_edge_route(EdgeId(1), base.reslice(1, 3, 4).unwrap())
            .unwrap();
        graph
            .bind_owner_node_point(NodeId(0), shared.clone())
            .unwrap();
        let mut copied = graph.clone();
        let a = copied.owner_edge_route(EdgeId(0)).unwrap();
        let b = copied.owner_edge_route(EdgeId(1)).unwrap();
        assert!(a.same_backing(&b));
        assert!(!a.same_backing(&base));
        let first = a.get(0).unwrap();
        assert!(first.same_allocation(&a.get(2).unwrap()));
        assert!(first.same_allocation(&copied.owner_node_point(NodeId(0)).unwrap()));
        assert!(!first.same_allocation(&shared));
        copied
            .write_owner_point(&first, Point { x: 7.0, y: 8.0 })
            .unwrap();
        assert_eq!(b.get(1).unwrap().get().unwrap().x, 7.0);
        assert_eq!(copied.position(NodeId(0)).unwrap().x, 7.0);
        assert_eq!(shared.get().unwrap().x, 3.0);
        assert!(matches!(
            copied.bind_owner_edge_route(EdgeId(0), base),
            Err(OwnerTopologyError::ForeignRouteScope)
        ));
    }
    #[test]
    fn route_append_retains_nil_slots_old_headers_and_initialized_spare_capacity() {
        let mut graph = graph();
        let nil = graph.allocate_owner_route_header(&[], 0, false).unwrap();
        let empty = graph.allocate_owner_route_header(&[], 0, true).unwrap();
        assert!(nil.is_nil());
        assert!(!empty.is_nil());
        let p = point(&mut graph, 1.0);
        let absent = graph.allocate_owner_node_point(None);
        let one = graph.append_owner_route(&nil, std::slice::from_ref(&p)).unwrap();
        let two = graph.append_owner_route(&one, &[absent]).unwrap();
        let three = graph.append_owner_route(&two, std::slice::from_ref(&p)).unwrap();
        assert_eq!(
            (
                nil.len(),
                one.len(),
                two.len(),
                three.len(),
                three.capacity()
            ),
            (0, 1, 2, 3, 4)
        );
        assert!(three.get(1).unwrap().get().is_none());
        assert!(three.capacity_point(3).unwrap().get().is_none());
        let four = graph.append_owner_route(&three, &[p]).unwrap();
        assert!(three.same_backing(&four));
        assert_eq!(three.len(), 3);
        assert_eq!(three.capacity_point(3).unwrap().get().unwrap().x, 1.0);
    }
}
