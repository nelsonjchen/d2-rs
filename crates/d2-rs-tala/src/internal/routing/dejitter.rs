// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Removal of short endpoint doglegs by transactional node motion.
//!
//! A trial is accepted only when it preserves attached segments, containment,
//! obstacle clearance, and local symmetry.

use super::*;

/// Translation of the recovered `Graph.dejitter` endpoint-dogleg pass.
///
/// TALA moves an unfixed leaf endpoint across a short (at most 80 units)
/// two-bend jog when doing so does not reverse another attached segment,
/// introduce an obstacle crossing, spill from its container, or reduce the
/// node's local symmetry. The accepted transaction updates every attached
/// endpoint and removes the consumed bends from the triggering edge.
pub(in crate::internal) fn dejitter(graph: &mut ArenaGraph) -> bool {
    let mut changed = false;
    // Graph.dejitter iterates the current Graph.Nodes slice, whose order is
    // hierarchy-preorder rather than the arena's stable serialized index.
    // Keeping that lifecycle order matters when two endpoints can consume the
    // same short dogleg: the first accepted transaction changes the candidate
    // seen by the next node.
    let node_ids = if graph.source_hierarchy_ordered {
        graph.graph_node_order()
    } else {
        graph.nodes.iter().map(|node| node.input_id).collect()
    };
    // Graph.dejitter creates one Transaction before visiting nodes. Clear
    // preserves its current-state snapshot across rejected trials, while
    // UpdateState refreshes it after each accepted move.
    let mut existing_overlaps = graph.existing_overlap_pairs();
    let mut existing_exact_overlaps = graph.exact_overlap_pairs();

    for node_id in node_ids {
        let node_index = node_id.0 as usize;
        let trace_node =
            crate::internal::trace_env_value("TALA_TRACE_DEJITTER_NODE").is_some_and(|raw| {
                raw == "all"
                    || raw
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|tala_id| graph.nodes[node_index].tala_id == tala_id)
            });
        if graph.nodes[node_index].cluster.is_some()
            || graph.nodes[node_index].is_container
            || graph.nodes[node_index].sequence.is_some()
            || graph.nodes[node_index].fixed_top_left.is_some()
        {
            continue;
        }
        // Recovered Graph.dejitter skips every NodeToTree member. Sentinel
        // routes are fixed before the ordinary OVG pass; moving a tree node
        // here would invalidate the topology selected by handleTreeSubgraph.
        if graph.tree_routing_nodes.contains_key(&node_id) {
            continue;
        }

        let previous_symmetry = graph.sized_symmetry(node_id, true);
        let edge_ids = graph.nodes[node_index].edges.read();
        for edge_id in edge_ids {
            let edge_index = edge_id.0 as usize;
            let edge = &graph.edge_records[edge_index];
            if trace_node {
                eprintln!(
                    "DEJITTER_EDGE_RUST node={} edge={} from={} to={} points={:?}",
                    graph.nodes[node_index].tala_id,
                    edge.input_id.0,
                    graph.nodes[edge.from.0 as usize].tala_id,
                    graph.nodes[edge.to.0 as usize].tala_id,
                    edge.route.read()
                );
            }
            if edge.route.len() < 4 {
                continue;
            }
            let adjacent_node = if edge.from == node_id {
                edge.to
            } else {
                edge.from
            };
            if graph.nodes[adjacent_node.0 as usize].cluster.is_some() {
                continue;
            }

            let from_node = edge.from == node_id;
            let (first_bend, second_bend, point_before, point_on_node) = if from_node {
                (
                    edge.route.point(1),
                    edge.route.point(2),
                    edge.route.point(3),
                    edge.route.point(0),
                )
            } else if edge.to == node_id {
                let len = edge.route.len();
                (
                    edge.route.point(len - 2),
                    edge.route.point(len - 3),
                    edge.route.point(len - 4),
                    edge.route.point(len - 1),
                )
            } else {
                continue;
            };

            if ((first_bend.x - second_bend.x).powi(2) + (first_bend.y - second_bend.y).powi(2))
                .sqrt()
                > 80.0
            {
                continue;
            }

            // The recovered name is counterintuitive: this is true when the
            // short jog is horizontal, so the node moves on the x axis.
            let move_x = first_bend.y == second_bend.y;
            if move_x {
                let bend_y = first_bend.y;
                if (bend_y < point_before.y && bend_y < point_on_node.y)
                    || (bend_y > point_before.y && bend_y > point_on_node.y)
                    || first_bend.x == second_bend.x
                {
                    continue;
                }
            } else {
                let bend_x = first_bend.x;
                if (bend_x < point_before.x && bend_x < point_on_node.x)
                    || (bend_x > point_before.x && bend_x > point_on_node.x)
                {
                    continue;
                }
            }

            let has_connected_straight =
                graph.nodes[node_index].edges.read().iter().any(|other_id| {
                    if *other_id == edge_id {
                        return false;
                    }
                    let other = &graph.edge_records[other_id.0 as usize];
                    if other.route.len() != 2 {
                        return false;
                    }
                    let (endpoint, adjacent) = if other.from == node_id {
                        (other.route.point(0), other.route.point(1))
                    } else {
                        (
                            other.route.point(other.route.len() - 1),
                            other.route.point(other.route.len() - 2),
                        )
                    };
                    if move_x {
                        endpoint.x == adjacent.x
                    } else {
                        endpoint.y == adjacent.y
                    }
                });
            if has_connected_straight {
                continue;
            }

            let delta = if move_x {
                (second_bend.x - first_bend.x).round()
            } else {
                (second_bend.y - first_bend.y).round()
            };
            if delta == 0.0 {
                continue;
            }
            let sign_flip_delta = delta + if delta < 0.0 { -5.0 } else { 5.0 };

            let mut reverses_segment = false;
            let mut proposed_segments = Vec::with_capacity(graph.nodes[node_index].edges.len());
            for other_id in &graph.nodes[node_index].edges.read() {
                let other = &graph.edge_records[other_id.0 as usize];
                if other.route.len() < 2 {
                    reverses_segment = true;
                    break;
                }
                let (endpoint_index, adjacent_index) = if other.from == node_id {
                    (0, 1)
                } else {
                    (other.route.len() - 1, other.route.len() - 2)
                };
                let endpoint = other.route.point(endpoint_index);
                let adjacent = other.route.point(adjacent_index);
                let attached_vertical = endpoint.x == adjacent.x;

                // TALA does not include a diagonal two-point route in
                // newSegments. It still moves that route's endpoint after a
                // successful transaction below.
                if other.route.len() == 2 && endpoint.x != adjacent.x && endpoint.y != adjacent.y {
                    continue;
                }

                let (proposed_endpoint, proposed_adjacent) = match (move_x, attached_vertical) {
                    (true, true) => (
                        Point {
                            x: endpoint.x + delta,
                            y: endpoint.y,
                        },
                        Point {
                            x: adjacent.x + delta,
                            y: adjacent.y,
                        },
                    ),
                    (true, false) => (
                        Point {
                            x: endpoint.x + delta,
                            y: endpoint.y,
                        },
                        adjacent,
                    ),
                    (false, false) => (
                        Point {
                            x: endpoint.x,
                            y: endpoint.y + delta,
                        },
                        Point {
                            x: adjacent.x,
                            y: adjacent.y + delta,
                        },
                    ),
                    (false, true) => (
                        Point {
                            x: endpoint.x,
                            y: endpoint.y + delta,
                        },
                        adjacent,
                    ),
                };
                proposed_segments.push((*other_id, proposed_endpoint, proposed_adjacent));

                let moves_along_endpoint_segment =
                    (move_x && attached_vertical) || (!move_x && !attached_vertical);
                let (moving, following) = if moves_along_endpoint_segment {
                    if *other_id == edge_id {
                        continue;
                    }
                    if other.route.len() < 3 {
                        reverses_segment = true;
                        break;
                    }
                    let following_index = if other.from == node_id {
                        2
                    } else {
                        other.route.len() - 3
                    };
                    (adjacent, other.route.point(following_index))
                } else {
                    (endpoint, adjacent)
                };
                reverses_segment = if move_x {
                    (moving.x + sign_flip_delta < following.x && moving.x > following.x)
                        || (moving.x + sign_flip_delta > following.x && moving.x < following.x)
                } else {
                    (moving.y + sign_flip_delta < following.y && moving.y > following.y)
                        || (moving.y + sign_flip_delta > following.y && moving.y < following.y)
                };
                if reverses_segment {
                    break;
                }
            }
            if reverses_segment {
                if trace_node {
                    eprintln!(
                        "DEJITTER_REJECT_RUST edge={} reason=sign-flip delta={}",
                        edge.input_id.0, delta
                    );
                }
                continue;
            }

            let intersects = proposed_segments.iter().any(
                |(segment_edge_id, proposed_endpoint, proposed_adjacent)| {
                    let segment_edge = &graph.edge_records[segment_edge_id.0 as usize];
                    graph.nodes.iter().any(|other_node| {
                        let other_id = other_node.input_id;
                        if other_id == node_id
                            || other_id == segment_edge.from
                            || other_id == segment_edge.to
                            || graph.is_descendant_of_scope(segment_edge.from, Some(other_id))
                            || graph.is_descendant_of_scope(segment_edge.to, Some(other_id))
                        {
                            return false;
                        }
                        other_node.position.is_some_and(|origin| {
                            ArenaGraph::segment_intersects_box(
                                *proposed_endpoint,
                                *proposed_adjacent,
                                origin,
                                other_node.rect.size,
                            )
                        })
                    })
                },
            );
            if intersects {
                if trace_node {
                    eprintln!(
                        "DEJITTER_REJECT_RUST edge={} reason=intersection delta={}",
                        edge.input_id.0, delta
                    );
                }
                continue;
            }

            let Some(old_position) = graph.nodes[node_index].position else {
                continue;
            };
            let old_rect = Rect {
                origin: old_position,
                size: graph.nodes[node_index].rect.size,
            };
            let old_crossings = graph
                .member_edges()
                .filter(|other| other.from != node_id && other.to != node_id)
                .flat_map(|other| other.route.segments())
                .filter(|segment| {
                    ArenaGraph::segment_intersects_box(
                        segment[0],
                        segment[1],
                        old_rect.origin,
                        old_rect.size,
                    )
                })
                .count();
            let mut candidate = graph.clone();
            let new_position = if move_x {
                Point {
                    x: old_position.x + delta,
                    y: old_position.y,
                }
            } else {
                Point {
                    x: old_position.x,
                    y: old_position.y + delta,
                }
            };
            candidate.nodes[node_index].position = Some(new_position);

            let new_rect = Rect {
                origin: new_position,
                size: candidate.nodes[node_index].rect.size,
            };
            let new_crossings = candidate
                .member_edges()
                .filter(|other| other.from != node_id && other.to != node_id)
                .flat_map(|other| other.route.segments())
                .filter(|segment| {
                    ArenaGraph::segment_intersects_box(
                        segment[0],
                        segment[1],
                        new_rect.origin,
                        new_rect.size,
                    )
                })
                .count();
            if new_crossings > old_crossings {
                if trace_node {
                    eprintln!(
                        "DEJITTER_REJECT_RUST edge={} reason=crossings old={} new={}",
                        edge.input_id.0, old_crossings, new_crossings
                    );
                }
                continue;
            }
            if let Some(container) = candidate.nodes[node_index].container {
                let Some(container_position) = candidate.nodes[container.0 as usize].position
                else {
                    continue;
                };
                let container_rect = Rect {
                    origin: container_position,
                    size: candidate.nodes[container.0 as usize].rect.size,
                };
                if !contained_by(new_rect, container_rect) {
                    if trace_node {
                        eprintln!(
                            "DEJITTER_REJECT_RUST edge={} reason=container",
                            edge.input_id.0
                        );
                    }
                    continue;
                }
            }
            if candidate.sized_symmetry(node_id, true) < previous_symmetry {
                if trace_node {
                    eprintln!(
                        "DEJITTER_REJECT_RUST edge={} reason=symmetry",
                        edge.input_id.0
                    );
                }
                continue;
            }

            // NewTransactionWithOptions(AffectContainers: true) performs its
            // generic Commit validation after the closure's Dejitter-specific
            // checks and before the accepted route mutation.
            candidate.reposition_ordinary_containers_without_sync();
            let fixed_moved = candidate.nodes.iter().enumerate().any(|(index, node)| {
                node.fixed_top_left.is_some() && node.position != graph.nodes[index].position
            });
            let transaction_invalid = !candidate.is_within_max_size()
                || candidate.transaction_has_new_overlap(&existing_overlaps)
                || candidate.existing_spacing_overlap_became_exact(
                    &existing_overlaps,
                    &existing_exact_overlaps,
                )
                || fixed_moved
                || !candidate.transaction_containment_is_valid()
                || !candidate.transaction_external_containers_are_valid();
            if transaction_invalid {
                if trace_node {
                    eprintln!(
                        "DEJITTER_REJECT_RUST edge={} reason=transaction within={} new_overlap={} exact={} fixed={} containment={} external={}",
                        edge.input_id.0,
                        candidate.is_within_max_size(),
                        candidate.transaction_has_new_overlap(&existing_overlaps),
                        candidate.existing_spacing_overlap_became_exact(
                            &existing_overlaps,
                            &existing_exact_overlaps,
                        ),
                        fixed_moved,
                        candidate.transaction_containment_is_valid(),
                        candidate.transaction_external_containers_are_valid(),
                    );
                }
                continue;
            }

            // Transaction.Commit publishes aggregate vessel/member geometry
            // only after its ordinary-container and bad-state checks pass.
            // A cluster carrier can still hold the pre-translation frame while
            // its retained members have already moved. TALA reconciles that
            // vessel before the sync boundary; do the same here rather than
            // allowing syncClusters to arrange members around a stale carrier.
            candidate.reconcile_root_cluster_vessel_positions();
            candidate.sync_clusters();

            for attached_id in candidate.nodes[node_index].edges.read() {
                let attached = &mut candidate.edge_records[attached_id.0 as usize];
                if attached.route.len() < 2 {
                    continue;
                }
                let endpoint_index = if attached.from == node_id {
                    0
                } else {
                    attached.route.len() - 1
                };
                let adjacent_index = if attached.from == node_id {
                    1
                } else {
                    attached.route.len() - 2
                };
                let endpoint = attached.route.point(endpoint_index);
                let adjacent = attached.route.point(adjacent_index);
                let attached_vertical = endpoint.x == adjacent.x;
                if attached.route.len() == 2 && endpoint.x != adjacent.x && endpoint.y != adjacent.y
                {
                    if move_x {
                        attached.route.point_mut(endpoint_index).x += delta;
                    } else {
                        attached.route.point_mut(endpoint_index).y += delta;
                    }
                    continue;
                }
                if move_x {
                    attached.route.point_mut(endpoint_index).x += delta;
                    if attached_vertical {
                        attached.route.point_mut(adjacent_index).x += delta;
                    }
                } else {
                    attached.route.point_mut(endpoint_index).y += delta;
                    if !attached_vertical {
                        attached.route.point_mut(adjacent_index).y += delta;
                    }
                }
            }

            let triggering = &mut candidate.edge_records[edge_index];
            if from_node {
                triggering.route.remove_in_place(1..3);
            } else {
                let len = triggering.route.len();
                triggering.route.remove_in_place(len - 3..len - 1);
            }
            *graph = candidate;
            if trace_node {
                eprintln!(
                    "DEJITTER_ACCEPT_RUST edge={} delta={} symmetry={}",
                    edge_id.0, delta, previous_symmetry
                );
            }
            existing_overlaps = graph.existing_overlap_pairs();
            existing_exact_overlaps = graph.exact_overlap_pairs();
            changed = true;
        }
    }
    if crate::internal::trace_env_enabled("TALA_TRACE_DEJITTER_RESULT") {
        eprintln!("DEJITTER_RESULT_RUST changed={changed}");
    }
    changed
}
