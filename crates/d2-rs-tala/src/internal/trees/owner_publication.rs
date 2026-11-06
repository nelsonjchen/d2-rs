// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use crate::internal::owner_tree_map::{OwnerTreeFields, OwnerTreeSlice};

impl ArenaGraph {
    /// Publish the numerical placement view's field assignment on the actual
    /// Tree allocation. Source setOrientation and mirrorAxes mutate the same
    /// records later consumed by routing; an absent/nil map entry stays absent.
    pub(super) fn publish_tree_orientation(&self, node: NodeId, orientation: Orientation) {
        if !self.shared_topology_initialized() {
            return;
        }
        if let Some(Some(tree)) = self
            .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(node))
            .expect("tree orientation graph")
        {
            let tree = self.owner_tree(tree).expect("tree orientation record");
            let mut fields = tree.fields();
            fields.orientation = Self::herd_orientation_raw(orientation);
            tree.set_fields(fields);
        }
    }
    /// Publish the surviving extraction records at buildNodeToTreeGuarded's
    /// producer boundary (trees/preprocess_helpers.go:651–703). Consumers of
    /// NodeToTree must observe this map, including an explicitly replaced map,
    /// rather than falling back to the numerical routing projection.
    pub(super) fn publish_preprocessed_owner_trees(&mut self) {
        let mut nodes = self.tree_routing_nodes.keys().copied().collect::<Vec<_>>();
        nodes.sort_by_key(|node| self.tree_routing_nodes[node].extraction_order);
        let trees = nodes
            .iter()
            .map(|&node| {
                (
                    node,
                    self.new_owner_tree(Some(node))
                        .expect("extracted Tree allocation"),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut roots = BTreeMap::<NodeId, Vec<_>>::new();
        let index = self.allocate_owner_node_to_tree_map(true);
        for &node in &nodes {
            let state = self.tree_routing_nodes[&node];
            let tree = trees[&node];
            let parent = trees.get(&state.parent).copied();
            let mut children = OwnerTreeSlice::allocated_empty();
            for child in self.tree_children_in_extraction_order(node) {
                children = children.append_tree(trees[&child]);
            }
            self.set_owner_tree_fields(
                tree,
                OwnerTreeFields {
                    node: Some(node),
                    parent,
                    children,
                    sentinel_edge: Some(state.sentinel_edge),
                    // None marks an unplaced numerical projection. NewTree's
                    // actual field starts at Go zero, while an explicitly
                    // oriented projection retains that supplied value.
                    orientation: match state.orientation {
                        Orientation::None => 0,
                        orientation => Self::herd_orientation_raw(orientation),
                    },
                },
            )
            .expect("extracted Tree fields");
            self.write_owner_node_to_tree_entry(&index, Some(node), Some(tree))
                .expect("extracted NodeToTree entry");
            if parent.is_none() {
                roots.entry(state.parent).or_default().push(Some(tree));
            }
        }
        let roots_map = self.allocate_owner_tree_roots_map(true);
        for (sentinel, roots) in roots {
            let header = self
                .allocate_owner_tree_roots_header(&roots, roots.len(), true)
                .expect("extracted root header");
            self.write_owner_tree_roots_entry(&roots_map, Some(sentinel), &header)
                .expect("extracted root entry");
        }
        self.set_owner_graph_tree_roots_map(OwnerGraphId(0), roots_map)
            .expect("extracted Trees publication");
        self.set_owner_graph_node_to_tree_map(OwnerGraphId(0), index)
            .expect("extracted NodeToTree publication");
    }
}
