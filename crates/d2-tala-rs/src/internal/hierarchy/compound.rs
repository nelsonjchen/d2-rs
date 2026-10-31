// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Rigid-block interface ordering from D2 v0.9.0 hierarchy/compound.go.
//! Proxy construction and guarded placement integration for outer compounds.
use super::limits::{Error, WorkGuard};
use crate::{Point, Size};

/// Isolated input to hierarchy ranking. Original identities are retained beside
/// dense proxy identities, so proxy allocation cannot mutate the source arena.
pub(super) struct CompoundProxy {
    pub graph: super::ArenaGraph,
    pub roots: Vec<crate::NodeId>,
    pub members: Vec<Vec<crate::NodeId>>,
    pub interfaces: Vec<CompoundInterface>,
    pub original_edges: Vec<crate::EdgeId>,
}

/// Requires the validated, fully placed input used by upstream PlaceCompound.
/// Validation, hierarchy placement and committing the proposal belong to the caller.
pub(super) fn build_compound_proxy(
    original: &super::ArenaGraph,
    guard: &mut WorkGuard<'_>,
) -> Result<Option<CompoundProxy>, Error> {
    use crate::{Edge, EdgeArrowheads, EdgeArrows, Graph, Node};
    let roots = original.containers.get(&None).unwrap_or_default();
    if roots.len() < 3 || roots.len() > 64 {
        return Ok(None);
    }
    let direction = original.directions.get(&None).copied().unwrap_or_default();
    let mut input = Graph::with_direction(direction);
    let mut owner = vec![None; original.nodes.len()];
    let mut members = vec![Vec::new(); roots.len()];
    for &root in &roots {
        guard.step()?;
        let node = &original.nodes[root.0 as usize];
        input.add_node(Node::new(node.tala_id.to_string(), node.rect.size));
    }
    let order = if original.node_order.is_empty() {
        original
            .nodes
            .iter()
            .map(|n| n.input_id)
            .collect::<Vec<_>>()
    } else {
        original.node_order.clone()
    };
    for id in order {
        guard.step()?;
        let node = &original.nodes[id.0 as usize];
        // This arena flag identifies active grouping vessels, not authored
        // sequence/container shapes. Ordinary descendants remain rigid members.
        if node.fixed_top_left.is_some() || node.scoring_is_aggregate_vessel {
            return Ok(None);
        }
        let mut root = id;
        while let Some(parent) = original.nodes[root.0 as usize].container {
            guard.step()?;
            root = parent;
        }
        let Some(index) = roots.iter().position(|&candidate| candidate == root) else {
            return Ok(None);
        };
        members[index].push(id);
        owner[id.0 as usize] = Some(index);
    }
    for node in original.tree_nodes_in_extraction_order() {
        guard.step()?;
        if owner[node.0 as usize].is_none() {
            return Ok(None);
        }
    }
    let mut interfaces = Vec::new();
    let mut original_edges = Vec::new();
    let mut detailed = false;
    for edge in original.member_edges() {
        guard.step()?;
        let (Some(from), Some(to)) = (owner[edge.from.0 as usize], owner[edge.to.0 as usize])
        else {
            return Ok(None);
        };
        if from == to {
            detailed |= original.nodes[roots[from].0 as usize].is_container && edge.from != edge.to;
            continue;
        }
        if interfaces.len() == 256 {
            return Ok(None);
        }
        let id = input.add_edge(Edge {
            source: crate::NodeId(from as u32),
            target: crate::NodeId(to as u32),
        });
        input.tala_edge_ids[id.0 as usize] = Some(edge.tala_id);
        input.set_edge_arrows(
            id,
            EdgeArrows {
                source: edge.source_arrow,
                target: edge.target_arrow,
            },
        );
        input.set_edge_arrowheads(
            id,
            EdgeArrowheads {
                source: edge.source_arrowhead.clone(),
                target: edge.target_arrowhead.clone(),
            },
        );
        input.set_edge_label(id, edge.label.clone());
        interfaces.push(CompoundInterface {
            from,
            to,
            from_offset: compound_endpoint_offset(
                original,
                edge.from,
                roots[from],
                edge.source_table_column,
            ),
            to_offset: compound_endpoint_offset(
                original,
                edge.to,
                roots[to],
                edge.target_table_column,
            ),
        });
        original_edges.push(edge.input_id);
    }
    if !detailed || !compound_connected(roots.len(), &interfaces) {
        return Ok(None);
    }
    let mut graph = super::ArenaGraph::from_input(&input);
    for (i, &root) in roots.iter().enumerate() {
        graph.nodes[i].tala_id = original.nodes[root.0 as usize].tala_id;
        graph.set_position(
            crate::NodeId(i as u32),
            original.position(root).unwrap_or_default(),
        );
    }
    Ok(Some(CompoundProxy {
        graph,
        roots,
        members,
        interfaces,
        original_edges,
    }))
}

fn place_compound_proxy(
    proxy: &mut CompoundProxy,
    guard: &mut WorkGuard<'_>,
    placement_rng: &mut super::go_rng::GoRng,
) -> Result<Option<Vec<CompoundBlock>>, Error> {
    use super::hierarchy_placement::HierarchyPlacement;
    use crate::{Direction, NodeId};
    let nodes = proxy.graph.node_order.clone();
    // DirectedEndpoints reverses source-only arrows before DAG construction.
    let edges = proxy
        .graph
        .member_edges()
        .map(|edge| {
            if edge.source_arrow && !edge.target_arrow {
                (edge.to, edge.from, true)
            } else {
                (edge.from, edge.to, edge.source_arrow != edge.target_arrow)
            }
        })
        .collect::<Vec<_>>();
    let Some(rank) = proxy.graph.rank_hierarchy_component(&nodes, &edges, true) else {
        return Ok(None);
    };
    let forward = edges
        .iter()
        .filter(|(from, to, directed)| *directed && rank.level[to] > rank.level[from])
        .count();
    if rank.level_count < 2
        || forward == 0
        || (forward as f64) < 1.5 * ((edges.len() - forward) as f64)
    {
        return Ok(None);
    }
    for &node in &nodes {
        guard.step()?;
        proxy
            .graph
            .assign_hierarchy_member(node, 0, None, rank.level[&node], rank.level_count);
    }
    // Place splits the connected proxy into BFS graph order before constructing
    // placement nodes; discovery above used the original proxy declaration order.
    let placement_order = proxy.graph.hierarchy_components(&nodes, &edges).remove(0);
    proxy.graph.compute_cell_size();
    let mut placement =
        HierarchyPlacement::build(&proxy.graph, &placement_order, &rank, placement_rng);
    let direction = proxy.graph.directions[&None];
    let horizontal = matches!(direction, Direction::Left | Direction::Right);
    if horizontal {
        for node in &mut proxy.graph.nodes {
            std::mem::swap(&mut node.rect.size.width, &mut node.rect.size.height);
        }
        // The reused level placer reads height for crossing-label spacing.
        for edge_index in proxy.graph.member_edge_indices() {
            let edge = &mut proxy.graph.edge_records[edge_index];
            if let Some(label) = edge.label.as_mut() {
                std::mem::swap(&mut label.size.width, &mut label.size.height);
            }
        }
    }
    let points = placement.place_ranked_nodes(&mut proxy.graph);
    for (node, mut position) in points {
        if horizontal {
            std::mem::swap(&mut position.x, &mut position.y);
        }
        proxy.graph.set_position(node, position);
    }
    if horizontal {
        for node in &mut proxy.graph.nodes {
            std::mem::swap(&mut node.rect.size.width, &mut node.rect.size.height);
        }
        for edge_index in proxy.graph.member_edge_indices() {
            let edge = &mut proxy.graph.edge_records[edge_index];
            if let Some(label) = edge.label.as_mut() {
                std::mem::swap(&mut label.size.width, &mut label.size.height);
            }
        }
    }
    for &node in &nodes {
        let mut position = proxy.graph.position(node).unwrap();
        let size = proxy.graph.nodes[node.0 as usize].rect.size;
        if direction == Direction::Left {
            position.x = -position.x - size.width;
        }
        if direction == Direction::Up {
            position.y = -position.y - size.height;
        }
        proxy.graph.set_position(node, position);
    }
    let mut blocks = nodes
        .iter()
        .map(|&id| CompoundBlock {
            position: proxy.graph.position(id).unwrap(),
            size: proxy.graph.nodes[id.0 as usize].rect.size,
            level: rank.level[&id],
        })
        .collect::<Vec<_>>();
    order_compound_interfaces(
        &mut blocks,
        &proxy.interfaces,
        rank.level_count,
        horizontal,
        guard,
    )?;
    for (index, block) in blocks.iter().enumerate() {
        proxy
            .graph
            .set_position(NodeId(index as u32), block.position);
    }
    Ok(Some(blocks))
}

impl super::ArenaGraph {
    pub(super) fn apply_compound_flow(&mut self) -> bool {
        use super::go_rng::GoRng;

        struct Background;
        impl super::limits::Context for Background {
            fn done(&self) -> Option<bool> {
                crate::internal::cancellation_done()
            }
            fn error(&self) -> Option<super::limits::ContextError> {
                crate::internal::cancellation_error()
            }
        }
        let mut guard = WorkGuard::new(
            Some(&Background),
            "PlaceCompoundTransactions",
            1_000_000_000,
        )
        .unwrap();
        let roots = self.containers.get(&None).unwrap_or_default();
        if self.nodes.len() > 128
            || self.edge_order.len() > 256
            || !(3..=64).contains(&roots.len())
            || !roots
                .iter()
                .any(|root| self.nodes[root.0 as usize].is_container)
            || self.nodes.iter().any(|node| node.fixed_top_left.is_some())
        {
            return false;
        }
        // D2 opens this stream after the outer admission checks and before
        // PlaceCompound validates/builds its proxy. Keep the event even when a
        // later inner check rejects the proposal.
        let mut placement_rng = GoRng::with_stream(0, "compound-placement");
        let mut candidate = self.clone();
        if !candidate
            .place_compound_guarded_with_rng(&mut guard, &mut placement_rng)
            .unwrap_or(false)
            || candidate.compound_cross_axis_detours() > self.compound_cross_axis_detours()
        {
            return false;
        }
        *self = candidate;
        // Go has one TopLeft per node. Compound placement moves the owner
        // points, so refresh the arena copies that routing reads directly
        // (route_line/node_rect); otherwise reroutePlaced's
        // StraightEdgesFallback measures direct lines from the old frame.
        for index in 0..self.nodes.len() {
            if let Some(position) = self.position(crate::NodeId(index as u32)) {
                self.nodes[index].position = Some(position);
                self.nodes[index].rect.origin = position;
            }
        }
        true
    }

    pub(super) fn compound_cross_axis_detours(&self) -> usize {
        let root_of = |mut id: crate::NodeId| {
            while let Some(parent) = self.nodes[id.0 as usize].container {
                id = parent;
            }
            id
        };
        let horizontal = matches!(
            self.directions.get(&None),
            Some(crate::Direction::Left | crate::Direction::Right)
        );
        self.member_edges()
            .filter(|edge| {
                let a = root_of(edge.from);
                let b = root_of(edge.to);
                if a == b {
                    return false;
                }
                let ap = self.position(a).unwrap();
                let bp = self.position(b).unwrap();
                let az = self.nodes[a.0 as usize].rect.size;
                let bz = self.nodes[b.0 as usize].rect.size;
                if horizontal {
                    ap.y + az.height < bp.y || bp.y + bz.height < ap.y
                } else {
                    ap.x + az.width < bp.x || bp.x + bz.width < ap.x
                }
            })
            .count()
    }

    fn place_compound_guarded(&mut self, guard: &mut WorkGuard<'_>) -> Result<bool, Error> {
        let mut placement_rng = super::go_rng::GoRng::new_untraced(0);
        self.place_compound_guarded_with_rng(guard, &mut placement_rng)
    }

    fn place_compound_guarded_with_rng(
        &mut self,
        guard: &mut WorkGuard<'_>,
        placement_rng: &mut super::go_rng::GoRng,
    ) -> Result<bool, Error> {
        let Some(mut proxy) = build_compound_proxy(self, guard)? else {
            return Ok(false);
        };
        let Some(blocks) = place_compound_proxy(&mut proxy, guard, placement_rng)? else {
            return Ok(false);
        };
        let proxy_nodes = proxy.graph.node_order.clone();
        let Some((top_left, bottom_right)) = proxy.graph.bin_pack_active_bounds(&proxy_nodes)
        else {
            return Ok(false);
        };
        if bottom_right.x - top_left.x > 30_000.0 || bottom_right.y - top_left.y > 30_000.0 {
            return Ok(false);
        }
        // v0.9.0 anchors the rigid translation against Nodes.BoundingBox(),
        // whose top-left is the ordinary content bound.  FixedBoundingBox()
        // is only used by packing and would replace that anchor with a fixed
        // origin when a child carries one.
        let Some((original_top_left, _)) = self.ordinary_node_bounds(&proxy.roots) else {
            return Ok(false);
        };
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMPOUND_NODES") {
            for &node in &proxy.roots {
                let value = &self.nodes[node.0 as usize];
                eprintln!(
                    "COMPOUND_NODE_RUST id={} pos={:?} size={:?} label={:?}/{:?} ext={:?} icon={:?} loop={:?} modifier=({}, {})",
                    value.tala_id,
                    self.position(node),
                    value.rect.size,
                    value.label_position,
                    value.label_size,
                    value.external_label.map(|label| (
                        label.side,
                        label.alignment,
                        label.size,
                        label.reserve_space
                    )),
                    value.icon_position,
                    value.loop_offsets,
                    value.is_multiple,
                    value.is_3d,
                );
            }
        }
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_COMPOUND") {
            eprintln!(
                "COMPOUND_BOUNDS_RUST original={:?} proxy={:?},{:?}",
                original_top_left, top_left, bottom_right
            );
        }
        // Keep mutation isolated until the guarded proposal is complete.
        let mut candidate = self.clone();
        let hierarchy_id = self
            .nodes
            .iter()
            .filter_map(|n| n.hierarchy.clone().map(|h| h.id))
            .max()
            .map_or(0, |id| id + 1);
        let level_count = blocks.iter().map(|b| b.level).max().unwrap() + 1;
        for (index, &root) in proxy.roots.iter().enumerate() {
            guard.step()?;
            let original = self.position(root).unwrap();
            let delta = Point {
                x: blocks[index].position.x - top_left.x + original_top_left.x - original.x,
                y: blocks[index].position.y - top_left.y + original_top_left.y - original.y,
            };
            for &member in &proxy.members[index] {
                guard.step()?;
                let position = self.position(member).unwrap();
                candidate.set_position(
                    member,
                    Point {
                        x: position.x + delta.x,
                        y: position.y + delta.y,
                    },
                );
            }
            candidate.remove_hierarchy_member(root);
            candidate.assign_hierarchy_member(
                root,
                hierarchy_id,
                None,
                blocks[index].level,
                level_count,
            );
        }
        guard.finish()?;
        // Upstream PlaceCompound reports `changed=false` when the ranked
        // outer blocks land at their existing positions.  A hierarchy marker
        // alone is not an observable compound proposal and must not trigger
        // the expensive reroute trial (or alter the final frame).
        let changed = proxy.roots.iter().enumerate().any(|(index, &root)| {
            candidate.position(root) != self.position(root)
                || proxy.members[index]
                    .iter()
                    .any(|&member| candidate.position(member) != self.position(member))
        });
        if !changed {
            return Ok(false);
        }
        *self = candidate;
        Ok(true)
    }
}

fn compound_endpoint_offset(
    graph: &super::ArenaGraph,
    endpoint: crate::NodeId,
    block: crate::NodeId,
    column: Option<usize>,
) -> Point {
    let node = &graph.nodes[endpoint.0 as usize];
    let position = graph.position(endpoint).unwrap_or_default();
    let mut center = Point {
        x: position.x + node.rect.size.width / 2.0,
        y: position.y + node.rect.size.height / 2.0,
    };
    if node.shape == crate::ShapeKind::SqlTable
        && let Some(column) = column
        && let Some(port) = super::routing::nth_port_point_on_side(
            crate::Rect {
                origin: position,
                size: node.rect.size,
            },
            node.shape,
            node.table_column_count,
            super::routing::PortSide::Right,
            column,
        )
    {
        center = port;
    }
    let root = &graph.nodes[block.0 as usize];
    let origin = graph.position(block).unwrap_or_default();
    Point {
        x: (center.x - origin.x).clamp(0.0, root.rect.size.width),
        y: (center.y - origin.y).clamp(0.0, root.rect.size.height),
    }
}

fn compound_connected(count: usize, interfaces: &[CompoundInterface]) -> bool {
    let mut adjacent = vec![Vec::new(); count];
    for edge in interfaces {
        adjacent[edge.from].push(edge.to);
        adjacent[edge.to].push(edge.from);
    }
    let mut seen = vec![false; count];
    seen[0] = true;
    let mut queue = vec![0];
    let mut index = 0;
    while index < queue.len() {
        for &next in &adjacent[queue[index]] {
            if !seen[next] {
                seen[next] = true;
                queue.push(next);
            }
        }
        index += 1;
    }
    queue.len() == count
}

#[derive(Clone, Debug)]
pub(super) struct CompoundBlock {
    pub position: Point,
    pub size: Size,
    pub level: usize,
}

#[derive(Clone, Debug)]
pub(super) struct CompoundInterface {
    pub from: usize,
    pub to: usize,
    pub from_offset: Point,
    pub to_offset: Point,
}

#[derive(Debug, PartialEq)]
pub(super) struct CompoundScore {
    pub crossings: usize,
    pub span: f64,
}

pub(super) fn compound_interface_score(
    blocks: &[CompoundBlock],
    interfaces: &[CompoundInterface],
    horizontal: bool,
    guard: &mut WorkGuard<'_>,
) -> Result<CompoundScore, Error> {
    let mut score = CompoundScore {
        crossings: 0,
        span: 0.0,
    };
    let mut segments = Vec::with_capacity(interfaces.len());
    for edge in interfaces {
        guard.step()?;
        let from = &blocks[edge.from];
        let to = &blocks[edge.to];
        let mut a = Point {
            x: from.position.x + edge.from_offset.x,
            y: from.position.y + edge.from_offset.y,
        };
        let mut b = Point {
            x: to.position.x + edge.to_offset.x,
            y: to.position.y + edge.to_offset.y,
        };
        if horizontal {
            if from.position.x + from.size.width / 2.0 < to.position.x + to.size.width / 2.0 {
                a.x = from.position.x + from.size.width;
                b.x = to.position.x;
            } else {
                a.x = from.position.x;
                b.x = to.position.x + to.size.width;
            }
            score.span += (a.y - b.y).abs();
        } else {
            if from.position.y + from.size.height / 2.0 < to.position.y + to.size.height / 2.0 {
                a.y = from.position.y + from.size.height;
                b.y = to.position.y;
            } else {
                a.y = from.position.y;
                b.y = to.position.y + to.size.height;
            }
            score.span += (a.x - b.x).abs();
        }
        segments.push((a, b));
    }
    fn orientation(a: Point, b: Point, c: Point) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    for (i, &(a, b)) in segments.iter().enumerate() {
        for &(c, d) in &segments[i + 1..] {
            guard.step()?;
            if orientation(a, b, c) * orientation(a, b, d) < 0.0
                && orientation(c, d, a) * orientation(c, d, b) < 0.0
            {
                score.crossings += 1;
            }
        }
    }
    Ok(score)
}

pub(super) fn order_compound_interfaces(
    blocks: &mut [CompoundBlock],
    interfaces: &[CompoundInterface],
    level_count: usize,
    horizontal: bool,
    guard: &mut WorkGuard<'_>,
) -> Result<(), Error> {
    let coordinate = |block: &CompoundBlock| {
        if horizontal {
            block.position.y
        } else {
            block.position.x
        }
    };
    let extent = |block: &CompoundBlock| {
        if horizontal {
            block.size.height
        } else {
            block.size.width
        }
    };
    let set_coordinate = |block: &mut CompoundBlock, value| {
        if horizontal {
            block.position.y = value
        } else {
            block.position.x = value
        }
    };
    let mut levels = vec![Vec::new(); level_count];
    for (i, block) in blocks.iter().enumerate() {
        levels[block.level].push(i);
    }
    for row in &mut levels {
        // Preserve Go's unstable PDQsort ordering for equal coordinates.
        super::go_sort::sort_by(row, |&a, &b| {
            coordinate(&blocks[a]) < coordinate(&blocks[b])
        });
    }
    let mut score = compound_interface_score(blocks, interfaces, horizontal, guard)?;
    for _ in 0..2 {
        let mut improved = false;
        for row in &mut levels {
            for i in 0..row.len().saturating_sub(1) {
                guard.step()?;
                let (a, b) = (row[i], row[i + 1]);
                let (old_a, old_b) = (coordinate(&blocks[a]), coordinate(&blocks[b]));
                let proposed_a = old_b + extent(&blocks[b]) - extent(&blocks[a]);
                set_coordinate(&mut blocks[a], proposed_a);
                set_coordinate(&mut blocks[b], old_a);
                let candidate = compound_interface_score(blocks, interfaces, horizontal, guard)?;
                if candidate.crossings < score.crossings
                    || (candidate.crossings == score.crossings && candidate.span < score.span)
                {
                    score = candidate;
                    row.swap(i, i + 1);
                    improved = true;
                } else {
                    set_coordinate(&mut blocks[a], old_a);
                    set_coordinate(&mut blocks[b], old_b);
                }
            }
        }
        if !improved {
            break;
        }
    }
    guard.finish()
}

#[cfg(test)]
mod tests {
    use super::super::limits::{Context, ContextError};
    use super::*;
    struct Background;
    impl Context for Background {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }
    fn witness() -> (Vec<CompoundBlock>, Vec<CompoundInterface>) {
        let blocks = [
            (0.0, 0.0, 200.0, 0),
            (0.0, 200.0, 40.0, 1),
            (160.0, 200.0, 40.0, 1),
        ]
        .into_iter()
        .map(|(x, y, width, level)| CompoundBlock {
            position: Point { x, y },
            size: Size {
                width,
                height: 60.0,
            },
            level,
        })
        .collect();
        let interfaces = [(1, 180.0), (2, 20.0)]
            .into_iter()
            .map(|(to, x)| CompoundInterface {
                from: 0,
                to,
                from_offset: Point { x, y: 0.0 },
                to_offset: Point { x: 20.0, y: 0.0 },
            })
            .collect();
        (blocks, interfaces)
    }
    // Translated from TestCompoundOrderingUsesBoundaryOffsets, pinned upstream
    // hierarchy/compound_test.go. Horizontal transposition extends that case.
    #[test]
    fn boundary_offsets_reorder_targets_in_both_axes() {
        for horizontal in [false, true] {
            let (mut blocks, mut interfaces) = witness();
            if horizontal {
                for b in &mut blocks {
                    std::mem::swap(&mut b.position.x, &mut b.position.y);
                    std::mem::swap(&mut b.size.width, &mut b.size.height);
                }
                for e in &mut interfaces {
                    std::mem::swap(&mut e.from_offset.x, &mut e.from_offset.y);
                    std::mem::swap(&mut e.to_offset.x, &mut e.to_offset.y);
                }
            }
            let mut guard =
                WorkGuard::new(Some(&Background), "compound ordering test", 10000).unwrap();
            assert_eq!(
                compound_interface_score(&blocks, &interfaces, horizontal, &mut guard)
                    .unwrap()
                    .crossings,
                1
            );
            order_compound_interfaces(&mut blocks, &interfaces, 2, horizontal, &mut guard).unwrap();
            assert_eq!(
                compound_interface_score(&blocks, &interfaces, horizontal, &mut guard).unwrap(),
                CompoundScore {
                    crossings: 0,
                    span: 0.0
                }
            );
            let cross = |b: &CompoundBlock| {
                if horizontal {
                    b.position.y
                } else {
                    b.position.x
                }
            };
            assert!(cross(&blocks[1]) > cross(&blocks[2]));
        }
    }
    // Input translated from upstream compoundTestGraph. This test covers proxy
    // construction only; it does not claim PlaceCompound geometry parity.
    fn proxy_witness() -> super::super::ArenaGraph {
        use crate::{Edge, EdgeArrows, Graph, Node, NodeId};
        let mut input = Graph::default();
        for i in 0..6 {
            input.add_node(Node::new(
                i.to_string(),
                if i == 3 {
                    Size {
                        width: 300.0,
                        height: 220.0,
                    }
                } else {
                    Size {
                        width: 120.0,
                        height: 90.0,
                    }
                },
            ));
        }
        for i in 6..8 {
            let mut child = Node::new(
                i.to_string(),
                Size {
                    width: 80.0,
                    height: 50.0,
                },
            );
            child.parent = Some(NodeId(3));
            input.add_node(child);
        }
        for (from, to) in [(0, 2), (0, 3), (1, 3), (2, 4), (3, 5), (6, 7), (7, 5)] {
            let id = input.add_edge(Edge {
                source: NodeId(from),
                target: NodeId(to),
            });
            input.set_edge_arrows(
                id,
                EdgeArrows {
                    source: false,
                    target: true,
                },
            );
        }
        let mut graph = super::super::ArenaGraph::from_input(&input);
        for i in 0..6 {
            graph.set_position(
                NodeId(i),
                Point {
                    x: f64::from(i) * 400.0,
                    y: 300.0,
                },
            );
        }
        graph.set_position(
            NodeId(6),
            Point {
                x: 1240.0,
                y: 360.0,
            },
        );
        graph.set_position(
            NodeId(7),
            Point {
                x: 1360.0,
                y: 420.0,
            },
        );
        graph
    }
    #[test]
    fn proxy_preserves_interfaces_and_isolates_detailed_members() {
        let mut graph = proxy_witness();
        for (index, edge) in graph.edge_records.iter_mut().enumerate() {
            edge.tala_id = -100 - index as i64;
            edge.invisible = true;
        }
        let original = graph.nodes.iter().map(|n| n.position).collect::<Vec<_>>();
        let mut guard = WorkGuard::new(Some(&Background), "proxy", 10000).unwrap();
        let mut proxy = build_compound_proxy(&graph, &mut guard).unwrap().unwrap();
        // Upstream compound proxies use fresh Connect edges, copying IDs and
        // arrows but not the original IsInvisible or style fields.
        assert!(proxy.graph.member_edges().all(|edge| !edge.is_invisible()));
        assert_eq!(proxy.graph.nodes.len(), 6);
        for (edge, original_id) in proxy.graph.member_edges().zip(&proxy.original_edges) {
            assert_eq!(
                edge.tala_id,
                graph.edge_records[original_id.0 as usize].tala_id
            );
        }
        assert_eq!(proxy.interfaces.len(), 6);
        assert_eq!(
            proxy.members[3],
            vec![crate::NodeId(3), crate::NodeId(6), crate::NodeId(7)]
        );
        assert_eq!(
            proxy.original_edges,
            vec![
                crate::EdgeId(0),
                crate::EdgeId(1),
                crate::EdgeId(2),
                crate::EdgeId(3),
                crate::EdgeId(4),
                crate::EdgeId(6)
            ]
        );
        assert_eq!(proxy.interfaces[5].from, 3);
        assert_eq!(
            proxy.interfaces[5].from_offset,
            Point { x: 200.0, y: 145.0 }
        );
        assert!(proxy.graph.member_edges().all(|edge| edge.target_arrow));
        assert!(proxy.graph.nodes.iter().all(|node| !node.is_container));
        proxy.graph.set_position(
            crate::NodeId(3),
            Point {
                x: -500.0,
                y: -500.0,
            },
        );
        assert_eq!(
            graph.nodes.iter().map(|n| n.position).collect::<Vec<_>>(),
            original
        );
    }
    #[test]
    fn compound_translation_keeps_interiors_rigid_in_every_direction() {
        use crate::{Direction, NodeId};
        for direction in [
            Direction::Down,
            Direction::Up,
            Direction::Left,
            Direction::Right,
        ] {
            let mut graph = proxy_witness();
            graph.directions.insert(None, direction);
            let before = graph.position(NodeId(7)).unwrap();
            let root_before = graph.position(NodeId(3)).unwrap();
            let mut guard = WorkGuard::new(Some(&Background), "compound", 1_000_000_000).unwrap();
            assert!(
                graph.place_compound_guarded(&mut guard).unwrap(),
                "{direction:?}"
            );
            let after = graph.position(NodeId(7)).unwrap();
            let root_after = graph.position(NodeId(3)).unwrap();
            assert_eq!(after.x - root_after.x, before.x - root_before.x);
            assert_eq!(after.y - root_after.y, before.y - root_before.y);
            assert!(graph.nodes[7].hierarchy.is_none());
            assert!(graph.nodes[3].hierarchy.is_some());
        }
    }
    #[test]
    fn compound_work_failure_leaves_original_positions_untouched() {
        let mut graph = proxy_witness();
        let before = graph.nodes.iter().map(|n| n.position).collect::<Vec<_>>();
        let mut guard = WorkGuard::new(Some(&Background), "compound", 1).unwrap();
        assert!(graph.place_compound_guarded(&mut guard).is_err());
        assert_eq!(
            graph.nodes.iter().map(|n| n.position).collect::<Vec<_>>(),
            before
        );
    }
    #[test]
    fn proxy_rejects_fixed_and_disconnected_inputs() {
        let mut graph = proxy_witness();
        graph.nodes[6].fixed_top_left = Some(Point {
            x: 1240.0,
            y: 360.0,
        });
        let mut guard = WorkGuard::new(Some(&Background), "proxy", 10000).unwrap();
        assert!(build_compound_proxy(&graph, &mut guard).unwrap().is_none());
        graph.nodes[6].fixed_top_left = None;
        let remove: Vec<_> = graph
            .member_edge_entries()
            .filter_map(|(index, edge)| {
                (edge.from == crate::NodeId(1)).then_some(crate::EdgeId(index as u32))
            })
            .collect();
        for edge in remove {
            graph.disconnect_edge(edge);
        }
        assert!(build_compound_proxy(&graph, &mut guard).unwrap().is_none());
    }
    #[test]
    fn non_table_column_metadata_uses_center_and_clamps_to_block() {
        let mut graph = proxy_witness();
        let center = compound_endpoint_offset(&graph, crate::NodeId(7), crate::NodeId(3), None);
        assert_eq!(
            compound_endpoint_offset(&graph, crate::NodeId(7), crate::NodeId(3), Some(0)),
            center
        );
        graph.set_position(
            crate::NodeId(7),
            Point {
                x: 2000.0,
                y: 2000.0,
            },
        );
        assert_eq!(
            compound_endpoint_offset(&graph, crate::NodeId(7), crate::NodeId(3), None),
            Point { x: 300.0, y: 220.0 }
        );
    }
    #[test]
    fn equal_score_preserves_positions() {
        let (mut blocks, _) = witness();
        let original = blocks.iter().map(|b| b.position).collect::<Vec<_>>();
        let mut guard = WorkGuard::new(Some(&Background), "ties", 10000).unwrap();
        order_compound_interfaces(&mut blocks, &[], 2, false, &mut guard).unwrap();
        assert_eq!(
            blocks.iter().map(|b| b.position).collect::<Vec<_>>(),
            original
        );
    }
    #[test]
    fn interface_and_pair_work_are_charged() {
        let (blocks, interfaces) = witness();
        let mut guard = WorkGuard::new(Some(&Background), "score", 2).unwrap();
        assert!(matches!(
            compound_interface_score(&blocks, &interfaces, false, &mut guard),
            Err(Error::Limit { limit: 2, .. })
        ));
    }
}
