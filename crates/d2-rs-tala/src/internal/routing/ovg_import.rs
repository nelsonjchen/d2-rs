// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! coordinator.go routeAdditionalEdges: retain occupied objects and Point pointers.
use super::ovg_intersections::{Budget, rect, step};
use super::ovg_ports::{PortBuild, push_pointer};
use super::{ArenaGraph, Point};
use crate::{EdgeId, internal::owner_points::SharedOwnerPoint};

#[derive(Default)]
pub(super) struct ImportedRoute {
    pub(super) nodes: Vec<usize>,
    pub(super) last: Option<SharedOwnerPoint>,
    pub(super) endpoints: Option<[Point; 2]>,
}

/// coordinator.go's completed Route, distinct from a partly imported path.
/// Node IDs refer to retained OVG objects; endpoints are copies taken when
/// import completed, not the first/last current OVG point.
#[derive(Clone, Debug)]
pub(super) struct ExistingRoute {
    pub(super) edge: EdgeId,
    pub(super) nodes: Vec<usize>,
    pub(super) endpoints: [Point; 2],
}

impl ImportedRoute {
    pub(super) fn complete(self, edge: EdgeId) -> Option<ExistingRoute> {
        self.endpoints.map(|endpoints| ExistingRoute {
            edge,
            nodes: self.nodes,
            endpoints,
        })
    }
}

impl PortBuild {
    pub(super) fn import_route_point(
        &mut self,
        point: Option<SharedOwnerPoint>,
        route: &mut ImportedRoute,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        step(budget)?;
        let point =
            point.expect("runtime error: invalid memory address or nil pointer dereference");
        if route
            .last
            .as_ref()
            .is_some_and(|last| last.get() == point.get())
        {
            return Ok(());
        }
        route.last = Some(point.clone());
        let node = if let Some(node) = self.occupied_node(point.get()) {
            push_pointer(&mut route.nodes, node);
            node
        } else {
            let node = self.allocate_node(point.get());
            self.nodes[node].write().point_ref = Some(point);
            self.nodes[node].write().index = self.members.len();
            push_pointer(&mut route.nodes, node);
            // The local route sees the object before either fallible reservation.
            step(budget)?;
            if let Some(b) = budget {
                b.reserve_nodes(1)?;
            }
            self.publish_member(node, budget.is_some());
            node
        };
        if route.nodes.len() > 1 {
            self.connect(route.nodes[route.nodes.len() - 2], node, budget)?;
        }
        Ok(())
    }

    pub(super) fn import_edge_route(
        &mut self,
        graph: &ArenaGraph,
        edge: EdgeId,
        route: &mut ImportedRoute,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        let center = |target| {
            let node = graph
                .owner_edge_endpoint(edge, target)
                .expect("fixed route endpoint")
                .expect("runtime error: invalid memory address or nil pointer dereference");
            let bounds = rect(graph, node);
            SharedOwnerPoint::new(Point {
                x: bounds.origin.x + bounds.size.width / 2.0,
                y: bounds.origin.y + bounds.size.height / 2.0,
            })
        };
        self.import_route_point(Some(center(false)), route, budget)?;
        // Go captures this header only after the source-center call. Each slot
        // is fetched before the callback and keeps its actual allocation.
        let points = graph.edge_records[edge.0 as usize].route.clone();
        for index in 0..points.len() {
            self.import_route_point(points.slot(index), route, budget)?;
        }
        self.import_route_point(Some(center(true)), route, budget)?;
        if route.nodes.len() > 1 {
            let points = &graph.edge_records[edge.0 as usize].route;
            if points.is_empty() {
                panic!("runtime error: index out of range [0] with length 0");
            }
            route.endpoints = Some([points.point(0), points.point(points.len() - 1)]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::route_search_guard::OvgBuildBudget;
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;
    struct Caller<'a> {
        polls: Cell<u64>,
        cancel: u64,
        mutate: &'a dyn Fn(u64),
    }
    impl Context for Caller<'_> {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            (self.mutate)(poll);
            (self.cancel != 0 && poll >= self.cancel).then_some(ContextError::Cancelled)
        }
    }
    #[test]
    fn local_route_precedes_membership_and_reads_pointer_after_reservation() {
        for cancel in [2, 3, 0] {
            let mut build = PortBuild::default();
            let mut route = ImportedRoute::default();
            let point = SharedOwnerPoint::new(Point { x: 1.0, y: 2.0 });
            let mutate = |poll| {
                if poll == 2 {
                    point.set(Point { x: 7.0, y: 8.0 });
                }
            };
            let context = Caller {
                polls: Cell::new(0),
                cancel,
                mutate: &mutate,
            };
            let budget = OvgBuildBudget::with_context(&context);
            let result = build.import_route_point(Some(point.clone()), &mut route, Some(&budget));
            assert_eq!(result.is_err(), cancel != 0);
            assert_eq!(route.nodes.len(), 1);
            assert_eq!(
                build.nodes[route.nodes[0]].read().point(),
                Point { x: 7.0, y: 8.0 }
            );
            assert_eq!(build.members.len(), usize::from(cancel == 0));
            assert!(build.occupied_node(Point { x: 1.0, y: 2.0 }).is_none());
            assert_eq!(
                build.occupied_node(Point { x: 7.0, y: 8.0 }).is_some(),
                cancel == 0
            );
        }
    }
    #[test]
    fn last_point_reads_the_shared_allocation_after_callback_mutation() {
        let mut build = PortBuild::default();
        let mut route = ImportedRoute::default();
        let point = SharedOwnerPoint::new(Point { x: 1.0, y: 2.0 });
        build
            .import_route_point(Some(point.clone()), &mut route, None)
            .unwrap();
        let mutate = |_| point.set(Point { x: 7.0, y: 8.0 });
        let context = Caller {
            polls: Cell::new(0),
            cancel: 0,
            mutate: &mutate,
        };
        let budget = OvgBuildBudget::with_context(&context);
        build
            .import_route_point(Some(point.clone()), &mut route, Some(&budget))
            .unwrap();
        assert_eq!(route.nodes.len(), 1);
        assert!(build.edges.is_empty());
        assert_eq!(build.occupied_node(Point { x: 1.0, y: 2.0 }), Some(0));
        assert!(build.occupied_node(Point { x: 7.0, y: 8.0 }).is_none());
    }
    #[test]
    fn retained_occupant_does_not_rejoin_membership() {
        let mut build = PortBuild::default();
        let point = Point { x: 1.0, y: 2.0 };
        let original = build.add_point(point, None).unwrap();
        build.remove_isolated(None).unwrap();
        let mut route = ImportedRoute::default();
        build
            .import_route_point(Some(SharedOwnerPoint::new(point)), &mut route, None)
            .unwrap();
        assert_eq!(route.nodes, [original]);
        assert!(build.members.is_empty());
        assert_eq!(build.nodes[original].read().index, 0);
    }
    #[test]
    fn nan_references_allocate_distinct_objects_with_shared_points() {
        let mut build = PortBuild::default();
        let mut route = ImportedRoute::default();
        let point = SharedOwnerPoint::new(Point {
            x: f64::NAN,
            y: 2.0,
        });
        build
            .import_route_point(Some(point.clone()), &mut route, None)
            .unwrap();
        build
            .import_route_point(Some(point.clone()), &mut route, None)
            .unwrap();
        assert_eq!(route.nodes, [0, 1]);
        assert_eq!(build.members, [0, 1]);
        assert_eq!(build.nodes[1].read().index, 1);
        assert_eq!(build.edges.len(), 1);
        assert!(
            build.nodes.iter().all(|n| n
                .read()
                .point_ref
                .as_ref()
                .unwrap()
                .same_allocation(&point))
        );
    }
}
