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
        self.0.get(slot).is_some_and(|&index| index != UNASSIGNED_SLOT)
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
    build: super::ovg_ports::PortBuild,
    objects: Vec<usize>,
    indices: SlotIndices,
    /// Keyed by allocation address, whose low bits are alignment zeros.
    edge_indices: crate::internal::FloatKeyHashMap<usize, usize>,
    /// Each object's shared point allocation, captured at refresh. Reads go
    /// through the same allocation as PortNode::point (so moved points stay
    /// live) without taking the port node's lock. `point_ref` is only
    /// replaced by build mutations, and every one is followed by a refresh.
    /// Shared with each flavor's route index as its retained-point table.
    point_refs: std::sync::Arc<[Option<crate::internal::owner_points::SharedOwnerPoint>]>,
    /// Per canonical index, PortNode::is_node_center. The flag is written only
    /// when a center is allocated during a build, before that build's refresh.
    node_centers: Vec<bool>,
    /// The per-hop search record of each canonical index. Its inputs are
    /// published only by refresh while this view exists, so it is rebuilt
    /// there instead of once per search.
    search_nodes: Vec<SearchNode>,
    /// Static edges for route indices, built on first use after a refresh
    /// and shared by every flavor routing over this view.
    static_edges: std::sync::OnceLock<std::sync::Arc<super::static_edge_index::StaticEdges>>,
    /// `objects` as route node identities, shared like `static_edges`.
    route_identities: std::sync::OnceLock<std::sync::Arc<[usize]>>,
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
            build,
            objects,
            indices,
            edge_indices: Default::default(),
            point_refs: Vec::new().into(),
            node_centers: Vec::new(),
            search_nodes: Vec::new(),
            static_edges: std::sync::OnceLock::new(),
            route_identities: std::sync::OnceLock::new(),
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
                [edge.from_slot(), edge.to_slot()]
                    .into_iter()
                    .all(|slot| {
                        view.build
                            .nodes
                            .get(slot)
                            .is_some_and(|node| node.read().point_ref.is_some())
                    })
            });
            if view.static_edges.get().is_none() && endpoints_bound {
                let edges =
                    super::static_edge_index::StaticEdges::from_build_taking_axes(&mut view.build);
                let _ = view.static_edges.set(std::sync::Arc::new(edges));
            }
        }
        std::sync::Arc::new(self)
    }
    pub(super) fn point_at_canonical(view: &CanonicalView, index: usize) -> Point {
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
        view.route_identities = std::sync::OnceLock::new();
        view.build.retain_incident_objects();
        // Size the lookup-only identity map for the build's edges at once
        // instead of through repeated doubling; later refreshes add only
        // their new edges.
        view.edge_indices
            .reserve(view.build.edges.len().saturating_sub(view.edge_indices.len()));
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
                view.objects.push(node);
            }
        }
        // Every table is laid out by canonical index (the objects order), so
        // search reads skip the objects indirection. One pass reads each
        // node's fields under a single shared borrow; refresh charges no work
        // and polls nothing, so the per-node read order is unobservable.
        let count = view.objects.len();
        let mut point_refs = Vec::with_capacity(count);
        let mut node_centers = Vec::with_capacity(count);
        // One flat table holds each slot's neighbor, distance, axis and
        // canonical edge identity (None for a nil slot, captured with the
        // same lookups canonical_slot_edge reads back). Size it exactly up
        // front so building it never regrows (and first-touches) a copy.
        let mut edges = Adjacency::with_capacity(count);
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
        let mut points = Vec::with_capacity(count);
        let mut search_indices = Vec::with_capacity(count);
        let mut point_containers = Vec::with_capacity(count);
        let mut port_owners = Vec::with_capacity(count);
        let mut near_port_owners = Vec::with_capacity(count);
        for &node in &view.objects {
            let shared = &view.build.nodes[node];
            let record = shared.read();
            point_refs.push(record.point_ref.clone());
            node_centers.push(record.is_node_center);
            let a = record.point();
            points.push(a);
            search_indices.push(record.index);
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
            point_containers.push(record.container);
            // owner_directions yields every owner key in map order.
            port_owners.push(record.owners.keys().copied().collect::<BTreeSet<_>>());
            near_port_owners.push(record.near_ports.as_deref().cloned().unwrap_or_default());
        }
        view.point_refs = point_refs.into();
        view.node_centers = node_centers;
        edges.entries.shrink_to_fit();
        self.points = points;
        self.search_indices = search_indices;
        self.edges = edges;
        self.centers = view
            .build
            .centers
            .iter()
            .map(|(&owner, node)| (owner, view.indices[node]))
            .collect();
        self.center_owners = vec![None; view.objects.len()];
        for (&owner, &node) in &view.build.centers {
            if view.build.nodes[node].read().is_node_center {
                self.center_owners[view.indices[&node]] = Some(owner);
            }
        }
        self.point_containers = point_containers;
        self.port_owners = port_owners;
        self.near_port_owners = near_port_owners;
        let search_nodes = self.build_search_nodes();
        if let Some(view) = &mut self.canonical {
            view.search_nodes = search_nodes;
        }
    }
    fn member_count(&self) -> usize {
        self.canonical
            .as_ref()
            .map_or(self.points.len(), |view| view.build.members.len())
    }
    fn is_node_center(&self, index: usize) -> bool {
        self.canonical.as_ref().map_or_else(
            || self.center_owners[index].is_some(),
            |view| view.node_centers[index],
        )
    }
    pub(super) fn static_edge_index(&self) -> super::static_edge_index::StaticEdgeIndex {
        let Some(view) = &self.canonical else {
            return super::static_edge_index::StaticEdgeIndex::from_segments(self.segments());
        };
        let shared = view.static_edges.get_or_init(|| {
            std::sync::Arc::new(super::static_edge_index::StaticEdges::from_build(&view.build))
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
    pub(super) fn route_node_identities(&self) -> std::sync::Arc<[usize]> {
        self.canonical.as_ref().map_or_else(
            || (0..self.points.len()).collect(),
            |view| {
                view.route_identities
                    .get_or_init(|| view.objects.as_slice().into())
                    .clone()
            },
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
    pub(super) fn add_existing_edge_route(
        &mut self,
        graph: &ArenaGraph,
        edge: crate::EdgeId,
        budget: Option<&OvgBuildBudget<'_>>,
    ) -> Result<Option<super::ovg_import::ExistingRoute>, ()> {
        let mut route = super::ovg_import::ImportedRoute::default();
        let result = self
            .canonical
            .as_mut()
            .expect("constructed OVG retains canonical objects")
            .build
            .import_edge_route(graph, edge, &mut route, budget);
        self.refresh_canonical();
        result?;
        let view = self.canonical.as_ref().unwrap();
        // Translate identities in place: keep the completed route's order and
        // pointer-slice capacity, including repeated objects and ghost nodes.
        for node in &mut route.nodes {
            *node = view.indices[node];
        }
        Ok(route.complete(edge))
    }
}

#[cfg(test)]
mod canonical_tests {
    use super::*;
    use crate::internal::owner_points::SharedOwnerPoint;
    #[test]
    fn foreign_incident_edges_retain_neighbors_and_distinct_search_identities() {
        let mut source = super::super::ovg_ports::PortBuild::default();
        let from = source.add_point(Point { x: 1.0, y: 2.0 }, None).unwrap();
        let foreign = source.allocate_node(Point { x: 11.0, y: 2.0 });
        source.connect(from, foreign, None).unwrap();
        source.edges[0].write().distance = 123.5;
        let mut receiver = super::super::ovg_ports::PortBuild::default();
        let a = receiver
            .add_point(Point { x: 101.0, y: 102.0 }, None)
            .unwrap();
        let b = receiver
            .add_point(Point { x: 111.0, y: 102.0 }, None)
            .unwrap();
        receiver.connect(a, b, None).unwrap();
        let local = receiver.import_object(source.nodes[from].clone());
        receiver.add_existing_node(local, None).unwrap();
        let end = receiver.add_point(Point { x: 1.0, y: 12.0 }, None).unwrap();
        receiver.connect(local, end, None).unwrap();
        drop(source);
        let mut view = VisibilityGraph::default();
        view.retain_canonical(receiver);
        let local_index = view
            .points
            .iter()
            .position(|p| *p == Point { x: 1.0, y: 2.0 })
            .unwrap();
        let foreign_index = view
            .points
            .iter()
            .position(|p| *p == Point { x: 11.0, y: 2.0 })
            .unwrap();
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
        build.connect(foreign_slot, end_slot, None).unwrap();
        view.refresh_canonical();
        assert_eq!(view.canonical_edge_at(local_index, 0), foreign_identity);
        assert_eq!(view.canonical_edge_at(foreign_index, 1), Some(2));
        assert_eq!(view.static_edge_index().edges.len(), 3);
    }
    #[test]
    fn search_adjacencies_preserve_distinct_static_edge_objects() {
        let mut build = super::super::ovg_ports::PortBuild::default();
        build.allocate_node(Point { x: 0.0, y: 0.0 });
        build.allocate_node(Point { x: 10.0, y: 0.0 });
        build.connect(0, 1, None).unwrap();
        build.connect(0, 1, None).unwrap();
        let mut view = VisibilityGraph::default();
        view.retain_canonical(build);
        assert_eq!(view.canonical_edge_at(0, 0), Some(0));
        assert_eq!(view.canonical_edge_at(0, 1), Some(1));
        assert_eq!(view.canonical_edge_at(1, 0), Some(0));
        assert_eq!(view.canonical_edge_at(1, 1), Some(1));
        assert_eq!(view.static_edge_index().edges.len(), 2);
    }

    #[test]
    fn route_edge_identity_uses_objects_across_repeated_search_membership() {
        use super::super::{
            published_route::PublishedRoute,
            routed_edge_set::{Edge, EdgeSet},
        };
        let mut build = super::super::ovg_ports::PortBuild::default();
        let first = build.add_point(Point { x: 1.0, y: 2.0 }, None).unwrap();
        let second = build.add_point(Point { x: 3.0, y: 2.0 }, None).unwrap();
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
    fn search_projection_keeps_ghost_identity_live_points_and_source_array_length() {
        let mut build = super::super::ovg_ports::PortBuild::default();
        let ghost = build.add_point(Point { x: 1.0, y: 2.0 }, None).unwrap();
        build.remove_isolated(None).unwrap();
        let point = SharedOwnerPoint::new(Point { x: 3.0, y: 4.0 });
        let mut route = super::super::ovg_import::ImportedRoute::default();
        build
            .import_route_point(Some(point.clone()), &mut route, None)
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
        let prepared = view.prepare_route_search(None).unwrap();
        assert_eq!(prepared.slots.len(), 1);
    }
}
