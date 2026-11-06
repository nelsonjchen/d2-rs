// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Source packing segment scans and their guarded callers.
use super::binpack_traversal::PackingNodeRef;
use super::checked_bounds::BoundsFailure;
use super::*;
type Segments = Option<Vec<(Point, Point)>>;
type RoutedSegments = Option<Vec<(EdgeId, Point, Point)>>;
#[derive(Default)]
pub(super) struct PackingSegments {
    pub(super) segments: Segments,
    pub(super) children: Vec<PackingNodeRef>,
}
fn incomplete_edge() -> limits::Error {
    limits::Error::Invalid(
        "layout invariant violated: BinPack segment estimate encountered an incomplete edge".into(),
    )
}
fn placed(p: Option<Point>) -> Point {
    p.expect("runtime error: invalid memory address or nil pointer dereference")
}
fn node(n: Option<PackingNodeRef>) -> PackingNodeRef {
    n.expect("runtime error: invalid memory address or nil pointer dereference")
}
fn route_point(points: &[Point], i: isize) -> Point {
    if i < 0 {
        panic!("runtime error: index out of range [{i}]");
    }
    points.get(i as usize).copied().unwrap_or_else(|| {
        panic!(
            "runtime error: index out of range [{i}] with length {}",
            points.len()
        )
    })
}
impl ArenaGraph {
    pub(super) fn packing_segment_incidents(&self, n: PackingNodeRef) -> Vec<Option<EdgeId>> {
        if let PackingNodeRef::Node(n) = n {
            if let Ok(slots) = self.owner_node_incident_slots(n) {
                return slots;
            }
            return self.nodes[n.0 as usize]
                .edges
                .read()
                .iter()
                .copied()
                .map(Some)
                .collect();
        }
        self.packing_incident_slots(n)
    }
    pub(super) fn packing_smallest_deltas_checked(
        &self,
        groups: &[Vec<Option<PackingNodeRef>>],
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(f64, f64), limits::Error> {
        let (mut x, mut y) = (f64::INFINITY, f64::INFINITY);
        for group in groups {
            guard.step()?;
            let (low, high) = self
                .packing_bounds_checked(group, guard)
                .map_err(|f| f.error)?;
            x = routing::channel_solver::minimum(x, high.x - low.x);
            y = routing::channel_solver::minimum(y, high.y - low.y);
        }
        guard.finish()?;
        Ok((x, y))
    }
    pub(super) fn packing_routed_segments_checked(
        &self,
        edges: &[Option<EdgeId>],
        horizontal: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<RoutedSegments, BoundsFailure<RoutedSegments>> {
        let mut out = Vec::new();
        for &edge in edges {
            guard.step()?;
            let edge = edge.ok_or_else(|| {
                limits::Error::Invalid(
                    "layout invariant violated: BinPack routed segment scan encountered a nil edge"
                        .into(),
                )
            })?;
            for pair in self.edge_records[edge.0 as usize].route.read().windows(2) {
                guard.step()?;
                let (a, b) = (pair[0], pair[1]);
                if horizontal && a.y == b.y {
                    out.push(if a.x < b.x {
                        (edge, a, b)
                    } else {
                        (edge, b, a)
                    });
                } else if !horizontal && a.x == b.x {
                    out.push(if a.y < b.y {
                        (edge, a, b)
                    } else {
                        (edge, b, a)
                    });
                }
            }
        }
        let value = Some(out);
        guard.finish().map_err(|error| BoundsFailure {
            value: value.clone(),
            error,
        })?;
        Ok(value)
    }
    fn packing_is_cluster_endpoint(&self, n: PackingNodeRef) -> bool {
        match n {
            PackingNodeRef::Node(n) => self.owner_is_cluster_vessel(n),
            PackingNodeRef::Cluster(_) => true,
            PackingNodeRef::Sequence(_) => false,
        }
    }
    fn packing_segment_endpoints(&self, n: PackingNodeRef) -> Vec<Point> {
        let (p, size, metadata) = self.packing_bounds_geometry(n);
        let p = placed(p);
        let center = Point {
            x: p.x + size.width / 2.0,
            y: p.y + size.height / 2.0,
        };
        if !self.packing_is_cluster_endpoint(n) {
            return vec![center];
        }
        // g.Clusters[e.From/To] is dereferenced only on the cluster branch.
        let i = self
            .clusters
            .iter()
            .enumerate()
            .find_map(|(i, _)| (self.packing_vessel(true, i) == n).then_some(i))
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let arrangement = self
            .owner_group_binding(TopologyVessel::Cluster(i))
            .map(|g| self.owner_cluster_policy(g).unwrap().arrangement)
            .or_else(|| {
                self.serialized_cluster_arrangements_by_tala
                    .get(&self.clusters[i].vessel_tala_id)
                    .cloned()
            })
            .unwrap_or_else(|| {
                match self.clusters[i].arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                }
                .into()
            });
        let (low, high) = if let Some(m) = metadata {
            Self::positioned_node_body_bounds(m, p, true)
        } else {
            (
                p,
                Point {
                    x: (p.x + size.width).round(),
                    y: (p.y + size.height).round(),
                },
            )
        };
        match arrangement.as_str() {
            "Row" => vec![
                Point {
                    x: low.x,
                    y: center.y,
                },
                Point {
                    x: high.x,
                    y: center.y,
                },
            ],
            "Column" => vec![
                Point {
                    x: center.x,
                    y: low.y,
                },
                Point {
                    x: center.x,
                    y: high.y,
                },
            ],
            _ => Vec::new(),
        }
    }
    pub(super) fn packing_estimate_edge_checked(
        &self,
        edge: Option<EdgeId>,
        xgap: f64,
        ygap: f64,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Segments, BoundsFailure<Segments>> {
        guard.step()?;
        let edge = edge.ok_or_else(incomplete_edge)?;
        let from = self
            .owner_edge_endpoint(edge, false)
            .expect("packing edge allocation")
            .ok_or_else(incomplete_edge)?;
        let to = self
            .owner_edge_endpoint(edge, true)
            .expect("packing edge allocation")
            .ok_or_else(incomplete_edge)?;
        let (from, to) = (
            self.packing_effective_node(from),
            self.packing_effective_node(to),
        );
        let (fp, fs, _) = self.packing_bounds_geometry(from);
        let (tp, ts, _) = self.packing_bounds_geometry(to);
        let (fp, tp) = (
            fp.ok_or_else(incomplete_edge)?,
            tp.ok_or_else(incomplete_edge)?,
        );
        let orientation = self.sized_box_orientation((fp, fs), (tp, ts));
        if !orientation.is_diagonal()
            && !self.packing_is_cluster_endpoint(from)
            && !self.packing_is_cluster_endpoint(to)
        {
            let a = Point {
                x: fp.x + fs.width / 2.0,
                y: fp.y + fs.height / 2.0,
            };
            let b = Point {
                x: tp.x + ts.width / 2.0,
                y: tp.y + ts.height / 2.0,
            };
            if (matches!(orientation, Orientation::Left | Orientation::Right)
                && (a.x - b.x).abs() > xgap)
                || (matches!(orientation, Orientation::Top | Orientation::Bottom)
                    && (a.y - b.y).abs() > ygap)
            {
                return Ok(Some(vec![(a, b)]));
            }
            return Ok(None);
        }
        let froms = self.packing_segment_endpoints(from);
        let tos = self.packing_segment_endpoints(to);
        let mut segments = Vec::new();
        for a in froms {
            for &b in &tos {
                guard.step()?;
                let c = Point { x: a.x, y: b.y };
                let d = Point { x: b.x, y: a.y };
                let mx = ((a.x + b.x) / 2.0).round();
                let my = ((a.y + b.y) / 2.0).round();
                segments.extend([
                    (a, c),
                    (c, b),
                    (a, d),
                    (d, b),
                    (Point { x: mx, y: a.y }, Point { x: mx, y: b.y }),
                    (Point { x: a.x, y: my }, Point { x: b.x, y: my }),
                ]);
            }
        }
        let value = Some(segments);
        guard.finish().map_err(|error| BoundsFailure {
            value: value.clone(),
            error,
        })?;
        Ok(value)
    }
    pub(super) fn packing_estimate_routes_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        xgap: f64,
        ygap: f64,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Segments, BoundsFailure<Segments>> {
        let mut segments = Vec::new();
        let mut unique = BTreeSet::new();
        for &n in nodes {
            guard.step()?;
            for edge in self.packing_segment_incidents(node(n)) {
                guard.step()?;
                if !unique.insert(edge) {
                    continue;
                }
                let next = self
                    .packing_estimate_edge_checked(edge, xgap, ygap, guard)
                    .map_err(|f| BoundsFailure::from(f.error))?;
                segments.extend(next.unwrap_or_default());
            }
        }
        let value = Some(segments);
        guard.finish().map_err(|error| BoundsFailure {
            value: value.clone(),
            error,
        })?;
        Ok(value)
    }
    fn packing_segment_edges_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        root: Option<PackingNodeRef>,
        unique: &mut BTreeSet<Option<EdgeId>>,
        edges: &mut Vec<Option<EdgeId>>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        for &n in nodes {
            guard.step()?;
            if n == root {
                continue;
            }
            if !self.packing_descendant_checked(n, root, true, guard)? {
                continue;
            }
            for e in self.packing_segment_incidents(node(n)) {
                guard.step()?;
                if unique.insert(e) {
                    edges.push(e);
                }
            }
        }
        Ok(())
    }
    fn packing_append_routed_checked(
        &self,
        edges: &[Option<EdgeId>],
        state: &mut PackingSegments,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let h = self
            .packing_routed_segments_checked(edges, true, guard)
            .map_err(|f| f.error)?;
        let v = self
            .packing_routed_segments_checked(edges, false, guard)
            .map_err(|f| f.error)?;
        for (_, a, b) in h.into_iter().flatten().chain(v.into_iter().flatten()) {
            guard.step()?;
            state.segments.get_or_insert_default().push((a, b));
        }
        Ok(())
    }
    fn packing_append_estimated_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        root: Option<PackingNodeRef>,
        xgap: f64,
        ygap: f64,
        state: &mut PackingSegments,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let mut filtered = Vec::new();
        for &n in nodes {
            guard.step()?;
            if n == root {
                continue;
            }
            if self.packing_descendant_checked(n, root, true, guard)? {
                filtered.push(n);
            }
        }
        let segments = self
            .packing_estimate_routes_checked(&filtered, xgap, ygap, guard)
            .map_err(|f| f.error)?;
        for segment in segments.into_iter().flatten() {
            guard.step()?;
            state.segments.get_or_insert_default().push(segment);
        }
        Ok(())
    }
    pub(super) fn packing_initial_segments_checked(
        &self,
        packed: &[Vec<Option<PackingNodeRef>>],
        remaining: &[Vec<Option<PackingNodeRef>>],
        root: Option<PackingNodeRef>,
        routed: bool,
        state: &mut PackingSegments,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        if routed {
            let mut edges = Vec::new();
            let mut unique = BTreeSet::new();
            for group in packed {
                guard.step()?;
                self.packing_segment_edges_checked(group, root, &mut unique, &mut edges, guard)?;
            }
            self.packing_append_routed_checked(&edges, state, guard)?;
            for group in packed {
                guard.step()?;
                for &n in group {
                    guard.step()?;
                    let n = node(n);
                    if self.packing_raw_container(n) == root {
                        state.children.push(n);
                    }
                }
            }
        } else {
            if packed.is_empty() {
                panic!("runtime error: index out of range [0] with length 0");
            }
            let _capacity = packed[0].len() * 10;
            state.segments = Some(Vec::new());
            for group in packed {
                guard.step()?;
                for &n in group {
                    guard.step()?;
                    let n = node(n);
                    if self.packing_raw_container(n) == root {
                        state.children.push(n);
                    }
                }
                let (x, y) = self.packing_smallest_deltas_checked(remaining, guard)?;
                self.packing_append_estimated_checked(group, root, x, y, state, guard)?;
            }
        }
        Ok(())
    }
    pub(super) fn packing_extend_segments_checked(
        &self,
        current: &[Option<PackingNodeRef>],
        remaining: &[Vec<Option<PackingNodeRef>>],
        root: Option<PackingNodeRef>,
        routed: bool,
        state: &mut PackingSegments,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        for &n in current {
            guard.step()?;
            state.children.push(node(n));
        }
        let (x, y) = self.packing_smallest_deltas_checked(remaining, guard)?;
        if routed {
            let mut edges = Vec::new();
            let mut unique = BTreeSet::new();
            self.packing_segment_edges_checked(current, root, &mut unique, &mut edges, guard)?;
            self.packing_append_routed_checked(&edges, state, guard)
        } else {
            self.packing_append_estimated_checked(current, root, x, y, state, guard)
        }
    }
    pub(super) fn packing_permissions_checked(
        &self,
        root: Option<PackingNodeRef>,
        routed: bool,
        permissions: &mut [bool; 4],
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        *permissions = [true; 4];
        let Some(root) = root.filter(|_| routed) else {
            return Ok(());
        };
        for edge in self.packing_segment_incidents(root) {
            guard.step()?;
            let e = edge.expect("runtime error: invalid memory address or nil pointer dereference");
            let edge = &self.edge_records[e.0 as usize];
            let from = self
                .owner_edge_endpoint(e, false)
                .expect("packing edge allocation")
                .map(|n| self.packing_effective_node(n));
            let (a, b) = if from == Some(root) {
                (
                    route_point(&edge.route.read(), 0),
                    route_point(&edge.route.read(), 1),
                )
            } else {
                let n = edge.route.len() as isize;
                (
                    route_point(&edge.route.read(), n - 1),
                    route_point(&edge.route.read(), n - 2),
                )
            };
            if a.y == b.y {
                if a.x < b.x {
                    permissions[1] = false;
                }
                if a.x > b.x {
                    permissions[0] = false;
                }
            } else if a.x == b.x {
                if a.y < b.y {
                    permissions[3] = false;
                }
                if a.y > b.y {
                    permissions[2] = false;
                }
            } else {
                *permissions = [false; 4];
                break;
            }
        }
        Ok(())
    }
}

// source packing/binpack.go::blocksRoutesWithDeltasGuarded
impl ArenaGraph {
    pub(super) fn packing_blocks_routes_checked(
        &self,
        current: &[Option<PackingNodeRef>],
        packed: &[Vec<Option<PackingNodeRef>>],
        packed_segments: &[(Point, Point)],
        deltas: (f64, f64),
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let segments = self
            .packing_estimate_routes_checked(current, deltas.0, deltas.1, guard)
            .map_err(|f| f.error)?
            .unwrap_or_default();
        let overlaps = |n: PackingNodeRef, a: Point, b: Point| {
            let (p, s, _) = self.packing_bounds_geometry(n);
            let p = p.expect("runtime error: invalid memory address or nil pointer dereference");
            let left = p.x - 20.;
            let right = p.x + s.width + 20.;
            let top = p.y - 20.;
            let bottom = p.y + s.height + 20.;
            let contains = |p: Point| left <= p.x && right >= p.x && top <= p.y && bottom >= p.y;
            if contains(a) || contains(b) {
                return true;
            }
            fn intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
                let cross = |a: Point, b: Point, c: Point| {
                    (b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y)
                };
                let interval = |a: f64, b: f64, c: f64, d: f64| {
                    routing::channel_solver::minimum(a, b)
                        <= super::owner_median::source_math_max(c, d)
                        && routing::channel_solver::minimum(c, d)
                            <= super::owner_median::source_math_max(a, b)
                };
                let straddles = |a: f64, b: f64| a == 0. || b == 0. || (a < 0.) != (b < 0.);
                {
                    let p = cross(a, b, c);
                    let q = cross(a, b, d);
                    let r = cross(c, d, a);
                    let s = cross(c, d, b);
                    if p == 0. && q == 0. && r == 0. && s == 0. {
                        interval(a.x, b.x, c.x, d.x) && interval(a.y, b.y, c.y, d.y)
                    } else {
                        straddles(p, q) && straddles(r, s)
                    }
                }
            }
            let tl = Point { x: left, y: top };
            let tr = Point { x: right, y: top };
            let br = Point {
                x: right,
                y: bottom,
            };
            let bl = Point { x: left, y: bottom };
            intersect(tl, tr, a, b)
                || intersect(tr, br, a, b)
                || intersect(br, bl, a, b)
                || intersect(bl, tl, a, b)
        };
        for group in packed {
            guard.step()?;
            for &n in group {
                guard.step()?;
                for &(a, b) in &segments {
                    guard.step()?;
                    if overlaps(node(n), a, b) {
                        guard.finish()?;
                        return Ok(true);
                    }
                }
            }
        }
        for &n in current {
            guard.step()?;
            for &(a, b) in packed_segments {
                guard.step()?;
                if overlaps(node(n), a, b) {
                    guard.finish()?;
                    return Ok(true);
                }
            }
        }
        guard.finish()?;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Context {
        polls: Cell<usize>,
        cancel_at: usize,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            (self.polls.get() == self.cancel_at).then_some(limits::ContextError::Cancelled)
        }
    }
    fn graph() -> ArenaGraph {
        let input=crate::internal::graphjson::graphjson_to_graph(br#"{"nodes":[{"id":1,"width":10,"height":10,"topLeft":{"x":0,"y":0}},{"id":2,"width":10,"height":10,"topLeft":{"x":100,"y":100}}],"edges":[{"id":-1,"from":1,"to":2,"points":[{"x":3,"y":4},{"x":1,"y":4},{"x":1,"y":4}]}],"containers":{"0":[1,2]}}"#).unwrap();
        let mut g = ArenaGraph::from_input(&input);
        g.initialize_shared_topology();
        g
    }
    #[test]
    fn routed_scan_preserves_sorted_endpoints_duplicates_and_finish_value() {
        let g = graph();
        let c = Context {
            polls: Cell::new(0),
            cancel_at: 2,
        };
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        let failure = g
            .packing_routed_segments_checked(&[Some(EdgeId(0)), Some(EdgeId(0))], true, &mut guard)
            .unwrap_err();
        let value = failure.value.unwrap();
        assert_eq!(value.len(), 4);
        assert_eq!(
            value[0],
            (
                EdgeId(0),
                Point { x: 1.0, y: 4.0 },
                Point { x: 3.0, y: 4.0 }
            )
        );
        assert_eq!(value[1].1, value[1].2);
        assert_eq!(guard.used(), 6);
        let c = Context {
            polls: Cell::new(0),
            cancel_at: 99,
        };
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            g.packing_routed_segments_checked(&[Some(EdgeId(0)), None], true, &mut guard)
                .unwrap_err()
                .value
                .is_none()
        );
        assert_eq!(guard.used(), 4);
    }
    #[test]
    fn estimate_caller_discards_inner_finish_partial_segments() {
        let g = graph();
        let c = Context {
            polls: Cell::new(0),
            cancel_at: 2,
        };
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert_eq!(
            g.packing_estimate_edge_checked(Some(EdgeId(0)), 0.0, 0.0, &mut guard)
                .unwrap_err()
                .value
                .unwrap()
                .len(),
            6
        );
        assert_eq!(guard.used(), 2);
        let c = Context {
            polls: Cell::new(0),
            cancel_at: 2,
        };
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            g.packing_estimate_routes_checked(
                &[Some(PackingNodeRef::Node(NodeId(0)))],
                0.0,
                0.0,
                &mut guard
            )
            .unwrap_err()
            .value
            .is_none()
        );
        assert_eq!(guard.used(), 4);
    }
    #[test]
    fn straight_estimate_uses_strict_gap_and_has_no_final_poll() {
        let mut g = graph();
        g.set_position(NodeId(1), Point { x: 100.0, y: 0.0 });
        let c = Context {
            polls: Cell::new(0),
            cancel_at: 2,
        };
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            g.packing_estimate_edge_checked(Some(EdgeId(0)), 100.0, 0.0, &mut guard)
                .unwrap()
                .is_none()
        );
        assert_eq!(guard.used(), 1);
        assert_eq!(c.polls.get(), 1);
        assert_eq!(
            g.packing_estimate_edge_checked(Some(EdgeId(0)), 99.0, 0.0, &mut guard)
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(c.polls.get(), 1);
    }
    #[test]
    fn unrouted_preparation_records_children_before_smallest_bounds_failure() {
        let g = graph();
        let c = Context {
            polls: Cell::new(0),
            cancel_at: 2,
        };
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        let mut state = PackingSegments::default();
        let n = PackingNodeRef::Node(NodeId(0));
        assert!(
            g.packing_initial_segments_checked(
                &[vec![Some(n)]],
                &[],
                None,
                false,
                &mut state,
                &mut guard
            )
            .is_err()
        );
        assert_eq!(state.children, vec![n]);
        assert_eq!(state.segments, Some(vec![]));
        assert_eq!(guard.used(), 2);
    }
}
