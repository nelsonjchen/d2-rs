// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! sized_optimizer.go445–526, using actual owner pointers and the caller RNG.
use super::super::container_slices::SharedNodeSlice;
use super::super::limits::{OptimizationError, OptimizationWorkGuard};
use super::super::owner_median::{optimizer_owner_median, source_math_max};
use super::super::owner_placement::{optimizer_owner_fixed_origin, owner_owning_container};
use super::super::{NodeId, OwnerScoreSession, Point};

impl super::SizedOptimizer<'_> {
    pub(super) fn owner_candidate_median(
        &mut self,
        node: NodeId,
        temperature: f64,
        session: &mut OwnerScoreSession<'_>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(Point, usize), OptimizationError> {
        session
            .update_current_member_positions(self.graph)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let canonical = session
            .canonical_node(node)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let (slots, present) = session.abductions().map_or_else(
            || (Vec::new(), false),
            |records| {
                (
                    records.records.iter().copied().map(Some).collect(),
                    records.present,
                )
            },
        );
        let abductions = session
            .owner_mut()
            .allocate_owner_abduction_header(&slots, slots.len(), present)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let mut children = SharedNodeSlice::with_len_cap(0, 0)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        // Go range loads the record pointer before Step, then reads the
        // record's endpoint fields after the callback. Preserve duplicates.
        for record in abductions.iter_slots() {
            Self::step_owner_work(Some(session), Some(guard))?;
            let record = record.ok_or_else(|| {
                OptimizationError::Invalid("TALA LocalOptimize found a nil edge abduction".into())
            })?;
            let record = session
                .owner()
                .owner_edge_abduction(record)
                .ok_or_else(|| {
                    OptimizationError::Invalid(format!("unknown owner abduction {record:?}"))
                })?;
            for (current, original) in [
                (record.current_from, record.originally_from),
                (record.current_to, record.originally_to),
            ] {
                if current == Some(canonical)
                    && let Some(original) = original
                {
                    children = children
                        .append_node(original)
                        .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
                }
            }
        }
        // medianPointGuarded captures the moving dimensions after collecting
        // protruding children, before neighbor/fixed-origin callbacks.
        let size = session.owner().nodes[canonical.0 as usize].rect.size;
        let width = size.width / self.cell_size;
        let height = size.height / self.cell_size;
        let mut median =
            self.owner_neighbor_median_after_sync(node, temperature, session, guard)?;
        let container = owner_owning_container(session.owner(), canonical)
            .map_err(|error| OptimizationError::Invalid(error.to_string()))?;
        let receiver = session.current_view();
        let origin =
            optimizer_owner_fixed_origin(session.owner_mut(), Some(receiver), container, guard)
                .map_err(OptimizationError::from)?;
        if let Some(origin) = origin {
            if median.x < origin.x / self.cell_size {
                median.x = origin.x / self.cell_size;
                median.x += temperature * width;
            }
            if median.y < origin.y / self.cell_size {
                median.y = origin.y / self.cell_size;
                median.y += temperature * height;
            }
        }
        median.x += self
            .rng
            .float64()
            .mul_add(2.0 * temperature * width, -temperature * width);
        median.y += self
            .rng
            .float64()
            .mul_add(2.0 * temperature * height, -temperature * height);
        if !children.is_empty() {
            let child_median = optimizer_owner_median(session.owner_mut(), &children, true, guard)
                .map_err(OptimizationError::from)?;
            let position = session
                .owner()
                .position(canonical)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            median.x -= child_median.x - position.x / self.cell_size;
            median.y -= child_median.y - position.y / self.cell_size;
        }
        if let Some(origin) = origin {
            median.x = source_math_max(median.x, origin.x / self.cell_size);
            median.y = source_math_max(median.y, origin.y / self.cell_size);
        }
        Ok((
            Point {
                x: (median.x * self.cell_size).round(),
                y: (median.y * self.cell_size).round(),
            },
            children.len(),
        ))
    }
}
