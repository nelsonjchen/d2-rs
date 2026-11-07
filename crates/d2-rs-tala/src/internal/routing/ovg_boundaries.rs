// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned ovg.go boundary layers, port connections, fallback and corners.
use super::ovg_intersections::{Budget, check, descendant, rect, step};
use super::ovg_ports::{PortBuild, PortList};
use super::route_geometry::recovered_segment_intersects_box;
use super::{ArenaGraph, NodeId, Point, Rect};
use std::collections::BTreeSet;

pub(super) type Bounds = Option<(Point, Point)>;
fn required(bounds: Bounds) -> (Point, Point) {
    bounds.expect("runtime error: invalid memory address or nil pointer dereference")
}
pub(super) fn directions(bits: u16) -> impl Iterator<Item = i64> {
    (0..=8).filter(move |&raw| {
        if bits == 0 {
            raw == 8
        } else {
            bits & (1 << raw) != 0
        }
    })
}
fn near_graph_node(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    point: Point,
    budget: Budget<'_, '_>,
) -> Result<bool, ()> {
    for &node in nodes {
        step(budget)?;
        if graph.nodes[node.0 as usize].is_container {
            continue;
        }
        let r = rect(graph, node);
        if r.origin.x - 20.0 <= point.x
            && r.origin.x + r.size.width + 20.0 >= point.x
            && r.origin.y - 20.0 <= point.y
            && r.origin.y + r.size.height + 20.0 >= point.y
        {
            return Ok(true);
        }
    }
    Ok(false)
}
impl PortBuild {
    // Most blockers have direction NONE. Keep their direction test and box
    // query outside the port walk, whose work guard needs a much larger frame.
    // ovg_resource.go:429-464 likewise skips the walk unless a port can exempt
    // this segment; its checks and point reads stay in the same order below.
    #[inline(always)]
    pub(super) fn passes_through_allowing_ports(
        &self,
        graph: &ArenaGraph,
        owner: NodeId,
        first: Point,
        second: Point,
        direction: i64,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        let allows_port = match direction {
            4 => first.x == second.x && first.y > second.y,
            6 => first.x == second.x && first.y < second.y,
            7 => first.y == second.y && first.x > second.x,
            5 => first.y == second.y && first.x < second.x,
            _ => false,
        };
        if allows_port && self.endpoint_is_port(owner, first, second, budget)? {
            return Ok(false);
        }
        // Source evaluates PassesThrough before guard.check; its nil box
        // origin yields false, unlike the proximity predicate's dereference.
        let blocked = match graph.position(owner) {
            Some(origin) => recovered_segment_intersects_box(
                first,
                second,
                Rect {
                    origin,
                    size: graph.nodes[owner.0 as usize].rect.size,
                },
            ),
            None => false,
        };
        check(budget)?;
        Ok(blocked)
    }

    #[inline(never)]
    fn endpoint_is_port(
        &self,
        owner: NodeId,
        first: Point,
        second: Point,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        // Only the exemption scan reads the owner's port list; the map is
        // not mutated by this predicate's context callbacks.
        let ports = self
            .ports
            .get(&owner)
            .map(PortList::nodes)
            .unwrap_or_default();
        for node in ports {
            step(budget)?;
            let point = node
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .point();
            if point == first || point == second {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn add_boundary_layers(
        &mut self,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        bounds: Bounds,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        for layer in 1..=3 {
            step(budget)?;
            let count = self.members.len();
            for slot in 0..count {
                let index = self.members[slot];
                step(budget)?;
                let point = self.nodes[index].point();
                let (tl, br) = required(bounds);
                let delta = 20.0 * f64::from(layer);
                let mut boundary = None;
                // Each later matching side overwrites the previous candidate.
                if point.y == tl.y {
                    boundary = Some(Point {
                        x: point.x,
                        y: point.y - delta,
                    });
                }
                if point.y == br.y {
                    boundary = Some(Point {
                        x: point.x,
                        y: point.y + delta,
                    });
                }
                if point.x == tl.x {
                    boundary = Some(Point {
                        x: point.x - delta,
                        y: point.y,
                    });
                }
                if point.x == br.x {
                    boundary = Some(Point {
                        x: point.x + delta,
                        y: point.y,
                    });
                }
                if let Some(boundary) = boundary
                    && !near_graph_node(graph, graph_nodes, boundary, budget)?
                {
                    self.add_point(boundary, budget)?;
                }
            }
        }
        Ok(())
    }

    fn create_port_boundary_connections(
        &mut self,
        owner: NodeId,
        ports: &[usize],
        bounds: Bounds,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        let mut seen = BTreeSet::new();
        for &index in ports {
            step(budget)?;
            if !seen.insert(index) {
                continue;
            }
            for direction in directions(self.port_directions(index, owner)) {
                step(budget)?;
                let point = self.nodes[index].point();
                let point = match direction {
                    4 => Point {
                        x: point.x,
                        y: point.y - 20.0,
                    },
                    6 => Point {
                        x: point.x,
                        y: point.y + 20.0,
                    },
                    7 => Point {
                        x: point.x - 20.0,
                        y: point.y,
                    },
                    5 => Point {
                        x: point.x + 20.0,
                        y: point.y,
                    },
                    _ => continue,
                };
                let new_node = self.add_point(point, budget)?;
                for layer in 1..=3 {
                    step(budget)?;
                    let (tl, br) = required(bounds);
                    // addPoint may reuse an earlier signed-zero representative.
                    let point = self.nodes[new_node].point();
                    let delta = 20.0 * f64::from(layer);
                    let pair = if direction == 4 || direction == 6 {
                        [
                            Point {
                                x: tl.x - delta,
                                y: point.y,
                            },
                            Point {
                                x: br.x + delta,
                                y: point.y,
                            },
                        ]
                    } else {
                        [
                            Point {
                                x: point.x,
                                y: tl.y - delta,
                            },
                            Point {
                                x: point.x,
                                y: br.y + delta,
                            },
                        ]
                    };
                    self.add_point(pair[0], budget)?;
                    self.add_point(pair[1], budget)?;
                }
            }
        }
        check(budget)
    }

    pub(super) fn add_port_boundary_nodes(
        &mut self,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        rectangles: &[NodeId],
        bounds: Bounds,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        for layer in 1..=3 {
            step(budget)?;
            // Go permits varying Ports-map traversal. This stable native
            // traversal is not a claim of a guaranteed source map order.
            for owner in self.port_order.clone() {
                step(budget)?;
                let ports = self.ports[&owner]
                    .iter()
                    .map(|&(index, _)| index)
                    .collect::<Vec<_>>();
                let mut added = false;
                let mut seen = BTreeSet::new();
                for &index in &ports {
                    step(budget)?;
                    if !seen.insert(index) {
                        continue;
                    }
                    for direction in directions(self.port_directions(index, owner)) {
                        step(budget)?;
                        let point = self.nodes[index].point();
                        let (tl, br) = required(bounds);
                        let delta = 20.0 * f64::from(layer);
                        let boundaries = [
                            Point {
                                x: point.x,
                                y: tl.y - delta,
                            },
                            Point {
                                x: point.x,
                                y: br.y + delta,
                            },
                            Point {
                                x: tl.x - delta,
                                y: point.y,
                            },
                            Point {
                                x: br.x + delta,
                                y: point.y,
                            },
                        ];
                        for boundary in boundaries {
                            step(budget)?;
                            let mut blocked = false;
                            for &rectangle in rectangles {
                                step(budget)?;
                                if self.passes_through_allowing_ports(
                                    graph, rectangle, point, boundary, direction, budget,
                                )? {
                                    let ancestor_container = graph.nodes[rectangle.0 as usize]
                                        .is_container
                                        && descendant(graph, Some(owner), rectangle, budget)?;
                                    if !ancestor_container {
                                        blocked = true;
                                        break;
                                    }
                                }
                            }
                            if blocked {
                                continue;
                            }
                            if !near_graph_node(graph, graph_nodes, boundary, budget)? {
                                self.add_point(boundary, budget)?;
                                added = true;
                            }
                        }
                    }
                }
                if !added {
                    self.create_port_boundary_connections(owner, &ports, bounds, budget)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn add_corners(
        &mut self,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        bounds: Bounds,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        for layer in 1..=3 {
            step(budget)?;
            let (tl, br) = required(bounds);
            let delta = 20.0 * f64::from(layer);
            let corners = [
                Point {
                    x: tl.x - delta,
                    y: tl.y - delta,
                },
                Point {
                    x: br.x + delta,
                    y: tl.y - delta,
                },
                Point {
                    x: br.x + delta,
                    y: br.y + delta,
                },
                Point {
                    x: tl.x - delta,
                    y: br.y + delta,
                },
            ];
            for corner in corners {
                if !near_graph_node(graph, graph_nodes, corner, budget)? {
                    self.add_point(corner, budget)?;
                }
            }
        }
        check(budget)
    }
}

#[cfg(test)]
mod tests {
    use super::super::route_search_guard::OvgBuildBudget;
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;
    fn graph() -> ArenaGraph {
        let input = serde_json::json!({"nodes":[{"id":1,"width":0,"height":0,"topLeft":{"x":0,"y":0}}],
            "edges":[{"id":11,"from":1,"to":1,"points":[]}],"containers":{"0":[1]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&input).unwrap()).unwrap(),
        )
    }
    struct Caller {
        polls: Cell<u64>,
        cancel_at: u64,
    }
    impl Context for Caller {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let n = self.polls.get() + 1;
            self.polls.set(n);
            (self.cancel_at != 0 && n >= self.cancel_at).then_some(ContextError::Cancelled)
        }
    }
    #[test]
    fn fallback_visits_every_coincident_port_direction_once() {
        let graph = graph();
        let mut build = PortBuild::default();
        build.add_ports(&graph, &[NodeId(0)], None).unwrap();
        let ports = build.ports[&NodeId(0)]
            .iter()
            .map(|&(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(ports.len(), 12);
        assert_eq!(build.nodes.len(), 1);
        build
            .create_port_boundary_connections(
                NodeId(0),
                &ports,
                Some((Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 })),
                None,
            )
            .unwrap();
        assert_eq!(build.nodes.len(), 25);
        for point in [
            Point { x: 0.0, y: -20.0 },
            Point { x: 20.0, y: 0.0 },
            Point { x: 0.0, y: 20.0 },
            Point { x: -20.0, y: 0.0 },
            Point { x: -60.0, y: 20.0 },
            Point { x: 20.0, y: -60.0 },
        ] {
            assert!(build.nodes.iter().any(|node| node.read().point() == point));
        }
    }
    #[test]
    fn nil_bounds_are_read_after_the_guarded_layer_and_node_steps() {
        let graph = graph();
        let mut build = PortBuild::default();
        build.add_boundary_layers(&graph, &[], None, None).unwrap();
        build.add_point(Point { x: 0.0, y: 0.0 }, None).unwrap();
        let context = Caller {
            polls: Cell::new(0),
            cancel_at: 3,
        };
        let budget = OvgBuildBudget::with_context(&context);
        assert!(
            build
                .add_boundary_layers(&graph, &[], None, Some(&budget))
                .is_err()
        );
        assert_eq!(context.polls.get(), 3);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build.add_boundary_layers(&graph, &[], None, None)
        }));
        assert!(panic.is_err());
    }
    #[test]
    fn box_clipping_rejects_nan_bounds_and_propagates_nan_clip_values() {
        let first = Point { x: -10.0, y: 0.0 };
        let second = Point { x: 10.0, y: 0.0 };
        let mut bounds = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: crate::Size {
                width: f64::NAN,
                height: 40.0,
            },
        };
        assert!(!recovered_segment_intersects_box(first, second, bounds));
        bounds.size.width = 40.0;
        assert!(!recovered_segment_intersects_box(
            Point {
                x: f64::NAN,
                y: -10.0
            },
            Point { x: 50.0, y: 50.0 },
            bounds
        ));
    }
    #[test]
    fn nil_origin_does_not_panic_in_passes_through_and_still_checks_context() {
        let mut graph = graph();
        let nil = graph.allocate_owner_node_point(None);
        graph.bind_owner_node_point(NodeId(0), nil).unwrap();
        let build = PortBuild::default();
        let context = Caller {
            polls: Cell::new(0),
            cancel_at: 1,
        };
        let budget = OvgBuildBudget::with_context(&context);
        assert!(
            build
                .passes_through_allowing_ports(
                    &graph,
                    NodeId(0),
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 100.0, y: 0.0 },
                    8,
                    Some(&budget)
                )
                .is_err()
        );
        assert_eq!(context.polls.get(), 1);
        assert!(
            !build
                .passes_through_allowing_ports(
                    &graph,
                    NodeId(0),
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 100.0, y: 0.0 },
                    8,
                    None
                )
                .unwrap()
        );
    }
}
