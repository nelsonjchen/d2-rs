// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! The source optimizer iteration and its enclosing restoration boundary.
use super::limits::{Context, OptimizationError, OptimizationWorkGuard};
use super::owner_placement::OwnerPlacementError;
use super::owner_sizeless_constructor::OwnerSizelessOptimizer;
use super::{ArenaGraph, NodeId, OwnerScoreSession};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

#[cfg(test)]
#[path = "owner_sizeless_loop_tests.rs"]
mod tests;

fn step(
    session: &mut OwnerScoreSession<'_>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.step_with_owner(session.owner_mut());
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = session;
        guard.step()
    }
}
fn finish(
    session: &mut OwnerScoreSession<'_>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.finish_with_owner(session.owner_mut());
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = session;
        guard.finish()
    }
}
impl OwnerSizelessOptimizer {
    /// Capture/copy failures are inert. Once both succeed, every later error
    /// or unwind restores the original graph/cost objects and occupied map.
    /// The caller's random stream remains advanced, as in the source.
    pub(super) fn optimize_with_limit(
        &self,
        workspace: &ArenaGraph,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn Context,
        temperature: f64,
        work_limit: u64,
    ) -> Result<(), OwnerPlacementError> {
        #[cfg(feature = "diagnostic-traces")]
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(context),
            "sizelessOptimizer.optimize",
            work_limit,
            session.owner_mut(),
        )?;
        #[cfg(not(feature = "diagnostic-traces"))]
        let mut guard =
            OptimizationWorkGuard::new(Some(context), "sizelessOptimizer.optimize", work_limit)?;
        if self.rng().is_none() {
            return Err(OptimizationError::Invalid(
                "TALA sizelessOptimizer.optimize requires a random generator".into(),
            )
            .into());
        }
        let mut scratch = self.take_mutation_scratch();
        let result = catch_unwind(AssertUnwindSafe(|| {
            scratch.capture_into(session.owner_mut(), self.selected_graph(), &mut guard)?;
            // Retain the actual field map, then release its RefCell borrow
            // before real context callbacks can replace that field.
            let occupied = self.occupied().borrow().clone();
            #[cfg(feature = "diagnostic-traces")]
            let occupied_snapshot = occupied.capture_with_owner(&mut guard, session.owner_mut())?;
            #[cfg(not(feature = "diagnostic-traces"))]
            let occupied_snapshot = occupied.capture_with_step(|| guard.step())?;
            let trial = catch_unwind(AssertUnwindSafe(|| {
                self.optimize_guarded(workspace, session, context, temperature, &mut guard)?;
                finish(session, &mut guard)?;
                Ok::<(), OwnerPlacementError>(())
            }));
            if !matches!(&trial, Ok(Ok(()))) {
                scratch
                    .restore(session.owner_mut())
                    .expect("captured optimizer owner binding");
                occupied_snapshot.restore(&mut self.occupied().borrow_mut());
            }
            match trial {
                Ok(result) => result,
                Err(panic) => resume_unwind(panic),
            }
        }));
        scratch.release();
        self.retain_mutation_scratch(scratch);
        match result {
            Ok(result) => result,
            Err(panic) => resume_unwind(panic),
        }
    }

    pub(super) fn optimize_guarded(
        &self,
        workspace: &ArenaGraph,
        session: &mut OwnerScoreSession<'_>,
        context: &dyn Context,
        temperature: f64,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OwnerPlacementError> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(SizelessOptimize);
        let count = self.nodes().len();
        if count > 10000 {
            return Err(OptimizationError::Invalid(
                "TALA sizelessOptimizer.optimize node count exceeds limit 10000".into(),
            )
            .into());
        }
        let mut indices = Vec::with_capacity(count);
        for index in 0..count {
            step(session, guard)?;
            indices.push(index);
        }
        let rng = self.rng().ok_or_else(|| {
            OptimizationError::Invalid(format!(
                "TALA {} shuffle requires a random generator",
                guard.location()
            ))
        })?;
        rng.shuffle_owner(&mut indices, session.owner_mut(), guard)?;
        for index in indices {
            step(session, guard)?;
            let node = self
                .nodes()
                .get(index)
                .expect("source optimizer member index");
            let node = node.ok_or_else(|| {
                OptimizationError::Invalid(
                    "TALA sizelessOptimizer.optimize found an unpositioned node".into(),
                )
            })?;
            let point = session.owner().position(node).ok_or_else(|| {
                OptimizationError::Invalid(
                    "TALA sizelessOptimizer.optimize found an unpositioned node".into(),
                )
            })?;
            self.occupied().borrow().delete(point);
            let median =
                self.median_point_guarded(session.owner_mut(), node, temperature, guard)?;
            let distance = self
                .find_closest_unoccupied_distance_guarded(
                    session.owner_mut(),
                    Some(node),
                    Some(&median),
                    guard,
                )
                .map_err(|error| {
                    let name = &session.owner().nodes[node.0 as usize].external_id;
                    OwnerPlacementError::Optimization(OptimizationError::Invalid(format!(
                        "could not find an unoccupied distance for {name}: {error}"
                    )))
                })?;
            let mut points = self.placement_points_guarded(
                session.owner_mut(),
                Some(node),
                Some(&median),
                distance,
                guard,
            )?;
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_SIZELESS_POINTS") {
                let point_bits = points
                    .iter()
                    .map(|point| {
                        let point = point.get();
                        serde_json::json!({
                            "x": format!("{:016x}", point.x.to_bits()),
                            "y": format!("{:016x}", point.y.to_bits()),
                        })
                    })
                    .collect::<Vec<_>>();
                let median_point = median.get();
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "sizeless-points",
                        "node_id": session.owner().nodes[node.0 as usize].external_id,
                        "start_x_bits": format!("{:016x}", point.x.to_bits()),
                        "start_y_bits": format!("{:016x}", point.y.to_bits()),
                        "median_x_bits": format!("{:016x}", median_point.x.to_bits()),
                        "median_y_bits": format!("{:016x}", median_point.y.to_bits()),
                        "distance_bits": format!("{:016x}", distance.to_bits()),
                        "points": point_bits,
                    })
                );
            }
            rng.shuffle_owner(&mut points, session.owner_mut(), guard)?;
            let local = session
                .current_mapping()
                .nodes()
                .iter()
                .position(|&n| n == node)
                .map(|index| NodeId(index as u32))
                .ok_or_else(|| {
                    OptimizationError::Invalid(format!(
                        "owner movement has no canonical mapping for {node:?}"
                    ))
                })?;
            let moved = super::sizeless::search_owner_node_to_best(
                workspace,
                local,
                points.into_iter(),
                |point| Some(point.get()),
                session,
                context,
                guard,
            )?;
            if !moved {
                let mapping = session.current_mapping().clone();
                let candidate = self.best_swap_candidate_guarded(
                    workspace,
                    session.owner_mut(),
                    &mapping,
                    Some(node),
                    context,
                    guard,
                )?;
                if let Some(candidate) = candidate {
                    self.commit_swap_guarded(
                        session.owner_mut(),
                        Some(node),
                        Some(candidate),
                        guard,
                    )?;
                    let point = session
                        .owner()
                        .position(candidate)
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    self.occupied().borrow().insert(point, Some(candidate));
                }
            }
            let point = session
                .owner()
                .position(node)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            self.occupied().borrow().insert(point, Some(node));
        }
        finish(session, guard)?;
        Ok(())
    }
}
