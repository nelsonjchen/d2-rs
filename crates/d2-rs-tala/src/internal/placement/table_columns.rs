// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! SQL-table column endpoint geometry shared by placement, scoring, and routing.
//!
//! D2 serializes an edge against the owning table plus an optional zero-based
//! row index at either endpoint. Recovered TALA keeps that metadata separate
//! from node identity and derives the precise left/right row port only after
//! the current endpoint boxes are known.

use super::routing::{PortSide, nth_port_point_on_side};
use super::scoring_context::{ScoreContext, score_interval, score_poll};
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct FacingTablePorts {
    pub(super) source: Option<Point>,
    pub(super) target: Option<Point>,
    pub(super) orientation: Orientation,
}

impl ArenaGraph {
    fn table_column_port(
        &self,
        node: NodeId,
        position: Point,
        size: Size,
        original_column_count: Option<usize>,
        side: PortSide,
        index: usize,
    ) -> Option<Point> {
        let node = &self.nodes[node.0 as usize];
        nth_port_point_on_side(
            Rect {
                origin: position,
                size,
            },
            if original_column_count.is_some() {
                ShapeKind::SqlTable
            } else {
                node.shape
            },
            original_column_count.or(node.table_column_count),
            side,
            index,
        )
    }

    /// Direct translation of recovered `Edge.getFacingTablePorts` for the
    /// supplied live/replacement endpoint boxes.
    pub(super) fn facing_table_ports_for_boxes(
        &self,
        edge_id: EdgeId,
        source_box: (Point, Size),
        target_box: (Point, Size),
    ) -> FacingTablePorts {
        self.facing_table_ports_for_boxes_with_owner_nodes(edge_id, source_box, target_box, None)
    }

    fn facing_table_ports_for_boxes_with_owner_nodes(
        &self,
        edge_id: EdgeId,
        source_box: (Point, Size),
        target_box: (Point, Size),
        owner_nodes: Option<(NodeId, NodeId)>,
    ) -> FacingTablePorts {
        let edge = &self.edge_records[edge_id.0 as usize];
        let (source_node, target_node) = owner_nodes.unwrap_or((edge.from, edge.to));
        if !edge.has_table_column() {
            return FacingTablePorts {
                source: None,
                target: None,
                orientation: Orientation::None,
            };
        }

        let orientation = self.sized_box_orientation(source_box, target_box);
        let (source_side, target_side) = match orientation {
            Orientation::TopRight | Orientation::Right | Orientation::BottomRight => {
                (PortSide::Left, PortSide::Right)
            }
            Orientation::TopLeft | Orientation::Left | Orientation::BottomLeft => {
                (PortSide::Right, PortSide::Left)
            }
            _ => {
                return FacingTablePorts {
                    source: None,
                    target: None,
                    orientation: Orientation::None,
                };
            }
        };

        FacingTablePorts {
            source: edge.source_table_column.and_then(|index| {
                self.table_column_port(
                    source_node,
                    source_box.0,
                    source_box.1,
                    if owner_nodes.is_some() {
                        None
                    } else {
                        edge.source_table_column_count
                    },
                    source_side,
                    index,
                )
            }),
            target: edge.target_table_column.and_then(|index| {
                self.table_column_port(
                    target_node,
                    target_box.0,
                    target_box.1,
                    if owner_nodes.is_some() {
                        None
                    } else {
                        edge.target_table_column_count
                    },
                    target_side,
                    index,
                )
            }),
            orientation,
        }
    }

    /// Direct translation of recovered
    /// `Graph.computeDistanceBetweenTableColumns`.
    pub(super) fn table_column_distance_for_boxes(
        &self,
        edge_id: EdgeId,
        source_box: (Point, Size),
        target_box: (Point, Size),
    ) -> f64 {
        self.table_column_distance_for_boxes_with_owner_nodes(
            edge_id,
            source_box,
            target_box,
            self.cell_size,
            None,
        )
    }

    /// Source geometry.go160 uses the scoring node Graph.CellSize and the
    /// actual replacement shapes, including their live row counts.
    pub(super) fn table_column_distance_for_boxes_with_owner_nodes(
        &self,
        edge_id: EdgeId,
        source_box: (Point, Size),
        target_box: (Point, Size),
        cell_size: f64,
        owner_nodes: Option<(NodeId, NodeId)>,
    ) -> f64 {
        let facing = self.facing_table_ports_for_boxes_with_owner_nodes(
            edge_id,
            source_box,
            target_box,
            owner_nodes,
        );
        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_DETAIL") {
            let edge = &self.edge_records[edge_id.0 as usize];
            let wanted = crate::internal::trace_env_value("TALA_TRACE_EDGE_DETAIL_NODE");
            let from = &self.nodes[edge.from.0 as usize];
            let to = &self.nodes[edge.to.0 as usize];
            if wanted.as_deref().is_none_or(|target| {
                target == "all" || target == from.external_id || target == to.external_id
            }) {
                eprintln!(
                    "TABLE_DISTANCE_RUST edge={} from={} to={} orientation={:?} source={:?} target={:?} source_col={:?} target_col={:?} source_count={:?} target_count={:?}",
                    edge_id.0,
                    from.external_id,
                    to.external_id,
                    facing.orientation,
                    facing.source,
                    facing.target,
                    edge.source_table_column,
                    edge.target_table_column,
                    edge.source_table_column_count,
                    edge.target_table_column_count,
                );
            }
        }
        if facing.orientation == Orientation::None {
            let left = source_box.0.x.min(target_box.0.x);
            let top = source_box.0.y.min(target_box.0.y);
            let right =
                (source_box.0.x + source_box.1.width).max(target_box.0.x + target_box.1.width);
            let bottom =
                (source_box.0.y + source_box.1.height).max(target_box.0.y + target_box.1.height);
            let dx = right - left;
            let dy = bottom - top;
            return (dx * dx + dy * dy).sqrt() * 2.0;
        }

        let multiplier = if matches!(facing.orientation, Orientation::Left | Orientation::Right) {
            1.0
        } else {
            2.0
        };
        let source = facing.source.unwrap_or(Point {
            x: source_box.0.x + source_box.1.width * 0.5,
            y: source_box.0.y + source_box.1.height * 0.5,
        });
        let target = facing.target.unwrap_or(Point {
            x: target_box.0.x + target_box.1.width * 0.5,
            y: target_box.0.y + target_box.1.height * 0.5,
        });
        let gap_cost = if (source.x - target.x).abs() < 150.0 {
            cell_size * 2.0
        } else {
            0.0
        };
        let dx = source.x - target.x;
        let dy = source.y - target.y;
        ((dx * dx + dy * dy).sqrt() + gap_cost) * multiplier
    }

    /// Recovered `Node.edgeLength` charges a crossing when two column edges
    /// approach the same table from the same side but their remote vertical
    /// order disagrees with the table-row order.
    pub(super) fn table_column_order_crossing_cost(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge_id: EdgeId,
        node_box: (Point, Size),
        adjacent_box: (Point, Size),
        projected_node: Option<ProjectedAdjacent>,
        projected_adjacent: Option<ProjectedAdjacent>,
        projected_table_neighbors: Option<&[ProjectedTableColumnNeighbor]>,
    ) -> f64 {
        let mut cost = 0.0;
        self.table_column_order_crossing_cost_into(
            node,
            adjacent,
            edge_id,
            node_box,
            adjacent_box,
            projected_node,
            projected_adjacent,
            projected_table_neighbors,
            &mut cost,
        );
        cost
    }

    /// Apply each crossing charge directly to the caller's running score.
    ///
    /// Go's `Node.edgeLength` adds `Graph.CrossingCost()` inside the neighbor
    /// loop. Returning a pre-summed penalty and adding it once changes the
    /// IEEE-754 rounding when the base edge distance is large, so the sized
    /// scorer uses this operation-ordered form.
    pub(super) fn table_column_order_crossing_cost_into(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge_id: EdgeId,
        node_box: (Point, Size),
        adjacent_box: (Point, Size),
        projected_node: Option<ProjectedAdjacent>,
        projected_adjacent: Option<ProjectedAdjacent>,
        projected_table_neighbors: Option<&[ProjectedTableColumnNeighbor]>,
        total: &mut f64,
    ) -> usize {
        self.table_column_order_crossing_cost_into_checked(
            node,
            adjacent,
            edge_id,
            node_box,
            adjacent_box,
            projected_node,
            projected_adjacent,
            projected_table_neighbors,
            total,
            ScoreContext::none(),
        )
        .expect("uncancelled table row-order scoring cannot fail")
    }

    /// Source `edge_length.go:1131–1181` polls each surviving adjacent edge
    /// before its column filter and applies charges directly to `distance`.
    /// There is no independent entry/final poll for this inline source term.
    pub(super) fn table_column_order_crossing_cost_into_checked(
        &self,
        node: NodeId,
        adjacent: NodeId,
        edge_id: EdgeId,
        node_box: (Point, Size),
        adjacent_box: (Point, Size),
        projected_node: Option<ProjectedAdjacent>,
        projected_adjacent: Option<ProjectedAdjacent>,
        projected_table_neighbors: Option<&[ProjectedTableColumnNeighbor]>,
        total: &mut f64,
        context: ScoreContext<'_>,
    ) -> Result<usize, limits::Error> {
        let edge = &self.edge_records[edge_id.0 as usize];
        if !edge.is_between_table_columns() {
            return Ok(0);
        }
        // This pointer comparison is intentionally against nodeReplacement,
        // while `edge.From` remains the current/reconnected endpoint. A
        // restored original pointer therefore cannot compare equal to the
        // current carrier even when both represent the same logical side.
        let current_from = self.active_aggregate_owner(edge.from);
        let column_index = if projected_node.is_none() && current_from == node {
            edge.target_table_column.expect("target table column")
        } else {
            edge.source_table_column.expect("source table column")
        };
        let edge_direction = self.sized_box_orientation(node_box, adjacent_box);
        let mut crossings = 0usize;

        let mut neighbor_count = 0usize;
        // Keep the actual charge in the traversal that produced this neighbor.
        // In particular, do not sum a separate preflight/replayed edge scan.
        let mut apply_neighbor = |other_box: (Point, Size), other_column_index: usize| {
            neighbor_count += 1;
            let other_edge_direction = self.sized_box_orientation(other_box, adjacent_box);
            let same_side = match edge_direction {
                Orientation::TopRight | Orientation::Right | Orientation::BottomRight => matches!(
                    other_edge_direction,
                    Orientation::TopRight | Orientation::Right | Orientation::BottomRight
                ),
                Orientation::TopLeft | Orientation::Left | Orientation::BottomLeft => matches!(
                    other_edge_direction,
                    Orientation::TopLeft | Orientation::Left | Orientation::BottomLeft
                ),
                _ => false,
            };
            if !same_side {
                return;
            }
            match self.sized_box_orientation(node_box, other_box) {
                Orientation::TopLeft | Orientation::Top | Orientation::TopRight
                    if other_column_index < column_index =>
                {
                    crossings += 1;
                    *total += self.crossing_cost_value();
                }
                Orientation::BottomLeft | Orientation::Bottom | Orientation::BottomRight
                    if other_column_index > column_index =>
                {
                    crossings += 1;
                    *total += self.crossing_cost_value();
                }
                _ => {}
            }
        };
        if let Some(projected_table_neighbors) = projected_table_neighbors {
            // Some(empty) is material: every edge on the restored endpoint
            // was reconnected away. Falling back to the carrier inventory
            // would reintroduce precisely the edges Go removed.
            // This existing projected inventory contains only column edges.
            // It lacks unfiltered source edge ordinals; exact source poll
            // ordering for mixed projected inventories remains unqualified.
            for (index, neighbor) in projected_table_neighbors.iter().enumerate() {
                score_interval!(
                    context,
                    self,
                    "node-edge.table-neighbors.projected",
                    Some(node),
                    index
                )?;
                if let Some(other_box) = self.sized_projected_box(neighbor.other) {
                    apply_neighbor(other_box, neighbor.column_index);
                }
            }
        } else if let Some(projected_adjacent) = projected_adjacent {
            // Root/aggregate scoring can restore a member without a hierarchy
            // SizedEdgeAbduction. Reconstruct that member's post-aggregate
            // edge slice from the stable arena: only edges internal to the
            // same active vessel remain attached to the original pointer.
            if let Some(original_index) = self
                .nodes
                .iter()
                .position(|candidate| candidate.tala_id == projected_adjacent.tala_id)
            {
                let original = NodeId(original_index as u32);
                let aggregate_owner = self.active_aggregate_owner(original);
                let original_is_aggregate_member =
                    aggregate_owner != original || self.active_node_is_aggregate(aggregate_owner);
                let mut surviving_index = 0usize;
                for other_edge_id in self.nodes[original_index].edges.read().iter().copied() {
                    let other_edge = &self.edge_records[other_edge_id.0 as usize];
                    let other_table = if other_edge.from == original {
                        other_edge.to
                    } else if other_edge.to == original {
                        other_edge.from
                    } else {
                        continue;
                    };
                    if original_is_aggregate_member
                        && self.active_aggregate_owner(other_table) != aggregate_owner
                    {
                        continue;
                    }
                    score_interval!(
                        context,
                        self,
                        "node-edge.table-neighbors.restored",
                        Some(node),
                        surviving_index
                    )?;
                    surviving_index += 1;
                    if !other_edge.is_between_table_columns() {
                        continue;
                    }
                    let other_column_index = if other_edge.from == original {
                        other_edge.source_table_column.expect("source table column")
                    } else {
                        other_edge.target_table_column.expect("target table column")
                    };
                    let Some(other_position) = self.position(other_table) else {
                        continue;
                    };
                    apply_neighbor(
                        (other_position, self.nodes[other_table.0 as usize].rect.size),
                        other_column_index,
                    );
                }
            }
        } else {
            // Ordinary Node.Edges follows Go range: capture the header,
            // copy the current slot before polling, and read later slots
            // only when reached. Synthetic aggregate incidence retains its
            // existing projected reconstruction until actual vessel binding.
            let shared = (!self.active_node_is_aggregate(adjacent))
                .then(|| self.nodes[adjacent.0 as usize].edges.header().clone());
            let projected = shared
                .is_none()
                .then(|| self.max_edge_length_incident_edges(adjacent));
            let incidence = shared.iter().flat_map(|header| header.iter_slots()).chain(
                projected
                    .iter()
                    .flat_map(|edges| edges.iter().copied().map(Some)),
            );
            for (index, other_slot) in incidence.enumerate() {
                score_interval!(
                    context,
                    self,
                    "node-edge.table-neighbors",
                    Some(node),
                    index
                )?;
                let other_edge_id = other_slot.expect("nil owner pointer slot");
                let other_edge = &self.edge_records[other_edge_id.0 as usize];
                if !other_edge.is_between_table_columns() {
                    continue;
                }
                let other_from = self.active_aggregate_owner(other_edge.from);
                let other_to = self.active_aggregate_owner(other_edge.to);
                let (other_table, other_column_index) = if other_from == adjacent {
                    (
                        other_to,
                        other_edge.source_table_column.expect("source table column"),
                    )
                } else {
                    (
                        other_from,
                        other_edge.target_table_column.expect("target table column"),
                    )
                };
                let Some(other_position) = self.active_node_position(other_table) else {
                    continue;
                };
                apply_neighbor(
                    (other_position, self.active_node_size(other_table)),
                    other_column_index,
                );
            }
        }

        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_DETAIL") {
            let edge = &self.edge_records[edge_id.0 as usize];
            let wanted = crate::internal::trace_env_value("TALA_TRACE_EDGE_DETAIL_NODE");
            let from = &self.nodes[edge.from.0 as usize];
            let to = &self.nodes[edge.to.0 as usize];
            if wanted.as_deref().is_none_or(|target| {
                target == "all" || target == from.external_id || target == to.external_id
            }) {
                eprintln!(
                    "TABLE_ORDER_CROSSING_RUST edge={} node={} adjacent={} column={} direction={:?} neighbors={} crossings={} crossing_cost={} cost={}",
                    edge_id.0,
                    self.nodes[node.0 as usize].external_id,
                    self.nodes[adjacent.0 as usize].external_id,
                    column_index,
                    edge_direction,
                    neighbor_count,
                    crossings,
                    self.raw_crossing_cost(),
                    *total,
                );
            }
        }
        Ok(crossings)
    }

    /// Direct translation of recovered
    /// `Node.getColumnToColumnCrossingCost`. This is deliberately separate
    /// from the row-order term inside `Node.edgeLength`: it intersects the
    /// actual facing-port segments incident to one SQL table and uses the
    /// fixed crossing constant times `CellSize`, not `Graph.getCrossingCost`.
    ///
    /// `sequence_abductions_only` mirrors AlignAxes, which passes its explicit
    /// sequence-only abduction slice (possibly nil). Other sized-placement
    /// callers pass the complete optimizer abduction slice.
    pub(super) fn column_to_column_crossing_cost(
        &self,
        node: NodeId,
        sequence_abductions_only: bool,
    ) -> f64 {
        self.column_to_column_crossing_cost_checked(
            node,
            sequence_abductions_only,
            ScoreContext::none(),
        )
        .expect("uncancelled column crossing scoring cannot fail")
    }

    /// D2 v0.9.0 `placementcost.ColumnCrossingCost` (`symmetry.go:575`).
    /// Check the operation context before even the non-table fast return,
    /// during the actual abduction/port loops, and in the shared intersection
    /// kernel. The geometry and summation order match the unchecked wrapper.
    pub(super) fn column_to_column_crossing_cost_checked(
        &self,
        node: NodeId,
        sequence_abductions_only: bool,
        context: ScoreContext<'_>,
    ) -> Result<f64, limits::Error> {
        score_poll!(context, self, "column-crossing.entry", Some(node), None)?;
        if self.nodes[node.0 as usize].shape != ShapeKind::SqlTable {
            return Ok(0.0);
        }

        // Source symmetry.go:591–596 builds this map once, in the supplied
        // slice's order. Inserting later records replaces earlier ones for
        // the same concrete edge, matching the former reverse lookup.
        let mut edge_to_abduction = std::collections::HashMap::new();
        for (index, abduction) in self
            .sized_edge_abductions
            .iter()
            .filter(|abduction| !sequence_abductions_only || abduction.sequence_abduction)
            .enumerate()
        {
            score_interval!(
                context,
                self,
                "column-crossing.abductions",
                Some(node),
                index
            )?;
            if let Some(edge) = abduction.edge {
                edge_to_abduction.insert(edge, abduction);
            }
        }

        let mut segments = Vec::new();
        for (index, edge_id) in self
            .max_edge_length_incident_edges(node)
            .into_iter()
            .enumerate()
        {
            score_interval!(context, self, "column-crossing.edges", Some(node), index)?;
            let edge = &self.edge_records[edge_id.0 as usize];
            let current_from = self.active_aggregate_owner(edge.from);
            let current_to = self.active_aggregate_owner(edge.to);
            let Some(source_position) = self.active_node_position(current_from) else {
                continue;
            };
            let Some(target_position) = self.active_node_position(current_to) else {
                continue;
            };
            let mut source_box = (source_position, self.active_node_size(current_from));
            let mut target_box = (target_position, self.active_node_size(current_to));

            // Unlike Node.edgeLength's pair-consumption logic, the recovered
            // method indexes its abduction map by the concrete Edge pointer.
            if let Some(abduction) = edge_to_abduction.get(&edge_id) {
                if let Some(projected) = abduction.originally_from
                    && let Some(projected_box) = projected
                        .projection()
                        .and_then(|projection| self.sized_projected_box(projection))
                {
                    source_box = projected_box;
                }
                if let Some(projected) = abduction.originally_to
                    && let Some(projected_box) = projected
                        .projection()
                        .and_then(|projection| self.sized_projected_box(projection))
                {
                    target_box = projected_box;
                }
            }

            let facing = self.facing_table_ports_for_boxes(edge_id, source_box, target_box);
            if let (Some(source), Some(target)) = (facing.source, facing.target) {
                segments.push((source, target));
            }
        }

        let crossings = count_segment_crossings_checked(self, node, &segments, context)?;
        Ok(crossings as f64 * (0.48_f64 * 0.48 * 0.48) * self.cell_size)
    }
}

fn segments_cross(u0: Point, u1: Point, v0: Point, v1: Point) -> bool {
    let denominator = (u1.y - u0.y) * (v1.x - v0.x) - (u1.x - u0.x) * (v1.y - v0.y);
    if denominator == 0.0 {
        return false;
    }
    let source_fraction =
        ((v0.y - u0.y) * (v1.x - v0.x) - (v0.x - u0.x) * (v1.y - v0.y)) / denominator;
    if !(0.0..=1.0).contains(&source_fraction) {
        return false;
    }
    let target_fraction =
        ((u1.x - u0.x) * (v0.y - u0.y) - (u1.y - u0.y) * (v0.x - u0.x)) / denominator;
    (0.0..=1.0).contains(&target_fraction)
}

/// D2 v0.9.0 `layoutgraph.CountSegmentCrossingsContext`: entry at
/// segment_crossing.go:39, outer i at 61, inner j-i-1 at 77, and final at 48.
/// Checks are in the real geometric kernel, including pairs rejected by its
/// broad phase. No extra outer-loop check is made for the last segment.
fn count_segment_crossings_checked(
    _graph: &ArenaGraph,
    _node: NodeId,
    segments: &[(Point, Point)],
    context: ScoreContext<'_>,
) -> Result<usize, limits::Error> {
    score_poll!(
        context,
        _graph,
        "segment-crossings.entry",
        Some(_node),
        None
    )?;
    let mut crossings = 0;
    for (index, &(first_start, first_end)) in segments
        .iter()
        .take(segments.len().saturating_sub(1))
        .enumerate()
    {
        score_interval!(
            context,
            _graph,
            "segment-crossings.outer",
            Some(_node),
            index
        )?;
        let min_x = first_start.x.min(first_end.x);
        let max_x = first_start.x.max(first_end.x);
        let min_y = first_start.y.min(first_end.y);
        let max_y = first_start.y.max(first_end.y);
        for (pair_index, &(second_start, second_end)) in segments.iter().skip(index + 1).enumerate()
        {
            score_interval!(
                context,
                _graph,
                "segment-crossings.inner",
                Some(_node),
                pair_index
            )?;
            if if second_start.x < second_end.x {
                second_end.x < min_x || second_start.x > max_x
            } else {
                second_start.x < min_x || second_end.x > max_x
            } {
                continue;
            }
            if if second_start.y < second_end.y {
                second_end.y < min_y || second_start.y > max_y
            } else {
                second_start.y < min_y || second_end.y > max_y
            } {
                continue;
            }
            if first_start == second_start || first_end == second_end {
                continue;
            }
            if segments_cross(first_start, first_end, second_start, second_end) {
                crossings += 1;
            }
        }
    }
    score_poll!(
        context,
        _graph,
        "segment-crossings.final",
        Some(_node),
        None
    )?;
    Ok(crossings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use crate::{ContentAlignment, Edge, EdgeTableColumns, Graph, Node};
    use std::cell::Cell;

    struct CancelAfterChecks {
        remaining: Cell<usize>,
        calls: Cell<usize>,
        #[cfg(feature = "diagnostic-traces")]
        sites: std::cell::RefCell<Vec<(&'static str, Option<usize>)>>,
    }

    impl CancelAfterChecks {
        fn new(remaining: usize) -> Self {
            Self {
                remaining: Cell::new(remaining),
                calls: Cell::new(0),
                #[cfg(feature = "diagnostic-traces")]
                sites: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Context for CancelAfterChecks {
        fn done(&self) -> Option<bool> {
            None
        }

        fn error(&self) -> Option<ContextError> {
            self.calls.set(self.calls.get() + 1);
            let remaining = self.remaining.get();
            if remaining == 0 {
                Some(ContextError::Cancelled)
            } else {
                self.remaining.set(remaining - 1);
                None
            }
        }

        #[cfg(feature = "diagnostic-traces")]
        fn scoring_state(
            &self,
            _graph: &ArenaGraph,
            site: &'static str,
            _node: Option<NodeId>,
            iteration: Option<usize>,
        ) {
            self.sites.borrow_mut().push((site, iteration));
        }
    }

    fn assert_scoring_canceled<T>(result: Result<T, limits::Error>) {
        match result {
            Err(error) => assert_eq!(error.to_string(), "EdgeLength: context canceled"),
            Ok(_) => panic!("expected actual scoring cancellation"),
        }
    }

    fn table(name: &str, position: Point, width: f64, columns: usize) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size {
                width,
                height: 36.0 * (columns as f64 + 1.0),
            },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: Some(position),
            constrained_x: None,
            constrained_y: None,
            near: None,
            fixed_width: false,
            fixed_height: false,
            direction: None,
            force_hierarchy: false,
            grid_rows: None,
            grid_columns: None,
            canvas_position: None,
            content_insets: Insets::uniform(0.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::SqlTable,
        }
    }

    fn graph_with_column_edges(count: usize) -> (ArenaGraph, NodeId) {
        let mut input = Graph::default();
        let source = input.add_node(table("table", Point::default(), 40.0, 1));
        input.set_table_column_count(source, Some(1));
        for index in 0..count {
            let target = input.add_node(table(
                &format!("other-{index}"),
                Point {
                    x: 100.0,
                    y: index as f64 * 50.0,
                },
                40.0,
                1,
            ));
            input.set_table_column_count(target, Some(1));
            let edge = input.add_edge(Edge { source, target });
            input.set_edge_table_columns(
                edge,
                EdgeTableColumns {
                    source: Some(0),
                    target: Some(0),
                },
            );
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        (graph, source)
    }

    fn edge_abduction(graph: &ArenaGraph, edge: EdgeId) -> SizedEdgeAbduction {
        let endpoint = &graph.edge_records[edge.0 as usize];
        SizedEdgeAbduction {
            source_edge: edge,
            edge_tala_id: graph.edge_records[edge.0 as usize].tala_id,
            edge: Some(edge),
            current_from: Some(AbductionCurrentNodeRef::local(endpoint.from)),
            current_to: Some(AbductionCurrentNodeRef::local(endpoint.to)),
            originally_from: None,
            originally_to: None,
            originally_from_table_neighbors: Vec::new(),
            originally_to_table_neighbors: Vec::new(),
            originally_from_container: None,
            originally_to_container: None,
            sequence_abduction: true,
            obstructions_from_to: Vec::new(),
            obstructions_to_from: Vec::new(),
            obstruction_sets_from_to: OrderedObstructionInventory::default(),
            obstruction_sets_to_from: OrderedObstructionInventory::default(),
        }
    }

    #[test]
    fn column_scoring_checks_cancellation_before_non_table_fast_return() {
        let (mut graph, node) = graph_with_column_edges(0);
        graph.nodes[node.0 as usize].shape = ShapeKind::Rectangle;
        let context = CancelAfterChecks::new(0);
        assert_scoring_canceled(graph.column_to_column_crossing_cost_checked(
            node,
            false,
            ScoreContext::new(&context),
        ));
        assert_eq!(context.calls.get(), 1);
    }

    #[test]
    fn empty_column_scoring_checks_the_actual_segment_kernel_final_poll() {
        let (graph, node) = graph_with_column_edges(0);
        let context = CancelAfterChecks::new(2);
        assert_scoring_canceled(graph.column_to_column_crossing_cost_checked(
            node,
            false,
            ScoreContext::new(&context),
        ));
        assert_eq!(context.calls.get(), 3);
        #[cfg(feature = "diagnostic-traces")]
        assert_eq!(
            &*context.sites.borrow(),
            &[
                ("column-crossing.entry", None),
                ("segment-crossings.entry", None),
                ("segment-crossings.final", None),
            ]
        );
    }

    #[test]
    fn column_scoring_checks_the_filtered_abduction_slice_at_index_64() {
        let (mut graph, node) = graph_with_column_edges(1);
        let edge = graph.active_edge_ids(node)[0];
        let abduction = edge_abduction(&graph, edge);
        for _ in 0..130 {
            let mut ignored = abduction.clone();
            ignored.sequence_abduction = false;
            graph.sized_edge_abductions.push(ignored);
            graph.sized_edge_abductions.push(abduction.clone());
        }
        let context = CancelAfterChecks::new(2);
        assert_scoring_canceled(graph.column_to_column_crossing_cost_checked(
            node,
            true,
            ScoreContext::new(&context),
        ));
        assert_eq!(context.calls.get(), 3);
        #[cfg(feature = "diagnostic-traces")]
        assert_eq!(
            &*context.sites.borrow(),
            &[
                ("column-crossing.entry", None),
                ("column-crossing.abductions", Some(0)),
                ("column-crossing.abductions", Some(64)),
            ]
        );
    }

    #[test]
    fn column_abduction_map_uses_the_last_record_for_each_selected_edge() {
        let (mut graph, node) = graph_with_column_edges(2);
        let edge = graph.active_edge_ids(node)[0];
        let baseline = graph.column_to_column_crossing_cost(node, false);
        let unprojected = edge_abduction(&graph, edge);
        let mut projected = unprojected.clone();
        projected.originally_from = Some(
            ProjectedAdjacent {
                owner: node,
                tala_id: graph.nodes[node.0 as usize].tala_id,
                container_tala_id: None,
                offset: Point { x: 0.0, y: 1000.0 },
                size: graph.active_node_size(node),
                cluster_member: false,
            }
            .into(),
        );
        graph.sized_edge_abductions.push(projected);
        let moved_score = graph.column_to_column_crossing_cost(node, false);
        assert_ne!(moved_score.to_bits(), baseline.to_bits());
        let mut ignored = unprojected.clone();
        ignored.sequence_abduction = false;
        graph.sized_edge_abductions.push(ignored);
        assert_eq!(
            graph.column_to_column_crossing_cost(node, true).to_bits(),
            moved_score.to_bits(),
            "the sequence-only source slice excludes the later ordinary record"
        );
        assert_eq!(
            graph.column_to_column_crossing_cost(node, false).to_bits(),
            baseline.to_bits(),
            "the source map's final ordinary record restores the live endpoint"
        );
        graph.sized_edge_abductions.push(unprojected);
        assert_eq!(
            graph.column_to_column_crossing_cost(node, true).to_bits(),
            baseline.to_bits()
        );
    }

    #[test]
    fn column_scoring_cancels_inside_the_actual_bounded_pair_scan() {
        // Source edge_length_runtime_test.go's 130-edge control: Column entry,
        // edge0/64/128, segment entry, outer0 and inner0 succeed; inner64 fails.
        let (graph, node) = graph_with_column_edges(130);
        let context = CancelAfterChecks::new(7);
        assert_scoring_canceled(graph.column_to_column_crossing_cost_checked(
            node,
            false,
            ScoreContext::new(&context),
        ));
        assert_eq!(context.calls.get(), 8);
        #[cfg(feature = "diagnostic-traces")]
        assert_eq!(
            context.sites.borrow().last(),
            Some(&("segment-crossings.inner", Some(64)))
        );
    }

    #[test]
    fn row_crossing_checks_current_edge_indices_before_non_column_filtering() {
        let (mut graph, adjacent) = graph_with_column_edges(130);
        let edges = graph
            .active_edge_ids(adjacent)
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let edge = edges[0];
        let node = graph.edge_records[edge.0 as usize].to;
        for edge in edges.iter().skip(1) {
            let edge = &mut graph.edge_records[edge.0 as usize];
            edge.source_table_column = None;
            edge.target_table_column = None;
        }
        let node_box = (graph.position(node).unwrap(), graph.active_node_size(node));
        let adjacent_box = (
            graph.position(adjacent).unwrap(),
            graph.active_node_size(adjacent),
        );
        let context = CancelAfterChecks::new(1);
        let mut distance = 17.0;
        assert_scoring_canceled(graph.table_column_order_crossing_cost_into_checked(
            node,
            adjacent,
            edge,
            node_box,
            adjacent_box,
            None,
            None,
            None,
            &mut distance,
            ScoreContext::new(&context),
        ));
        assert_eq!(context.calls.get(), 2);
        assert_eq!(distance.to_bits(), 17.0_f64.to_bits());
        #[cfg(feature = "diagnostic-traces")]
        assert_eq!(
            &*context.sites.borrow(),
            &[
                ("node-edge.table-neighbors", Some(0)),
                ("node-edge.table-neighbors", Some(64)),
            ]
        );
    }

    #[test]
    fn facing_column_ports_drive_alignment_and_distance() {
        let mut input = Graph::default();
        let source = input.add_node(table("users", Point { x: 0.0, y: 0.0 }, 215.0, 3));
        let target = input.add_node(table("teams", Point { x: 365.0, y: 36.0 }, 194.0, 3));
        input.set_table_column_count(source, Some(3));
        input.set_table_column_count(target, Some(3));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_table_columns(
            edge,
            EdgeTableColumns {
                source: Some(1),
                target: Some(0),
            },
        );

        let graph = ArenaGraph::from_input(&input);
        let source_box = (Point { x: 0.0, y: 0.0 }, input.node(source).unwrap().size);
        let target_box = (
            Point { x: 365.0, y: 36.0 },
            input.node(target).unwrap().size,
        );
        let facing = graph.facing_table_ports_for_boxes(edge, source_box, target_box);

        assert_eq!(facing.source, Some(Point { x: 215.0, y: 90.0 }));
        assert_eq!(facing.target, Some(Point { x: 365.0, y: 90.0 }));
        assert_eq!(facing.orientation, Orientation::Left);
        assert_eq!(
            graph.table_column_distance_for_boxes(edge, source_box, target_box),
            150.0
        );
        assert!(graph.edge_axis_aligned(edge));
        assert_eq!(graph.alignment_deltas(edge), Point { x: 0.0, y: 0.0 });
    }

    #[test]
    fn incident_column_segments_charge_the_fixed_cell_scaled_crossing_cost() {
        let mut input = Graph::default();
        let source = input.add_node(table("source", Point { x: 0.0, y: 0.0 }, 180.0, 3));
        let lower = input.add_node(table("lower", Point { x: 400.0, y: 120.0 }, 180.0, 3));
        let upper = input.add_node(table(
            "upper",
            Point {
                x: 400.0,
                y: -120.0,
            },
            180.0,
            3,
        ));
        for node in [source, lower, upper] {
            input.set_table_column_count(node, Some(3));
        }
        let descending = input.add_edge(Edge {
            source,
            target: lower,
        });
        input.set_edge_table_columns(
            descending,
            EdgeTableColumns {
                source: Some(0),
                target: Some(0),
            },
        );
        let ascending = input.add_edge(Edge {
            source,
            target: upper,
        });
        input.set_edge_table_columns(
            ascending,
            EdgeTableColumns {
                source: Some(2),
                target: Some(2),
            },
        );

        let mut graph = ArenaGraph::from_input(&input);
        graph.cell_size = 20.0;
        let expected = (0.48_f64 * 0.48 * 0.48) * 20.0;
        assert_eq!(
            graph.column_to_column_crossing_cost(source, false),
            expected
        );
        let context = CancelAfterChecks::new(usize::MAX);
        assert_eq!(
            graph
                .column_to_column_crossing_cost_checked(source, false, ScoreContext::new(&context),)
                .unwrap()
                .to_bits(),
            expected.to_bits()
        );
        assert_eq!(graph.column_to_column_crossing_cost(lower, false), 0.0);
        assert_eq!(graph.column_to_column_crossing_cost(upper, false), 0.0);
    }

    #[test]
    fn row_order_crossing_uses_the_restored_tables_surviving_inventory() {
        let mut input = Graph::default();
        let external = input.add_node(table("external", Point { x: 400.0, y: 0.0 }, 120.0, 3));
        let mut carrier_node = table("carrier", Point { x: 0.0, y: 0.0 }, 120.0, 3);
        carrier_node.shape = ShapeKind::Rectangle;
        let carrier = input.add_node(carrier_node);
        let other = input.add_node(table("other", Point { x: 400.0, y: 200.0 }, 120.0, 3));
        for node in [external, other] {
            input.set_table_column_count(node, Some(3));
        }
        let edge = input.add_edge(Edge {
            source: external,
            target: carrier,
        });
        input.set_edge_table_columns(
            edge,
            EdgeTableColumns {
                source: Some(0),
                target: Some(1),
            },
        );

        let graph = ArenaGraph::from_input(&input);
        graph.set_crossing_cost(77.0);
        let restored_table = ProjectedAdjacent {
            owner: carrier,
            tala_id: 91_001,
            container_tala_id: Some(graph.nodes[carrier.0 as usize].tala_id),
            offset: Point::default(),
            size: Size {
                width: 120.0,
                height: 144.0,
            },
            cluster_member: false,
        };
        let surviving = [ProjectedTableColumnNeighbor {
            other: ProjectedAdjacent {
                owner: other,
                tala_id: graph.nodes[other.0 as usize].tala_id,
                container_tala_id: Some(graph.nodes[carrier.0 as usize].tala_id),
                offset: Point::default(),
                size: graph.nodes[other.0 as usize].rect.size,
                cluster_member: false,
            },
            column_index: 0,
        }];
        let external_box = (
            graph.position(external).unwrap(),
            graph.nodes[external.0 as usize].rect.size,
        );
        let restored_box = graph.sized_projected_box(restored_table).unwrap();

        assert_eq!(
            graph.table_column_order_crossing_cost(
                external,
                carrier,
                edge,
                external_box,
                restored_box,
                None,
                Some(restored_table),
                Some(&surviving),
            ),
            77.0
        );
        assert_eq!(
            graph.table_column_order_crossing_cost(
                external,
                carrier,
                edge,
                external_box,
                restored_box,
                None,
                Some(restored_table),
                Some(&[]),
            ),
            0.0,
            "an explicitly empty restored slice must not fall back to carrier.Edges"
        );
        let context = CancelAfterChecks::new(0);
        let mut distance = 17.0;
        assert_eq!(
            graph
                .table_column_order_crossing_cost_into_checked(
                    external,
                    carrier,
                    edge,
                    external_box,
                    restored_box,
                    None,
                    Some(restored_table),
                    Some(&[]),
                    &mut distance,
                    ScoreContext::new(&context),
                )
                .unwrap(),
            0
        );
        assert_eq!(distance.to_bits(), 17.0_f64.to_bits());
        assert_eq!(
            context.calls.get(),
            0,
            "the source inline term has no entry poll"
        );
    }
}
