// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Container ordering and point-to-scope mapping for visibility routing.
//!
//! Deepest-first traversal assigns nodes, ports, and route points to the
//! smallest eligible container while respecting boundary tolerances.

use super::{ArenaGraph, NodeId};

impl ArenaGraph {
    /// Graph.ContainerRDFSOrder's guarded traversal used by NormalizeGaps.
    /// Charge each child reference, including non-container leaves and cluster
    /// members, before deciding whether it contributes to the returned order.
    pub(in crate::internal) fn gap_container_reverse_dfs_order(
        &self,
        guard: &mut crate::internal::limits::WorkGuard<'_>,
    ) -> Result<Vec<NodeId>, crate::internal::limits::Error> {
        fn append_scope(
            graph: &ArenaGraph,
            root: Option<NodeId>,
            order: &mut Vec<NodeId>,
            guard: &mut crate::internal::limits::WorkGuard<'_>,
        ) -> Result<(), crate::internal::limits::Error> {
            if !graph.containers.contains_key(&root) {
                return Ok(());
            }
            for child in graph.container_node_order(root).into_iter().rev() {
                guard.step()?;
                if !graph.active_node_is_aggregate(child)
                    && graph.nodes[child.0 as usize].is_container
                {
                    append_scope(graph, Some(child), order, guard)?;
                    order.push(child);
                } else if let Some(cluster) = graph.active_cluster_index(child) {
                    for member in graph.clusters[cluster].members.iter().rev().copied() {
                        guard.step()?;
                        if graph.nodes[member.0 as usize].is_container {
                            append_scope(graph, Some(member), order, guard)?;
                            order.push(member);
                        }
                    }
                }
            }
            Ok(())
        }
        let mut order = Vec::new();
        append_scope(self, None, &mut order, guard)?;
        Ok(order)
    }

    /// Direct translation of recovered `Graph.containerRDFSOrder`.
    ///
    /// TALA walks each scope's children in reverse graph order, recursively
    /// visits a child container's descendants, and only then appends that
    /// container. The resulting deepest-first order is used by
    /// `OVG.mapNodesToContainer`.
    pub(in crate::internal) fn container_reverse_dfs_order(&self) -> Vec<NodeId> {
        fn append_scope(graph: &ArenaGraph, root: Option<NodeId>, order: &mut Vec<NodeId>) {
            for child in graph.containers.get(&root).into_iter().flatten().rev() {
                if !graph.nodes[child.0 as usize].is_container {
                    continue;
                }
                append_scope(graph, Some(child), order);
                order.push(child);
            }
        }

        let mut order = Vec::new();
        append_scope(self, None, &mut order);
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAlignment, Graph, Insets, LabelPosition, Node, Point, ShapeKind, Size};

    fn node(name: &str, width: f64, height: f64) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size { width, height },
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

    fn nested_arena() -> (ArenaGraph, NodeId, NodeId, NodeId) {
        let mut input = Graph::default();
        let outer = input.add_node(node("outer", 300.0, 300.0));
        let sibling = input.add_node(node("sibling", 80.0, 80.0));
        let inner = input.add_node(node("inner", 100.0, 100.0));
        let leaf = input.add_node(node("leaf", 20.0, 20.0));
        let sibling_leaf = input.add_node(node("sibling.leaf", 20.0, 20.0));
        input.node_mut(sibling).unwrap().parent = Some(outer);
        input.node_mut(inner).unwrap().parent = Some(outer);
        input.node_mut(leaf).unwrap().parent = Some(inner);
        input.node_mut(sibling_leaf).unwrap().parent = Some(sibling);

        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(outer, Point { x: 0.0, y: 0.0 });
        arena.set_position(sibling, Point { x: 20.0, y: 20.0 });
        arena.set_position(inner, Point { x: 150.0, y: 150.0 });
        arena.set_position(leaf, Point { x: 170.0, y: 170.0 });
        arena.set_position(sibling_leaf, Point { x: 30.0, y: 30.0 });
        (arena, outer, sibling, inner)
    }

    #[test]
    fn reverse_dfs_order_is_deepest_first_and_reverses_siblings() {
        let (arena, outer, sibling, inner) = nested_arena();
        assert_eq!(
            arena.container_reverse_dfs_order(),
            vec![inner, sibling, outer]
        );
    }
}
