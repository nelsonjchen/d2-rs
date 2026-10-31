// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Source-ordered hierarchy and placement candidate checks for BinPack.
use super::binpack_traversal::PackingNodeRef;
use super::checked_bounds::BoundsFailure;
use super::*;

pub(super) type HierarchyBox = (Option<Point>, Option<Point>);
type Boxes = Option<Vec<HierarchyBox>>;
type Candidates = Option<Vec<Point>>;

fn placed(p: Option<Point>) -> Point {
    p.expect("runtime error: invalid memory address or nil pointer dereference")
}
fn node(n: Option<PackingNodeRef>) -> PackingNodeRef {
    n.expect("runtime error: invalid memory address or nil pointer dereference")
}

impl ArenaGraph {
    fn packing_has_hierarchy(&self, n: Option<PackingNodeRef>) -> bool {
        matches!(node(n), PackingNodeRef::Node(n) if self.nodes[n.0 as usize].hierarchy.is_some())
    }

    pub(super) fn packing_hierarchy_boxes_checked(
        &self,
        groups: &[Vec<Option<PackingNodeRef>>],
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Boxes, BoundsFailure<Boxes>> {
        let mut boxes = Vec::new();
        for group in groups {
            guard.step()?;
            if group.is_empty() {
                return Err(limits::Error::Invalid(
                    "layout invariant violated: BinPack contains an empty packed subgraph".into(),
                )
                .into());
            }
            if !self.packing_has_hierarchy(group[0]) {
                continue;
            }
            let (low, high) = self
                .packing_bounds_checked(group, guard)
                .map_err(|f| f.error)?;
            boxes.push((Some(low), Some(high)));
        }
        let value = Some(boxes);
        guard.finish().map_err(|error| BoundsFailure {
            value: value.clone(),
            error,
        })?;
        Ok(value)
    }

    pub(super) fn packing_point_in_hierarchy_checked(
        boxes: &[HierarchyBox],
        point: Option<Point>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let point = point.ok_or_else(|| {
            limits::Error::Invalid(
                "layout invariant violated: BinPack hierarchy check received a nil point".into(),
            )
        })?;
        for &(low, high) in boxes {
            guard.step()?;
            // Preserve source short circuits, including nullable diagnostic corners.
            if placed(low).x <= point.x
                && point.x <= placed(high).x
                && placed(low).y <= point.y
                && point.y <= placed(high).y
            {
                return Ok(true);
            }
        }
        guard.finish()?;
        Ok(false)
    }

    pub(super) fn packing_placement_candidates_checked(
        &self,
        nodes: &[Option<PackingNodeRef>],
        root: Option<PackingNodeRef>,
        consider_within: bool,
        edges_placed: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Candidates, BoundsFailure<Candidates>> {
        let mut candidates = Vec::new();
        let mut contained = Vec::new();
        for &n in nodes {
            guard.step()?;
            let n = node(n);
            if self.packing_raw_container(n) != root {
                continue;
            }
            contained.push(Some(n));
            if !consider_within {
                continue;
            }
            let (position, size, metadata) = self.packing_bounds_geometry(n);
            let side_delta = if metadata.is_some_and(|n| n.shape == ShapeKind::SqlTable) {
                120.0
            } else {
                20.0
            };
            let (mut right, mut bottom, mut diagonal) = (false, false, false);
            if !edges_placed {
                for edge in self.packing_segment_incidents(n) {
                    guard.step()?;
                    let edge = edge
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    let endpoint = |is_to| {
                        self.owner_edge_endpoint(edge, is_to)
                            .expect("packing edge allocation")
                            .map(|n| self.packing_effective_node(n))
                    };
                    let from = endpoint(false);
                    let adjacent = if from == Some(n) {
                        endpoint(true)
                    } else {
                        from
                    };
                    // Node.orientation first checks the receiver's nil TopLeft.
                    // Only a positioned receiver dereferences the adjacent node.
                    let orientation = if let Some(position) = position {
                        let (other, other_size, _) = self.packing_bounds_geometry(node(adjacent));
                        other.map_or(Orientation::None, |other| {
                            self.sized_box_orientation((position, size), (other, other_size))
                        })
                    } else {
                        Orientation::None
                    };
                    match orientation {
                        Orientation::Left => right = true,
                        Orientation::Top => bottom = true,
                        Orientation::TopLeft => diagonal = true,
                        _ => {}
                    }
                    if right && bottom && diagonal {
                        break;
                    }
                }
            }
            if !right {
                let p = placed(position);
                candidates.push(Point {
                    x: p.x + size.width + side_delta,
                    y: p.y,
                });
            }
            if !bottom {
                let p = placed(position);
                candidates.push(Point {
                    x: p.x,
                    y: p.y + size.height + 20.0,
                });
            }
            if !diagonal {
                let p = placed(position);
                candidates.push(Point {
                    x: p.x + size.width + side_delta,
                    y: p.y + size.height,
                });
            }
        }
        let (low, high) = self
            .packing_fixed_bounds_checked(&contained, guard)
            .map_err(|f| f.error)?;
        candidates.extend([
            Point {
                x: high.x,
                y: low.y,
            },
            high,
            Point {
                x: low.x,
                y: high.y,
            },
            Point {
                x: high.x + 20.0,
                y: low.y,
            },
            Point {
                x: high.x + 20.0,
                y: high.y,
            },
            Point {
                x: low.x,
                y: high.y + 20.0,
            },
        ]);
        let value = Some(candidates);
        guard.finish().map_err(|error| BoundsFailure {
            value: value.clone(),
            error,
        })?;
        Ok(value)
    }

    /// The caller block at binpack.go:504-555. A failure discards local candidates;
    /// no candidate evaluation or removal of the next group has happened yet.
    pub(super) fn packing_prepare_candidates_checked(
        &self,
        packed: &[Vec<Option<PackingNodeRef>>],
        root: Option<PackingNodeRef>,
        root_top_left: Option<Point>,
        edges_placed: bool,
        occupied: &[Point],
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(Vec<(Point, Point)>, Vec<Point>, Point), limits::Error> {
        let boxes = self
            .packing_hierarchy_boxes_checked(packed, guard)
            .map_err(|f| f.error)?
            .unwrap();
        let mut candidates = vec![placed(root_top_left)];
        let mut low = Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        for group in packed {
            guard.step()?;
            let (group_low, _) = self
                .packing_bounds_checked(group, guard)
                .map_err(|f| f.error)?;
            low.x = routing::channel_solver::minimum(low.x, group_low.x);
            low.y = routing::channel_solver::minimum(low.y, group_low.y);
            let placements = self
                .packing_placement_candidates_checked(
                    group,
                    root,
                    !self.packing_has_hierarchy(group[0]),
                    edges_placed,
                    guard,
                )
                .map_err(|f| f.error)?
                .unwrap();
            for point in placements {
                guard.step()?;
                if occupied.contains(&point) {
                    continue;
                }
                if Self::packing_point_in_hierarchy_checked(&boxes, Some(point), guard)? {
                    continue;
                }
                if !candidates.contains(&point) {
                    candidates.push(point);
                }
            }
        }
        Ok((
            boxes
                .into_iter()
                .map(|(a, b)| (placed(a), placed(b)))
                .collect(),
            candidates,
            low,
        ))
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
    fn graph() -> ArenaGraph {
        let mut input = Graph::default();
        for i in 0..4 {
            input.add_node(crate::Node::new(
                i.to_string(),
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ));
        }
        for i in 1..4 {
            input.add_edge(crate::Edge {
                source: NodeId(0),
                target: NodeId(i),
            });
        }
        let mut graph = ArenaGraph::from_input(&input);
        for (i, p) in [
            (0, (0.0, 0.0)),
            (1, (100.0, 0.0)),
            (2, (0.0, 100.0)),
            (3, (100.0, 100.0)),
        ] {
            graph.set_position(NodeId(i), Point { x: p.0, y: p.1 });
        }
        graph.initialize_shared_topology();
        graph
    }
    #[test]
    fn hierarchy_positive_hit_skips_finish_and_preserves_corner_short_circuits() {
        let boxes = [(
            Some(Point { x: 0.0, y: 0.0 }),
            Some(Point { x: 10.0, y: 10.0 }),
        )];
        let c = context(2);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            ArenaGraph::packing_point_in_hierarchy_checked(
                &boxes,
                Some(Point { x: 10.0, y: 10.0 }),
                &mut guard
            )
            .unwrap()
        );
        assert_eq!(c.polls.get(), 1);
        assert_eq!(guard.used(), 1);
        let c = context(2);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            ArenaGraph::packing_point_in_hierarchy_checked(
                &boxes,
                Some(Point { x: 11.0, y: 10.0 }),
                &mut guard
            )
            .is_err()
        );
        let c = context(0);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            !ArenaGraph::packing_point_in_hierarchy_checked(
                &[(Some(Point { x: 1.0, y: 0.0 }), None)],
                Some(Point { x: 0.0, y: 0.0 }),
                &mut guard
            )
            .unwrap()
        );
    }
    #[test]
    fn candidate_direction_short_circuit_does_not_visit_a_later_nil_edge() {
        let mut graph = graph();
        let header = graph
            .allocate_owner_incident_header(
                &[Some(EdgeId(0)), Some(EdgeId(1)), Some(EdgeId(2)), None],
                4,
                true,
            )
            .unwrap();
        graph
            .set_owner_node_incident_header(NodeId(0), header)
            .unwrap();
        let c = context(0);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        let out = graph
            .packing_placement_candidates_checked(
                &[Some(PackingNodeRef::Node(NodeId(0)))],
                None,
                true,
                false,
                &mut guard,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            out.len(),
            6,
            "all three directional candidates are blocked before the nil incidence"
        );
    }
    #[test]
    fn candidate_finish_keeps_values_but_inner_bounds_failure_discards_them() {
        let graph = graph();
        let nodes = [Some(PackingNodeRef::Node(NodeId(0)))];
        let c = context(0);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        let expected = graph
            .packing_placement_candidates_checked(&nodes, None, true, true, &mut guard)
            .unwrap();
        let final_poll = c.polls.get();
        let c = context(final_poll);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert_eq!(
            graph
                .packing_placement_candidates_checked(&nodes, None, true, true, &mut guard)
                .unwrap_err()
                .value,
            expected
        );
        let c = context(final_poll - 1);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        assert!(
            graph
                .packing_placement_candidates_checked(&nodes, None, true, true, &mut guard)
                .unwrap_err()
                .value
                .is_none()
        );
    }
    #[test]
    fn candidate_preparation_validates_groups_before_reading_the_root_point() {
        let graph = graph();
        let c = context(0);
        let mut guard = limits::WorkGuard::new(Some(&c), "BinPack", 100).unwrap();
        let err = graph
            .packing_prepare_candidates_checked(&[vec![]], None, None, false, &[], &mut guard)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "layout invariant violated: BinPack contains an empty packed subgraph"
        );
        assert_eq!(guard.used(), 1);
    }
}

/// Inputs retained across every speculative Commit for one current group.
/// These are source pointers, not a node list rebuilt from trial geometry.
pub(super) struct PackingPreparedTrial {
    pub(super) scoring_nodes: Vec<Option<PackingNodeRef>>,
    pub(super) packed_deltas: (f64, f64),
}

impl ArenaGraph {
    /// binpack.go:587-617: materialize packedWithCurr, append copies of the
    /// original current positions, then measure the packed groups once.
    pub(super) fn packing_prepare_trial_checked(
        &self,
        children: &[Option<PackingNodeRef>],
        current: &[Option<PackingNodeRef>],
        packed: &[Vec<Option<PackingNodeRef>>],
        originals: &super::binpack_staging::PackingStaging,
        candidates: &mut Vec<Point>,
        seen: &mut Vec<Point>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<PackingPreparedTrial, limits::Error> {
        let mut scoring_nodes = Vec::with_capacity(children.len() + current.len());
        for &node in children {
            guard.step()?;
            scoring_nodes.push(node);
        }
        for &node in current {
            guard.step()?;
            scoring_nodes.push(node);
        }
        for &node in current {
            guard.step()?;
            // A missing entry in Go's map has the zero Point value. Copying
            // the value must not read the member's current translated point.
            let point = node
                .map(|node| originals.position_value(node))
                .unwrap_or_default();
            if !seen.contains(&point) {
                candidates.push(point);
                seen.push(point);
            }
        }
        let packed_deltas = self.packing_smallest_deltas_checked(packed, guard)?;
        Ok(PackingPreparedTrial {
            scoring_nodes,
            packed_deltas,
        })
    }
}

