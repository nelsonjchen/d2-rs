// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Deterministic reduction of unnecessary gaps between placed structures.
//!
//! Directional translation trials use transaction rollback and the recovered
//! open precision comparison to avoid changing equal-score layouts.

use super::*;

const GAP_NORMALIZATION_PRECISION: f64 = 0.0001;

// Exact translation of geo.PrecisionCompare as inlined at recovered
// gapreduction.go:542 and :628. The equality boundary is open, and Go's
// final branch returns Greater when either operand is NaN.
fn gap_normalization_precision_compare(moved: f64, baseline: f64) -> std::cmp::Ordering {
    if (moved - baseline).abs() < GAP_NORMALIZATION_PRECISION {
        std::cmp::Ordering::Equal
    } else if moved < baseline {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Greater
    }
}

#[cfg(test)]
mod gap_normalization_precision_tests {
    use super::{GAP_NORMALIZATION_PRECISION, gap_normalization_precision_compare};
    use std::cmp::Ordering;

    #[test]
    fn inner_score_comparison_matches_recovered_open_precision_boundary() {
        let just_inside = GAP_NORMALIZATION_PRECISION.next_down();
        let just_outside = GAP_NORMALIZATION_PRECISION.next_up();

        assert_eq!(
            gap_normalization_precision_compare(-just_inside, 0.0),
            Ordering::Equal
        );
        assert_eq!(
            gap_normalization_precision_compare(just_inside, 0.0),
            Ordering::Equal
        );
        assert_eq!(
            gap_normalization_precision_compare(-GAP_NORMALIZATION_PRECISION, 0.0),
            Ordering::Less
        );
        assert_eq!(
            gap_normalization_precision_compare(GAP_NORMALIZATION_PRECISION, 0.0),
            Ordering::Greater
        );
        assert_eq!(
            gap_normalization_precision_compare(-just_outside, 0.0),
            Ordering::Less
        );
        assert_eq!(
            gap_normalization_precision_compare(just_outside, 0.0),
            Ordering::Greater
        );
    }

    #[test]
    fn inner_score_comparison_matches_recovered_nan_fallthrough() {
        assert_eq!(
            gap_normalization_precision_compare(f64::NAN, 0.0),
            Ordering::Greater
        );
        assert_eq!(
            gap_normalization_precision_compare(0.0, f64::NAN),
            Ordering::Greater
        );
        assert_eq!(
            gap_normalization_precision_compare(f64::NAN, f64::NAN),
            Ordering::Greater
        );
    }
}

enum OrdinaryGapAttempt {
    Accepted { reported_score: f64 },
    Illegal,
    Regression,
}

// Go returns its last accepted score even when a subsequent refresh fails.
// Callers that only handle success use into_result; the source-shaped outcome
// remains available for precise internal contracts and failure qualification.
struct GapReductionOutcome {
    moved: bool,
    score: f64,
    error: Option<limits::Error>,
}
impl GapReductionOutcome {
    fn into_result(self) -> Result<(bool, f64), limits::Error> {
        match self.error {
            Some(error) => Err(error),
            None => Ok((self.moved, self.score)),
        }
    }
}

pub(super) type GapTrialSnapshot = super::trial_geometry::TrialGeometrySnapshot;

impl ArenaGraph {
    pub(super) fn gap_herd_node(&self, node: NodeId) -> NodeId {
        // gapreduction.go301–333 reads the actual vessel's HerdAssignment.
        // A numerical carrier is an absorbed member and has its own distinct
        // (often nil) assignment. Resolve only the explicit vessel binding.
        if let Some(index) = self.active_cluster_index(node)
            && self.clusters[index].members.first() == Some(&node)
            && let Some(vessel) =
                self.topology_vessel_node(crate::internal::TopologyVessel::Cluster(index))
        {
            return vessel;
        }
        if let Some(index) = self.active_sequence_index(node)
            && self.sequences[index].members.first() == Some(&node)
            && let Some(vessel) =
                self.topology_vessel_node(crate::internal::TopologyVessel::Sequence(index))
        {
            return vessel;
        }
        node
    }
    pub(super) fn gap_trial_snapshot(&self) -> GapTrialSnapshot {
        self.trial_geometry_snapshot()
    }
    pub(super) fn restore_gap_trial_snapshot(&mut self, snapshot: GapTrialSnapshot) {
        self.restore_trial_geometry_snapshot(snapshot);
    }

    pub(super) fn ordinary_gap_trial_is_valid(&self, prior: &Self) -> bool {
        self.ordinary_gap_trial_is_valid_with_container_escape(prior, false)
    }

    /// Validate the source-container orientation transaction, whose upstream
    /// options set `IgnoreContainerEscape` while retaining the other graph
    /// invariants and overlap checks.
    pub(super) fn ordinary_gap_trial_is_valid_ignoring_container_escape(
        &self,
        prior: &Self,
    ) -> bool {
        self.ordinary_gap_trial_is_valid_with_container_escape(prior, true)
    }

    fn ordinary_gap_trial_is_valid_with_container_escape(
        &self,
        prior: &Self,
        ignore_container_escape: bool,
    ) -> bool {
        let prior_overlaps = prior.existing_overlap_pairs();
        let prior_exact_overlaps = prior.exact_overlap_pairs_from(&prior_overlaps);
        self.ordinary_gap_trial_is_valid_with_overlaps(
            prior,
            &prior_overlaps,
            &prior_exact_overlaps,
            ignore_container_escape,
        )
    }

    fn ordinary_gap_trial_is_valid_with_overlaps(
        &self,
        prior: &Self,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        ignore_container_escape: bool,
    ) -> bool {
        let prior_active_boxes = prior
            .nodes
            .iter()
            .map(|node| {
                (
                    prior.active_node_position(node.input_id),
                    prior.active_node_size(node.input_id),
                )
            })
            .collect::<Vec<_>>();
        let prior_positions = prior
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>();
        self.ordinary_gap_trial_is_valid_against_with_container_escape(
            prior_overlaps,
            prior_exact_overlaps,
            &prior_active_boxes,
            &prior_positions,
            ignore_container_escape,
        )
    }

    pub(super) fn ordinary_gap_trial_is_valid_against(
        &self,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
    ) -> bool {
        self.ordinary_gap_trial_is_valid_against_with_container_escape(
            prior_overlaps,
            prior_exact_overlaps,
            prior_active_boxes,
            prior_positions,
            false,
        )
    }

    fn ordinary_gap_trial_is_valid_against_with_container_escape(
        &self,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        ignore_container_escape: bool,
    ) -> bool {
        // Transaction.Commit iterates TALA's current Graph.Nodes slice. Once
        // AddSequences/AddClusters has installed a vessel, the retained
        // members are no longer in that slice even though the stable arena
        // still owns their boxes for cleanup. Do not let those hidden boxes
        // participate in current-graph validation.
        let graph_nodes = self.graph_node_order();
        let moved = graph_nodes
            .iter()
            .copied()
            .filter(|&node| {
                let (prior_position, prior_size) = prior_active_boxes[node.0 as usize];
                self.active_node_position(node) != prior_position
                    || self.active_node_size(node) != prior_size
            })
            .collect::<BTreeSet<_>>();
        let fixed_moved = self
            .nodes
            .iter()
            .zip(prior_positions)
            .any(|(candidate, old)| {
                candidate.fixed_top_left.is_some() && candidate.position != *old
            });
        // Preserve the validation short-circuit order. Diagnostics report only
        // the first failed predicate and never evaluate additional checks.
        macro_rules! require_valid {
            ($reason:literal, $condition:expr) => {
                if !$condition {
                    #[cfg(feature = "diagnostic-traces")]
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP_VALIDATION") {
                        eprintln!(
                            "GAP_INVALID_RUST reason={} moved={:?}",
                            $reason,
                            moved
                                .iter()
                                .map(|&node| self.active_node_tala_id(node))
                                .collect::<Vec<_>>()
                        );
                    }
                    return false;
                }
            };
        }
        // Transaction.Commit checks container overlaps immediately after
        // refitting and before publishing cluster geometry. A post-sync
        // projection must not erase an illegal pre-sync container overlap.
        require_valid!(
            "container-overlap",
            !self.transaction_containers_have_bad_state_overlap(prior_overlaps)
        );
        // Go's Transaction.Commit validates the shared *Node boxes after a
        // vessel move. A hidden cluster member can retain a nested container
        // in the induced Rust frame until the transaction is accepted, while
        // the Go pointer already observes the member translation. Project
        // those descendants on a validation-only clone so overlap checks use
        // the same pointer-visible coordinates without publishing the move
        // to the live graph.
        let validation_projection = self
            .clusters
            .iter()
            .any(|cluster| {
                cluster.members.iter().any(|member| {
                    !self.node_order_contains(*member)
                        && self
                            .active_descendants(*member)
                            .iter()
                            .any(|descendant| self.nodes[descendant.0 as usize].is_container)
                })
            })
            .then(|| {
                let mut projected = self.clone();
                projected.sync_clusters_with_descendants();
                projected
            });
        let validation = validation_projection.as_ref().unwrap_or(self);
        // Source Transaction.Commit has no graph-size predicate. The pipeline
        // validates completed GapNormalization bounds; alignment/orientation
        // retain their separate trial limits.
        require_valid!(
            "new-overlap",
            !validation.transaction_has_new_overlap_for_nodes(prior_overlaps, &moved)
        );
        require_valid!(
            "bad-state-overlap",
            !validation.transaction_has_bad_state_overlap_for_nodes(prior_overlaps, &moved)
        );
        require_valid!(
            "spacing-became-exact",
            !validation.existing_spacing_overlap_became_exact(prior_overlaps, prior_exact_overlaps)
        );
        // IgnoreContainerEscape relaxes containment only. Go's transaction
        // still rejects any current node behind its container's fixed origin,
        // independently of whether the fixed node itself moved.
        require_valid!(
            "fixed-origin",
            validation.transaction_fixed_origin_is_valid()
        );
        require_valid!("fixed-moved", !fixed_moved);
        if !ignore_container_escape {
            require_valid!("containment", validation.transaction_containment_is_valid());
        }
        require_valid!(
            "external-containers",
            validation.transaction_external_containers_are_valid()
        );
        true
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_gap_inner(
        &self,
        node: NodeId,
        horizontal: bool,
        amount: f64,
        baseline: f64,
        score: Option<f64>,
        decision: &str,
    ) {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP") {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "gap-inner", "node_id": self.active_node_tala_id(node).to_string(),
                    "horizontal": horizontal, "amount_bits": format!("0x{:016x}", amount.to_bits()),
                    "old_score_bits": format!("0x{:016x}", baseline.to_bits()),
                    "score_bits": score.map(|v| format!("0x{:016x}", v.to_bits())), "decision": decision,
                })
            );
        }
    }

    fn ordinary_gap_inner_trial(
        &mut self,
        node: NodeId,
        horizontal: bool,
        amount: f64,
        baseline: f64,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
    ) -> Option<f64> {
        gap_guard::with_background_work(|work| {
            self.ordinary_gap_inner_trial_checked(
                node,
                horizontal,
                amount,
                baseline,
                prior_overlaps,
                prior_exact_overlaps,
                prior_active_boxes,
                prior_positions,
                work,
            )
        })
    }

    fn ordinary_gap_inner_trial_checked(
        &mut self,
        node: NodeId,
        horizontal: bool,
        amount: f64,
        baseline: f64,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        work: &mut gap_guard::GapWork<'_, '_>,
    ) -> Result<Option<f64>, limits::Error> {
        let mut checkpoint = GeometryCheckpoint::from_snapshot(self.gap_trial_snapshot());
        self.ordinary_gap_inner_trial_with_checkpoint_checked(
            node,
            horizontal,
            amount,
            baseline,
            prior_overlaps,
            prior_exact_overlaps,
            prior_active_boxes,
            prior_positions,
            work,
            &mut checkpoint,
            true,
        )
    }

    fn ordinary_gap_inner_trial_with_checkpoint_checked(
        &mut self,
        node: NodeId,
        horizontal: bool,
        amount: f64,
        baseline: f64,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        work: &mut gap_guard::GapWork<'_, '_>,
        checkpoint: &mut GeometryCheckpoint,
        refresh: bool,
    ) -> Result<Option<f64>, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GapInnerTrial);
        work.anchor_numeric_overlaps(self, prior_overlaps, prior_exact_overlaps);
        // The first inner attempt refreshes the enclosing candidate; a fallback
        // reuses its rejected prior and refreshes only after acceptance.
        if refresh {
            work.refresh_geometry_request(self, checkpoint)?;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work.before_operation(self)?;
            self.translate_active_node_with_children(
                node,
                Point {
                    x: if horizontal { amount } else { 0.0 },
                    y: if horizontal { 0.0 } else { amount },
                },
            );
            // Commit refits containers, synchronizes aggregate geometry, then
            // validates that published state before an inner-boundary score.
            work.check_context(self)?;
            if !work.refit_containers(self, checkpoint)? {
                return Ok(None);
            }
            self.sync_clusters_with_descendants();
            self.sync_sequences();
            work.check_context(self)?;
            let valid = work.validate_commit(self, checkpoint)?;
            if !valid {
                #[cfg(feature = "diagnostic-traces")]
                self.trace_gap_inner(node, horizontal, amount, baseline, None, "invalid");
                return Ok(None);
            }
            // GapNormalization runs after PlaceTrees has restored the live
            // Graph.Containers membership. Recovered Node.edgeLength reads that
            // current membership, rather than the pre-PlaceTrees snapshot used by
            // hierarchy construction scoring.
            let score = work.score(self)?;
            if gap_normalization_precision_compare(score, baseline) == std::cmp::Ordering::Less {
                #[cfg(feature = "diagnostic-traces")]
                self.trace_gap_inner(node, horizontal, amount, baseline, Some(score), "accepted");
                // Keep an accepted transaction in place. The caller immediately
                // adopts this state, so cloning the complete ArenaGraph only to
                // restore the snapshot and assign it back would duplicate the
                // entire hot-path trial.
                Ok(Some(score))
            } else {
                #[cfg(feature = "diagnostic-traces")]
                self.trace_gap_inner(
                    node,
                    horizontal,
                    amount,
                    baseline,
                    Some(score),
                    "regression",
                );
                Ok(None)
            }
        }));
        match result {
            Ok(Ok(Some(score))) => Ok(Some(score)),
            failed => {
                checkpoint.rollback(self);
                match failed {
                    Ok(result) => result,
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            }
        }
    }

    pub(super) fn nearest_shared_container(&self, left: NodeId, right: NodeId) -> Option<NodeId> {
        let mut left_chain = Vec::new();
        let mut current = self.active_node_container(left);
        while let Some(container) = current {
            left_chain.push(container);
            current = self.active_node_container(container);
        }
        let mut current = self.active_node_container(right);
        while let Some(container) = current {
            if left_chain.contains(&container) {
                return Some(container);
            }
            current = self.active_node_container(container);
        }
        None
    }

    fn node_is_blocked_between(
        &self,
        candidate: ConnectedNode,
        behind: ConnectedNode,
        ahead: ConnectedNode,
        horizontal: bool,
    ) -> bool {
        let (
            Some((candidate_position, candidate_size)),
            Some((behind_position, behind_size)),
            Some((ahead_position, ahead_size)),
        ) = (
            self.connected_node_box(candidate),
            self.connected_node_box(behind),
            self.connected_node_box(ahead),
        )
        else {
            return false;
        };
        if horizontal {
            candidate_position.x >= behind_position.x + behind_size.width
                && candidate_position.x + candidate_size.width <= ahead_position.x
                && candidate_position.y <= behind_position.y.max(ahead_position.y)
                && candidate_position.y + candidate_size.height
                    >= (behind_position.y + behind_size.height)
                        .min(ahead_position.y + ahead_size.height)
        } else {
            candidate_position.y >= behind_position.y + behind_size.height
                && candidate_position.y + candidate_size.height <= ahead_position.y
                && candidate_position.x <= behind_position.x.max(ahead_position.x)
                && candidate_position.x + candidate_size.width
                    >= (behind_position.x + behind_size.width)
                        .min(ahead_position.x + ahead_size.width)
        }
    }

    fn promote_gap_candidate(
        &self,
        mut candidate: NodeId,
        target_container: Option<NodeId>,
    ) -> Option<NodeId> {
        while self.active_node_container(candidate) != target_container {
            candidate = self.active_node_container(candidate)?;
        }
        Some(candidate)
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_gap_trial(
        &self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
        phase: &str,
        score: f64,
        decision: &str,
    ) {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP") {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "gap-trial", "node_id": self.active_node_tala_id(node).to_string(),
                    "horizontal": horizontal, "forwards": forwards, "phase": phase,
                    "score_bits": format!("0x{:016x}", score.to_bits()), "decision": decision,
                })
            );
        }
    }

    fn ordinary_gap_direct_attempt(
        &mut self,
        node: NodeId,
        connected: &[ConnectedNode],
        horizontal: bool,
        forwards: bool,
        amount: f64,
        recover_symmetry: bool,
        old_score: f64,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        work: &mut gap_guard::GapWork<'_, '_>,
        checkpoint: &mut GeometryCheckpoint,
        backward_checkpoint: &mut Option<GeometryCheckpoint>,
    ) -> Result<OrdinaryGapAttempt, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GapInnerTrial);
        work.refresh_geometry_request(self, checkpoint)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work.before_operation(self)?;
            self.translate_connected_node_boxes(
                connected,
                Point {
                    x: if horizontal { amount } else { 0.0 },
                    y: if horizontal { 0.0 } else { amount },
                },
            );
            // This call is inside TALA's transaction operation, before
            // Transaction.Commit refits ordinary containers.
            self.sync_clusters();
            self.sync_sequences();
            let candidate_score = work.score(self)?;
            #[cfg(feature = "diagnostic-traces")]
            self.trace_gap_trial(
                node,
                horizontal,
                forwards,
                "score",
                candidate_score,
                "pending",
            );

            let mut mirror_accepted = false;
            if recover_symmetry {
                if backward_checkpoint.is_none() {
                    #[cfg(feature = "diagnostic-traces")]
                    work.observe(self, limits::TransactionPollSite::InnerGeometryClone);
                    *backward_checkpoint = Some(checkpoint.clone_geometry(work.guard)?);
                }
                let backward = backward_checkpoint
                    .as_mut()
                    .expect("initialized backward checkpoint");
                backward.update(self, work.guard)?;
                #[cfg(feature = "diagnostic-traces")]
                work.observe(self, limits::TransactionPollSite::InnerGeometryClone);
                let mut mirrored = backward.clone_geometry(work.guard)?;
                let mirror_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work.before_operation(self)?;

                    // backwardTxn.UpdateState refreshes geometry after the outer
                    // direct movement, but Go deliberately retains the transaction's
                    // original overlap exceptions. Mirror that split baseline: the
                    // nested trial sees the post-direct boxes while a newly created
                    // overlap is still judged against the enclosing transaction's
                    // anchored overlap map.
                    let mirror_prior_active_boxes = self
                        .nodes
                        .iter()
                        .map(|candidate| {
                            (
                                self.active_node_position(candidate.input_id),
                                self.active_node_size(candidate.input_id),
                            )
                        })
                        .collect::<Vec<_>>();
                    let mirror_prior_positions = self
                        .nodes
                        .iter()
                        .map(|candidate| candidate.position)
                        .collect::<Vec<_>>();
                    let (moved, mirrored_score) = self
                        .reduce_gap_to_neighbors_scored_with_checkpoint_checked(
                            node,
                            horizontal,
                            !forwards,
                            false,
                            prior_overlaps,
                            prior_exact_overlaps,
                            &mirror_prior_active_boxes,
                            &mirror_prior_positions,
                            work,
                            &mut mirrored,
                        )?;
                    if moved && mirrored_score < candidate_score && mirrored_score < old_score {
                        // The Go symmetry probe runs in a nested request
                        // transaction. Its successful Commit performs one complete
                        // refit/publication cycle before the enclosing candidate is
                        // committed. Reproduce that boundary here; omitting it
                        // leaves half-pixel cluster members on the opposite side of
                        // the next outer SyncClusters rounding tie.
                        work.check_context(self)?;
                        if !work.refit_containers(self, backward)? {
                            backward.rollback(self);
                            return Ok(());
                        }
                        self.sync_clusters();
                        self.sync_sequences();
                        work.check_context(self)?;
                        // backwardTxn.Commit validates against its own refreshed prior,
                        // not the inner reducer snapshot updated during its trials.
                        if work.validate_commit(self, backward)? {
                            mirror_accepted = true;
                        } else {
                            backward.rollback(self);
                        }
                    } else {
                        backward.rollback(self);
                    }
                    Ok::<(), limits::Error>(())
                }));
                match mirror_result {
                    Ok(Ok(())) => {}
                    failed => {
                        backward.rollback(self);
                        match failed {
                            Ok(Err(error)) => return Err(error),
                            Err(payload) => std::panic::resume_unwind(payload),
                            Ok(Ok(())) => unreachable!(),
                        }
                    }
                }
            }
            // TALA accepts only a strict f64 improvement here. Keep the release
            // comparison exact, including the f64 ordering boundary.
            if !mirror_accepted && candidate_score >= old_score {
                #[cfg(feature = "diagnostic-traces")]
                self.trace_gap_trial(
                    node,
                    horizontal,
                    forwards,
                    "decision",
                    candidate_score,
                    "regression",
                );
                return Ok(OrdinaryGapAttempt::Regression);
            }

            // Transaction.Commit refits containers and validates the resulting
            // graph, but the caller retains `candidate_score` from the operation.
            // Commit refits containers and publishes aggregate geometry before
            // its invariant and overlap validation.
            work.check_context(self)?;
            if !work.refit_containers(self, checkpoint)? {
                return Ok(OrdinaryGapAttempt::Illegal);
            }
            self.sync_clusters();
            self.sync_sequences();
            work.check_context(self)?;
            let valid = work.validate_commit(self, checkpoint)?;
            if !valid {
                #[cfg(feature = "diagnostic-traces")]
                self.trace_gap_trial(
                    node,
                    horizontal,
                    forwards,
                    "decision",
                    candidate_score,
                    "invalid",
                );
                return Ok(OrdinaryGapAttempt::Illegal);
            }
            #[cfg(feature = "diagnostic-traces")]
            self.trace_gap_trial(
                node,
                horizontal,
                forwards,
                "decision",
                candidate_score,
                "accepted",
            );

            Ok(OrdinaryGapAttempt::Accepted {
                reported_score: candidate_score,
            })
        }));
        match result {
            Ok(Ok(accepted @ OrdinaryGapAttempt::Accepted { .. })) => Ok(accepted),
            failed => {
                checkpoint.rollback(self);
                match failed {
                    Ok(result) => result,
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            }
        }
    }

    pub(super) fn nearest_connected_ahead(
        &self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
    ) -> Option<NodeId> {
        let position = self.active_node_position(node)?;
        let size = self.active_node_size(node);
        let mut nearest = None;
        for edge in self.active_edge_ids(node) {
            let adjacent = self.active_adjacent(node, edge);
            let Some(other) = self.active_node_position(adjacent) else {
                continue;
            };
            let other_size = self.active_node_size(adjacent);
            let eligible = if horizontal {
                if forwards {
                    other.x >= position.x + size.width
                } else {
                    other.x + other_size.width <= position.x
                }
            } else if forwards {
                other.y >= position.y + size.height
            } else {
                other.y + other_size.height <= position.y
            };
            if !eligible {
                continue;
            }
            let coordinate = if horizontal {
                if forwards {
                    other.x
                } else {
                    other.x + other_size.width
                }
            } else if forwards {
                other.y
            } else {
                other.y + other_size.height
            };
            if nearest.is_none_or(|current| {
                let current_position = self.active_node_position(current).unwrap();
                let current_size = self.active_node_size(current);
                let current_coordinate = if horizontal {
                    if forwards {
                        current_position.x
                    } else {
                        current_position.x + current_size.width
                    }
                } else if forwards {
                    current_position.y
                } else {
                    current_position.y + current_size.height
                };
                if forwards {
                    coordinate < current_coordinate
                } else if coordinate > current_coordinate {
                    true
                } else {
                    // Node.getNearestConnectedAhead keeps the first edge in
                    // the receiver's edge slice on an exact coordinate tie.
                    // Do not add a container-based tie-break: the recovered
                    // Go loop only replaces `nearest` on a strict nearer
                    // comparison, so edge order is observable here.
                    false
                }
            }) {
                nearest = Some(adjacent);
            }
        }
        nearest
    }

    pub(super) fn reduce_gap_to_neighbors(
        &mut self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
    ) -> bool {
        let prior_overlaps = self.existing_overlap_pairs();
        let prior_exact_overlaps = self.exact_overlap_pairs();
        let prior_active_boxes = self
            .nodes
            .iter()
            .map(|candidate| {
                (
                    self.active_node_position(candidate.input_id),
                    self.active_node_size(candidate.input_id),
                )
            })
            .collect::<Vec<_>>();
        let prior_positions = self
            .nodes
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>();
        gap_guard::with_background_work(|work| {
            self.reduce_gap_to_neighbors_scored_with_overlaps_checked(
                node,
                horizontal,
                forwards,
                recover_symmetry,
                &prior_overlaps,
                &prior_exact_overlaps,
                &prior_active_boxes,
                &prior_positions,
                work,
            )
        })
        .0
    }

    fn reduce_gap_to_neighbors_scored(
        &mut self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
    ) -> (bool, f64) {
        let prior_overlaps = self.existing_overlap_pairs();
        let prior_exact_overlaps = self.exact_overlap_pairs();
        let prior_active_boxes = self
            .nodes
            .iter()
            .map(|candidate| {
                (
                    self.active_node_position(candidate.input_id),
                    self.active_node_size(candidate.input_id),
                )
            })
            .collect::<Vec<_>>();
        let prior_positions = self
            .nodes
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>();
        gap_guard::with_background_work(|work| {
            self.reduce_gap_to_neighbors_scored_with_overlaps_checked(
                node,
                horizontal,
                forwards,
                recover_symmetry,
                &prior_overlaps,
                &prior_exact_overlaps,
                &prior_active_boxes,
                &prior_positions,
                work,
            )
        })
    }

    /// Source reduceGapToNeighbors with a nil transaction, used by the
    /// OptimizeClusters enclosing operation. The request is captured lazily.
    pub(super) fn cluster_optimizer_gap_checked(
        &mut self,
        node: NodeId,
        horizontal: bool,
        work: &mut gap_guard::GapWork<'_, '_>,
    ) -> Result<(), limits::Error> {
        let overlaps = self.existing_overlap_pairs();
        let exact = self.exact_overlap_pairs();
        let boxes = self
            .nodes
            .iter()
            .map(|n| {
                (
                    self.active_node_position(n.input_id),
                    self.active_node_size(n.input_id),
                )
            })
            .collect::<Vec<_>>();
        let points = self.nodes.iter().map(|n| n.position).collect::<Vec<_>>();
        // A nil reducer request owns a separate exception baseline. Keep
        // the enclosing Commit anchored to its original request snapshot.
        let mut reducer = gap_guard::GapWork::new(work.guard, work.operation_context, false);
        reducer.begin_unbound_request();
        self.reduce_gap_to_neighbors_scored_with_overlaps_checked(
            node,
            horizontal,
            true,
            true,
            &overlaps,
            &exact,
            &boxes,
            &points,
            &mut reducer,
        )?;
        Ok(())
    }

    fn reduce_gap_to_neighbors_scored_with_overlaps_checked(
        &mut self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        work: &mut gap_guard::GapWork<'_, '_>,
    ) -> Result<(bool, f64), limits::Error> {
        let mut checkpoint = GeometryCheckpoint::from_snapshot(self.gap_trial_snapshot());
        self.reduce_gap_to_neighbors_scored_with_checkpoint_checked(
            node,
            horizontal,
            forwards,
            recover_symmetry,
            prior_overlaps,
            prior_exact_overlaps,
            prior_active_boxes,
            prior_positions,
            work,
            &mut checkpoint,
        )
    }

    fn reduce_gap_to_neighbors_scored_with_checkpoint_checked(
        &mut self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        work: &mut gap_guard::GapWork<'_, '_>,
        checkpoint: &mut GeometryCheckpoint,
    ) -> Result<(bool, f64), limits::Error> {
        self.reduce_gap_to_neighbors_outcome_checked(
            node,
            horizontal,
            forwards,
            recover_symmetry,
            prior_overlaps,
            prior_exact_overlaps,
            prior_active_boxes,
            prior_positions,
            work,
            checkpoint,
        )
        .into_result()
    }

    fn reduce_gap_to_neighbors_outcome_checked(
        &mut self,
        node: NodeId,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_active_boxes: &[(Option<Point>, Size)],
        prior_positions: &[Option<Point>],
        work: &mut gap_guard::GapWork<'_, '_>,
        checkpoint: &mut GeometryCheckpoint,
    ) -> GapReductionOutcome {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GapReduce);
        work.anchor_numeric_overlaps(self, prior_overlaps, prior_exact_overlaps);
        let mut reported_score = 0.0;
        let result = (|| -> Result<(bool, f64), limits::Error> {
            work.check_context(self)?;
            if self.cell_size == 0.0 {
                self.compute_cell_size();
            }
            let Some(nearest) = self.nearest_connected_ahead(node, horizontal, forwards) else {
                return Ok((false, 0.0));
            };
            let mut ancestor = self.active_node_container(nearest);
            while let Some(container) = ancestor {
                if self.nodes[container.0 as usize].fixed_top_left.is_some() {
                    return Ok((false, 0.0));
                }
                ancestor = self.active_node_container(container);
            }

            // Preserve the slice order used by Go gapreduction.go: the initiating
            // node, matching herd siblings in container order, then fixed graph
            // nodes in Graph.Nodes order. Membership is set-like inside the
            // connected traversal, but this input order is observable in traces.
            let mut backward_checkpoint = None;
            let mut excluded = vec![node];
            #[cfg(feature = "diagnostic-traces")]
            let trace_gap = crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP");
            #[cfg(feature = "diagnostic-traces")]
            let mut trace_excluded = trace_gap.then(|| vec![node]);
            if let Some(herd) = self.herd_assignment_snapshot(self.gap_herd_node(node)) {
                // Go reads the live Containers slice after sequence/cluster
                // replacement. Reconstruct that slice here so hidden aggregate
                // members do not reappear as duplicate vessel siblings.
                for sibling in self.container_node_order(self.active_node_container(node)) {
                    if sibling == node
                        || !self.has_herd_assignment(self.gap_herd_node(sibling))
                        || self.is_descendant_of(nearest, sibling)
                    {
                        continue;
                    }
                    if self
                        .herd_assignment_snapshot(self.gap_herd_node(sibling))
                        .is_some_and(|assignment| assignment.orientation == herd.orientation)
                    {
                        excluded.push(sibling);
                        #[cfg(feature = "diagnostic-traces")]
                        if let Some(trace_excluded) = &mut trace_excluded {
                            trace_excluded.push(sibling);
                        }
                    }
                }
            }
            let connected_for_between = self.connected_node_views(
                ConnectedNode::Active(nearest),
                &excluded
                    .iter()
                    .copied()
                    .map(ConnectedNode::Active)
                    .collect::<Vec<_>>(),
            );
            let shared_container = self.nearest_shared_container(node, nearest);
            let nearest_between_match = self.nearest_connected_node_between(
                &connected_for_between,
                ConnectedNode::Active(node),
                ConnectedNode::Active(nearest),
                shared_container,
                horizontal,
                forwards,
            );
            let nearest_between = nearest_between_match.unwrap_or(ConnectedNode::Active(nearest));

            excluded.extend(self.node_order.iter().copied().filter(|node| {
                !self.active_node_is_aggregate(*node)
                    && self.nodes[node.0 as usize].fixed_top_left.is_some()
            }));
            #[cfg(feature = "diagnostic-traces")]
            let trace_graph_nodes = if trace_gap {
                self.graph_node_order()
            } else {
                Vec::new()
            };
            #[cfg(feature = "diagnostic-traces")]
            let trace_fixed_nodes = trace_graph_nodes
                .iter()
                .copied()
                .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
                .collect::<Vec<_>>();
            #[cfg(feature = "diagnostic-traces")]
            if let Some(trace_excluded) = &mut trace_excluded {
                trace_excluded.extend(trace_fixed_nodes.iter().copied());
            }
            let connected = self.connected_node_views(
                ConnectedNode::Active(nearest),
                &excluded
                    .iter()
                    .copied()
                    .map(ConnectedNode::Active)
                    .collect::<Vec<_>>(),
            );
            #[cfg(feature = "diagnostic-traces")]
            if let Some(trace_excluded) = &trace_excluded {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "gap-connectivity",
                        "node_id": self.active_node_tala_id(node).to_string(),
                        "nearest_ahead_id": self.active_node_tala_id(nearest).to_string(),
                        "nearest_between_id": nearest_between_match.map(|between| self.connected_node_id(between).to_string()).unwrap_or_default(),
                        "ordered_connected": connected_for_between.iter().map(|view| self.connected_node_id(*view).to_string()).collect::<Vec<_>>(),
                        "movement_connected": connected.iter().map(|view| self.connected_node_id(*view).to_string()).collect::<Vec<_>>(),
                        "owner_nodes": trace_graph_nodes.iter().map(|node| self.active_node_tala_id(*node).to_string()).collect::<Vec<_>>(),
                        "fixed_nodes": trace_fixed_nodes.iter().map(|node| self.active_node_tala_id(*node).to_string()).collect::<Vec<_>>(),
                        "excluded_after_fixed": trace_excluded.iter().map(|node| self.active_node_tala_id(*node).to_string()).collect::<Vec<_>>(),
                    })
                );
            }
            let old_score = work.score(self)?;
            reported_score = old_score;
            let connected_set = connected.iter().copied().collect::<BTreeSet<_>>();
            let mut candidates = vec![node];
            candidates.extend(self.graph_node_order().into_iter().filter(|candidate| {
                *candidate != node
                    && ConnectedNode::Active(*candidate) != nearest_between
                    && !connected_set.contains(&ConnectedNode::Active(*candidate))
                    && if forwards {
                        self.node_is_blocked_between(
                            ConnectedNode::Active(*candidate),
                            ConnectedNode::Active(node),
                            nearest_between,
                            horizontal,
                        )
                    } else {
                        self.node_is_blocked_between(
                            ConnectedNode::Active(*candidate),
                            nearest_between,
                            ConnectedNode::Active(node),
                            horizontal,
                        )
                    }
            }));
            for raw_candidate in candidates {
                let target_container = self.connected_owning_container(nearest_between);
                let Some(candidate) = self.promote_gap_candidate(raw_candidate, target_container)
                else {
                    continue;
                };
                let Some(position) = self.active_node_position(candidate) else {
                    continue;
                };
                let size = self.active_node_size(candidate);
                let Some((nearest_position, nearest_size)) =
                    self.connected_node_box(nearest_between)
                else {
                    continue;
                };
                let gap = if horizontal {
                    if forwards {
                        nearest_position.x - (position.x + size.width)
                    } else {
                        position.x - (nearest_position.x + nearest_size.width)
                    }
                } else if forwards {
                    nearest_position.y - (position.y + size.height)
                } else {
                    position.y - (nearest_position.y + nearest_size.height)
                };
                if gap <= self.cell_size * 0.5 {
                    continue;
                }
                let mut amount = 150.0 - gap;
                if !forwards {
                    amount = -amount;
                }
                if amount == 0.0 {
                    continue;
                }
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP") {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "gap-proposal", "node_id": self.active_node_tala_id(node).to_string(),
                            "candidate_id": self.active_node_tala_id(candidate).to_string(),
                            "nearest_id": self.connected_node_id(nearest_between).to_string(),
                            "horizontal": horizontal, "forwards": forwards, "recover_symmetry": recover_symmetry,
                            "amount_bits": format!("0x{:016x}", amount.to_bits()),
                            "old_score_bits": format!("0x{:016x}", old_score.to_bits()),
                            "connected": connected.iter().map(|n| self.connected_node_id(*n).to_string()).collect::<Vec<_>>(),
                        })
                    );
                }
                match self.ordinary_gap_direct_attempt(
                    node,
                    &connected,
                    horizontal,
                    forwards,
                    amount,
                    recover_symmetry,
                    old_score,
                    prior_overlaps,
                    prior_exact_overlaps,
                    prior_active_boxes,
                    prior_positions,
                    work,
                    checkpoint,
                    &mut backward_checkpoint,
                )? {
                    OrdinaryGapAttempt::Accepted {
                        reported_score: score,
                    } => {
                        reported_score = score;
                        work.refresh_geometry_request(self, checkpoint)?;
                        break;
                    }
                    OrdinaryGapAttempt::Illegal => {
                        continue;
                    }
                    OrdinaryGapAttempt::Regression => {
                        break;
                    }
                }
            }

            // Recovered inner-container phase (gapreduction.go:475-640). After
            // the connected-side trial, TALA walks the initiating node toward the
            // nearest node's container level and removes excess space between the
            // node and each container's padded inside boundary. Each accepted
            // move becomes the baseline for the next ancestor.
            let mut inner_node = node;
            while self.active_node_container(inner_node)
                != self.connected_owning_container(nearest_between)
                && (self.active_node_is_aggregate(inner_node)
                    || self.nodes[inner_node.0 as usize].fixed_top_left.is_none())
            {
                let Some(container) = self.active_node_container(inner_node) else {
                    break;
                };
                let Some(inner_position) = self.active_node_position(inner_node) else {
                    break;
                };
                let inner_size = self.active_node_size(inner_node);
                let Some(inner_box) = self.shape_inner_box(container) else {
                    break;
                };
                let gap_size = if horizontal {
                    if forwards {
                        inner_box.right() - (inner_position.x + inner_size.width)
                    } else {
                        inner_position.x - inner_box.origin.x
                    }
                } else if forwards {
                    inner_box.bottom() - (inner_position.y + inner_size.height)
                } else {
                    inner_position.y - inner_box.origin.y
                };
                let padding = self.shape_fit_padding_with_children(container, true);
                let mut delta = if horizontal {
                    gap_size
                        - if forwards {
                            padding.right
                        } else {
                            padding.left
                        }
                } else {
                    gap_size
                        - if forwards {
                            padding.bottom
                        } else {
                            padding.top
                        }
                };
                if !forwards {
                    delta = -delta;
                }
                if gap_size > self.cell_size * 0.5 && delta.abs() > 1e-9 {
                    if let Some(score) = self.ordinary_gap_inner_trial_with_checkpoint_checked(
                        inner_node,
                        horizontal,
                        delta,
                        reported_score,
                        prior_overlaps,
                        prior_exact_overlaps,
                        prior_active_boxes,
                        prior_positions,
                        work,
                        checkpoint,
                        true,
                    )? {
                        reported_score = score;
                    } else {
                        // Recovered line-573 fallback: after the inner-boundary
                        // transaction rolls back, try only as far as the first
                        // sibling's 150-unit clearance boundary. Lines 627-636
                        // still compare the fallback score with the current
                        // baseline and roll the transaction back on regression.
                        let inner_position = self.active_node_position(inner_node).unwrap();
                        let inner_size = self.active_node_size(inner_node);
                        let mut least_distance = f64::INFINITY;
                        for sibling in self.container_node_order(Some(container)) {
                            if sibling == inner_node {
                                continue;
                            }
                            let Some(sibling_position) = self.active_node_position(sibling) else {
                                continue;
                            };
                            let sibling_size = self.active_node_size(sibling);
                            let distance = if horizontal {
                                if forwards {
                                    sibling_position.x
                                        - 150.0
                                        - (inner_position.x + inner_size.width)
                                } else {
                                    inner_position.x
                                        - (sibling_position.x + sibling_size.width + 150.0)
                                }
                            } else if forwards {
                                sibling_position.y - 150.0 - (inner_position.y + inner_size.height)
                            } else {
                                inner_position.y
                                    - (sibling_position.y + sibling_size.height + 150.0)
                            };
                            if distance > 0.0 {
                                least_distance = least_distance.min(distance);
                            }
                        }
                        if least_distance.is_finite() {
                            let fallback_delta = if forwards {
                                least_distance
                            } else {
                                -least_distance
                            };
                            if let Some(score) = self
                                .ordinary_gap_inner_trial_with_checkpoint_checked(
                                    inner_node,
                                    horizontal,
                                    fallback_delta,
                                    reported_score,
                                    prior_overlaps,
                                    prior_exact_overlaps,
                                    prior_active_boxes,
                                    prior_positions,
                                    work,
                                    checkpoint,
                                    false,
                                )?
                            {
                                reported_score = score;
                                work.refresh_geometry_request(self, checkpoint)?;
                            }
                        }
                    }
                }
                inner_node = container;
                if self.active_node_container(inner_node).is_none() {
                    break;
                }
            }

            Ok((reported_score != old_score, reported_score))
        })();
        match result {
            Ok((moved, score)) => GapReductionOutcome {
                moved,
                score,
                error: None,
            },
            Err(error) => GapReductionOutcome {
                moved: false,
                score: reported_score,
                error: Some(error),
            },
        }
    }

    pub(super) fn nearest_node_between(
        &self,
        candidates: &[NodeId],
        behind: NodeId,
        ahead: NodeId,
        in_container: Option<NodeId>,
        horizontal: bool,
        forwards: bool,
    ) -> Option<NodeId> {
        self.nearest_connected_node_between(
            &candidates
                .iter()
                .copied()
                .map(ConnectedNode::Active)
                .collect::<Vec<_>>(),
            ConnectedNode::Active(behind),
            ConnectedNode::Active(ahead),
            in_container,
            horizontal,
            forwards,
        )
        .map(ConnectedNode::arena_node)
    }

    pub(super) fn nearest_connected_node_between(
        &self,
        candidates: &[ConnectedNode],
        behind: ConnectedNode,
        ahead: ConnectedNode,
        in_container: Option<NodeId>,
        horizontal: bool,
        forwards: bool,
    ) -> Option<ConnectedNode> {
        let (behind_position, behind_size) = self.connected_node_box(behind)?;
        let (ahead_position, ahead_size) = self.connected_node_box(ahead)?;
        let mut nearest = None;
        for candidate in candidates.iter().copied() {
            if candidate == behind || candidate == ahead {
                continue;
            }
            if self.connected_owning_container(candidate) != in_container {
                continue;
            }
            let (position, size) = self.connected_node_box(candidate).unwrap();
            // Direct translation of Node.isBetween. TALA uses the pair's
            // getDeltaTo clearance on the perpendicular axis; CellSize is not
            // the inclusion tolerance here. This distinction matters when a
            // nearby node is visually offset from the corridor: treating a
            // whole cell as clearance can select the wrong gap boundary.
            let delta = self.connected_spacing_delta(behind, candidate);
            let perpendicular_overlap = if horizontal {
                position.y + size.height >= behind_position.y - delta
                    && position.y <= behind_position.y + behind_size.height + delta
            } else {
                position.x + size.width >= behind_position.x - delta
                    && position.x <= behind_position.x + behind_size.width + delta
            };
            if !perpendicular_overlap {
                continue;
            }

            let (axis_behind_position, axis_behind_size, axis_ahead_position) = if forwards {
                (behind_position, behind_size, ahead_position)
            } else {
                (ahead_position, ahead_size, behind_position)
            };
            let lies_between = if horizontal {
                position.x + size.width >= axis_behind_position.x + axis_behind_size.width
                    && position.x <= axis_ahead_position.x
            } else {
                position.y + size.height >= axis_behind_position.y + axis_behind_size.height
                    && position.y <= axis_ahead_position.y
            };
            if !lies_between {
                continue;
            }

            if nearest.is_none_or(|current| {
                let (current_position, current_size) = self.connected_node_box(current).unwrap();
                let current_edge = if horizontal {
                    if forwards {
                        current_position.x
                    } else {
                        current_position.x + current_size.width
                    }
                } else if forwards {
                    current_position.y
                } else {
                    current_position.y + current_size.height
                };
                let candidate_edge = if horizontal {
                    if forwards {
                        position.x
                    } else {
                        position.x + size.width
                    }
                } else if forwards {
                    position.y
                } else {
                    position.y + size.height
                };
                if forwards {
                    candidate_edge < current_edge
                } else {
                    candidate_edge > current_edge
                }
            }) {
                nearest = Some(candidate);
            }
        }
        nearest
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_gap_pass(&self, phase: &str, node: NodeId, horizontal: bool, forwards: bool) {
        if !crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP") {
            return;
        }
        let bits = |value: f64| format!("0x{:016x}", value.to_bits());
        let nodes = self
            .graph_node_order()
            .into_iter()
            .map(|node| {
                let position = self.active_node_position(node);
                let size = self.active_node_size(node);
                serde_json::json!({
                    "id": self.active_node_tala_id(node).to_string(),
                    "x_bits": position.map(|p| bits(p.x)),
                    "y_bits": position.map(|p| bits(p.y)),
                    "w_bits": bits(size.width), "h_bits": bits(size.height),
                })
            })
            .collect::<Vec<_>>();
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "gap-pass", "phase": phase,
                "node_id": self.active_node_tala_id(node).to_string(),
                "horizontal": horizontal, "forwards": forwards, "nodes": nodes,
            })
        );
    }

    pub(super) fn gap_normalization_pass_for(
        &mut self,
        nodes: &[NodeId],
        horizontal: bool,
        forwards: bool,
    ) -> bool {
        let prior_overlaps = self.existing_overlap_pairs();
        let prior_exact_overlaps = self.exact_overlap_pairs_from(&prior_overlaps);
        gap_guard::with_background_work(|work| {
            self.gap_normalization_pass_for_checked(
                nodes,
                horizontal,
                forwards,
                &prior_overlaps,
                &prior_exact_overlaps,
                work,
            )
        })
    }

    pub(super) fn gap_normalization_pass_for_checked(
        &mut self,
        nodes: &[NodeId],
        horizontal: bool,
        forwards: bool,
        anchored_overlaps: &BTreeSet<(NodeId, NodeId)>,
        anchored_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        work: &mut gap_guard::GapWork<'_, '_>,
    ) -> Result<bool, limits::Error> {
        let mut checkpoint = GeometryCheckpoint::from_snapshot(self.gap_trial_snapshot());
        self.gap_normalization_pass_for_with_checkpoint_checked(
            nodes,
            horizontal,
            forwards,
            anchored_overlaps,
            anchored_exact_overlaps,
            work,
            &mut checkpoint,
        )
    }

    pub(super) fn gap_normalization_pass_for_with_checkpoint_checked(
        &mut self,
        nodes: &[NodeId],
        horizontal: bool,
        forwards: bool,
        anchored_overlaps: &BTreeSet<(NodeId, NodeId)>,
        anchored_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        work: &mut gap_guard::GapWork<'_, '_>,
        checkpoint: &mut GeometryCheckpoint,
    ) -> Result<bool, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GapPass);
        work.anchor_numeric_overlaps(self, anchored_overlaps, anchored_exact_overlaps);
        #[cfg(feature = "diagnostic-traces")]
        work.observe(self, limits::TransactionPollSite::General);
        work.guard.finish()?;
        let node_to_tree = routing::routing_tree_nodes(self);
        // TALA's live Graph.Nodes slice contains one synthetic node for each
        // active sequence/cluster vessel. The stable Rust arena retains the
        // vessel's members, so deduplicate them by their canonical owner
        // before the observable sort-and-attempt loop. Keep the first stable
        // member as the transaction carrier; processing a later member as the
        // same vessel repeats a stateful gap transaction.
        let mut seen = BTreeSet::new();
        let mut nodes = nodes
            .iter()
            .copied()
            .filter(|node| seen.insert(self.active_aggregate_owner(*node)))
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| {
            self.active_edge_count(*right)
                .cmp(&self.active_edge_count(*left))
                .then_with(|| {
                    // Go breaks equal-edge ties by ascending Node.ID. Node.ID
                    // is the recovered TALA hash identity, so the stable
                    // arena insertion index is not an equivalent tie-break.
                    self.active_node_tala_id(*left)
                        .cmp(&self.active_node_tala_id(*right))
                })
        });
        let mut changed = false;
        for node in nodes {
            // Recovered `Nodes.GapNormalization` excludes both tree-owned
            // nodes and hierarchy members before constructing its
            // transaction. Their geometry belongs to the corresponding
            // specialized placement pass.
            if node_to_tree.contains(&node)
                || (!self.active_node_is_aggregate(node)
                    && self.nodes[node.0 as usize].hierarchy.is_some())
            {
                continue;
            }
            // Nodes.GapNormalization wraps each node attempt in an outer
            // AffectContainers transaction. Commit refits ordinary
            // containers even when reduceGapToNeighbors made no inner move;
            // a failed commit restores the complete pre-attempt state.
            #[cfg(feature = "diagnostic-traces")]
            self.trace_gap_pass("before", node, horizontal, forwards);
            #[cfg(feature = "diagnostic-traces")]
            work.observe(self, limits::TransactionPollSite::OuterGeometryClone);
            let mut candidate_checkpoint = checkpoint.clone_geometry(work.guard)?;

            work.before_operation(self)?;
            let prior_overlaps = anchored_overlaps;
            let prior_exact_overlaps = anchored_exact_overlaps;
            let prior_active_boxes = self
                .nodes
                .iter()
                .map(|candidate| {
                    (
                        self.active_node_position(candidate.input_id),
                        self.active_node_size(candidate.input_id),
                    )
                })
                .collect::<Vec<_>>();
            let prior_positions = self
                .nodes
                .iter()
                .map(|candidate| candidate.position)
                .collect::<Vec<_>>();
            let (moved, _) = self.reduce_gap_to_neighbors_scored_with_checkpoint_checked(
                node,
                horizontal,
                forwards,
                true,
                &prior_overlaps,
                &prior_exact_overlaps,
                &prior_active_boxes,
                &prior_positions,
                work,
                &mut candidate_checkpoint,
            )?;
            // Transaction.Commit runs repositionContainers followed by
            // Graph.SyncClusters/SyncSequences for every queued operation,
            // including a gap attempt whose inner reduction was a no-op.
            // The publication is observable for half-pixel aggregate ties,
            // so keep it outside the `moved` branch.
            let _ = moved;
            // Transaction.Commit performs this refit before its final
            // SyncClusters/SyncSequences publication.
            work.check_context(self)?;
            if !work.refit_containers(self, checkpoint)? {
                checkpoint.rollback(self);
                continue;
            }
            self.sync_clusters();
            self.sync_sequences();
            work.check_context(self)?;
            if work.validate_commit(self, checkpoint)? {
                work.refresh_geometry_request(self, checkpoint)?;
                changed = true;
            } else {
                checkpoint.rollback(self);
            }
            #[cfg(feature = "diagnostic-traces")]
            self.trace_gap_pass("after", node, horizontal, forwards);
        }
        Ok(changed)
    }

    pub(super) fn gap_normalization(&mut self) -> bool {
        let mut changed = false;
        // GapNormalizationStage first walks every container in recovered
        // RDFS (deepest descendants first), then repeats the four directional
        // passes over the live Graph.Nodes slice.  A single flat pass changes
        // the transaction baseline and lets a leaf from one container move a
        // sibling aggregate before its own container pass has run.
        let containers = self
            .node_order
            .iter()
            .copied()
            .filter(|node| self.nodes[node.0 as usize].is_container)
            .rev()
            .collect::<Vec<_>>();
        for container in containers {
            let nodes = self.active_descendants(container);
            for horizontal in [true, false] {
                changed |= self.gap_normalization_pass_for(&nodes, horizontal, true);
                changed |= self.gap_normalization_pass_for(&nodes, horizontal, false);
            }
        }
        let nodes = self.graph_node_order();
        for horizontal in [true, false] {
            changed |= self.gap_normalization_pass_for(&nodes, horizontal, true);
            changed |= self.gap_normalization_pass_for(&nodes, horizontal, false);
        }
        changed
    }
}

#[cfg(test)]
mod restored_tree_obstruction_tests {
    use super::*;
    use crate::{ContentAlignment, Edge, Node};

    fn node(name: &str) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.to_owned(),
            size: Size {
                width: 100.0,
                height: 80.0,
            },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: None,
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
            content_insets: Insets::uniform(60.0),
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
            shape: ShapeKind::Rectangle,
        }
    }

    #[test]
    fn inner_trial_rejects_when_a_restored_tree_child_blocks_the_other_corner() {
        let mut input = Graph::default();

        let mut target_container_node = node("target-container");
        target_container_node.size = Size {
            width: 277.0,
            height: 518.0,
        };
        let target_container = input.add_node(target_container_node);

        let mut source_node = node("source");
        source_node.size = Size {
            width: 83.0,
            height: 66.0,
        };
        let source = input.add_node(source_node);

        let mut target_node = node("target");
        target_node.parent = Some(target_container);
        target_node.size = Size {
            width: 52.0,
            height: 66.0,
        };
        let target = input.add_node(target_node);

        let mut restored_sibling_node = node("restored-sibling");
        restored_sibling_node.parent = Some(target_container);
        restored_sibling_node.size = Size {
            width: 53.0,
            height: 66.0,
        };
        let restored_sibling = input.add_node(restored_sibling_node);

        let mut outer_obstruction_node = node("outer-obstruction");
        outer_obstruction_node.size = Size {
            width: 1_197.0,
            height: 718.0,
        };
        let outer_obstruction = input.add_node(outer_obstruction_node);

        input.add_edge(Edge { source, target });

        let mut graph = ArenaGraph::from_input(&input);
        graph.set_turn_cost(100.0);
        graph.set_position(
            target_container,
            Point {
                x: 5_178.0,
                y: 3_047.0,
            },
        );
        graph.set_position(
            source,
            Point {
                x: 7_550.0,
                y: 2_302.0,
            },
        );
        graph.set_position(
            target,
            Point {
                x: 5_290.0,
                y: 3_439.0,
            },
        );
        graph.set_position(
            restored_sibling,
            Point {
                x: 5_290.0,
                y: 3_273.0,
            },
        );
        graph.set_position(
            outer_obstruction,
            Point {
                x: 7_345.0,
                y: 2_628.0,
            },
        );

        // Before PlaceTrees restores the retained child, the snapshot contains
        // only the sentinel. GapNormalization runs later and must instead score
        // the live Graph.Containers slice containing `restored_sibling`.
        graph
            .preprocessed_tree_children
            .insert(Some(target_container), vec![target]);

        let amount = 21.0;
        let mut moved = graph.clone();
        moved.translate_active_node_with_children(source, Point { x: amount, y: 0.0 });
        moved.reposition_ordinary_containers();
        assert!(moved.ordinary_gap_trial_is_valid(&graph));

        let snapshot_score = moved.global_sized_edge_length_with_direction(false);
        let live_score = moved.global_sized_edge_length_after_tree_restoration(false);
        assert_eq!(live_score - snapshot_score, 2.0 * graph.turn_cost_value());

        // A baseline between the two scores makes the strict decision depend
        // solely on whether the restored sibling participates in route scoring.
        let baseline = (snapshot_score + live_score) * 0.5;
        let prior_overlaps = graph.existing_overlap_pairs();
        let prior_exact_overlaps = graph.exact_overlap_pairs();
        let prior_active_boxes = graph
            .nodes
            .iter()
            .map(|candidate| {
                (
                    graph.active_node_position(candidate.input_id),
                    graph.active_node_size(candidate.input_id),
                )
            })
            .collect::<Vec<_>>();
        let prior_positions = graph
            .nodes
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>();
        assert!(
            graph
                .ordinary_gap_inner_trial(
                    source,
                    true,
                    amount,
                    baseline,
                    &prior_overlaps,
                    &prior_exact_overlaps,
                    &prior_active_boxes,
                    &prior_positions,
                )
                .is_none()
        );
    }
}
