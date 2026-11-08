// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned ovg.go ordinary line connections and per-owner eligibility.
use super::ovg_boundaries::directions;
use super::ovg_groups::group;
use super::ovg_intersections::{AxisMap, Budget, check, descendant, rect, step};
use super::ovg_ports::PortBuild;
use super::{ArenaGraph, NodeId, Point};
use crate::internal::owner_groups::OwnerGroupKind::Sequence;
use std::collections::BTreeSet;

fn sort_work(budget: Budget<'_, '_>, n: usize) -> Result<(), ()> {
    budget.map_or(Ok(()), |b| b.reserve_sort_work(n))
}
pub(super) fn edge_capacity(
    existing: u64,
    nodes: u64,
    horizontal: u64,
    vertical: u64,
    limit: u64,
    int_limit: u64,
) -> Result<usize, String> {
    const PREFIX: &str = "TALA OVG resource limit exceeded";
    if existing > limit {
        return Err(format!(
            "{PREFIX}: edge count {existing} exceeds limit {limit}"
        ));
    }
    let overflow = || format!("{PREFIX}: edge capacity arithmetic overflow");
    let twice = nodes.checked_mul(2).ok_or_else(overflow)?;
    let lines = horizontal.checked_add(vertical).ok_or_else(overflow)?;
    let extra = twice.checked_sub(lines).ok_or_else(overflow)?;
    let capacity = existing.checked_add(extra).ok_or_else(overflow)?.min(limit);
    if capacity > int_limit {
        return Err(format!(
            "{PREFIX}: allocation capacity {capacity} exceeds limit {int_limit}"
        ));
    }
    Ok(capacity as usize)
}
fn sorted_keys(lines: &AxisMap<Vec<usize>>, budget: Budget<'_, '_>) -> Result<Vec<f64>, ()> {
    let mut keys = Vec::new();
    for (key, _) in lines.entries() {
        step(budget)?;
        keys.push(*key);
    }
    sort_work(budget, keys.len())?;
    keys.sort_by(|a, b| {
        if a.is_nan() {
            if b.is_nan() {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Less
            }
        } else if b.is_nan() {
            std::cmp::Ordering::Greater
        } else {
            a.partial_cmp(b).unwrap()
        }
    });
    Ok(keys)
}
fn contains(graph: &ArenaGraph, owner: NodeId, point: Point) -> bool {
    let r = rect(graph, owner);
    r.origin.x <= point.x
        && r.origin.x + r.size.width >= point.x
        && r.origin.y <= point.y
        && r.origin.y + r.size.height >= point.y
}
impl PortBuild {
    pub(super) fn restricted_sequence_port(
        &self,
        graph: &ArenaGraph,
        index: usize,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        let mut any = false;
        for (owner, bits) in self.owner_directions(index) {
            any = true;
            step(budget)?;
            let Some(sequence) = group(graph, owner, Sequence) else {
                return Ok(false);
            };
            for direction in directions(bits) {
                let allowed = match direction {
                    7 => sequence.first(graph) == Some(owner),
                    5 => sequence.last(graph) == Some(owner),
                    _ => true,
                };
                if allowed {
                    return Ok(false);
                }
            }
        }
        check(budget)?;
        Ok(any)
    }
    pub(super) fn misdirected_pair(
        &self,
        graph: &ArenaGraph,
        a: usize,
        b: usize,
        horizontal: bool,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        check(budget)?;
        if self.owner_directions(a).next().is_none() || self.owner_directions(b).next().is_none() {
            check(budget)?;
            return Ok(false);
        }
        for (first, first_bits) in self.owner_directions(a) {
            step(budget)?;
            for (second, second_bits) in self.owner_directions(b) {
                step(budget)?;
                if first == second {
                    return Ok(false);
                }
                let correct = if horizontal {
                    first_bits & (1 << 5) != 0 && second_bits & (1 << 7) != 0
                } else {
                    first_bits & (1 << 6) != 0 && second_bits & (1 << 4) != 0
                };
                let below_second = descendant(graph, Some(first), second, budget)?;
                let below_first = descendant(graph, Some(second), first, budget)?;
                if correct || below_second || below_first {
                    return Ok(false);
                }
            }
        }
        check(budget)?;
        Ok(true)
    }
    fn passes_obstacle(
        &self,
        graph: &ArenaGraph,
        from: usize,
        to: usize,
        owner: NodeId,
        budget: Budget<'_, '_>,
    ) -> Result<bool, ()> {
        for direction in directions(self.port_directions(from, owner)) {
            if !self.passes_through_allowing_ports(
                graph,
                owner,
                self.nodes[from].point(),
                self.nodes[to].point(),
                direction,
                budget,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn connect_line(
        &mut self,
        graph: &ArenaGraph,
        bounding: &[NodeId],
        nodes: &mut [usize],
        position: f64,
        horizontal: bool,
        fixed: &BTreeSet<NodeId>,
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        let mut candidates = Vec::new();
        for &node in bounding {
            step(budget)?;
            if fixed.contains(&node) {
                continue;
            }
            let r = rect(graph, node);
            if if horizontal {
                r.origin.y <= position && r.origin.y + r.size.height >= position
            } else {
                r.origin.x <= position && r.origin.x + r.size.width >= position
            } {
                candidates.push(node);
            }
        }
        sort_work(budget, candidates.len())?;
        crate::internal::go_sort::sort_by(&mut candidates, |a, b| {
            let a = rect(graph, *a);
            let b = rect(graph, *b);
            if horizontal {
                a.origin.x < b.origin.x
            } else {
                a.origin.y < b.origin.y
            }
        });
        sort_work(budget, nodes.len())?;
        crate::internal::go_sort::sort_by(nodes, |a, b| {
            let a = self.nodes[*a].point();
            let b = self.nodes[*b].point();
            if horizontal { a.x < b.x } else { a.y < b.y }
        });
        let mut start = 0;
        for pair in nodes.windows(2) {
            step(budget)?;
            let a = pair[0];
            let b = pair[1];
            if self.misdirected_pair(graph, a, b, horizontal, budget)? {
                continue;
            }
            let mut connect = true;
            for (j, &owner) in candidates.iter().enumerate().skip(start) {
                step(budget)?;
                let r = rect(graph, owner);
                let first = self.nodes[a].point();
                let second = self.nodes[b].point();
                if (horizontal && r.origin.x > second.x) || (!horizontal && r.origin.y > second.y) {
                    break;
                }
                let c1 = self.passes_obstacle(graph, a, b, owner, budget)?;
                let c2 = self.passes_obstacle(graph, b, a, owner, budget)?;
                if c1 && c2 {
                    if group(graph, owner, Sequence).is_some() && !horizontal {
                        let r = rect(graph, owner);
                        if r.origin.y == second.y || r.origin.y + r.size.height == first.y {
                            continue;
                        }
                    }
                    if graph.nodes[owner.0 as usize].is_container
                        && (contains(graph, owner, first) || contains(graph, owner, second))
                    {
                        continue;
                    }
                    connect = false;
                    break;
                } else {
                    start = j;
                }
            }
            if connect {
                self.connect(a, b, budget)?;
            }
        }
        Ok(())
    }
    pub(super) fn connect_nodes(
        &mut self,
        graph: &ArenaGraph,
        bounding: &[NodeId],
        budget: Budget<'_, '_>,
    ) -> Result<(), String> {
        // Preserve the original context error for the public routing caller.
        let failure = || "TALA OVG resource limit exceeded".to_owned();
        check(budget).map_err(|_| failure())?;
        if let Some(b) = budget {
            let used = b.reserved_edge_count();
            if self.edges.len() as u64 > used {
                b.reserve_edges(self.edges.len() as u64 - used)
                    .map_err(|_| failure())?;
            }
        }
        let mut horizontal = AxisMap::<Vec<usize>>::default();
        let mut vertical = AxisMap::<Vec<usize>>::default();
        let mut eligible = 0;
        for &index in &self.members {
            step(budget).map_err(|_| failure())?;
            if self.nodes[index].read().is_tunnel() {
                continue;
            }
            if self
                .restricted_sequence_port(graph, index, budget)
                .map_err(|_| failure())?
            {
                continue;
            }
            eligible += 1;
            horizontal.entry(self.nodes[index].point().y).push(index);
            vertical.entry(self.nodes[index].point().x).push(index);
        }
        let capacity = edge_capacity(
            self.edges.len() as u64,
            eligible,
            horizontal.len() as u64,
            vertical.len() as u64,
            super::route_search_guard::MAX_OVG_EDGES,
            isize::MAX as u64,
        )?;
        let mut edges = Vec::with_capacity(capacity);
        edges.extend(self.edges.iter().cloned());
        self.edges = edges;
        self.edges_allocated = true;
        let fixed = self
            .fixed_overlaps(graph, bounding, budget)
            .map_err(|_| failure())?;
        for y in sorted_keys(&horizontal, budget).map_err(|_| failure())? {
            step(budget).map_err(|_| failure())?;
            // NaN map keys never look up their own entries in Go.
            let mut nodes = horizontal.get(y).cloned().unwrap_or_default();
            self.connect_line(graph, bounding, &mut nodes, y, true, &fixed, budget)
                .map_err(|_| failure())?;
        }
        for x in sorted_keys(&vertical, budget).map_err(|_| failure())? {
            step(budget).map_err(|_| failure())?;
            let mut nodes = vertical.get(x).cloned().unwrap_or_default();
            self.connect_line(graph, bounding, &mut nodes, x, false, &fixed, budget)
                .map_err(|_| failure())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::routing::route_search_guard::OvgBuildBudget;
    fn graph(container: bool) -> ArenaGraph {
        let value = serde_json::json!({"nodes":[{"id":1,"width":100,"height":100,"topLeft":{"x":0,"y":0}}],"edges":[],"containers":{"0":[1]}});
        let input =
            crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap();
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[0].is_container = container;
        graph
    }
    #[test]
    fn connection_failure_preserves_the_callers_context_error() {
        struct Cancelled;
        impl crate::internal::limits::Context for Cancelled {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<crate::internal::limits::ContextError> {
                Some(crate::internal::limits::ContextError::Cancelled)
            }
        }
        let graph = graph(false);
        let budget = OvgBuildBudget::with_context(&Cancelled);
        let mut ports = PortBuild::default();
        assert!(ports.connect_nodes(&graph, &[], Some(&budget)).is_err());
        assert_eq!(
            budget.take_context_error().unwrap().to_string(),
            "EdgeRouting: context canceled"
        );
    }
    #[test]
    fn edge_capacity_checks_before_clamping_and_preserves_error_order() {
        assert_eq!(edge_capacity(3, 6, 2, 3, 20, 100).unwrap(), 10);
        assert_eq!(edge_capacity(3, 6, 2, 3, 8, 100).unwrap(), 8);
        assert!(
            edge_capacity(21, u64::MAX, 0, 0, 20, 100)
                .unwrap_err()
                .contains("edge count 21")
        );
        assert!(
            edge_capacity(0, u64::MAX, 0, 0, 20, 100)
                .unwrap_err()
                .contains("arithmetic overflow")
        );
        assert!(
            edge_capacity(0, 1, 2, 1, 20, 100)
                .unwrap_err()
                .contains("arithmetic overflow")
        );
        assert!(
            edge_capacity(3, 6, 2, 3, 20, 9)
                .unwrap_err()
                .contains("allocation capacity 10")
        );
    }
    #[test]
    fn nan_lines_are_not_retrievable_but_allocate_an_empty_edge_slice() {
        let graph = graph(false);
        let mut ports = PortBuild::default();
        ports
            .add_point(
                Point {
                    x: f64::NAN,
                    y: f64::NAN,
                },
                None,
            )
            .unwrap();
        ports
            .add_point(
                Point {
                    x: f64::NAN,
                    y: f64::NAN,
                },
                None,
            )
            .unwrap();
        ports.connect_nodes(&graph, &[], None).unwrap();
        assert!(ports.edges.is_empty());
        assert_eq!(ports.edges.capacity(), 0);
        assert!(ports.edges_allocated);
    }
    #[test]
    fn container_exception_includes_interior_points() {
        for container in [false, true] {
            let graph = graph(container);
            let mut ports = PortBuild::default();
            ports.add_point(Point { x: 20.0, y: 50.0 }, None).unwrap();
            ports.add_point(Point { x: 40.0, y: 50.0 }, None).unwrap();
            let budget = OvgBuildBudget::new();
            ports
                .connect_line(
                    &graph,
                    &[NodeId(0)],
                    &mut [0, 1],
                    50.0,
                    true,
                    &BTreeSet::new(),
                    Some(&budget),
                )
                .unwrap();
            assert_eq!(ports.edges.len(), usize::from(container));
            if container {
                assert_eq!(ports.edges[0].read().distance, 20.0);
            }
        }
    }
}
