// SPDX-License-Identifier: MPL-2.0
//! placement/cluster_optimization.go request transactions and atomic stage journal.
use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

struct Request {
    geometry: GeometryCheckpoint,
    overlaps: BTreeSet<(NodeId, NodeId)>,
    exact: BTreeSet<(NodeId, NodeId)>,
}
impl Request {
    fn new(graph: &ArenaGraph, guard: &mut limits::WorkGuard<'_>) -> Result<Self, limits::Error> {
        let geometry = GeometryCheckpoint::capture_request(graph, guard)?;
        let (overlaps, exact) = graph.transaction_overlap_snapshot_checked(guard)?;
        guard.finish()?;
        Ok(Self {
            geometry,
            overlaps,
            exact,
        })
    }
    fn commit<F>(
        &self,
        graph: &mut ArenaGraph,
        context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
        operation: F,
    ) -> Result<bool, limits::Error>
    where
        F: FnOnce(&mut ArenaGraph, &mut gap_guard::GapWork<'_, '_>) -> Result<(), limits::Error>,
    {
        let mut work = gap_guard::GapWork::new(guard, context, false);
        work.anchor_canonical_overlaps(self.overlaps.clone(), self.exact.clone());
        let result = catch_unwind(AssertUnwindSafe(|| {
            work.before_operation(graph)?;
            operation(graph, &mut work)?;
            work.check_context(graph)?;
            if !work.refit_containers(graph, &self.geometry)? {
                return Ok(false);
            }
            graph.sync_clusters();
            graph.sync_sequences();
            work.check_context(graph)?;
            work.validate_commit(graph, &self.geometry)
        }));
        match result {
            Ok(Ok(true)) => Ok(true),
            Ok(value) => {
                self.geometry.rollback(graph);
                value
            }
            Err(payload) => {
                self.geometry.rollback(graph);
                resume_unwind(payload)
            }
        }
    }
}
struct Journal {
    geometry: Option<GeometryCheckpoint>,
    desired: Option<(usize, ClusterArrangement)>,
    cell: f64,
    costs: Option<PlacementCostSnapshot>,
}
impl Journal {
    fn record_request(&mut self, request: &Request) -> bool {
        if self.geometry.is_some() {
            return false;
        }
        self.geometry = Some(request.geometry.preserve());
        true
    }
    fn restore(&self, graph: &mut ArenaGraph) {
        if let Some(geometry) = &self.geometry {
            geometry.rollback(graph);
        }
        if let Some((index, desired)) = self.desired {
            graph.set_cluster_desired_arrangement(index, desired);
        }
        graph.cell_size = self.cell;
        if let Some(costs) = &self.costs {
            costs.restore();
        }
    }
}
impl ArenaGraph {
    pub(super) fn optimize_clusters_transactional(
        &mut self,
        context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        guard.finish()?;
        let order = self.cluster_reverse_dfs_order();
        if order.is_empty() {
            return Ok(false);
        }
        let mut journal = Journal {
            geometry: None,
            desired: None,
            cell: self.cell_size,
            costs: None,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut changed = false;
            for index in order {
                changed |=
                    self.optimize_one_cluster_transactional(index, context, guard, &mut journal)?;
            }
            Ok(changed)
        }));
        match result {
            Ok(Ok(changed)) => Ok(changed),
            Ok(Err(error)) => {
                journal.restore(self);
                Err(error)
            }
            Err(payload) => {
                journal.restore(self);
                resume_unwind(payload)
            }
        }
    }
    fn optimize_one_cluster_transactional(
        &mut self,
        index: usize,
        context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
        journal: &mut Journal,
    ) -> Result<bool, limits::Error> {
        // The inner EnsureTransactionWorkGuard runs before policy selection.
        guard.finish()?;
        let desired = self.desired_arrangement_from_external_geometry(
            index,
            self.clusters[index].desired_arrangement,
        );
        if journal.geometry.is_none()
            && journal.desired.is_none()
            && self.clusters[index].desired_arrangement != desired
        {
            journal.desired = Some((index, self.clusters[index].desired_arrangement));
        }
        self.set_cluster_desired_arrangement(index, desired);
        let mut request = None;
        let mut owns_stage_snapshot = false;
        let mut changed = false;
        if self.clusters[index].arrangement != desired {
            let mut txn = Request::new(self, guard)?;
            owns_stage_snapshot = journal.record_request(&txn);
            let carrier = self.clusters[index].members[0];
            let vessel = self
                .topology_vessel_node(TopologyVessel::Cluster(index))
                .unwrap_or(carrier);
            let size = txn
                .geometry
                .original_size(vessel)
                .expect("captured cluster vessel dimensions");
            let accepted = txn.commit(self, context, guard, |graph, _| {
                graph.cluster_flip_operation(index, true, size);
                Ok(())
            })?;
            if !accepted
                && !txn.commit(self, context, guard, |graph, _| {
                    graph.cluster_flip_operation(index, false, size);
                    Ok(())
                })?
            {
                return Ok(false);
            }
            txn.geometry.update(self, guard)?;
            request = Some(txn);
            changed = true;
        }
        if self.clusters[index].arrangement != desired {
            return Ok(changed);
        }
        let mut txn = match request {
            Some(txn) => txn,
            None => {
                let txn = Request::new(self, guard)?;
                owns_stage_snapshot = journal.record_request(&txn);
                txn
            }
        };
        gap_guard::capture_placement_costs_once(
            &mut journal.costs,
            "OptimizeClustersTransactions",
            guard,
            || self.cost_cache.edge_length_entries(),
            || self.snapshot_placement_costs(),
        )?;
        let mut best =
            self.global_sized_edge_length_after_tree_restoration_checked(true, context)?;
        let mut move_vessel = false;
        let mut move_nodes = false;
        if txn.commit(self, context, guard, |graph, _| {
            graph.cluster_alignment_operation(index, true);
            Ok(())
        })? {
            let score =
                self.global_sized_edge_length_after_tree_restoration_checked(true, context)?;
            if cluster_precision_compare(score, best) == Ordering::Less {
                best = score;
                move_vessel = true;
            }
        }
        txn.geometry.rollback(self);
        if txn.commit(self, context, guard, |graph, _| {
            graph.cluster_alignment_operation(index, false);
            Ok(())
        })? {
            let score =
                self.global_sized_edge_length_after_tree_restoration_checked(true, context)?;
            if cluster_precision_compare(score, best) == Ordering::Less {
                move_nodes = true;
            }
        }
        txn.geometry.rollback(self);
        if move_nodes || move_vessel {
            // Rejection at this final commit is a real source error. The
            // deterministic operation was already validated in its trial.
            if !txn.commit(self, context, guard, |graph, _| {
                graph.cluster_alignment_operation(index, !move_nodes);
                Ok(())
            })? {
                return Err(limits::Error::Invalid("invalid layout candidate".into()));
            }
            changed = true;
            if owns_stage_snapshot {
                txn.geometry = txn.geometry.clone_geometry(guard)?;
            }
            txn.geometry.update(self, guard)?;
            let carrier = self.clusters[index].members[0];
            let horizontal = self.clusters[index].arrangement == ClusterArrangement::Column;
            // A rejection from this gap commit is deliberately ignored.
            txn.commit(self, context, guard, |graph, work| {
                graph.cluster_optimizer_gap_checked(carrier, horizontal, work)?;
                Ok(())
            })?;
        }
        Ok(changed)
    }
    fn cluster_flip_operation(&mut self, index: usize, center: bool, original: Size) {
        // No source callbacks occur inside this unmetered geometry operation.
        // Publish its values into the existing source point bindings before
        // Commit performs its actual post-operation context check.
        let mut trial = self.clone();
        let carrier = trial.clusters[index].members[0];
        let origin = trial
            .active_node_position(carrier)
            .expect("positioned vessel");
        let arrangement = match trial.clusters[index].arrangement {
            ClusterArrangement::Row => ClusterArrangement::Column,
            ClusterArrangement::Column => ClusterArrangement::Row,
        };
        trial.clusters[index].arrangement = arrangement;
        trial.clusters[index].padding = trial.prearranged_cluster_padding(index, arrangement);
        trial.resize_cluster_members(index);
        if trial.cluster_vessel_sizes.len() <= index {
            trial
                .cluster_vessel_sizes
                .resize(index + 1, Size::default());
        }
        let size = trial.cluster_vessel_size_for_arrangement(index, arrangement);
        trial.cluster_vessel_sizes[index] = size;
        if let Some(group) = trial.owner_group_binding(TopologyVessel::Cluster(index)) {
            let name = |a| {
                match a {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                }
                .to_owned()
            };
            trial
                .set_owner_cluster_orientation_policy(
                    group,
                    name(arrangement),
                    name(trial.clusters[index].desired_arrangement),
                    trial.clusters[index].padding,
                )
                .expect("bound cluster policy");
        }
        trial.set_projected_node_size(trial.clusters[index].vessel_tala_id, size);
        trial.arrange_cluster_members(index, origin);
        if center {
            let delta = match arrangement {
                ClusterArrangement::Row => Point {
                    x: ((original.width - size.width) / 2.).round(),
                    y: 0.,
                },
                ClusterArrangement::Column => Point {
                    x: 0.,
                    y: ((original.height - size.height) / 2.).round(),
                },
            };
            trial.translate_active_node_with_children(carrier, delta);
        }
        trial.refit_cluster_container_preserving_origin(index);
        self.publish_cluster_optimization_candidate(trial);
    }
    fn cluster_alignment_operation(&mut self, index: usize, move_vessel: bool) {
        let external = self.cluster_external_nodes(index);
        if external.is_empty() {
            return;
        }
        let mut trial = self.clone();
        let carrier = trial.clusters[index].members[0];
        let vessel = Rect {
            origin: trial
                .active_node_position(carrier)
                .expect("positioned vessel"),
            size: trial.cluster_vessel_size(index),
        };
        let horizontal = trial.clusters[index].arrangement == ClusterArrangement::Row;
        if move_vessel {
            let mut center = Point::default();
            for &node in &external {
                let p = trial.active_node_center(node);
                center.x += p.x;
                center.y += p.y;
            }
            let count = external.len() as f64;
            let delta = if horizontal {
                Point {
                    x: (center.x / count - vessel.size.width / 2.0 - vessel.origin.x).round(),
                    y: 0.,
                }
            } else {
                Point {
                    x: 0.,
                    y: (center.y / count - vessel.size.height / 2.0 - vessel.origin.y).round(),
                }
            };
            trial.translate_active_node_box(carrier, delta);
        } else {
            for node in external {
                if !trial.active_node_is_aggregate(node)
                    && trial.nodes[node.0 as usize].fixed_top_left.is_some()
                {
                    continue;
                }
                let rect = Rect {
                    origin: trial
                        .active_node_position(node)
                        .expect("positioned external node"),
                    size: trial.active_node_size(node),
                };
                let overlaps = if horizontal {
                    rect.origin.x <= vessel.right() && vessel.origin.x <= rect.right()
                } else {
                    rect.origin.y <= vessel.bottom() && vessel.origin.y <= rect.bottom()
                };
                if !overlaps {
                    continue;
                }
                let delta = if horizontal {
                    Point {
                        x: (vessel.origin.x + vessel.size.width / 2.0
                            - rect.origin.x
                            - rect.size.width / 2.0)
                            .round(),
                        y: 0.,
                    }
                } else {
                    Point {
                        x: 0.,
                        y: (vessel.origin.y + vessel.size.height / 2.0
                            - rect.origin.y
                            - rect.size.height / 2.0)
                            .round(),
                    }
                };
                trial.translate_active_node_with_children(node, delta);
            }
        }
        self.publish_cluster_optimization_candidate(trial);
    }
}
