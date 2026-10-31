// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! compaction.go candidateMoves: validate the entire range before allocation,
//! then charge the real loop and discard partial results on cancellation.
use super::*;
pub(super) struct CompactionContext;
impl limits::Context for CompactionContext {
    fn stable_compaction_workspace(&self) -> bool {
        true
    }
    fn done(&self) -> Option<bool> {
        crate::internal::cancellation_done()
    }
    fn error(&self) -> Option<limits::ContextError> {
        crate::internal::cancellation_error()
    }
}
pub(super) fn optimization_error(error: limits::Error) -> limits::OptimizationError {
    match error {
        limits::Error::Context { location, cause } => {
            limits::OptimizationError::Context { location, cause }
        }
        error => limits::OptimizationError::Invalid(error.to_string()),
    }
}
pub(super) fn positions(
    guard: &mut limits::WorkGuard<'_>,
    start: f64,
    ceiling: f64,
    current: Point,
    horizontal: bool,
    scale: f64,
) -> Result<Vec<Point>, limits::Error> {
    if !start.is_finite() || !ceiling.is_finite() {
        return Err(limits::Error::Invalid(
            "layout invariant violated: compaction candidate range is not finite".into(),
        ));
    }
    let count = if start <= ceiling {
        (ceiling - start).floor() + 1.0
    } else {
        0.0
    };
    const MAX_CANDIDATES: usize = 30_000 + 3;
    if count > MAX_CANDIDATES as f64 {
        return Err(limits::Error::Invalid(format!(
            "TALA compaction candidate count {count:.0} exceeds limit {MAX_CANDIDATES}"
        )));
    }
    let mut points = Vec::with_capacity(count as usize);
    let mut axis = start;
    while axis <= ceiling {
        guard.step()?;
        points.push(if horizontal {
            Point {
                x: axis * scale,
                y: current.y,
            }
        } else {
            Point {
                x: current.x,
                y: axis * scale,
            }
        });
        axis += 1.0;
    }
    guard.finish()?;
    Ok(points)
}

pub(super) fn validate_factor(factor: f64) -> Result<(), limits::OptimizationError> {
    if factor <= 0.0 || !factor.is_finite() {
        return Err(limits::OptimizationError::Invalid(
            "TALA Compaction requires a finite positive factor".into(),
        ));
    }
    Ok(())
}
pub(super) fn check_context(
    context: &dyn limits::Context,
) -> Result<(), limits::OptimizationError> {
    if let Some(cause) = context.error() {
        return Err(limits::OptimizationError::Context {
            location: "Compaction".into(),
            cause,
        });
    }
    Ok(())
}

// compaction.go: zero and values above the package ceiling select the default.
pub(super) fn normalized_move_work_limit(requested: u64) -> u64 {
    if requested == 0 || requested > limits::MAX_OPTIMIZATION_WORK_UNITS {
        limits::MAX_OPTIMIZATION_WORK_UNITS
    } else {
        requested
    }
}
