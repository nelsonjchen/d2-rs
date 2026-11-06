// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recursive container packing and already-materialized grid detection.
//!
//! This layer decides when child scopes must be combined and when a grid has
//! already published geometry that later stages must preserve.

use super::*;

impl ArenaGraph {
    /// Surface on which pristine `Graph.placeNodes` recursively materializes
    /// grid cells and feeds the resulting child containers to
    /// `CombineSubgraphs`; the later pipeline does not run a second grid
    /// layout over those already-positioned children.
    pub(super) fn has_recursive_grid_container_scope(&self) -> bool {
        self.edge_order.is_empty()
            && self.nodes.iter().any(|container| {
                container.is_container
                    && (container.grid_rows.is_some() || container.grid_columns.is_some())
                    && self
                        .containers
                        .get(&Some(container.input_id))
                        .is_some_and(|children| {
                            !children.is_empty()
                                && children
                                    .iter()
                                    .all(|child| self.nodes[child.0 as usize].is_container)
                        })
            })
    }

    pub(super) fn is_recursive_grid_container(&self, container: NodeId) -> bool {
        let node = &self.nodes[container.0 as usize];
        self.edge_order.is_empty()
            && (node.grid_rows.is_some() || node.grid_columns.is_some())
            && self
                .containers
                .get(&Some(container))
                .is_some_and(|children| {
                    !children.is_empty()
                        && children
                            .iter()
                            .all(|child| self.nodes[child.0 as usize].is_container)
                })
    }

    pub(super) fn is_root_grid_only(&self) -> bool {
        self.edge_order.is_empty()
            && self.containers.get(&None).is_some_and(|roots| {
                // A single root grid is the explicit grid diagram surface.
                // When several grid containers are siblings, D2's plugin
                // receives them as ordinary disconnected TALA components;
                // treating the whole root as one grid changes their shelf
                // orientation and applies a second cell pack.
                roots.len() == 1
                    && roots.iter().all(|root| {
                        let node = &self.nodes[root.0 as usize];
                        node.is_container
                            && (node.grid_rows.is_some() || node.grid_columns.is_some())
                    })
            })
    }

    /// Recovered recursive-grid branch of `Graph.BinPack`. Its candidate
    /// transaction and score remain wholly on the recovered BinPack surface.
    pub(super) fn bin_pack_root_grid(&mut self) -> Result<(), limits::Error> {
        if self.root_grid_bin_pack_completed {
            return Ok(());
        }
        self.combine_placement_components(true, true, false)?;
        let context = binpack_guard::PackingContext;
        let mut guard =
            limits::WorkGuard::new(Some(&context), "BinPack", limits::MAX_ENGINE_WORK_UNITS)?;
        guard.set_limit(binpack_guard::MAX_BIN_PACK_WORK_UNITS);
        self.bin_pack_atomic_with_guard(&mut guard, true)?;
        self.root_grid_bin_pack_completed = true;
        Ok(())
    }
}
