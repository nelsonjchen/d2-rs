// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! graphbounds/bounds.go and the initial packing bounds/score callers.
use super::binpack_traversal::PackingNodeRef;
use super::bounds::external_label_position;
use super::*;

/// A source final Finish can return both a value and an error. Earlier failures
/// use the default (nil corners/point or zero score), as their caller specifies.
#[derive(Debug)]
pub(super) struct BoundsFailure<T> {
    pub(super) value: T,
    pub(super) error: limits::Error,
}
impl<T: Default> From<limits::Error> for BoundsFailure<T> {
    fn from(error: limits::Error) -> Self {
        Self {
            value: T::default(),
            error,
        }
    }
}
type BoundsResult = Result<(Point, Point), BoundsFailure<(Option<Point>, Option<Point>)>>;

impl ArenaGraph {
    pub(super) fn packing_bounds_geometry(
        &self,
        node: PackingNodeRef,
    ) -> (Option<Point>, Size, Option<&ArenaNode>) {
        match node {
            PackingNodeRef::Node(n) => (
                self.position(n),
                self.nodes[n.0 as usize].rect.size,
                Some(&self.nodes[n.0 as usize]),
            ),
            PackingNodeRef::Cluster(i) => {
                let group = &self.clusters[i];
                if !self.packing_group_active(true, i) {
                    return (
                        self.serialized_vessel_positions_by_tala
                            .get(&group.vessel_tala_id)
                            .copied(),
                        self.serialized_vessel_sizes_by_tala
                            .get(&group.vessel_tala_id)
                            .copied()
                            .unwrap_or_default(),
                        None,
                    );
                }
                (
                    group
                        .members
                        .first()
                        .and_then(|&n| self.active_node_position(n)),
                    self.cluster_vessel_size(i),
                    None,
                )
            }
            PackingNodeRef::Sequence(i) => {
                let group = &self.sequences[i];
                if !self.packing_group_active(false, i) {
                    return (
                        self.serialized_vessel_positions_by_tala
                            .get(&group.vessel_tala_id)
                            .copied(),
                        self.serialized_vessel_sizes_by_tala
                            .get(&group.vessel_tala_id)
                            .copied()
                            .unwrap_or_default(),
                        None,
                    );
                }
                (
                    group
                        .members
                        .first()
                        .and_then(|&n| self.active_node_position(n)),
                    self.sequence_vessel_size(i),
                    None,
                )
            }
        }
    }

    fn packing_bounds_extreme(
        &self,
        nodes: &[Option<PackingNodeRef>],
        node: PackingNodeRef,
        position: Point,
        size: Size,
        side: usize,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        for &other in nodes {
            guard.step()?;
            // Go first compares pointers; otherwise a nil node is dereferenced.
            if other == Some(node) {
                continue;
            }
            let other =
                other.expect("runtime error: invalid memory address or nil pointer dereference");
            let (Some(p), s, _) = self.packing_bounds_geometry(other) else {
                continue;
            };
            if match side {
                0 => p.x < position.x,
                1 => p.y < position.y,
                2 => p.x + s.width > position.x + size.width,
                _ => p.y + s.height > position.y + size.height,
            } {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn packing_node_bounds_checked(
        &self,
        node: Option<PackingNodeRef>,
        all: Option<&[Option<PackingNodeRef>]>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> BoundsResult {
        guard.step()?;
        let unplaced = || {
            limits::Error::Invalid(
                "layout invariant violated: BinPack bounding box contains an unplaced node".into(),
            )
        };
        let node = node.ok_or_else(unplaced)?;
        let (position, size, metadata) = self.packing_bounds_geometry(node);
        let position = position.ok_or_else(unplaced)?;
        let (mut low, mut high) = if let Some(metadata) = metadata {
            Self::positioned_node_body_bounds(metadata, position, true)
        } else {
            (
                position,
                Point {
                    x: (position.x + size.width).round(),
                    y: (position.y + size.height).round(),
                },
            )
        };
        if let (Some(metadata), Some(all)) = (metadata, all) {
            if let Some(label) = Self::selected_external_label(metadata) {
                let label_point = labels::point_on_box(
                    external_label_position(label),
                    Rect {
                        origin: position,
                        size,
                    },
                    label.size,
                );
                for side in 0..4 {
                    let extends = match side {
                        0 => label_point.x < low.x,
                        1 => label_point.y < low.y,
                        2 => label_point.x > high.x,
                        _ => label_point.y > high.y,
                    };
                    if !extends {
                        continue;
                    }
                    let padding =
                        if self.packing_bounds_extreme(all, node, position, size, side, guard)? {
                            5.0
                        } else {
                            10.0
                        };
                    match side {
                        0 => low.x = (label_point.x - padding).floor(),
                        1 => low.y = (label_point.y - padding).floor(),
                        2 => high.x = (label_point.x + label.size.width + padding).ceil(),
                        _ => high.y = (label_point.y + label.size.height + padding).ceil(),
                    }
                }
            }
            if let Some((side, alignment)) = (metadata.shape != ShapeKind::Image)
                .then_some(metadata.icon_position)
                .flatten()
                .and_then(Self::outside_side_alignment)
            {
                let icon = ExternalLabel {
                    size: Size {
                        width: 64.0,
                        height: 64.0,
                    },
                    side,
                    alignment,
                    automatic: false,
                    reserve_space: true,
                };
                let point = labels::point_on_box(
                    external_label_position(icon),
                    Rect {
                        origin: position,
                        size,
                    },
                    icon.size,
                );
                low.x = routing::channel_solver::minimum(low.x, (point.x - 10.0).floor());
                low.y = routing::channel_solver::minimum(low.y, (point.y - 10.0).floor());
                high.x = routing::channel_solver::maximum(high.x, (point.x + 64.0 + 10.0).ceil());
                high.y = routing::channel_solver::maximum(high.y, (point.y + 64.0 + 10.0).ceil());
            }
        }
        Ok((low, high))
    }

    pub(super) fn packing_bounds_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        guard: &mut limits::WorkGuard<'_>,
    ) -> BoundsResult {
        if nodes.is_empty() {
            let low = Point {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
            };
            let high = Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            };
            return guard
                .finish()
                .map(|()| (low, high))
                .map_err(|error| BoundsFailure {
                    value: (Some(low), Some(high)),
                    error,
                });
        }
        let mut low = Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        let mut high = Point {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        for &node in nodes {
            let (a, b) = self.packing_node_bounds_checked(node, Some(nodes), guard)?;
            low.x = routing::channel_solver::minimum(low.x, a.x);
            low.y = routing::channel_solver::minimum(low.y, a.y);
            high.x = routing::channel_solver::maximum(high.x, b.x);
            high.y = routing::channel_solver::maximum(high.y, b.y);
        }
        guard.finish()?;
        Ok((low, high))
    }

    fn packing_fixed_origin_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Option<Point>, limits::Error> {
        let mut container = None;
        let mut minimum = usize::MAX;
        for &node in nodes {
            guard.step()?;
            let current = self.packing_owning_container(node.expect("nil fixed origin node"));
            if current.is_none() {
                container = None;
                break;
            }
            let mut level = 0;
            let mut seen = BTreeSet::new();
            let mut parent = current;
            while let Some(node) = parent {
                guard.step()?;
                if !seen.insert(node) {
                    return Err(limits::Error::Invalid(
                        "layout invariant violated: BinPack found a cycle in container ancestry"
                            .into(),
                    ));
                }
                level += 1;
                parent = self.packing_owning_container(node);
            }
            if level < minimum {
                minimum = level;
                container = current;
            }
        }
        for &node in nodes {
            guard.step()?;
            let node = node.expect("nil fixed origin node");
            if self.packing_owning_container(node) != container {
                continue;
            }
            let (position, _, metadata) = self.packing_bounds_geometry(node);
            if let (Some(position), Some(fixed)) =
                (position, metadata.and_then(|n| n.fixed_top_left))
            {
                return Ok(Some(Point {
                    x: position.x - fixed.x,
                    y: position.y - fixed.y,
                }));
            }
        }
        guard.finish()?;
        Ok(None)
    }

    pub(super) fn packing_fixed_bounds_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        guard: &mut limits::WorkGuard<'_>,
    ) -> BoundsResult {
        let (mut low, high) = self
            .packing_bounds_checked(nodes, guard)
            .map_err(|failure| failure.error)?;
        if let Some(origin) = self.packing_fixed_origin_checked(nodes, guard)? {
            low = origin;
        }
        guard
            .finish()
            .map(|()| (low, high))
            .map_err(|error| BoundsFailure {
                value: (Some(low), Some(high)),
                error,
            })
    }

    pub(super) fn packing_score_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        root: Option<NodeId>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<f64, BoundsFailure<f64>> {
        let (low, high) = self
            .packing_fixed_bounds_checked(nodes, guard)
            .map_err(|failure| failure.error)?;
        let width = high.x - low.x;
        let height = high.y - low.y;
        let penalty = root.map_or(0.0, |root| {
            let node = &self.nodes[root.0 as usize];
            match (node.desired_width, node.desired_height) {
                (Some(desired), None) if width < desired => desired - width,
                (None, Some(desired)) if height < desired => desired - height,
                _ => 0.0,
            }
        });
        // Go math.Pow handles NaN before its exponentiation path and returns
        // math.NaN()'s payload, including the infinity-minus-infinity case.
        let square = |value: f64| {
            if value.is_nan() {
                f64::from_bits(0x7ff8_0000_0000_0001)
            } else {
                value * value
            }
        };
        let score = width * height + 0.5 * square(width - height) + 0.5 * square(penalty);
        guard
            .finish()
            .map(|()| score)
            .map_err(|error| BoundsFailure {
                value: score,
                error,
            })
    }

    pub(super) fn packing_container_origin_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        root: Option<NodeId>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Point, BoundsFailure<Option<Point>>> {
        let (low, high) = self
            .packing_fixed_bounds_checked(nodes, guard)
            .map_err(|failure| failure.error)?;
        let result = if let Some(root) = root {
            if self.nodes[root.0 as usize].folded_label_min_size.is_some() {
                // The existing folded-container adapter stores its original content
                // origin in child coordinates. Bounds still perform every source check.
                low
            } else {
                self.shape_inside_placement_absolute(
                    root,
                    Size {
                        width: high.x - low.x,
                        height: high.y - low.y,
                    },
                    self.shape_fit_padding(root),
                )
                .expect("runtime error: invalid memory address or nil pointer dereference")
            }
        } else {
            low
        };
        guard
            .finish()
            .map(|()| result)
            .map_err(|error| BoundsFailure {
                value: Some(result),
                error,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Context {
        polls: Cell<usize>,
        cancel_at: usize,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            (self.cancel_at > 0 && poll >= self.cancel_at)
                .then_some(limits::ContextError::Cancelled)
        }
    }
    fn context(cancel_at: usize) -> Context {
        Context {
            polls: Cell::new(0),
            cancel_at,
        }
    }
    fn arena(value: serde_json::Value) -> ArenaGraph {
        let input = graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap();
        let mut arena = ArenaGraph::from_input(&input);
        for i in 0..arena.nodes.len() {
            arena.nodes[i].position = input.initial_positions[i];
            arena.nodes[i].rect.origin = input.initial_positions[i].unwrap_or_default();
        }
        arena.initialize_shared_topology();
        arena
    }
    fn plain() -> ArenaGraph {
        arena(
            json!({"nodes":[{"id":1,"width":80,"height":60,"topLeft":{"x":0,"y":0},"shapeType":"Square"}],"edges":[],"containers":{"0":[1]},"clusters":{}}),
        )
    }
    fn node() -> Option<PackingNodeRef> {
        Some(PackingNodeRef::Node(NodeId(0)))
    }

    #[test]
    fn empty_bounds_keep_corners_on_final_error_but_nonempty_bounds_discard_them() {
        let graph = plain();
        for empty in [true, false] {
            let context = context(2);
            let mut guard = limits::WorkGuard::new(Some(&context), "BinPack", 100).unwrap();
            let nodes = if empty { vec![] } else { vec![node()] };
            let failure = graph
                .packing_bounds_checked(&nodes, &mut guard)
                .unwrap_err();
            assert_eq!(failure.error.to_string(), "BinPack: context canceled");
            if empty {
                assert_eq!(
                    failure.value,
                    (
                        Some(Point {
                            x: f64::NEG_INFINITY,
                            y: f64::NEG_INFINITY
                        }),
                        Some(Point {
                            x: f64::INFINITY,
                            y: f64::INFINITY
                        })
                    )
                );
                assert_eq!(guard.used(), 0);
            } else {
                assert_eq!(failure.value, (None, None));
                assert_eq!(guard.used(), 1);
            }
        }
    }

    #[test]
    fn fixed_bounds_and_callers_preserve_their_own_final_return_values() {
        let graph = plain();
        let nodes = [node()];
        let c = context(4);
        let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .packing_fixed_bounds_checked(&nodes, &mut g)
                .unwrap_err()
                .value,
            (Some(Point::default()), Some(Point { x: 80.0, y: 60.0 }))
        );
        for cancel_at in [4, 5] {
            let c = context(cancel_at);
            let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
            let failure = graph
                .packing_score_checked(&nodes, None, &mut g)
                .unwrap_err();
            assert_eq!(failure.value, if cancel_at == 4 { 0.0 } else { 5000.0 });
            assert_eq!(g.used(), 3);
            let c = context(cancel_at);
            let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
            let failure = graph
                .packing_container_origin_checked(&nodes, None, &mut g)
                .unwrap_err();
            assert_eq!(
                failure.value,
                if cancel_at == 4 {
                    None
                } else {
                    Some(Point::default())
                }
            );
            assert_eq!(g.used(), 3);
        }
    }

    #[test]
    fn decoded_desired_dimensions_remain_distinct_from_the_current_box() {
        let graph = arena(
            json!({"nodes":[{"id":1,"width":40,"height":20,"desiredWidth":75.25,"topLeft":{"x":0,"y":0},"shapeType":"Square"}],"edges":[],"containers":{"0":[1]},"clusters":{}}),
        );
        assert_eq!(
            graph.nodes[0].rect.size,
            Size {
                width: 40.0,
                height: 20.0
            }
        );
        assert_eq!(graph.nodes[0].desired_width, Some(75.25));
        assert_eq!(graph.nodes[0].desired_height, None);
        let c = context(0);
        let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .packing_score_checked(&[node()], Some(NodeId(0)), &mut g)
                .unwrap(),
            1621.28125
        );
    }

    #[test]
    fn checked_fixed_origin_reports_container_cycle_after_actual_parent_visits() {
        let mut graph = plain();
        graph.nodes[0].container = Some(NodeId(0));
        let c = context(0);
        let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        let failure = graph
            .packing_fixed_bounds_checked(&[node()], &mut g)
            .unwrap_err();
        assert_eq!(
            failure.error.to_string(),
            "layout invariant violated: BinPack found a cycle in container ancestry"
        );
        assert_eq!(failure.value, (None, None));
        assert_eq!(g.used(), 4);
    }

    #[test]
    fn extremity_budget_check_precedes_a_nil_peer_dereference() {
        let graph = arena(
            json!({"nodes":[{"id":1,"width":80,"height":60,"topLeft":{"x":0,"y":0},"shapeType":"Square","label":{"Text":"wide","Position":"OUTSIDE_TOP_LEFT","Width":120,"Height":30}}],"edges":[],"containers":{"0":[1]},"clusters":{}}),
        );
        let c = context(0);
        let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 1).unwrap();
        let failure = graph
            .packing_node_bounds_checked(node(), Some(&[None]), &mut g)
            .unwrap_err();
        assert!(matches!(failure.error, limits::Error::Limit { .. }));
        assert_eq!(g.used(), 2);
        let c = context(0);
        let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 2).unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            graph.packing_node_bounds_checked(node(), Some(&[None]), &mut g)
        }))
        .unwrap_err();
        assert_eq!(
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied()),
            Some("runtime error: invalid memory address or nil pointer dereference")
        );
        assert_eq!(g.used(), 2);
    }

    #[test]
    fn empty_score_keeps_go_pow_nan_payload_and_all_finish_polls() {
        let graph = plain();
        let c = context(0);
        let mut g = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .packing_score_checked(&[], None, &mut g)
                .unwrap()
                .to_bits(),
            0x7ff8_0000_0000_0001
        );
        assert_eq!(g.used(), 0);
        assert_eq!(c.polls.get(), 5);
    }
}
