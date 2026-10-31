// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Retained routing/Route objects. Route identity is independent of GEdge.
use super::Point;
use crate::internal::owner_points::SharedOwnerPoint;
use std::borrow::Cow;

#[derive(Clone)]
pub(super) struct PublishedRoute {
    pub(super) edge: usize,
    pub(super) nodes: Vec<Point>,
    pub(super) identities: Option<Vec<usize>>,
    pub(super) endpoints: Option<[Point; 2]>,
    live: Vec<Option<SharedOwnerPoint>>,
}
impl PublishedRoute {
    pub(super) fn new(
        edge: usize,
        nodes: Vec<Point>,
        identities: Option<Vec<usize>>,
        endpoints: Option<[Point; 2]>,
        live: Vec<Option<SharedOwnerPoint>>,
    ) -> Self {
        debug_assert!(live.is_empty() || live.len() == nodes.len());
        Self {
            edge,
            nodes,
            identities,
            endpoints,
            live,
        }
    }
    pub(super) fn point(&self, index: usize) -> Point {
        self.live
            .get(index)
            .and_then(Option::as_ref)
            .map_or_else(|| self.nodes[index], SharedOwnerPoint::get)
    }
    pub(super) fn node_ref(
        &self,
        index: usize,
        route_id: usize,
        canonical_nodes: &[usize],
    ) -> super::routed_edge_set::NodeRef {
        use super::routed_edge_set::{NodeIdentity, NodeRef};
        let identity = match self.identities.as_ref().map(|ids| ids[index]) {
            Some(id) if id < canonical_nodes.len() => NodeIdentity::Ovg(canonical_nodes[id]),
            Some(id) => NodeIdentity::Synthetic(route_id, id),
            None => NodeIdentity::Synthetic(route_id, index),
        };
        NodeRef {
            identity,
            value: self.nodes[index],
            shared: self.live.get(index).cloned().flatten(),
        }
    }
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }
    pub(super) fn points(&self) -> Cow<'_, [Point]> {
        if self.live.is_empty() {
            Cow::Borrowed(&self.nodes)
        } else {
            Cow::Owned((0..self.len()).map(|index| self.point(index)).collect())
        }
    }
    pub(super) fn segment_endpoints(&self) -> Option<Vec<Point>> {
        super::route_geometry::segment_endpoints_from_ovg_nodes(&self.points())
    }

    /// Route.createSegmentEndpoints reads the retained points at each use and
    /// appends fresh copies with Go pointer-slice growth, including spare slots.
    pub(super) fn endpoint_slice(&self) -> crate::internal::RoutePoints {
        let mut points = crate::internal::RoutePoints::literal(&[]);
        points.append_value(self.point(1));
        for index in 2..self.len() - 2 {
            let previous = self.point(index - 1);
            let current = self.point(index);
            let next = self.point(index + 1);
            if (current.x == previous.x && current.x != next.x)
                || (current.y == previous.y && current.y != next.y)
            {
                points.append_value(current);
            }
        }
        points.append_value(self.point(self.len() - 2));
        points
    }

    pub(super) fn reverse(&mut self) {
        if let Some(ports) = &mut self.endpoints {
            ports.swap(0, 1);
        }
        self.nodes.reverse();
        if let Some(identities) = &mut self.identities {
            identities.reverse();
        }
        self.live.reverse();
    }
}
