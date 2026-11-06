// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recognition, carrier projection, placement, and restoration of step sequences.
//!
//! Connected movable steps retain stable member order while a temporary vessel
//! represents their combined geometry in surrounding scopes.

use super::*;

const STEP_OVERLAP: f64 = 35.0;
const MIN_STEP_WIDTH: f64 = 70.0;

/// sequence.go::SequenceAdvance follows the Step shape's narrow-width wedge.
fn sequence_advance(width: f64) -> f64 {
    if width <= 0.0 {
        0.0
    } else if width <= STEP_OVERLAP {
        width - width / 2.0
    } else {
        width - STEP_OVERLAP
    }
}

/// Sequence.resizeWithWork's numerical path, including an empty sequence.
fn sequence_geometry(sizes: impl IntoIterator<Item = Size>) -> Size {
    use super::owner_median::source_math_max;
    let mut size = Size::default();
    let mut offset = 0.0;
    for child in sizes {
        size.width = source_math_max(size.width, offset + source_math_max(0.0, child.width));
        offset += sequence_advance(child.width);
        size.height = source_math_max(size.height, source_math_max(0.0, child.height));
    }
    size
}

include!("sequence_ids.rs");
#[cfg(test)]
thread_local! {
    static COMPACT_VISITS: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
}

#[path = "sequence_compact_sync.rs"]
mod compact_sync;

#[path = "sequence_owner_sync.rs"]
mod owner_sync;

impl ArenaGraph {
    /// Loads pointer-owned sequence state from graphjson. The decoded
    /// sequence vessel is not in `Graph.Nodes`; its member nodes stay active
    /// until AddSequences revalidates and materializes the remembered group.
    pub(super) fn load_serialized_sequences(
        &mut self,
        serialized: &[SerializedSequenceState],
    ) -> bool {
        if serialized.is_empty() {
            return false;
        }
        let by_tala_id = self
            .nodes
            .iter()
            .map(|node| (node.tala_id, node.input_id))
            .collect::<BTreeMap<_, _>>();
        let mut loaded = Vec::new();
        for sequence in serialized {
            if sequence.member_tala_ids.len() < 2 {
                continue;
            }
            let Some(members) = sequence
                .member_tala_ids
                .iter()
                .map(|id| by_tala_id.get(id).copied())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let container = (sequence.container_tala_id != 0)
                .then(|| by_tala_id.get(&sequence.container_tala_id).copied())
                .flatten();
            if let Some(position) = sequence.vessel_position {
                self.serialized_vessel_positions_by_tala
                    .insert(sequence.vessel_tala_id, position);
            }
            if sequence.vessel_size != Size::default() {
                self.serialized_vessel_sizes_by_tala
                    .insert(sequence.vessel_tala_id, sequence.vessel_size);
            }
            loaded.push(SequenceState {
                members,
                vessel_tala_id: sequence.vessel_tala_id,
                container,
                has_edge_abductions: !sequence.edge_abductions.is_empty(),
            });
        }
        if loaded.is_empty() {
            return false;
        }
        self.sequences = loaded;
        self.sequence_backlinks_complete = false;
        true
    }

    /// Restores TALA's `IdentifySequences`/`BuildSequence` preprocessing.
    ///
    /// Each scope is visited in reverse DFS order followed by the root scope.
    /// Within a scope, consecutive movable step nodes form a sequence only
    /// while each adjacent pair has a graph connection. The stable arena keeps
    /// the original member IDs, while placement scopes project them through
    /// the temporary vessel that TALA inserts into `Graph.Nodes`.
    pub(super) fn assign_sequences(&mut self, misc_rng: &mut go_rng::GoRng) {
        let mut occupied_ids = self.sequence_occupied_node_ids();
        let validity = self.owner_remembered_sequence_validity();
        let remembered_headers = self
            .sequences
            .iter()
            .enumerate()
            .filter_map(|(index, sequence)| {
                let group = self.owner_group_binding(TopologyVessel::Sequence(index))?;
                (validity.get(&index) != Some(&false)).then(|| {
                    (
                        (sequence.vessel_tala_id, sequence.members.clone()),
                        self.owner_group_fields(group).unwrap().nodes,
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        let remembered_sequences = std::mem::take(&mut self.sequences)
            .into_iter()
            .enumerate()
            .filter_map(|(index, sequence)| {
                (validity.get(&index) != Some(&false)).then_some(sequence)
            })
            .collect::<Vec<_>>();
        self.begin_owner_group_discovery(super::owner_groups::OwnerGroupKind::Sequence);
        // AddSequences discards each decoded inactive vessel and constructs a
        // fresh one with the remembered ID. Its dimensions and top-left are
        // recomputed from the live steps; a GraphJSON vessel snapshot must not
        // override that geometry later in placement scopes.
        for sequence in &remembered_sequences {
            self.serialized_vessel_positions_by_tala
                .remove(&sequence.vessel_tala_id);
            self.serialized_vessel_sizes_by_tala
                .remove(&sequence.vessel_tala_id);
        }
        for node in &mut self.nodes {
            node.sequence = None;
        }

        let mut scopes = self
            .container_reverse_dfs_order()
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        scopes.push(None);

        for scope in scopes {
            let step_nodes = self
                .containers
                .get(&scope)
                .into_iter()
                .flatten()
                .filter(|node| {
                    let node = &self.nodes[node.0 as usize];
                    !node.is_container
                        && node.fixed_top_left.is_none()
                        && node.shape == ShapeKind::Step
                })
                .collect::<Vec<_>>();
            if step_nodes.len() <= 1 {
                continue;
            }

            // AddSequences receives the complete IdentifySequences result for
            // one scope before BuildSequence starts disconnecting defining
            // edges. Preserve that phase boundary: a disconnection from an
            // earlier sequence must not alter identification later in the
            // same scope.
            let active_nodes = self
                .owner_graph_member_header(OwnerGraphId(0))
                .unwrap()
                .iter_slots()
                .flatten()
                .collect::<BTreeSet<_>>();
            let identified_steps = step_nodes
                .iter()
                .copied()
                .filter(|&node| {
                    if !active_nodes.contains(&node) {
                        return false;
                    }
                    self.owner_node_group_vessel(
                        node,
                        super::owner_groups::OwnerGroupKind::Sequence,
                    )
                    .expect("sequence pointer")
                    .is_none_or(|vessel| {
                        vessel.is_some_and(|vessel| self.topology_node_owner(vessel).is_some())
                    })
                })
                .collect::<Vec<_>>();
            let mut identified = Vec::new();
            if let Some((&first, rest)) = identified_steps.split_first() {
                let mut members = vec![first];
                for &current in rest {
                    let previous = *members.last().expect("sequence has a first step");
                    if self.nodes_connected(previous, current) {
                        members.push(current);
                    } else {
                        if members.len() > 1 {
                            identified.push(std::mem::take(&mut members));
                        }
                        members = vec![current];
                    }
                }
                if members.len() > 1 {
                    identified.push(members);
                }
            }

            // Go AddSequences validates and appends remembered groups after
            // freshly identified sequences. Their members are ordinary live
            // Graph.Nodes after graphjson cleanup, so retain the old vessel
            // identity only when the exact ordered members still occupy one
            // contiguous run of movable steps in the same container.
            let positions = step_nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (*node, index))
                .collect::<BTreeMap<_, _>>();
            for remembered in remembered_sequences
                .iter()
                .filter(|sequence| sequence.container == scope && sequence.members.len() > 1)
            {
                let Some(member_positions) = remembered
                    .members
                    .iter()
                    .map(|member| positions.get(member).copied())
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                if !member_positions
                    .windows(2)
                    .all(|pair| pair[1] == pair[0] + 1)
                {
                    continue;
                }
                if !identified.contains(&remembered.members) {
                    identified.push(remembered.members.clone());
                }
            }
            identified
                .sort_by_key(|members| positions.get(&members[0]).copied().unwrap_or(usize::MAX));
            let remembered_ids = remembered_sequences
                .iter()
                .filter(|sequence| sequence.container == scope)
                .map(|sequence| (sequence.members[0], sequence.vessel_tala_id))
                .collect::<BTreeMap<_, _>>();
            let generated_ids = (0..identified.len())
                .map(|_| misc_rng.int63())
                .collect::<Vec<_>>();
            for (members, vessel_id) in sequence_id_slots(
                identified,
                &generated_ids,
                &remembered_ids,
                &mut occupied_ids,
            ) {
                let remembered = remembered_headers
                    .get(&(vessel_id, members.clone()))
                    .cloned();
                self.commit_sequence(scope, &members, vessel_id, remembered);
            }
        }
        self.sequence_backlinks_complete = true;
        self.rebuild_sequence_node_order();
    }

    fn nodes_connected(&self, first: NodeId, second: NodeId) -> bool {
        self.nodes[first.0 as usize]
            .edges
            .read()
            .iter()
            .any(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                (edge.from == first && edge.to == second)
                    || (edge.from == second && edge.to == first)
            })
    }

    fn commit_sequence(
        &mut self,
        container: Option<NodeId>,
        members: &[NodeId],
        vessel_tala_id: i64,
        remembered: Option<OwnerMemberHeader>,
    ) {
        if members.len() <= 1 {
            return;
        }

        let maximum_height = members
            .iter()
            .map(|member| self.nodes[member.0 as usize].rect.size.height)
            .fold(0.0_f64, f64::max);
        for member in members {
            let size = &mut self.nodes[member.0 as usize].rect.size;
            if size.width <= STEP_OVERLAP {
                size.width = MIN_STEP_WIDTH;
            }
            size.height = maximum_height;
        }

        // BuildSequence disconnects the defining connection between every
        // consecutive pair before constructing or abducting into the vessel.
        // Graph.Disconnect removes the edge from Graph.Edges and both endpoint
        // slices permanently; CleanupStuff never restores it.
        for pair in members.windows(2) {
            if let Some(edge) = self.connection_between(pair[0], pair[1]) {
                self.disconnect_edge(edge);
            }
        }

        let sequence_index = self.sequences.len();
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let has_edge_abductions = self
            .member_edges()
            .any(|edge| member_set.contains(&edge.from) != member_set.contains(&edge.to));
        for member in members {
            self.nodes[member.0 as usize].sequence = Some(sequence_index);
        }
        self.sequences.push(SequenceState {
            members: members.to_vec(),
            vessel_tala_id,
            container,
            has_edge_abductions,
        });
        self.publish_sequence_creation(sequence_index, remembered)
            .expect("valid sequence creation");
    }

    /// Recovered `Node.getConnectionTo`: return the first incident connection
    /// in the node's current edge-slice order, irrespective of direction.
    fn connection_between(&self, first: NodeId, second: NodeId) -> Option<EdgeId> {
        self.nodes[first.0 as usize]
            .edges
            .read()
            .iter()
            .copied()
            .find(|edge_id| {
                let edge = &self.edge_records[edge_id.0 as usize];
                (edge.from == first && edge.to == second)
                    || (edge.from == second && edge.to == first)
            })
    }

    /// layoutgraph.Graph.Disconnect retains the canonical Edge allocation.
    pub(super) fn disconnect_edge(&mut self, removed: EdgeId) {
        let from = self
            .owner_edge_endpoint(removed, false)
            .expect("known edge")
            .expect("runtime error: invalid memory address or nil pointer dereference");
        self.nodes[from.0 as usize].edges.remove_first(removed);
        // Go observes To only after the From removal, including on failure.
        let to = self
            .owner_edge_endpoint(removed, true)
            .expect("known edge")
            .expect("runtime error: invalid memory address or nil pointer dereference");
        self.nodes[to.0 as usize].edges.remove_first(removed);
        // Replace Graph.Edges with make(0,len(old)) plus survivors.
        self.edge_order.disconnect(removed);
    }

    pub(super) fn sequence_vessel_size(&self, sequence_index: usize) -> Size {
        let sequence = &self.sequences[sequence_index];
        if let Some(size) = self
            .serialized_vessel_sizes_by_tala
            .get(&sequence.vessel_tala_id)
            .copied()
            .filter(|size| *size != Size::default())
        {
            return size;
        }
        sequence_geometry(
            sequence
                .members
                .iter()
                .map(|member| self.nodes[member.0 as usize].rect.size),
        )
    }

    /// Member geometry inside TALA's temporary sequence vessel after
    /// `Sequence.sync`.
    pub(super) fn sequence_member_geometry(
        &self,
        sequence_index: usize,
        member: NodeId,
    ) -> Option<(Point, Size)> {
        let sequence = &self.sequences[sequence_index];
        let mut offset = Point::default();
        for candidate in sequence.members.iter().copied() {
            let size = self.nodes[candidate.0 as usize].rect.size;
            if candidate == member {
                return Some((offset, size));
            }
            offset.x += sequence_advance(size.width);
        }
        None
    }

    /// sequence.go::SyncGeometry resizes before testing the vessel's TopLeft,
    /// then replaces each step point using the source Step-shape advance.
    pub(super) fn sync_sequences(&mut self) {
        if self.owner_sequence_sync_ready() {
            self.sync_owner_graph_sequences(shared_topology::OwnerGraphId(0));
            self.sync_external_sequences();
            return;
        }
        self.sync_compact_sequences();
        // Graph.syncSequences owns both current and retained shared-pointer
        // sequences. Keep the projected pass here so transaction commit, gap
        // normalization, and hierarchy post-passes all cross the same source
        // lifecycle boundary.
        self.sync_external_sequences();
    }

    /// Numerical Sequence.SyncGeometry; traversal chooses the reachable group.
    pub(super) fn sync_compact_sequence_geometry(&mut self, sequence_index: usize) {
        #[cfg(test)]
        COMPACT_VISITS.with(|v| {
            if let Some(v) = v.borrow_mut().as_mut() {
                v.push(sequence_index);
            }
        });
        let members = self.sequences[sequence_index].members.clone();
        let vessel_id = self.sequences[sequence_index].vessel_tala_id;
        let size = sequence_geometry(
            members
                .iter()
                .map(|member| self.nodes[member.0 as usize].rect.size),
        );
        // Entity IDs do not establish node identity. Resize a separately
        // bound vessel by its arena handle; an unrelated node may share ID.
        let vessel = self.topology_vessel_node(TopologyVessel::Sequence(sequence_index));
        if let Some(vessel) = vessel {
            self.nodes[vessel.0 as usize].rect.size = size;
        } else {
            self.set_projected_node_size(vessel_id, size);
        }
        self.serialized_vessel_sizes_by_tala.insert(vessel_id, size);
        // A real vessel's nil TopLeft is authoritative. Only the compact
        // representation without a separate vessel uses its member carrier.
        let top_left = if let Some(vessel) = vessel {
            self.position(vessel)
        } else {
            self.serialized_vessel_positions_by_tala
                .get(&vessel_id)
                .copied()
                .or_else(|| members.first().and_then(|&first| self.position(first)))
        };
        let Some(mut top_left) = top_left else {
            return;
        };
        for member in members {
            self.set_position(member, top_left);
            top_left.x += sequence_advance(self.nodes[member.0 as usize].rect.size.width);
        }
    }

    /// Synchronize a sequence whose vessel is live in an induced graph while
    /// its ordered step pointers remain in the external aggregate projection.
    ///
    /// Sequence.SyncGeometry resizes the temporary vessel, then replaces
    /// each step TopLeft using the Step shape's clamped overlap.
    /// Unlike cluster arrangement, `arrangeSteps` does not move descendants.
    pub(super) fn sync_external_sequence(&mut self, vessel_tala_id: i64) {
        let Some(members) = self
            .transaction_external_aggregate_children
            .get(&vessel_tala_id)
            .cloned()
        else {
            return;
        };
        let size = sequence_geometry(members.iter().map(|member| member.rect.size));
        self.set_projected_node_size(vessel_tala_id, size);
        self.serialized_vessel_sizes_by_tala
            .insert(vessel_tala_id, size);

        // Sequence.resize above does not depend on TopLeft. arrangeSteps alone
        // returns for an unpositioned vessel.
        let vessel_top_left = self
            .projected_node_position_by_tala(vessel_tala_id)
            .or_else(|| {
                self.serialized_vessel_positions_by_tala
                    .get(&vessel_tala_id)
                    .copied()
            });
        let Some(vessel_top_left) = vessel_top_left else {
            return;
        };
        let mut top_left = vessel_top_left;
        for member in members {
            self.set_projected_node_position(member.tala_id, top_left);
            top_left.x += sequence_advance(member.rect.size.width);
        }
        self.reconcile_projected_offsets_from_external_cluster(vessel_tala_id);
    }

    /// Projected portion of recovered `Graph.syncSequences`.
    ///
    /// Transaction.Commit walks every current Graph.Nodes entry in reverse DFS
    /// order. That reaches nested sequence vessels retained through container
    /// and cluster pointers even when the induced Rust arena has no live
    /// `SequenceState`. Build the same pointer walk from the projection maps,
    /// then synchronize only reachable sequence keys.
    pub(super) fn sync_external_sequences(&mut self) {
        // A projected sequence vessel has aggregate children.
        if self.transaction_external_aggregate_children.is_empty() {
            return;
        }
        for tala_id in self.projected_reverse_dfs_order() {
            if self.projected_node_is_sequence_vessel(tala_id) {
                self.sync_external_sequence(tala_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAlignment, Insets, LabelPosition, Node};

    fn step(name: &str, width: f64, height: f64) -> Node {
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
            shape: ShapeKind::Step,
        }
    }

    #[test]
    fn connected_steps_are_replaced_by_one_recovered_sequence_vessel() {
        let mut input = Graph::default();
        let first = input.add_node(step("first", 120.0, 80.0));
        let second = input.add_node(step("second", 20.0, 100.0));
        let third = input.add_node(step("third", 140.0, 90.0));
        input.add_edge(crate::Edge {
            source: first,
            target: second,
        });
        input.add_edge(crate::Edge {
            source: second,
            target: third,
        });
        let mut outside_node = step("outside", 100.0, 80.0);
        outside_node.shape = ShapeKind::Rectangle;
        let outside = input.add_node(outside_node);
        let external_edge = input.add_edge(crate::Edge {
            source: third,
            target: outside,
        });

        let mut pipeline = Pipeline::new(&input, 1, false, false);
        pipeline.run_preprocess_sequences();

        assert_eq!(pipeline.graph.sequences.len(), 1);
        assert_eq!(
            pipeline.graph.sequences[0].members,
            vec![first, second, third]
        );
        assert_eq!(
            pipeline.graph.sequence_vessel_size(0),
            Size {
                width: 260.0,
                height: 100.0,
            }
        );
        assert_eq!(
            pipeline.graph.sequence_member_geometry(0, first),
            Some((
                Point { x: 0.0, y: 0.0 },
                Size {
                    width: 120.0,
                    height: 100.0,
                },
            ))
        );
        assert_eq!(
            pipeline.graph.sequence_member_geometry(0, second),
            Some((
                Point { x: 85.0, y: 0.0 },
                Size {
                    width: 70.0,
                    height: 100.0,
                },
            ))
        );
        assert_eq!(
            pipeline.graph.sequence_member_geometry(0, third),
            Some((
                Point { x: 120.0, y: 0.0 },
                Size {
                    width: 140.0,
                    height: 100.0,
                },
            ))
        );
        let placement = pipeline
            .placement_scope_graph(None)
            .expect("root placement scope");
        assert_eq!(placement.graph.nodes().count(), 2);

        assert_eq!(pipeline.graph.edge_records.len(), 3);
        assert_eq!(pipeline.graph.edge_order.read(), vec![external_edge]);
        assert_eq!(
            pipeline.graph.edge_records[external_edge.0 as usize].input_id,
            external_edge
        );
        let vessel = pipeline
            .graph
            .topology_vessel_node(TopologyVessel::Sequence(0))
            .unwrap();
        assert_ne!(vessel, first);
        assert_eq!(
            pipeline.graph.edge_records[external_edge.0 as usize].from,
            vessel
        );
        assert_eq!(
            pipeline.graph.edge_records[external_edge.0 as usize].to,
            outside
        );
        assert_eq!(
            pipeline.graph.nodes[vessel.0 as usize].edges,
            vec![external_edge]
        );
        assert_eq!(
            pipeline.graph.nodes[outside.0 as usize].edges,
            vec![external_edge]
        );
        assert!(pipeline.graph.nodes[first.0 as usize].edges.is_empty());
        assert!(pipeline.graph.nodes[second.0 as usize].edges.is_empty());
        assert!(pipeline.graph.nodes[third.0 as usize].edges.is_empty());
    }

    #[test]
    fn disconnected_consecutive_steps_do_not_form_a_sequence() {
        let mut input = Graph::default();
        for name in ["first", "second", "third"] {
            input.add_node(step(name, 100.0, 80.0));
        }

        let mut pipeline = Pipeline::new(&input, 1, false, false);
        pipeline.run_preprocess_sequences();

        assert!(pipeline.graph.sequences.is_empty());
    }
}
