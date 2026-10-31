// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Gap reduction inside materialized cluster arrangements.
//!
//! Vessel and member translations are compared transactionally while
//! preserving container ownership and recovered precision ties.

use super::super::*;
use super::cluster_precision_compare;

#[derive(Clone, Copy)]
enum ClusterGapCandidate {
    Vessel,
    Node(NodeId),
}

impl ArenaGraph {
    fn cluster_gap_candidate_rect(
        &self,
        cluster_index: usize,
        candidate: ClusterGapCandidate,
        target_container: Option<NodeId>,
    ) -> Option<Rect> {
        match candidate {
            ClusterGapCandidate::Vessel => {
                let vessel = *self.clusters[cluster_index].members.first()?;
                let mut current = ConnectedNode::Active(vessel);
                while self.connected_owning_container(current) != target_container {
                    current = ConnectedNode::Active(self.connected_owning_container(current)?);
                }
                let (origin, size) = self.connected_node_box(current)?;
                Some(Rect { origin, size })
            }
            ClusterGapCandidate::Node(node_id) => {
                let mut current = ConnectedNode::Active(node_id);
                while self.connected_owning_container(current) != target_container {
                    current = ConnectedNode::Active(self.connected_owning_container(current)?);
                }
                let (origin, size) = self.connected_node_box(current)?;
                Some(Rect { origin, size })
            }
        }
    }

    pub(super) fn cluster_nearest_connected_ahead(
        &self,
        cluster_index: usize,
        horizontal: bool,
        forwards: bool,
    ) -> Option<NodeId> {
        let vessel = *self.clusters[cluster_index].members.first()?;
        self.nearest_connected_ahead(vessel, horizontal, forwards)
    }

    fn cluster_node_between(
        &self,
        vessel: Rect,
        ahead: NodeId,
        candidate: NodeId,
        horizontal: bool,
        forwards: bool,
    ) -> bool {
        if candidate == ahead {
            return false;
        }
        let Some(position) = self.active_node_position(candidate) else {
            return false;
        };
        let rect = Rect {
            origin: position,
            size: self.active_node_size(candidate),
        };
        let Some(ahead_position) = self.active_node_position(ahead) else {
            return false;
        };
        let ahead_rect = Rect {
            origin: ahead_position,
            size: self.active_node_size(ahead),
        };
        let perpendicular_overlap = if horizontal {
            rect.bottom() >= vessel.origin.y && rect.origin.y <= vessel.bottom()
        } else {
            rect.right() >= vessel.origin.x && rect.origin.x <= vessel.right()
        };
        if !perpendicular_overlap {
            return false;
        }
        if horizontal {
            if forwards {
                rect.right() >= vessel.right() && rect.origin.x <= ahead_rect.origin.x
            } else {
                rect.right() >= ahead_rect.right() && rect.origin.x <= vessel.origin.x
            }
        } else if forwards {
            rect.bottom() >= vessel.bottom() && rect.origin.y <= ahead_rect.origin.y
        } else {
            rect.bottom() >= ahead_rect.bottom() && rect.origin.y <= vessel.origin.y
        }
    }

    fn cluster_gap_trial_is_valid(
        &self,
        prior: &Self,
        prior_overlaps: &BTreeSet<(NodeId, NodeId)>,
        prior_exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> bool {
        let fixed_moved = self
            .nodes
            .iter()
            .zip(&prior.nodes)
            .any(|(node, old)| node.fixed_top_left.is_some() && node.position != old.position);
        !fixed_moved
            && self.is_within_max_size()
            && !self.transaction_has_new_overlap(prior_overlaps)
            && !self.transaction_has_bad_state_overlap(prior_overlaps)
            && !self.existing_spacing_overlap_became_exact(prior_overlaps, prior_exact_overlaps)
            && self.transaction_containment_is_valid()
            && self.transaction_fixed_origin_is_valid()
    }

    fn cluster_gap_direct_trial(
        &self,
        cluster_index: usize,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
    ) -> Option<Self> {
        let nearest_ahead =
            self.cluster_nearest_connected_ahead(cluster_index, horizontal, forwards)?;
        let mut ancestor = self.active_node_container(nearest_ahead);
        while let Some(container) = ancestor {
            if self.nodes[container.0 as usize].fixed_top_left.is_some() {
                return None;
            }
            ancestor = self.active_node_container(container);
        }

        let members = self.clusters[cluster_index].members.clone();
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let vessel_member = *members.first()?;
        let vessel_view = ConnectedNode::Active(vessel_member);
        let connected_for_between =
            self.connected_node_views(ConnectedNode::Active(nearest_ahead), &[vessel_view]);
        let shared_container = self.nearest_shared_container(vessel_member, nearest_ahead);
        let nearest_between_match = self.nearest_connected_node_between(
            &connected_for_between,
            vessel_view,
            ConnectedNode::Active(nearest_ahead),
            shared_container,
            horizontal,
            forwards,
        );
        let nearest_between = nearest_between_match.unwrap_or(ConnectedNode::Active(nearest_ahead));
        #[cfg(feature = "diagnostic-traces")]
        let trace_gap = crate::internal::trace_env_enabled("WEFTAN_TRACE_GAP");
        #[cfg(feature = "diagnostic-traces")]
        let trace_owner_nodes = trace_gap.then(|| {
            self.graph_node_order()
                .iter()
                .map(|node| self.active_node_tala_id(*node).to_string())
                .collect::<Vec<_>>()
        });
        let mut movement_excluded = vec![vessel_view];
        movement_excluded.extend(
            self.graph_node_order()
                .into_iter()
                .filter(|node| self.nodes[node.0 as usize].fixed_top_left.is_some())
                .map(ConnectedNode::Active),
        );
        let connected =
            self.connected_node_views(ConnectedNode::Active(nearest_ahead), &movement_excluded);
        #[cfg(feature = "diagnostic-traces")]
        if let Some(owner_nodes) = trace_owner_nodes {
            let excluded_after_fixed = movement_excluded
                .iter()
                .map(|view| self.connected_node_id(*view).to_string())
                .collect::<Vec<_>>();
            let fixed_nodes = excluded_after_fixed
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<_>>();
            let connected_ids = |nodes: &[ConnectedNode]| {
                nodes
                    .iter()
                    .map(|view| self.connected_node_id(*view).to_string())
                    .collect::<Vec<_>>()
            };
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "gap-connectivity",
                    "node_id": self.active_node_tala_id(vessel_member).to_string(),
                    "nearest_ahead_id": self.nodes[nearest_ahead.0 as usize].tala_id.to_string(),
                    "nearest_between_id": nearest_between_match.map(|between| self.connected_node_id(between).to_string()).unwrap_or_default(),
                    "ordered_connected": connected_ids(&connected_for_between),
                    "movement_connected": connected_ids(&connected),
                    "owner_nodes": owner_nodes,
                    "fixed_nodes": fixed_nodes,
                    "excluded_after_fixed": excluded_after_fixed,
                })
            );
        }
        if connected.is_empty() {
            return None;
        }
        let connected_set = connected.iter().copied().collect::<BTreeSet<_>>();
        let (vessel_position, vessel_size) = self.connected_node_box(vessel_view)?;
        let vessel = Rect {
            origin: vessel_position,
            size: vessel_size,
        };
        let mut candidates = vec![ClusterGapCandidate::Vessel];
        candidates.extend(
            self.graph_node_order()
                .into_iter()
                .filter(|node| {
                    !member_set.contains(node)
                        && *node != nearest_between.arena_node()
                        && !connected_set.contains(&ConnectedNode::Active(*node))
                        && self.cluster_node_between(
                            vessel,
                            nearest_between.arena_node(),
                            *node,
                            horizontal,
                            forwards,
                        )
                })
                .map(ClusterGapCandidate::Node),
        );

        let old_length = self.global_sized_edge_length_with_direction(false);
        let trace_gap_target = crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_GAP");
        let trace_gap = crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_FLIP").as_deref()
            == Some("all")
            && trace_gap_target.as_deref().is_some_and(|target| {
                target == "all" || target == self.clusters[cluster_index].vessel_tala_id.to_string()
            });
        if trace_gap {
            eprintln!(
                "GAP_RUST connectivity vessel={} h={} f={} nearest={} ordered_connected={:?} nearest_between={:?} movement_connected={:?}",
                self.clusters[cluster_index].vessel_tala_id,
                horizontal,
                forwards,
                self.nodes[nearest_ahead.0 as usize].tala_id,
                connected_for_between
                    .iter()
                    .map(|node| self.connected_node_id(*node))
                    .collect::<Vec<_>>(),
                nearest_between_match.map(|node| self.connected_node_id(node)),
                connected
                    .iter()
                    .map(|node| self.connected_node_id(*node))
                    .collect::<Vec<_>>()
            );
        }
        let prior_overlaps = self.existing_overlap_pairs();
        let prior_exact_overlaps = self.exact_overlap_pairs();
        for candidate in candidates {
            let target_container = self.connected_owning_container(nearest_between);
            let Some(candidate_rect) =
                self.cluster_gap_candidate_rect(cluster_index, candidate, target_container)
            else {
                continue;
            };
            let Some((nearest_position, nearest_size)) = self.connected_node_box(nearest_between)
            else {
                continue;
            };
            let nearest_rect = Rect {
                origin: nearest_position,
                size: nearest_size,
            };
            let gap = if horizontal {
                if forwards {
                    nearest_rect.origin.x - candidate_rect.right()
                } else {
                    candidate_rect.origin.x - nearest_rect.right()
                }
            } else if forwards {
                nearest_rect.origin.y - candidate_rect.bottom()
            } else {
                candidate_rect.origin.y - nearest_rect.bottom()
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
            if trace_gap {
                eprintln!(
                    "GAP_RUST candidate={:?} rect={:?} nearest_rect={:?} gap={} amount={} old={} old_bits=0x{:016x}",
                    match candidate {
                        ClusterGapCandidate::Vessel => "vessel",
                        ClusterGapCandidate::Node(_) => "node",
                    },
                    candidate_rect,
                    nearest_rect,
                    gap,
                    amount,
                    old_length,
                    old_length.to_bits(),
                );
            }

            let mut trial = self.clone();
            trial.translate_connected_node_boxes(
                &connected,
                Point {
                    x: if horizontal { amount } else { 0.0 },
                    y: if horizontal { 0.0 } else { amount },
                },
            );
            // The recovered gap-reduction closure synchronizes cluster and
            // sequence aggregates inside the op before AffectContainers
            // repositioning runs during Commit.
            trial.sync_clusters();
            trial.sync_sequences();
            trial.reposition_ordinary_containers();
            if trace_gap {
                let traced = [
                    1563578235_i64,
                    1408816216,
                    3040778151,
                    2517300255,
                    3369913289,
                    3030713537,
                    3429528291,
                    3860869894,
                    5920220759044228662,
                    2352770385463105992,
                ];
                eprintln!(
                    "GAP_RUST positions {}",
                    traced
                        .iter()
                        .filter_map(|id| trial
                            .nodes
                            .iter()
                            .find(|n| n.tala_id == *id)
                            .map(|n| format!("{}@{:?}", id, n.position)))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                eprintln!(
                    "GAP_RUST validity candidate={} fixed={} max={} overlap={} bad={} spacing={} containment={}",
                    match candidate {
                        ClusterGapCandidate::Vessel => "vessel",
                        ClusterGapCandidate::Node(_) => "node",
                    },
                    trial
                        .nodes
                        .iter()
                        .zip(&self.nodes)
                        .any(|(node, old)| node.fixed_top_left.is_some()
                            && node.position != old.position),
                    !trial.is_within_max_size(),
                    trial.transaction_has_new_overlap(&prior_overlaps),
                    trial.transaction_has_bad_state_overlap(&prior_overlaps),
                    trial.existing_spacing_overlap_became_exact(
                        &prior_overlaps,
                        &prior_exact_overlaps
                    ),
                    !trial.transaction_containment_is_valid(),
                );
                eprintln!(
                    "GAP_RUST graph_nodes {:?}",
                    trial
                        .graph_node_order()
                        .iter()
                        .map(|node| trial.active_node_tala_id(*node))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "GAP_RUST graph_positions {:?}",
                    trial
                        .graph_node_order()
                        .iter()
                        .filter_map(|node| trial.active_node_position(*node).map(|position| (
                            trial.active_node_tala_id(*node),
                            position,
                            trial.active_node_size(*node)
                        )))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "GAP_RUST root_children {:?}",
                    trial
                        .container_node_order(None)
                        .iter()
                        .map(|node| trial.active_node_tala_id(*node))
                        .collect::<Vec<_>>()
                );
            }
            let trial_valid =
                trial.cluster_gap_trial_is_valid(self, &prior_overlaps, &prior_exact_overlaps);
            let direct_length = trial.global_sized_edge_length_with_direction(false);
            if trace_gap {
                eprintln!(
                    "GAP_RUST trial candidate={:?} length={} length_bits=0x{:016x} pos140={:?} pos386={:?} pos235={:?}",
                    match candidate {
                        ClusterGapCandidate::Vessel => "vessel",
                        ClusterGapCandidate::Node(_) => "node",
                    },
                    direct_length,
                    direct_length.to_bits(),
                    trial
                        .nodes
                        .iter()
                        .find(|n| n.tala_id == 1408816216)
                        .and_then(|n| n.position),
                    trial
                        .nodes
                        .iter()
                        .find(|n| n.tala_id == 3860869894)
                        .and_then(|n| n.position),
                    trial
                        .nodes
                        .iter()
                        .find(|n| n.tala_id == 2352770385463105992)
                        .and_then(|n| n.position),
                );
            }
            if recover_symmetry
                && let Some(mirrored) =
                    trial.cluster_gap_direct_trial(cluster_index, horizontal, !forwards, false)
            {
                let mirrored_length = mirrored.global_sized_edge_length_with_direction(false);
                if trace_gap {
                    eprintln!(
                        "GAP_RUST mirrored length={} pending={:?} active={:?}",
                        mirrored_length,
                        mirrored.pending_cluster_vessel_positions,
                        mirrored
                            .node_order
                            .iter()
                            .filter_map(|node| mirrored
                                .position(*node)
                                .map(|p| (mirrored.active_node_tala_id(*node), p)))
                            .collect::<Vec<_>>(),
                    );
                }
                if mirrored_length < direct_length && mirrored_length < old_length {
                    return Some(mirrored);
                }
            }
            if direct_length >= old_length {
                // The recovered outer closure checks its forward score only
                // after the optional mirrored transaction has run.
                break;
            }
            // Go performs the symmetry recursion inside the candidate
            // transaction operation, before the outer Commit validates the
            // forward trial. The forward state can therefore be temporarily
            // overlapping when the mirrored transaction supplies the final
            // accepted state.
            if !trial_valid {
                continue;
            }
            return Some(trial);
        }
        None
    }

    fn translate_cluster_gap_subject(
        &mut self,
        cluster_index: usize,
        subject: ClusterGapCandidate,
        delta: Point,
    ) {
        match subject {
            ClusterGapCandidate::Vessel => {
                let members = self.clusters[cluster_index].members.clone();
                for member in members {
                    self.translate_node_with_children(member, delta);
                }
            }
            ClusterGapCandidate::Node(node) => self.translate_node_with_children(node, delta),
        }
    }

    fn cluster_gap_subject_container(
        &self,
        cluster_index: usize,
        subject: ClusterGapCandidate,
    ) -> Option<NodeId> {
        match subject {
            ClusterGapCandidate::Vessel => self.clusters[cluster_index]
                .members
                .first()
                .and_then(|member| self.nodes[member.0 as usize].container),
            ClusterGapCandidate::Node(node) => self.nodes[node.0 as usize].container,
        }
    }

    fn cluster_gap_inner_trial(
        &self,
        cluster_index: usize,
        subject: ClusterGapCandidate,
        horizontal: bool,
        amount: f64,
        baseline: f64,
    ) -> Option<Self> {
        let prior_overlaps = self.existing_overlap_pairs();
        let prior_exact_overlaps = self.exact_overlap_pairs();
        let mut trial = self.clone();
        trial.translate_cluster_gap_subject(
            cluster_index,
            subject,
            Point {
                x: if horizontal { amount } else { 0.0 },
                y: if horizontal { 0.0 } else { amount },
            },
        );
        trial.sync_clusters();
        trial.sync_sequences();
        trial.reposition_ordinary_containers();
        if !trial.cluster_gap_trial_is_valid(self, &prior_overlaps, &prior_exact_overlaps) {
            return None;
        }
        let score = trial.global_sized_edge_length_with_direction(false);
        (cluster_precision_compare(score, baseline) == std::cmp::Ordering::Less).then_some(trial)
    }

    fn cluster_gap_inner_and_fallback(
        &self,
        cluster_index: usize,
        nearest_between: NodeId,
        horizontal: bool,
        forwards: bool,
    ) -> Self {
        let target_container = self.nodes[nearest_between.0 as usize].container;
        let members = self.clusters[cluster_index].members.clone();
        if members
            .iter()
            .any(|member| self.nodes[member.0 as usize].fixed_top_left.is_some())
        {
            return self.clone();
        }

        let mut state = self.clone();
        let mut subject = ClusterGapCandidate::Vessel;
        while let Some(container) = state.cluster_gap_subject_container(cluster_index, subject) {
            if Some(container) == target_container {
                break;
            }
            let Some(subject_rect) =
                state.cluster_gap_candidate_rect(cluster_index, subject, Some(container))
            else {
                break;
            };
            let Some(inner_box) = state.shape_inner_box(container) else {
                break;
            };
            let gap = if horizontal {
                if forwards {
                    inner_box.right() - subject_rect.right()
                } else {
                    subject_rect.origin.x - inner_box.origin.x
                }
            } else if forwards {
                inner_box.bottom() - subject_rect.bottom()
            } else {
                subject_rect.origin.y - inner_box.origin.y
            };
            let padding = state.shape_fit_padding_with_children(container, true);
            let mut amount = if horizontal {
                gap - if forwards {
                    padding.right
                } else {
                    padding.left
                }
            } else {
                gap - if forwards {
                    padding.bottom
                } else {
                    padding.top
                }
            };
            if !forwards {
                amount = -amount;
            }

            let baseline = state.global_sized_edge_length_with_direction(false);
            if gap > state.cell_size * 0.5 && amount != 0.0 {
                let accepted = state.cluster_gap_inner_trial(
                    cluster_index,
                    subject,
                    horizontal,
                    amount,
                    baseline,
                );
                if let Some(trial) = accepted {
                    state = trial;
                } else {
                    // Recovered line-573 fallback: stop at the first sibling's
                    // 150-unit clearance boundary that lies ahead of the mutable
                    // subject at this container level.
                    let subject_rect = state
                        .cluster_gap_candidate_rect(cluster_index, subject, Some(container))
                        .unwrap();
                    let member_set = members.iter().copied().collect::<BTreeSet<_>>();
                    let mut least = f64::INFINITY;
                    for sibling in state.containers.get(&Some(container)).into_iter().flatten() {
                        if member_set.contains(&sibling)
                            || matches!(subject, ClusterGapCandidate::Node(node) if node == sibling)
                        {
                            continue;
                        }
                        let node = &state.nodes[sibling.0 as usize];
                        let Some(position) = node.position else {
                            continue;
                        };
                        let sibling_rect = Rect {
                            origin: position,
                            size: node.rect.size,
                        };
                        let distance = if horizontal {
                            if forwards {
                                sibling_rect.origin.x - 150.0 - subject_rect.right()
                            } else {
                                subject_rect.origin.x - (sibling_rect.right() + 150.0)
                            }
                        } else if forwards {
                            sibling_rect.origin.y - 150.0 - subject_rect.bottom()
                        } else {
                            subject_rect.origin.y - (sibling_rect.bottom() + 150.0)
                        };
                        if distance > 0.0 {
                            least = least.min(distance);
                        }
                    }
                    if least.is_finite() {
                        let amount = if forwards { least } else { -least };
                        if let Some(trial) = state.cluster_gap_inner_trial(
                            cluster_index,
                            subject,
                            horizontal,
                            amount,
                            baseline,
                        ) {
                            state = trial;
                        }
                    }
                }
            }

            subject = ClusterGapCandidate::Node(container);
            if state.nodes[container.0 as usize].container.is_none() {
                break;
            }
        }
        state
    }

    /// Recovered cluster call to
    /// `Vessel.reduceGapToNeighbors(ctx, nil, horizontal, forwards, recover)`.
    ///
    /// Cluster members remain stable arena nodes, so the vessel is represented
    /// by their aggregate rectangle while edge-abducted connectivity excludes
    /// the members exactly as the materialized Go vessel would.
    pub(super) fn reduce_cluster_gap_to_neighbors(
        &mut self,
        cluster_index: usize,
        horizontal: bool,
        forwards: bool,
        recover_symmetry: bool,
    ) -> bool {
        let trace_target = crate::internal::trace_env_value("WEFTAN_TRACE_CLUSTER_GAP");
        let trace = trace_target.as_deref() == Some("all")
            || trace_target
                .as_deref()
                .and_then(|value| value.parse::<i64>().ok())
                .is_some_and(|value| value == self.clusters[cluster_index].vessel_tala_id);
        if trace {
            eprintln!(
                "GAP_CLUSTER_RUST begin vessel={} horizontal={} forwards={} recover={} pos={:?} members={:?}",
                self.clusters[cluster_index].vessel_tala_id,
                horizontal,
                forwards,
                recover_symmetry,
                self.position(self.clusters[cluster_index].members[0]),
                self.clusters[cluster_index]
                    .members
                    .iter()
                    .map(|member| (
                        self.nodes[member.0 as usize].tala_id,
                        self.position(*member)
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let Some(nearest_ahead) =
            self.cluster_nearest_connected_ahead(cluster_index, horizontal, forwards)
        else {
            if trace {
                eprintln!("GAP_CLUSTER_RUST no-nearest");
            }
            return false;
        };
        let mut ancestor = self.active_node_container(nearest_ahead);
        while let Some(container) = ancestor {
            if self.nodes[container.0 as usize].fixed_top_left.is_some() {
                return false;
            }
            ancestor = self.active_node_container(container);
        }
        let members = self.clusters[cluster_index].members.clone();
        let vessel_member = members[0];
        let vessel_view = ConnectedNode::Active(vessel_member);
        let connected =
            self.connected_node_views(ConnectedNode::Active(nearest_ahead), &[vessel_view]);
        if trace {
            eprintln!(
                "GAP_CLUSTER_RUST nearest={} connected={:?}",
                self.nodes[nearest_ahead.0 as usize].tala_id,
                connected
                    .iter()
                    .map(|node| self.connected_node_id(*node))
                    .collect::<Vec<_>>()
            );
        }
        let shared_container = self.nearest_shared_container(vessel_member, nearest_ahead);
        let nearest_between = self
            .nearest_connected_node_between(
                &connected,
                vessel_view,
                ConnectedNode::Active(nearest_ahead),
                shared_container,
                horizontal,
                forwards,
            )
            .unwrap_or(ConnectedNode::Active(nearest_ahead))
            .arena_node();
        let old_length = self.global_sized_edge_length_with_direction(false);
        let direct = self
            .cluster_gap_direct_trial(cluster_index, horizontal, forwards, recover_symmetry)
            .unwrap_or_else(|| self.clone());
        let trial = direct.cluster_gap_inner_and_fallback(
            cluster_index,
            nearest_between,
            horizontal,
            forwards,
        );
        let new_length = trial.global_sized_edge_length_with_direction(false);
        if cluster_precision_compare(new_length, old_length) == std::cmp::Ordering::Less {
            self.publish_cluster_optimization_candidate(trial);
            if trace {
                eprintln!(
                    "GAP_CLUSTER_RUST accept old={} old_bits=0x{:016x} new={} new_bits=0x{:016x} members={:?}",
                    old_length,
                    old_length.to_bits(),
                    new_length,
                    new_length.to_bits(),
                    self.clusters[cluster_index]
                        .members
                        .iter()
                        .map(|member| (
                            self.nodes[member.0 as usize].tala_id,
                            self.position(*member)
                        ))
                        .collect::<Vec<_>>()
                );
            }
            true
        } else {
            if trace {
                eprintln!(
                    "GAP_CLUSTER_RUST reject old={} new={}",
                    old_length, new_length
                );
            }
            false
        }
    }
}
