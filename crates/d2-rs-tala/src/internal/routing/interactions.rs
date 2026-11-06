// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Route-overlap compatibility derived from edge rendering semantics.
//!
//! Arrow presence and identity, explicit style fields, labels, cluster
//! membership, and direction decide whether two edges may share geometry.

use super::ArenaGraph;
use crate::NodeId;
use std::collections::BTreeSet;

/// sourceAndTargetClusterNodesGuarded scans all memberships before adding
/// endpoint peers. Retain separate source and target insertion passes.
pub(super) fn cluster_memberships_with_budget(
    graph: &ArenaGraph,
    source: NodeId,
    target: NodeId,
    budget: Option<&super::route_search_guard::LocalRouteSearchWork>,
) -> Option<(BTreeSet<NodeId>, BTreeSet<NodeId>)> {
    cluster_memberships_with_work(
        graph,
        source,
        target,
        budget.map(|guard| guard as &dyn super::route_line::LineWork),
    )
}
pub(super) fn cluster_memberships_with_work(
    graph: &ArenaGraph,
    source: NodeId,
    target: NodeId,
    budget: Option<&dyn super::route_line::LineWork>,
) -> Option<(BTreeSet<NodeId>, BTreeSet<NodeId>)> {
    scan_cluster_memberships(
        graph
            .clusters
            .iter()
            .map(|cluster| cluster.members.as_slice()),
        source,
        target,
        budget,
    )
}

fn scan_cluster_memberships<'a>(
    clusters: impl IntoIterator<Item = &'a [NodeId]>,
    source: NodeId,
    target: NodeId,
    budget: Option<&dyn super::route_line::LineWork>,
) -> Option<(BTreeSet<NodeId>, BTreeSet<NodeId>)> {
    let step = || -> Option<()> {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        Some(())
    };
    let mut source_nodes = BTreeSet::new();
    let mut target_nodes = BTreeSet::new();
    for members in clusters {
        step()?;
        let mut source_member = false;
        let mut target_member = false;
        for &member in members {
            step()?;
            source_member |= member == source;
            target_member |= member == target;
        }
        if source_member {
            for &member in members {
                step()?;
                if member != source {
                    source_nodes.insert(member);
                }
            }
        }
        if target_member {
            for &member in members {
                step()?;
                if member != target {
                    target_nodes.insert(member);
                }
            }
        }
    }
    Some((source_nodes, target_nodes))
}

fn arrowhead_identity(enabled: bool, explicit: &Option<String>) -> Option<&str> {
    enabled.then_some(explicit.as_deref().unwrap_or("__d2_default_arrowhead__"))
}

fn endpoint_cluster_members(graph: &ArenaGraph, node: NodeId) -> Option<BTreeSet<NodeId>> {
    let cluster = graph.nodes[node.0 as usize].cluster?;
    Some(
        graph.clusters[cluster]
            .members
            .iter()
            .copied()
            .filter(|member| *member != node)
            .collect(),
    )
}

/// Translation of D2 v0.9.0 `edgeCanOverlapEdges` for native edge metadata.
///
/// TALA evaluates the complete set of overlapping edges together. That
/// collective shared-node rule is materially different from accepting every
/// overlap pair independently.
pub(super) fn edges_can_overlap_all_with_members(
    graph: &ArenaGraph,
    edge_index: usize,
    other_indices: &[usize],
    source_cluster: Option<&BTreeSet<NodeId>>,
    target_cluster: Option<&BTreeSet<NodeId>>,
) -> bool {
    edges_can_overlap_all_with_members_and_style(
        graph,
        edge_index,
        other_indices,
        source_cluster,
        target_cluster,
        None,
    )
}

/// quickRoute's temporary prior edges carry empty Style; ordinary routing
/// retains each original edge's explicit style.
pub(super) fn edges_can_overlap_all_with_members_and_style(
    graph: &ArenaGraph,
    edge_index: usize,
    other_indices: &[usize],
    source_cluster: Option<&BTreeSet<NodeId>>,
    target_cluster: Option<&BTreeSet<NodeId>>,
    prior_style_override: Option<&crate::EdgeStyle>,
) -> bool {
    if other_indices.is_empty() {
        return true;
    }
    let edge = &graph.edge_records[edge_index];
    if edge.source_arrowhead_label.is_some() || edge.target_arrowhead_label.is_some() {
        return false;
    }
    if other_indices.iter().any(|index| {
        let other = &graph.edge_records[*index];
        other.source_arrowhead_label.is_some() || other.target_arrowhead_label.is_some()
    }) {
        return false;
    }
    if other_indices.iter().any(|index| {
        let other = prior_style_override.unwrap_or(&graph.edge_records[*index].style);
        // layoutgraph.Edge.EquivalentStyles compares only these fields.
        edge.style.opacity != other.opacity
            || edge.style.stroke != other.stroke
            || edge.style.stroke_width != other.stroke_width
            || edge.style.stroke_dash != other.stroke_dash
            || edge.style.animated != other.animated
    }) {
        return false;
    }

    let all_touch_cluster = |members: &BTreeSet<NodeId>| {
        other_indices.iter().all(|index| {
            let other = &graph.edge_records[*index];
            members.contains(&other.from) || members.contains(&other.to)
        })
    };
    let is_cluster_edge = source_cluster.is_some_and(all_touch_cluster)
        || target_cluster.is_some_and(all_touch_cluster);

    let directed = edge.source_arrow != edge.target_arrow;
    if directed
        && other_indices.iter().all(|index| {
            let other = &graph.edge_records[*index];
            if other.source_arrow == other.target_arrow {
                return false;
            }
            let same_directed_end = edge.to == other.to || edge.from == other.from;
            same_directed_end
                && arrowhead_identity(edge.source_arrow, &edge.source_arrowhead)
                    == arrowhead_identity(other.source_arrow, &other.source_arrowhead)
                && arrowhead_identity(edge.target_arrow, &edge.target_arrowhead)
                    == arrowhead_identity(other.target_arrow, &other.target_arrowhead)
        })
    {
        return true;
    }

    let bidirectional = edge.source_arrow && edge.target_arrow;
    let own_arrowheads_match = arrowhead_identity(true, &edge.source_arrowhead)
        == arrowhead_identity(true, &edge.target_arrowhead);
    let all_bidirectional = bidirectional
        && own_arrowheads_match
        && other_indices.iter().all(|index| {
            let other = &graph.edge_records[*index];
            if !other.source_arrow || !other.target_arrow {
                return false;
            }
            let source = arrowhead_identity(true, &edge.source_arrowhead);
            let target = arrowhead_identity(true, &edge.target_arrowhead);
            let other_source = arrowhead_identity(true, &other.source_arrowhead);
            let other_target = arrowhead_identity(true, &other.target_arrowhead);
            (source == other_source && target == other_target)
                || (source == other_target && target == other_source)
        });
    let undirected = !edge.source_arrow && !edge.target_arrow;
    let all_undirected = undirected
        && other_indices.iter().all(|index| {
            let other = &graph.edge_records[*index];
            !other.source_arrow && !other.target_arrow
        });
    if !all_bidirectional && !all_undirected {
        return false;
    }
    if is_cluster_edge {
        return true;
    }

    let total_edge_count = other_indices.len() + 1;
    let mut endpoint_counts = std::collections::BTreeMap::<NodeId, usize>::new();
    *endpoint_counts.entry(edge.from).or_default() += 1;
    *endpoint_counts.entry(edge.to).or_default() += 1;
    for index in other_indices {
        let other = &graph.edge_records[*index];
        *endpoint_counts.entry(other.from).or_default() += 1;
        *endpoint_counts.entry(other.to).or_default() += 1;
    }
    endpoint_counts
        .values()
        .any(|count| *count == total_edge_count)
}

pub(super) fn edges_can_overlap_all_with_style(
    graph: &ArenaGraph,
    edge_index: usize,
    other_indices: &[usize],
    prior_style_override: Option<&crate::EdgeStyle>,
) -> bool {
    if other_indices.is_empty() {
        return true;
    }
    let edge = &graph.edge_records[edge_index];
    let source_cluster = endpoint_cluster_members(graph, edge.from);
    let target_cluster = endpoint_cluster_members(graph, edge.to);
    edges_can_overlap_all_with_members_and_style(
        graph,
        edge_index,
        other_indices,
        source_cluster.as_ref(),
        target_cluster.as_ref(),
        prior_style_override,
    )
}

pub(super) fn edges_can_overlap_all(
    graph: &ArenaGraph,
    edge_index: usize,
    other_indices: &[usize],
) -> bool {
    if other_indices.is_empty() {
        return true;
    }
    let edge = &graph.edge_records[edge_index];
    let source_cluster = endpoint_cluster_members(graph, edge.from);
    let target_cluster = endpoint_cluster_members(graph, edge.to);
    let result = edges_can_overlap_all_with_members(
        graph,
        edge_index,
        other_indices,
        source_cluster.as_ref(),
        target_cluster.as_ref(),
    );
    if crate::internal::trace_env_enabled("TALA_TRACE_OVERLAP") && edge_index == 4 {
        eprintln!(
            "OVERLAP_RUST edge={} others={:?} source_arrow={} target_arrow={} result={}",
            edge_index, other_indices, edge.source_arrow, edge.target_arrow, result
        );
        for index in other_indices {
            let other = &graph.edge_records[*index];
            eprintln!(
                "OVERLAP_RUST_OTHER index={} from={} to={} source_arrow={} target_arrow={} style_equal={}",
                index,
                other.from.0,
                other.to.0,
                other.source_arrow,
                other.target_arrow,
                other.style == edge.style
            );
        }
    }
    result
}

/// Search-specific table-column augmentation of TALA's cluster maps. RouteLine
/// and slingshot deliberately use `edges_can_overlap_all` above without this
/// behavior; only OVG search adds opposite-end adjacencies as pseudo-members.
pub(super) fn edges_can_overlap_all_for_search(
    graph: &ArenaGraph,
    edge_index: usize,
    other_indices: &[usize],
) -> bool {
    // The shared rule accepts an empty set before reading either cluster.
    if other_indices.is_empty() {
        return true;
    }
    let edge = &graph.edge_records[edge_index];
    let mut source_cluster = endpoint_cluster_members(graph, edge.from).unwrap_or_default();
    let mut target_cluster = endpoint_cluster_members(graph, edge.to).unwrap_or_default();
    if edge.source_table_column.is_some() {
        for other_edge in &graph.nodes[edge.to.0 as usize].edges.read() {
            let other = &graph.edge_records[other_edge.0 as usize];
            source_cluster.insert(if other.from == edge.to {
                other.to
            } else {
                other.from
            });
        }
    }
    if edge.target_table_column.is_some() {
        for other_edge in &graph.nodes[edge.from.0 as usize].edges.read() {
            let other = &graph.edge_records[other_edge.0 as usize];
            target_cluster.insert(if other.from == edge.from {
                other.to
            } else {
                other.from
            });
        }
    }
    edges_can_overlap_all_with_members(
        graph,
        edge_index,
        other_indices,
        (!source_cluster.is_empty()).then_some(&source_cluster),
        (!target_cluster.is_empty()).then_some(&target_cluster),
    )
}

/// Search endpoint cluster sets that depend only on immutable graph fields:
/// cluster membership without the table-column pseudo-member augmentation,
/// whose incident-edge reads stay per call.
pub(super) struct SearchClusters {
    source: BTreeSet<NodeId>,
    target: BTreeSet<NodeId>,
}

pub(super) fn search_cluster_members(
    graph: &ArenaGraph,
    edge_index: usize,
) -> Option<SearchClusters> {
    let edge = &graph.edge_records[edge_index];
    if edge.source_table_column.is_some() || edge.target_table_column.is_some() {
        return None;
    }
    Some(SearchClusters {
        source: endpoint_cluster_members(graph, edge.from).unwrap_or_default(),
        target: endpoint_cluster_members(graph, edge.to).unwrap_or_default(),
    })
}

/// `edges_can_overlap_all_for_search` with clusters prepared for this edge
/// by `search_cluster_members` against the same immutable graph.
pub(super) fn edges_can_overlap_all_for_search_with(
    graph: &ArenaGraph,
    edge_index: usize,
    other_indices: &[usize],
    clusters: Option<&SearchClusters>,
) -> bool {
    let Some(clusters) = clusters else {
        return edges_can_overlap_all_for_search(graph, edge_index, other_indices);
    };
    if other_indices.is_empty() {
        return true;
    }
    edges_can_overlap_all_with_members(
        graph,
        edge_index,
        other_indices,
        (!clusters.source.is_empty()).then_some(&clusters.source),
        (!clusters.target.is_empty()).then_some(&clusters.target),
    )
}

pub(super) fn edges_can_overlap(graph: &ArenaGraph, edge_index: usize, other_index: usize) -> bool {
    edges_can_overlap_all(graph, edge_index, &[other_index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContentAlignment, Edge, EdgeArrows, EdgeStyle, Graph, Insets, LabelPosition, Node,
        ShapeKind, Size,
    };

    #[test]
    fn cluster_scan_preserves_full_membership_and_separate_peer_passes() {
        use super::super::route_search_guard::LocalRouteSearchWork;
        let clusters = [
            vec![NodeId(0), NodeId(1), NodeId(2)],
            vec![NodeId(2), NodeId(3)],
        ];
        let work = LocalRouteSearchWork::new(15);
        let result = scan_cluster_memberships(
            clusters.iter().map(Vec::as_slice),
            NodeId(0),
            NodeId(2),
            Some(&work),
        )
        .unwrap();
        assert_eq!(result.0, BTreeSet::from([NodeId(1), NodeId(2)]));
        assert_eq!(result.1, BTreeSet::from([NodeId(0), NodeId(1), NodeId(3)]));
        assert_eq!(work.used(), 15);
        for limit in 0..15 {
            let work = LocalRouteSearchWork::new(limit);
            assert!(
                scan_cluster_memberships(
                    clusters.iter().map(Vec::as_slice),
                    NodeId(0),
                    NodeId(2),
                    Some(&work)
                )
                .is_none()
            );
            assert_eq!(work.used(), limit);
        }
        let work = LocalRouteSearchWork::new(10);
        let (source, target) =
            scan_cluster_memberships([clusters[0].as_slice()], NodeId(0), NodeId(0), Some(&work))
                .unwrap();
        assert_eq!(source, target);
        assert_eq!(work.used(), 10);
        let work = LocalRouteSearchWork::new(1);
        assert_eq!(
            scan_cluster_memberships([&[][..]], NodeId(0), NodeId(1), Some(&work)),
            Some((BTreeSet::new(), BTreeSet::new()))
        );
        assert_eq!(work.used(), 1);
    }

    fn node(name: &str) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size {
                width: 40.0,
                height: 40.0,
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
    fn default_triangle_and_explicit_triangle_share_directed_routes() {
        let mut input = Graph::default();
        let left = input.add_node(node("left"));
        let right = input.add_node(node("right"));
        let target = input.add_node(node("target"));
        for source in [left, right] {
            let id = input.add_edge(Edge { source, target });
            input.set_edge_arrows(
                id,
                EdgeArrows {
                    source: false,
                    target: true,
                },
            );
        }
        input.set_edge_arrowheads(
            crate::EdgeId(1),
            crate::EdgeArrowheads {
                source: None,
                target: Some("triangle".into()),
            },
        );
        let mut graph = ArenaGraph::from_input(&input);
        assert!(edges_can_overlap(&graph, 0, 1));
        assert!(edges_can_overlap(&graph, 1, 0));
        graph.edge_records[1].target_arrowhead = Some("unfilled-triangle".into());
        assert!(!edges_can_overlap(&graph, 0, 1));
        graph.edge_records[1].target_arrow = false;
        graph.edge_records[1].target_arrowhead = Some("none".into());
        assert!(!edges_can_overlap(&graph, 0, 1));
    }

    #[test]
    fn quickroute_prior_style_is_projected_without_mutating_original_metadata() {
        let mut input = Graph::default();
        let source = input.add_node(node("source"));
        let target = input.add_node(node("target"));
        let prior = input.add_edge(Edge {
            source,
            target: source,
        });
        let current = input.add_edge(Edge { source, target });
        for edge in [prior, current] {
            input.set_edge_arrows(
                edge,
                EdgeArrows {
                    source: false,
                    target: true,
                },
            );
            input.set_edge_style(
                edge,
                EdgeStyle {
                    stroke: Some("red".into()),
                    ..EdgeStyle::default()
                },
            );
        }
        let original = ArenaGraph::from_input(&input);
        assert!(edges_can_overlap_all(
            &original,
            current.0 as usize,
            &[prior.0 as usize]
        ));
        let zero_style = EdgeStyle::default();
        assert!(!edges_can_overlap_all_with_style(
            &original,
            current.0 as usize,
            &[prior.0 as usize],
            Some(&zero_style)
        ));
        input.set_edge_style(current, zero_style.clone());
        let current_unstyled = ArenaGraph::from_input(&input);
        assert!(!edges_can_overlap_all(
            &current_unstyled,
            current.0 as usize,
            &[prior.0 as usize]
        ));
        assert!(edges_can_overlap_all_with_style(
            &current_unstyled,
            current.0 as usize,
            &[prior.0 as usize],
            Some(&zero_style)
        ));
        assert_eq!(
            original.edge_records[prior.0 as usize]
                .style
                .stroke
                .as_deref(),
            Some("red")
        );
    }

    #[test]
    fn directed_routes_require_equivalent_explicit_styles_to_overlap() {
        let mut input = Graph::default();
        let source = input.add_node(node("source"));
        let first_target = input.add_node(node("first"));
        let second_target = input.add_node(node("second"));
        let solid = input.add_edge(Edge {
            source,
            target: first_target,
        });
        let dashed = input.add_edge(Edge {
            source,
            target: second_target,
        });
        for edge in [solid, dashed] {
            input.set_edge_arrows(
                edge,
                EdgeArrows {
                    source: false,
                    target: true,
                },
            );
        }

        let arena = ArenaGraph::from_input(&input);
        assert!(edges_can_overlap_all(
            &arena,
            dashed.0 as usize,
            &[solid.0 as usize]
        ));

        input.set_edge_style(
            dashed,
            EdgeStyle {
                stroke: Some("#000E3D".into()),
                stroke_dash: Some("4".into()),
                ..EdgeStyle::default()
            },
        );
        let arena = ArenaGraph::from_input(&input);
        assert!(!edges_can_overlap_all(
            &arena,
            dashed.0 as usize,
            &[solid.0 as usize]
        ));
    }
}
