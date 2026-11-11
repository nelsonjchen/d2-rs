// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Included by visibility.rs: search identities are independent of Nodes/Index.
/// Canonical index of each build slot, densely by slot. Refresh assigns
/// every slot, so an unassigned read is the missing-key panic of a map.
#[derive(Clone, Debug, Default)]
struct SlotIndices(Vec<usize>);
const UNASSIGNED_SLOT: usize = usize::MAX;
impl SlotIndices {
    fn insert(&mut self, slot: usize, index: usize) {
        if slot >= self.0.len() {
            self.0.resize(slot + 1, UNASSIGNED_SLOT);
        }
        self.0[slot] = index;
    }
    fn contains(&self, slot: usize) -> bool {
        self.0
            .get(slot)
            .is_some_and(|&index| index != UNASSIGNED_SLOT)
    }
}
impl std::ops::Index<&usize> for SlotIndices {
    type Output = usize;
    fn index(&self, slot: &usize) -> &usize {
        let index = &self.0[*slot];
        assert!(*index != UNASSIGNED_SLOT, "missing canonical OVG slot");
        index
    }
}
#[derive(Clone, Debug)]
struct CanonicalView {
    // Routing sizes its search arrays from the source Nodes slice, which may
    // repeat objects or omit retained ghosts. Once frozen only its length is
    // needed; the canonical identity inventory below keeps the objects.
    frozen_member_count: Option<usize>,
    // Go's Ports lists retain only node identities during routing. The Port
    // adapter records belong to construction and are released at freeze.
    frozen_ports: super::ovg_ports::PortLists<Vec<usize>>,
    build: super::ovg_ports::PortBuild,
    // Route indices retain this same immutable identity inventory. A refresh
    // appends through copy-on-write only when an earlier reader still owns it.
    objects: std::sync::Arc<Vec<usize>>,
    indices: SlotIndices,
    /// Keyed by allocation address, whose low bits are alignment zeros.
    edge_indices: crate::internal::FloatKeyHashMap<usize, usize>,
    /// Each object's shared point allocation, captured at refresh. Reads go
    /// through the same allocation as PortNode::point (so moved points stay
    /// live) without taking the port node's lock. `point_ref` is only
    /// replaced by build mutations, and every one is followed by a refresh.
    /// Shared with each flavor's route index as its retained-point table.
    point_refs: std::sync::Arc<[Option<crate::internal::owner_points::SharedOwnerPoint>]>,
    /// The per-hop search record of each canonical index. Its inputs are
    /// published only by refresh while this view exists, so it is rebuilt
    /// there instead of once per search.
    search_nodes: Vec<SearchNode>,
    /// Static edges for route indices, built on first use after a refresh
    /// and shared by every flavor routing over this view.
    static_edges: std::sync::OnceLock<std::sync::Arc<super::static_edge_index::StaticEdges>>,
}
impl CanonicalView {
    /// Frozen occupied-point and port lookups already hold canonical indices;
    /// mutable builds still hold construction slots and need the inverse map.
    fn lookup_index(&self, slot: usize) -> usize {
        if self.frozen_member_count.is_some() {
            slot
        } else {
            self.indices[&slot]
        }
    }
}
impl VisibilityGraph {
    fn retain_canonical(&mut self, build: super::ovg_ports::PortBuild) {
        let objects = build.members.clone();
        // A repeated member keeps its last position, as map collection does.
        let mut indices = SlotIndices::default();
        for (index, &node) in objects.iter().enumerate() {
            indices.insert(node, index);
        }
        self.canonical = Some(Box::new(CanonicalView {
            frozen_member_count: None,
            frozen_ports: Default::default(),
            build,
            objects: std::sync::Arc::new(objects),
            indices,
            edge_indices: Default::default(),
            point_refs: Vec::new().into(),
            search_nodes: Vec::new(),
            static_edges: std::sync::OnceLock::new(),
        }));
        self.refresh_canonical();
    }
    /// Freeze this OVG for routing. A shared graph is never mutated again,
    /// so no build import (and no refresh) can follow; release the tables
    /// only those mutation paths read.
    pub(super) fn into_shared(mut self) -> std::sync::Arc<Self> {
        if let Some(view) = &mut self.canonical {
            view.edge_indices = Default::default();
            view.build.release_mutation_tables();
            // The build's Connect axes are read only to construct the static
            // edges. Construct them now and move the axes in, unless an edge
            // endpoint lacks its point (whose panic stays with first use).
            let endpoints_bound = view.build.edges.iter().all(|edge| {
                let edge = edge.read();
                [edge.from_slot(), edge.to_slot()].into_iter().all(|slot| {
                    view.build
                        .nodes
                        .get(slot)
                        .is_some_and(|node| node.read().point_ref.is_some())
                })
            });
            if view.static_edges.get().is_none() && endpoints_bound {
                let edges = super::static_edge_index::StaticEdges::take_shared_points(
                    &mut view.build,
                    view.point_refs.clone(),
                    &view.indices.0,
                    &view.objects,
                );
                let _ = view.static_edges.set(std::sync::Arc::new(edges));
            }
            if view.static_edges.get().is_some() {
                // No build mutation follows into_shared. Routing reads the
                // already published adjacency, ownership and search records,
                // and the static edges retain the same live Point allocations.
                view.frozen_member_count = Some(view.build.members.len());
                // Go's OccupiedPoints and Ports refer directly to retained
                // nodes (ovg.go:21,25). Publish those lookups in the same
                // index space as the retained points, then release the
                // construction-slot translation table too.
                view.frozen_ports = view.build.freeze_lookups(&view.indices.0);
                view.indices = SlotIndices::default();
            }
        }
        std::sync::Arc::new(self)
    }
    fn point_at_canonical(view: &CanonicalView, index: usize) -> Point {
        view.point_refs[index]
            .as_ref()
            .expect("runtime error: invalid memory address or nil pointer dereference")
            .get()
    }
    fn refresh_canonical(&mut self) {
        let Some(view) = &mut self.canonical else {
            return;
        };
        // Every build mutation is followed by this refresh.
        view.static_edges = std::sync::OnceLock::new();
        // Canonical reads use the live point references and packed search
        // records below. These arrays belong only to numerical-only graphs;
        // discard a constructor's preview before publishing the shared view.
        self.points = Vec::new();
        self.search_indices = Vec::new();
        self.center_owners = Vec::new();
        self.point_containers = Vec::new();
        view.build.retain_incident_objects();
        // Size the lookup-only identity map for the build's edges at once
        // instead of through repeated doubling; later refreshes add only
        // their new edges.
        view.edge_indices.reserve(
            view.build
                .edges
                .len()
                .saturating_sub(view.edge_indices.len()),
        );
        for (index, edge) in view.build.edges.iter().enumerate() {
            view.edge_indices.insert(edge.allocation_key(), index);
        }
        for node in &view.build.nodes {
            node.read().edges.with_slots(|slots| {
                for edge in slots.iter().flatten() {
                    let next = view.edge_indices.len();
                    view.edge_indices
                        .entry(edge.allocation_key())
                        .or_insert(usize::MAX - next);
                }
            });
        }
        // Append storage for retained map references and new allocations without
        // changing previously published route identities. Nodes remains separate.
        for node in 0..view.build.nodes.len() {
            if !view.indices.contains(node) {
                view.indices.insert(node, view.objects.len());
                std::sync::Arc::make_mut(&mut view.objects).push(node);
            }
        }
        // Every table is laid out by canonical index (the objects order), so
        // search reads skip the objects indirection. One pass reads each
        // node's fields under a single shared borrow; refresh charges no work
        // and polls nothing, so the per-node read order is unobservable.
        let count = view.objects.len();
        let mut point_refs = Vec::with_capacity(count);
        let mut search_nodes = std::mem::take(&mut view.search_nodes);
        search_nodes.clear();
        search_nodes.reserve(count);
        // One flat table holds each slot's neighbor, distance, axis and
        // canonical edge identity (None for a nil slot, captured with the
        // same lookups canonical_slot_edge reads back). Size it exactly up
        // front so building it never regrows (and first-touches) a copy.
        // No search can borrow this mutable graph. Reuse the previous view's
        // buffers rather than retaining a complete old view while building
        // its replacement after fixed-route imports.
        let mut edges = std::mem::take(&mut self.edges);
        edges.offsets.clear();
        edges.offsets.reserve(count + 1);
        edges.offsets.push(0);
        edges.entries.clear();
        edges.entries.reserve_exact(
            view.objects
                .iter()
                .map(|&node| {
                    view.build.nodes[node]
                        .read()
                        .edges
                        .with_slots(|slots| slots.len())
                })
                .sum(),
        );
        let mut port_owners = std::mem::take(&mut self.port_owners);
        port_owners.clear();
        port_owners.offsets.reserve(count);
        let mut near_port_owners = std::mem::take(&mut self.near_port_owners);
        near_port_owners.clear();
        near_port_owners.offsets.reserve(count);
        for &node in view.objects.iter() {
            let shared = &view.build.nodes[node];
            let record = shared.read();
            point_refs.push(record.point_ref.clone());
            let a = record.point();
            record.edges.with_slots(|slots| {
                for edge in slots.iter() {
                    let identity = edge
                        .as_ref()
                        .map(|edge| view.edge_indices[&edge.allocation_key()]);
                    let (neighbor, distance, axis) = edge
                        .as_ref()
                        .expect("runtime error: invalid memory address or nil pointer dereference")
                        .with_record(|edge| {
                            let adjacent = if std::sync::Arc::ptr_eq(&shared.0, &edge.from_object) {
                                &edge.to_object
                            } else {
                                &edge.from_object
                            };
                            let other = view
                                .build
                                .object_slot_by_key(std::sync::Arc::as_ptr(adjacent) as usize);
                            let b = view.build.nodes[other].point();
                            let axis = if a.y == b.y {
                                Axis::Horizontal
                            } else if a.x == b.x {
                                Axis::Vertical
                            } else {
                                Axis::None
                            };
                            (view.indices[&other], edge.distance, axis)
                        });
                    edges.push_slot(AdjacencyEntry::new(neighbor, distance, axis, identity));
                }
            });
            edges.finish_node();
            // owner_directions yields every owner key in map order.
            port_owners.push(record.owners.keys().copied());
            near_port_owners.push(record.near_ports.as_deref().into_iter().flatten().copied());
            search_nodes.push(SearchNode::new(
                record.index(),
                record.container(),
                if record.owners.is_empty() {
                    0
                } else {
                    SEARCH_NODE_PORT_OWNED
                } | if record
                    .near_ports
                    .as_ref()
                    .is_some_and(|owners| !owners.is_empty())
                {
                    SEARCH_NODE_NEAR_PORT
                } else {
                    0
                } | if record.is_node_center() {
                    SEARCH_NODE_CENTER
                } else {
                    0
                },
            ));
        }
        view.point_refs = point_refs.into();
        view.search_nodes = search_nodes;
        self.edges = edges;
        self.centers = view
            .build
            .centers
            .iter()
            .map(|(&owner, node)| (owner, view.indices[node]))
            .collect();
        self.port_owners = port_owners;
        self.near_port_owners = near_port_owners;
    }
    fn member_count(&self) -> usize {
        self.canonical.as_ref().map_or(self.points.len(), |view| {
            view.frozen_member_count.unwrap_or(view.build.members.len())
        })
    }
    fn is_node_center(&self, index: usize) -> bool {
        self.canonical.as_ref().map_or_else(
            || self.center_owners[index].is_some(),
            |view| view.search_nodes[index].flags() & SEARCH_NODE_CENTER != 0,
        )
    }
    pub(super) fn static_edge_index(&self) -> super::static_edge_index::StaticEdgeIndex {
        let Some(view) = &self.canonical else {
            return super::static_edge_index::StaticEdgeIndex::from_segments(self.segments());
        };
        let shared = view.static_edges.get_or_init(|| {
            std::sync::Arc::new(super::static_edge_index::StaticEdges::from_shared_points(
                &view.build,
                view.point_refs.clone(),
                &view.indices.0,
                &view.objects,
                view.build.horizontal_edges.clone(),
                view.build.vertical_edges.clone(),
            ))
        });
        super::static_edge_index::StaticEdgeIndex::from_shared(shared.clone())
    }
    #[cfg(test)]
    fn canonical_edge_at(&self, node: usize, adjacency: usize) -> Option<usize> {
        self.canonical_adjacency(node)
            .map(|slots| Self::canonical_slot_edge(slots, adjacency))
    }
    /// `node`'s adjacency slots with their canonical edge identities,
    /// resolved once per search expansion; refresh publishes them in the
    /// search table itself. A node without slots still panics only at a
    /// slot read, in canonical_slot_edge's range check.
    fn canonical_adjacency(&self, node: usize) -> Option<&[AdjacencyEntry]> {
        self.canonical
            .as_ref()
            .map(|_| self.edges.get(node).unwrap_or(&[]))
    }
    #[inline]
    fn canonical_slot_edge(slots: &[AdjacencyEntry], adjacency: usize) -> usize {
        assert!(adjacency < slots.len(), "OVG edge slice index out of range");
        slots[adjacency]
            .edge()
            .expect("runtime error: invalid memory address or nil pointer dereference")
    }
    /// `PortBuild::node_ports` of the canonical build. Refresh imports only
    /// foreign objects, never port inventories, so this is the constructing
    /// build's port map.
    pub(super) fn canonical_node_ports(&self) -> BTreeMap<NodeId, Vec<Port>> {
        self.canonical
            .as_ref()
            .expect("constructed OVG retains canonical objects")
            .build
            .node_ports()
    }
    pub(super) fn route_node_identities(&self) -> std::sync::Arc<Vec<usize>> {
        self.canonical.as_ref().map_or_else(
            || std::sync::Arc::new((0..self.points.len()).collect()),
            |view| view.objects.clone(),
        )
    }
    /// Each canonical index's retained `point_ref` allocation (None if nil),
    /// as captured by the last refresh; empty without canonical objects.
    pub(super) fn shared_route_points(
        &self,
    ) -> std::sync::Arc<[Option<crate::internal::owner_points::SharedOwnerPoint>]> {
        self.canonical
            .as_ref()
            .map_or_else(|| Vec::new().into(), |view| view.point_refs.clone())
    }
    #[cfg(test)]
    pub(super) fn add_existing_edge_route(
        &mut self,
        graph: &ArenaGraph,
        edge: crate::EdgeId,
        budget: &OvgBuildBudget<'_>,
    ) -> Result<Option<super::ovg_import::ExistingRoute>, ()> {
        let route = self.import_existing_edge_route(graph, edge, budget);
        Ok(self
            .publish_existing_edge_routes(vec![(edge, route?)])
            .pop()
            .flatten())
    }
    /// Import one fixed route into the canonical build without publishing it.
    /// Imports connect only this build's own objects, so they discover no
    /// foreign incidences, and refresh charges no work: a batch published by
    /// one refresh assigns exactly the indices and edge identities that a
    /// refresh after every import would.
    pub(super) fn import_existing_edge_route(
        &mut self,
        graph: &ArenaGraph,
        edge: crate::EdgeId,
        budget: &OvgBuildBudget<'_>,
    ) -> Result<super::ovg_import::ImportedRoute, ()> {
        let mut route = super::ovg_import::ImportedRoute::default();
        self.canonical
            .as_mut()
            .expect("constructed OVG retains canonical objects")
            .build
            .import_edge_route(graph, edge, &mut route, budget)?;
        Ok(route)
    }
    /// Refresh once for every imported route, then translate each route's
    /// build slots to canonical indices, in import order.
    pub(super) fn publish_existing_edge_routes(
        &mut self,
        routes: Vec<(crate::EdgeId, super::ovg_import::ImportedRoute)>,
    ) -> Vec<Option<super::ovg_import::ExistingRoute>> {
        if routes.is_empty() {
            return Vec::new();
        }
        self.refresh_canonical();
        let view = self.canonical.as_ref().unwrap();
        routes
            .into_iter()
            .map(|(edge, mut route)| {
                // Translate identities in place: keep the completed route's
                // order and pointer-slice capacity, including repeated objects
                // and ghost nodes.
                for node in &mut route.nodes {
                    *node = view.indices[node];
                }
                route.complete(edge)
            })
            .collect()
    }
}

#[cfg(test)]
mod canonical_tests {
    use super::*;
    use crate::internal::owner_points::SharedOwnerPoint;
    #[test]
    fn foreign_incident_edges_retain_neighbors_and_distinct_search_identities() {
        let mut source = super::super::ovg_ports::PortBuild::default();
        let from = source.add_point(Point { x: 1.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let foreign = source.allocate_node(Point { x: 11.0, y: 2.0 });
        source.connect(from, foreign, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        source.edges[0].write().distance = 123.5;
        {
            let mut node = source.nodes[from].write();
            for owner in [NodeId(20), NodeId(10)] {
                node.owners.insert(owner, Default::default());
            }
            node.near_ports = Some(Box::new(BTreeSet::from([NodeId(30), NodeId(40)])));
            node.set_container(Some(NodeId(70)));
            node.set_index(11);
        }
        let mut receiver = super::super::ovg_ports::PortBuild::default();
        let a = receiver
            .add_point(Point { x: 101.0, y: 102.0 }, &super::super::route_search_guard::OvgBuildBudget::new())
            .unwrap();
        let b = receiver
            .add_point(Point { x: 111.0, y: 102.0 }, &super::super::route_search_guard::OvgBuildBudget::new())
            .unwrap();
        receiver.connect(a, b, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let local = receiver.import_object(source.nodes[from].clone());
        receiver.add_existing_node(local, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let end = receiver.add_point(Point { x: 1.0, y: 12.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        receiver.connect(local, end, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        receiver.nodes[local].write().set_node_center(true);
        drop(source);
        let mut view = VisibilityGraph::default();
        view.retain_canonical(receiver);
        let local_index = (0..view.edges.len())
            .find(|&node| view.point(node) == Point { x: 1.0, y: 2.0 })
            .unwrap();
        let foreign_index = (0..view.edges.len())
            .find(|&node| view.point(node) == Point { x: 11.0, y: 2.0 })
            .unwrap();
        assert_eq!(view.search_index(local_index), 11);
        assert_eq!(
            view.search_nodes()[local_index].container(),
            Some(NodeId(70))
        );
        assert_eq!(
            view.search_nodes()[local_index].flags(),
            SEARCH_NODE_PORT_OWNED | SEARCH_NODE_NEAR_PORT | SEARCH_NODE_CENTER
        );
        assert!(view.is_node_center(local_index));
        assert!(!view.is_node_center(foreign_index));
        assert_eq!(
            view.port_owners[local_index]
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([NodeId(10), NodeId(20)])
        );
        assert_eq!(
            view.near_port_owners[local_index]
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([NodeId(30), NodeId(40)])
        );
        assert!(view.port_owners[foreign_index].is_empty());
        assert!(view.near_port_owners[foreign_index].is_empty());
        assert_eq!(
            view.edges[local_index][0].tuple(),
            (foreign_index, 123.5, Axis::Horizontal)
        );
        assert_ne!(
            view.canonical_edge_at(local_index, 0),
            view.canonical_edge_at(0, 0)
        );
        assert_ne!(
            view.canonical_edge_at(local_index, 0),
            view.canonical_edge_at(local_index, 1)
        );
        assert_eq!(
            view.static_edge_index().edges.len(),
            2,
            "foreign incidences do not publish global edges"
        );
        let foreign_identity = view.canonical_edge_at(local_index, 0);
        let build = &mut view.canonical.as_mut().unwrap().build;
        {
            let mut node = build.nodes[local].write();
            node.owners.remove(&NodeId(10));
            node.owners.insert(NodeId(50), Default::default());
            node.near_ports = None;
        }
        let foreign_slot = build
            .nodes
            .iter()
            .position(|node| node.point() == Point { x: 11.0, y: 2.0 })
            .unwrap();
        let end_slot = build
            .nodes
            .iter()
            .position(|node| node.point() == Point { x: 1.0, y: 12.0 })
            .unwrap();
        build.connect(foreign_slot, end_slot, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        view.refresh_canonical();
        assert_eq!(
            view.port_owners[local_index]
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([NodeId(20), NodeId(50)])
        );
        assert!(view.near_port_owners[local_index].is_empty());
        assert_eq!(
            view.search_nodes()[local_index].flags(),
            SEARCH_NODE_PORT_OWNED | SEARCH_NODE_CENTER
        );
        assert_eq!(view.canonical_edge_at(local_index, 0), foreign_identity);
        assert_eq!(view.canonical_edge_at(foreign_index, 1), Some(2));
        assert_eq!(view.static_edge_index().edges.len(), 3);
        // OVGNode.Index is independent of the retained inventory position.
        // Refresh and freeze must keep it, along with cleared flags and nil
        // Container (pinned routing/ovg_node.go, ovg.go:224-227).
        let search_index = super::super::route_search_guard::MAX_OVG_NODES as usize - 1;
        {
            let mut node = view.canonical.as_mut().unwrap().build.nodes[local].write();
            node.set_index(search_index);
            node.set_container(None);
            node.set_node_center(false);
            node.owners = Default::default();
        }
        view.refresh_canonical();
        let view = view.into_shared();
        assert_eq!(view.search_index(local_index), search_index);
        assert_eq!(view.search_nodes()[local_index].container(), None);
        assert_eq!(view.search_nodes()[local_index].flags(), 0);
        assert!(!view.is_node_center(local_index));
        assert!(view.port_owners[local_index].is_empty());
    }
    #[test]
    fn search_adjacencies_preserve_distinct_static_edge_objects() {
        let mut build = super::super::ovg_ports::PortBuild::default();
        build.add_point(Point { x: 0.0, y: 0.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        build.add_point(Point { x: 10.0, y: 0.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        build.connect(0, 1, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        build.connect(0, 1, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let original_node = std::sync::Arc::downgrade(&build.nodes[0].0);
        let point = build.nodes[0].read().point_ref.clone().unwrap();
        let mut view = VisibilityGraph::default();
        view.retain_canonical(build);
        assert_eq!(view.canonical_edge_at(0, 0), Some(0));
        assert_eq!(view.canonical_edge_at(0, 1), Some(1));
        assert_eq!(view.canonical_edge_at(1, 0), Some(0));
        assert_eq!(view.canonical_edge_at(1, 1), Some(1));
        assert_eq!(view.static_edge_index().edges.len(), 2);
        let segments = view.segments();
        let view = view.into_shared();
        assert_eq!(view.segments(), segments);
        assert_eq!(view.segment_iter().collect::<Vec<_>>(), segments);
        assert_eq!(view.canonical_edge_at(0, 1), Some(1));
        assert_eq!(view.index_of(Point { x: 0., y: 0. }), Some(0));
        assert!(original_node.upgrade().is_none());
        point.set(Point { x: 5., y: 7. });
        assert_eq!(view.point(0), Point { x: 5., y: 7. });
        assert_eq!(view.segments(), vec![[view.point(0), view.point(1)]; 2]);
        let index = view.static_edge_index();
        assert_eq!(index.edge_points(0), [view.point(0), view.point(1)]);
        assert_eq!(index.edges[0].distance, 10.);
    }

    #[test]
    fn route_edge_identity_uses_objects_across_repeated_search_membership() {
        use super::super::{
            published_route::PublishedRoute,
            routed_edge_set::{Edge, EdgeSet},
        };
        let mut build = super::super::ovg_ports::PortBuild::default();
        let first = build.add_point(Point { x: 1.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let second = build.add_point(Point { x: 3.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        build.members = vec![first, first, second];
        let mut view = VisibilityGraph::default();
        view.retain_canonical(build);
        let identities = view.route_node_identities();
        assert_eq!(&identities[..], &[first, first, second]);
        let route = PublishedRoute::new(
            0,
            vec![view.point(0), view.point(1), view.point(2)],
            Some(vec![0, 1, 2]),
            None,
            Vec::new(),
        );
        let mut set = EdgeSet::default();
        for from in [0, 1] {
            set.add(Edge {
                from: route.node_ref(from, 0, &identities),
                to: route.node_ref(2, 0, &identities),
                distance: 0.0,
            });
        }
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn retained_route_identities_keep_membership_across_import_and_freeze() {
        let mut build = super::super::ovg_ports::PortBuild::default();
        let first = build.add_point(Point { x: 1.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let second = build.add_point(Point { x: 3.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        // Go's OccupiedPoints and Ports keep the same node pointers when
        // Nodes repeats an object (ovg.go:18-42). Canonical lookups use its
        // last occurrence, even when that differs from its allocation slot.
        let owner = NodeId(10);
        build.add_tree_port_owner(first, owner, 4, PortSide::Top);
        build.add_tree_port_owner(second, owner, 4, PortSide::Top);
        build.connect(first, second, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let point = build.nodes[first].read().point_ref.clone().unwrap();
        build.members = vec![first, first, second];
        let mut view = VisibilityGraph::default();
        view.retain_canonical(build);
        let retained = view.route_node_identities();
        let imported = view
            .canonical
            .as_mut()
            .unwrap()
            .build
            .add_point(Point { x: 5.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new())
            .unwrap();
        view.refresh_canonical();
        // Like a captured Go slice header, an existing router retains its
        // original length and identities after the OVG gains another object.
        assert_eq!(&retained[..], &[first, first, second]);
        let current = view.route_node_identities();
        assert_eq!(&current[..], &[first, first, second, imported]);
        for (point, index) in [(Point { x: 1.0, y: 2.0 }, 1), (Point { x: 3.0, y: 2.0 }, 2)] {
            assert_eq!(view.index_of(point), Some(index));
            assert_eq!(view.port_index(owner, point), Some(index));
        }
        let source_member_count = view.member_count();
        let view = view.into_shared();
        assert_eq!(&view.route_node_identities()[..], &current[..]);
        assert_eq!(source_member_count, 4);
        assert_eq!(view.member_count(), source_member_count);
        assert_eq!(view.canonical.as_ref().unwrap().build.members.capacity(), 0);
        assert_eq!(view.canonical.as_ref().unwrap().indices.0.capacity(), 0);
        assert_eq!(
            view.canonical.as_ref().unwrap().frozen_ports.get(&owner),
            Some(&vec![1, 2])
        );
        assert!(
            view.canonical
                .as_ref()
                .unwrap()
                .build
                .ports
                .get(&owner)
                .is_none()
        );
        assert_eq!(
            view.canonical.as_ref().unwrap().build.port_order.capacity(),
            0
        );
        assert_eq!(&retained[..], &[first, first, second]);
        assert_eq!(view.point(3), Point { x: 5.0, y: 2.0 });
        assert_eq!(view.index_of(Point { x: 1.0, y: 2.0 }), Some(1));
        assert_eq!(view.index_of(Point { x: 3.0, y: 2.0 }), Some(2));
        assert_eq!(view.port_index(owner, Point { x: 3.0, y: 2.0 }), Some(2));
        point.set(Point { x: 7.0, y: 8.0 });
        assert_eq!(view.port_index(owner, point.get()), Some(1));
        assert_eq!(view.port_index(owner, Point { x: 1.0, y: 2.0 }), None);
        assert_eq!(view.index_of(Point { x: 1.0, y: 2.0 }), Some(1));
        assert_eq!(view.index_of(point.get()), None);
        assert_eq!(
            view.static_edge_index().edge_points(0),
            [point.get(), view.point(2)]
        );
    }

    #[test]
    fn search_projection_keeps_ghost_identity_live_points_and_source_array_length() {
        let mut build = super::super::ovg_ports::PortBuild::default();
        let ghost = build.add_point(Point { x: 1.0, y: 2.0 }, &super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        build.remove_isolated(&super::super::route_search_guard::OvgBuildBudget::new()).unwrap();
        let point = SharedOwnerPoint::new(Point { x: 3.0, y: 4.0 });
        let mut route = super::super::ovg_import::ImportedRoute::default();
        build
            .import_route_point(Some(point.clone()), &mut route, &super::super::route_search_guard::OvgBuildBudget::new())
            .unwrap();
        let mut view = VisibilityGraph::default();
        view.retain_canonical(build);
        assert_eq!(view.member_count(), 1);
        let ghost_view = view.index_of(Point { x: 1.0, y: 2.0 }).unwrap();
        assert_eq!(view.canonical.as_ref().unwrap().objects[ghost_view], ghost);
        let live_view = view.index_of(Point { x: 3.0, y: 4.0 }).unwrap();
        point.set(Point { x: 7.0, y: 8.0 });
        assert_eq!(view.point(live_view), Point { x: 7.0, y: 8.0 });
        assert_eq!(view.index_of(Point { x: 3.0, y: 4.0 }), Some(live_view));
        assert!(view.index_of(Point { x: 7.0, y: 8.0 }).is_none());
        let prepared = view.prepare_route_search(&LocalRouteSearchWork::new(u64::MAX)).unwrap();
        assert_eq!(prepared.slots.len(), 1);
        let view = view.into_shared();
        assert_eq!(view.member_count(), 1);
        assert_eq!(view.route_node_identities().len(), 2);
        assert_eq!(view.canonical.as_ref().unwrap().build.members.capacity(), 0);
        assert_eq!(view.canonical.as_ref().unwrap().indices.0.capacity(), 0);
        point.set(Point { x: 9.0, y: 10.0 });
        assert_eq!(view.point(live_view), Point { x: 9.0, y: 10.0 });
        assert_eq!(view.index_of(Point { x: 3.0, y: 4.0 }), Some(live_view));
        assert_eq!(view.index_of(Point { x: 1.0, y: 2.0 }), Some(ghost_view));
        assert!(view.index_of(Point { x: 9.0, y: 10.0 }).is_none());
        assert_eq!(view.prepare_route_search(&LocalRouteSearchWork::new(u64::MAX)).unwrap().slots.len(), 1);
    }
}
