// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Stable node and edge order views across hierarchy and aggregate rewrites.
//!
//! Dense arena identity stays fixed while these slices reproduce observable
//! insertion, removal, restoration, and endpoint-local ordering.

use super::*;

impl ArenaGraph {
    pub(super) fn invalidate_node_order_membership(&mut self) {
        self.node_order_membership_valid = false;
    }

    pub(super) fn refresh_node_order_membership(&mut self) {
        if self.node_order_membership.len() != self.nodes.len() {
            self.node_order_membership.resize(self.nodes.len(), false);
        } else {
            self.node_order_membership.fill(false);
        }
        for node in self.node_order.iter().copied() {
            if let Some(member) = self.node_order_membership.get_mut(node.0 as usize) {
                *member = true;
            }
        }
        self.node_order_membership_valid = true;
        self.active_sequence_flags = self
            .sequences
            .iter()
            .map(|sequence| {
                sequence.members.len() > 1
                    && sequence
                        .members
                        .iter()
                        .skip(1)
                        .any(|member| !self.node_order_contains(*member))
            })
            .collect();
        self.active_cluster_flags = self
            .clusters
            .iter()
            .map(|cluster| {
                cluster.members.len() > 1
                    && cluster.members.first().is_some_and(|owner| {
                        self.node_order_contains(*owner)
                            && cluster
                                .members
                                .iter()
                                .skip(1)
                                .any(|member| !self.node_order_contains(*member))
                    })
            })
            .collect();
        let bound_sequence_indices = self
            .sequences
            .iter()
            .enumerate()
            .filter(|(index, _)| self.active_sequence_flags[*index])
            .filter_map(|(index, _)| {
                self.topology_vessel_node(TopologyVessel::Sequence(index))
                    .filter(|vessel| !self.node_order_contains(*vessel))
                    .map(|vessel| (vessel, index))
            })
            .collect::<HashMap<_, _>>();
        self.active_sequence_indices = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                bound_sequence_indices
                    .get(&NodeId(index as u32))
                    .copied()
                    .or(node.sequence)
                    .filter(|&sequence_index| {
                        self.active_sequence_flags
                            .get(sequence_index)
                            .copied()
                            .unwrap_or(false)
                    })
            })
            .collect();
        self.active_sequence_owners = (0..self.nodes.len())
            .map(|index| {
                let node = NodeId(index as u32);
                self.active_sequence_indices[index]
                    .and_then(|sequence_index| self.sequences[sequence_index].members.first())
                    .copied()
                    .unwrap_or(node)
            })
            .collect();
        let bound_cluster_indices = self
            .clusters
            .iter()
            .enumerate()
            .filter(|(index, _)| self.active_cluster_flags[*index])
            .filter_map(|(index, _)| {
                self.topology_vessel_node(TopologyVessel::Cluster(index))
                    .map(|vessel| (vessel, index))
            })
            .fold(HashMap::new(), |mut indices, (vessel, index)| {
                indices.entry(vessel).or_insert(index);
                indices
            });
        self.active_cluster_indices = self
            .active_sequence_owners
            .iter()
            .map(|sequence_owner| {
                bound_cluster_indices
                    .get(sequence_owner)
                    .copied()
                    .or(self.nodes[sequence_owner.0 as usize].cluster)
                    .filter(|&cluster_index| {
                        self.active_cluster_flags
                            .get(cluster_index)
                            .copied()
                            .unwrap_or(false)
                    })
            })
            .collect();
        self.active_cluster_owners = self
            .active_sequence_owners
            .iter()
            .copied()
            .enumerate()
            .map(|(index, sequence_owner)| {
                self.active_cluster_indices[index]
                    .and_then(|cluster_index| self.clusters[cluster_index].members.first())
                    .copied()
                    .unwrap_or(sequence_owner)
            })
            .collect();
        self.active_node_containers = Arc::new(
            (0..self.nodes.len())
                .map(|index| {
                    if let Some(cluster_index) = self.active_cluster_indices[index] {
                        let first = *self.clusters[cluster_index].members.first()?;
                        if let Some(sequence_index) = self.active_sequence_indices[first.0 as usize]
                        {
                            return self.sequences[sequence_index].container;
                        }
                        if let Some(container_tala_id) = self
                            .serialized_cluster_containers_by_tala
                            .get(&self.clusters[cluster_index].vessel_tala_id)
                        {
                            return container_tala_id.and_then(|container_tala_id| {
                                self.nodes
                                    .iter()
                                    .find(|candidate| candidate.tala_id == container_tala_id)
                                    .map(|candidate| candidate.input_id)
                            });
                        }
                        return self.nodes[first.0 as usize].container;
                    }
                    if let Some(sequence_index) = self.active_sequence_indices[index] {
                        return self.sequences[sequence_index].container;
                    }
                    self.nodes[index].container
                })
                .collect(),
        );
        let mut aggregate_vessels_by_member = HashMap::with_capacity(
            self.transaction_external_aggregate_children
                .values()
                .map(Vec::len)
                .sum(),
        );
        for (&vessel_tala_id, members) in &self.transaction_external_aggregate_children {
            for member in members {
                // The uncached lookup walks the BTreeMap in key order and
                // returns the first containing vessel. Retain that exact
                // choice if malformed/projected state repeats one member.
                aggregate_vessels_by_member
                    .entry(member.tala_id)
                    .or_insert(vessel_tala_id);
            }
        }
        self.active_aggregate_vessels_by_member = Arc::new(aggregate_vessels_by_member);
        let mut aggregate_vessel_nodes = HashMap::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            if node.scoring_is_aggregate_vessel && node.position.is_some() {
                aggregate_vessel_nodes
                    .entry(node.tala_id)
                    .or_insert(NodeId(index as u32));
            }
        }
        self.active_aggregate_vessel_nodes = Arc::new(aggregate_vessel_nodes);
        // Lookup-only (never iterated), so presizing cannot reach output.
        let mut tala_carriers: crate::internal::IdHashMap<i64, (NodeId, bool)> =
            std::collections::HashMap::with_capacity_and_hasher(
                self.node_order.len(),
                Default::default(),
            );
        for &node in &self.node_order {
            let carrier = tala_carriers
                .entry(self.nodes[node.0 as usize].tala_id)
                .or_insert((node, false));
            carrier.1 |= carrier.0 != node;
        }
        self.node_order_tala_carriers = Some(Arc::new(tala_carriers));
        self.active_graph_node_order = self
            .node_order
            .iter()
            .copied()
            .filter(|node| self.active_aggregate_owner(*node) == *node)
            .collect();
    }

    pub(super) fn numerical_aggregate_node(&self, node: NodeId) -> NodeId {
        // Both graph_node_order and its indexed view filter node_order by
        // active_aggregate_owner(node) == node. Test just this candidate even
        // when the index is invalid: constructing the entire visible slice
        // here repeats every node's group resolution for each container child.
        // This callback-free lookup observes the same live topology throughout.
        let visible = |candidate| {
            self.node_order_contains(candidate)
                && self.active_aggregate_owner(candidate) == candidate
        };
        if visible(node) {
            return node;
        }
        // Retained owner headers contain the allocated vessel pointer. A
        // numerical graph can still expose its first-member carrier instead.
        // Keep fitting, movement and containment in that same explicit view;
        // this lookup does not mutate the retained owner header.
        let carrier = match self.topology_node_vessel(node) {
            Some(TopologyVessel::Cluster(index)) => self.clusters[index].members.first(),
            Some(TopologyVessel::Sequence(index)) => self.sequences[index].members.first(),
            None => None,
        };
        carrier
            .copied()
            .filter(|&carrier| visible(carrier))
            .unwrap_or(node)
    }

    pub(super) fn container_node_order(&self, scope: Option<NodeId>) -> Vec<NodeId> {
        let trace_container_order =
            crate::internal::trace_env_enabled("TALA_TRACE_CONTAINER_ORDER");
        let sequence_owners = self
            .sequences
            .iter()
            .filter(|sequence| {
                sequence.container == scope
                    && sequence.members.len() > 1
                    && sequence
                        .members
                        .iter()
                        .skip(1)
                        .any(|member| !self.node_order_contains(*member))
            })
            .filter_map(|sequence| sequence.members.first().copied())
            .collect::<Vec<_>>();
        let cluster_owners = self
            .clusters
            .iter()
            .filter(|cluster| {
                cluster.members.len() > 1
                    && cluster
                        .members
                        .iter()
                        .all(|member| self.nodes[member.0 as usize].container == scope)
                    && cluster
                        .members
                        .iter()
                        .skip(1)
                        .any(|member| !self.node_order_contains(*member))
            })
            .filter_map(|cluster| cluster.members.first().copied())
            .collect::<Vec<_>>();

        if sequence_owners.is_empty() && cluster_owners.is_empty() {
            // Graph.placeNodes and the Go transaction walk consume the
            // existing Graph.Containers slice.  Keep that insertion order
            // here; rebuilding it from node_order can differ after a nested
            // scope has been flattened because the arena's dense NodeId order
            // is independent of the shared pointer slice.
            if !trace_container_order {
                // The same filter and mapping in one pass, with no
                // intermediate copy.
                return self
                    .containers
                    .get(&scope)
                    .into_iter()
                    .flatten()
                    .filter(|candidate| self.nodes[candidate.0 as usize].container == scope)
                    .map(|node| self.numerical_aggregate_node(node))
                    .collect();
            }
            let ordered = self
                .containers
                .get(&scope)
                .into_iter()
                .flatten()
                .filter(|candidate| self.nodes[candidate.0 as usize].container == scope)
                .collect::<Vec<_>>();
            if trace_container_order {
                eprintln!(
                    "CONTAINER_ORDER_RUST scope={:?} source=node-order containers={:?} nodeOrder={:?} rawContainers={:?}",
                    scope.map(|node| self.nodes[node.0 as usize].tala_id),
                    ordered
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.containers
                        .get(&scope)
                        .into_iter()
                        .flatten()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    self.containers.get(&scope).map(|children| {
                        children
                            .iter()
                            .map(|node| self.nodes[node.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    }),
                );
            }
            return ordered
                .into_iter()
                .map(|node| self.numerical_aggregate_node(node))
                .collect();
        }

        let mut active_sequences = vec![false; self.nodes.len()];
        for owner in sequence_owners.iter().copied() {
            active_sequences[owner.0 as usize] = true;
        }
        let mut active_clusters = vec![false; self.nodes.len()];
        for owner in cluster_owners.iter().copied() {
            active_clusters[owner.0 as usize] = true;
        }
        let mut absorbed = vec![false; self.nodes.len()];
        for sequence in &self.sequences {
            if sequence
                .members
                .first()
                .is_some_and(|owner| active_sequences[owner.0 as usize])
            {
                for member in sequence.members.iter().copied() {
                    absorbed[member.0 as usize] = true;
                }
            }
        }
        for cluster in &self.clusters {
            if cluster
                .members
                .first()
                .is_some_and(|owner| active_clusters[owner.0 as usize])
            {
                for member in cluster.members.iter().copied() {
                    absorbed[member.0 as usize] = true;
                }
            }
        }

        let mut ordered = self
            .containers
            .get(&scope)
            .into_iter()
            .flatten()
            .filter(|candidate| !absorbed[candidate.0 as usize])
            .collect::<Vec<_>>();
        // AddSequences runs before AddClusters. Each operation removes its
        // members from Graph.Containers[scope] and appends the new vessel, so
        // this container order is distinct from SplitSubgraphs' Graph.Nodes
        // connectivity order.
        for owner in sequence_owners {
            // Once the canonical Sequence.Vessel is published in the retained
            // container slice, its first-member numerical carrier must not be
            // appended as a second node. Go retains only the vessel pointer.
            let published = self
                .sequences
                .iter()
                .enumerate()
                .filter(|(_, group)| group.members.first() == Some(&owner))
                .filter_map(|(index, _)| self.topology_vessel_node(TopologyVessel::Sequence(index)))
                .any(|vessel| ordered.contains(&vessel));
            if !published {
                ordered.push(owner);
            }
        }
        for owner in cluster_owners {
            // AddCluster removes every member from Containers and retains
            // only its allocated vessel. Once that pointer is published,
            // the numerical first-member carrier is no longer another child.
            let published = self
                .clusters
                .iter()
                .enumerate()
                .filter(|(_, group)| group.members.first() == Some(&owner))
                .filter_map(|(index, _)| self.topology_vessel_node(TopologyVessel::Cluster(index)))
                .any(|vessel| ordered.contains(&vessel));
            if !published {
                ordered.push(owner);
            }
        }
        if trace_container_order {
            eprintln!(
                "CONTAINER_ORDER_RUST scope={:?} source=containers+owners result={:?} containers={:?} nodeOrder={:?}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                ordered
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.containers
                    .get(&scope)
                    .into_iter()
                    .flatten()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
            );
        }
        ordered
            .into_iter()
            .map(|node| self.numerical_aggregate_node(node))
            .collect()
    }

    /// Returns the Graph.Containers order that `Node.edgeLength` observes
    /// after the preprocessing mutations. This is intentionally separate
    /// from the general child traversal order above: AddSequences first
    /// removes sequence members and appends a sequence vessel, then
    /// AddClusters removes cluster members and appends cluster vessels. The
    /// obstruction scan is the one release path where that temporal order is
    /// observable; other layout traversals continue to use
    /// `container_node_order`'s active-arena view.
    fn obstruction_container_node_order(&self, scope: Option<NodeId>) -> Vec<NodeId> {
        self.obstruction_container_node_order_with_materialized(scope, false)
    }

    pub(super) fn obstruction_container_node_order_with_materialized(
        &self,
        scope: Option<NodeId>,
        allow_materialized: bool,
    ) -> Vec<NodeId> {
        let cluster_owners = self
            .clusters
            .iter()
            .filter(|cluster| {
                cluster.members.len() > 1
                    && cluster
                        .members
                        .iter()
                        .all(|member| self.nodes[member.0 as usize].container == scope)
                    && cluster
                        .members
                        .iter()
                        .skip(1)
                        .any(|member| !self.node_order_contains(*member))
            })
            .filter_map(|cluster| cluster.members.first().copied())
            .collect::<Vec<_>>();
        let Some(preprocessed) = self.preprocessed_tree_children.get(&scope) else {
            // Node.edgeLength walks Graph.Containers, whose insertion order is
            // distinct from the connectivity-oriented Graph.Nodes order once
            // the preprocessing passes have restored ordinary children. Keep
            // that source order for the unaggregated case. Aggregate scopes
            // still use the lifecycle-aware general helper below.
            let active_sequence = self.sequences.iter().any(|sequence| {
                sequence.container == scope
                    && sequence.members.len() > 1
                    && sequence
                        .members
                        .iter()
                        .skip(1)
                        .any(|member| !self.node_order_contains(*member))
            });
            if !active_sequence && cluster_owners.is_empty() {
                if crate::internal::trace_env_enabled("TALA_TRACE_OBSTRUCTION_ORDER") {
                    eprintln!(
                        "OBSTRUCTION_ORDER_RUST scope={:?} source=raw current={:?}",
                        scope.map(|node| self.nodes[node.0 as usize].tala_id),
                        self.containers.get(&scope).map(|children| {
                            children
                                .iter()
                                .map(|node| self.nodes[node.0 as usize].tala_id)
                                .collect::<Vec<_>>()
                        }),
                    );
                }
                return self
                    .containers
                    .get(&scope)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|candidate| self.nodes[candidate.0 as usize].container == scope)
                    .collect();
            }
            return self.container_node_order(scope);
        };

        if crate::internal::trace_env_enabled("TALA_TRACE_OBSTRUCTION_ORDER") {
            eprintln!(
                "OBSTRUCTION_ORDER_RUST scope={:?} preprocessed={:?} current={:?} clusterOwners={:?} allowMaterialized={}",
                scope.map(|node| self.nodes[node.0 as usize].tala_id),
                preprocessed
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.containers.get(&scope).map(|children| {
                    children
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                }),
                cluster_owners
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                allow_materialized,
            );
        }

        // With no active cluster owner in this scope, the existing
        // preprocessed child order is already the complete result. Avoid the
        // temporary membership set and filtering pass on every edge-length
        // route probe; the materialized append path below remains unchanged.
        if cluster_owners.is_empty()
            && (!allow_materialized
                || self
                    .containers
                    .get(&scope)
                    .is_none_or(|current| current.len() <= preprocessed.len()))
        {
            return preprocessed.clone();
        }

        let active_cluster_members = self
            .clusters
            .iter()
            .filter(|cluster| {
                cluster
                    .members
                    .first()
                    .is_some_and(|owner| cluster_owners.contains(owner))
            })
            .flat_map(|cluster| cluster.members.iter().copied())
            .collect::<Vec<_>>();
        // Graph.Nodes may still contain only an active sequence vessel after
        // a nested scope has materialized the sequence's original pointers
        // back into the shared Graph.Containers slice. Do not reinterpret
        // those concrete children through `container_node_order`: recovered
        // Node.edgeLength consumes Graph.Containers directly, including all
        // of the materialized steps in their existing order.
        let mut ordered = preprocessed
            .iter()
            .copied()
            .filter(|candidate| !active_cluster_members.contains(candidate))
            .chain(cluster_owners)
            .collect::<Vec<_>>();
        // A parent scope is projected before its nested child scopes are
        // fitted, so its preprocessed tree slice initially contains only the
        // surviving tree root. Once the nested `PlaceTrees` pass has run,
        // Graph.Containers grows to the root plus its retained descendants.
        // The stable arena's live slice still contains sequence and cluster
        // members that TALA replaced with aggregate vessels, so preserve the
        // aggregate-aware order above and append only genuinely restored
        // current children.
        if allow_materialized
            && let Some(current) = self.containers.get(&scope)
            && current.len() > preprocessed.len()
        {
            let mut present = ordered.clone();
            for candidate in current.iter().copied() {
                if present.contains(&candidate)
                    || self.active_aggregate_owner(candidate) != candidate
                {
                    continue;
                }
                present.push(candidate);
                ordered.push(candidate);
            }
        }
        ordered
    }

    /// Returns the current recovered `Graph.Nodes` ownership order.
    pub(super) fn graph_node_order(&self) -> Vec<NodeId> {
        // TALA removes sequence/cluster members from Graph.Nodes while their
        // aggregate vessel is active. Keep the stable arena entries allocated
        // for cleanup, but expose only the live owner for current-graph
        // traversals and transaction validation.
        if self.node_order_membership_valid {
            return self.active_graph_node_order.clone();
        }
        self.node_order
            .iter()
            .copied()
            .filter(|node| self.active_aggregate_owner(*node) == *node)
            .collect()
    }

    /// Reconstructs `Graph.getAllDescendantNodes(root, false)` while
    /// aggregate vessels are active. The vessel itself is present in an
    /// ordinary container's child slice, but its hidden cluster members or
    /// sequence steps are not returned; only descendants nested beneath those
    /// hidden members remain observable.
    pub(super) fn active_descendants(&self, root: NodeId) -> Vec<NodeId> {
        fn current_children(graph: &ArenaGraph, parent: NodeId) -> Vec<NodeId> {
            let ordered = graph.container_node_order(Some(parent));
            if !ordered.is_empty() {
                return ordered;
            }
            let retained = graph.containers.get(&Some(parent)).unwrap_or_default();
            if !retained.is_empty() {
                return retained;
            }
            // Split placement scopes retain shared Go child pointers in the
            // transaction projection after the induced arena's live
            // Containers map becomes sparse. Candidate moves still traverse
            // those pointers, so recover their stable IDs by TALA identity.
            let external = graph
                .transaction_external_container_children
                .get(&graph.nodes[parent.0 as usize].tala_id)
                .into_iter()
                .flatten()
                .filter_map(|child| {
                    graph
                        .nodes
                        .iter()
                        .position(|candidate| candidate.tala_id == child.tala_id)
                        .map(|index| NodeId(index as u32))
                })
                .collect::<Vec<_>>();
            if !external.is_empty() {
                return external;
            }
            graph
                .source_container_child_order
                .get(&graph.nodes[parent.0 as usize].tala_id)
                .into_iter()
                .flatten()
                .filter_map(|child_tala_id| {
                    graph
                        .nodes
                        .iter()
                        .position(|candidate| candidate.tala_id == *child_tala_id)
                        .map(|index| NodeId(index as u32))
                })
                .collect()
        }

        fn append_hidden_member_descendants(
            graph: &ArenaGraph,
            member: NodeId,
            output: &mut Vec<NodeId>,
        ) {
            for child in current_children(graph, member) {
                output.push(child);
                append_current_descendants(graph, child, output);
            }
        }

        fn append_current_descendants(graph: &ArenaGraph, node: NodeId, output: &mut Vec<NodeId>) {
            if let Some(cluster_index) = graph.active_cluster_index(node)
                && graph.clusters[cluster_index].members.first() == Some(&node)
            {
                for member in graph.clusters[cluster_index].members.iter().copied() {
                    if let Some(sequence_index) = graph.active_sequence_index(member)
                        && graph.sequences[sequence_index].members.first() == Some(&member)
                    {
                        for step in graph.sequences[sequence_index].members.iter().copied() {
                            append_hidden_member_descendants(graph, step, output);
                        }
                    } else {
                        append_hidden_member_descendants(graph, member, output);
                    }
                }
            }
            if let Some(sequence_index) = graph.active_sequence_index(node)
                && graph.sequences[sequence_index].members.first() == Some(&node)
                && graph.active_cluster_owner(node) == node
            {
                for step in graph.sequences[sequence_index].members.iter().copied() {
                    append_hidden_member_descendants(graph, step, output);
                }
            }
            // `getAllDescendantNodes` only follows Graph.Containers when the
            // receiver is an actual Go container. A recovered placement
            // scope can retain a child slice for an ordinary node (for
            // example, a cluster vessel carried by a non-container root),
            // but that slice is not part of the Go move closure. Respect the
            // explicit container marker so candidate moves do not translate
            // hidden vessels that the oracle leaves in place.
            // Real container nodes retain Go's `Node.isContainer` bit even
            // when a temporary placement projection clears the separate
            // scoring marker.  The shared-pointer implementation therefore
            // still walks their nested children during a move.  Synthetic
            // aggregate vessels have `is_container == false` and continue
            // to rely on the scoring marker above.
            if graph.nodes[node.0 as usize].is_container {
                for child in current_children(graph, node) {
                    output.push(child);
                    append_current_descendants(graph, child, output);
                }
            }
        }

        let mut descendants = Vec::new();
        // Go's `AllDescendantNodes` also walks synthetic aggregate vessels.
        // Their child pointers live in the transaction projection rather than
        // in `Containers`; preserve that closure for CombineSubgraphs and
        // other pointer-move callers.
        if let Some(members) = self
            .transaction_external_aggregate_children
            .get(&self.nodes[root.0 as usize].tala_id)
        {
            for member in members {
                let Some(member_id) = self
                    .nodes
                    .iter()
                    .position(|candidate| candidate.tala_id == member.tala_id)
                    .map(|index| NodeId(index as u32))
                else {
                    continue;
                };
                descendants.push(member_id);
                append_current_descendants(self, member_id, &mut descendants);
            }
        }
        if self.nodes[root.0 as usize].is_container
            || self.active_cluster_index(root).is_some()
            || self.active_sequence_index(root).is_some()
            || self
                .transaction_external_container_children
                .contains_key(&self.nodes[root.0 as usize].tala_id)
        {
            for child in current_children(self, root) {
                descendants.push(child);
                append_current_descendants(self, child, &mut descendants);
            }
        }

        // `Graph.placeNodes` keeps shared container pointers in the owner
        // graph even when SplitSubgraphs leaves the induced arena's live
        // Containers map sparse.  The recovered Go
        // `getAllDescendantNodes(container, true)` still follows those
        // pointers during optimizer moves.  Preserve the same walk from the
        // container-ancestor metadata retained on every projected node.
        if self.nodes[root.0 as usize].scoring_is_container {
            let root_tala_id = self.nodes[root.0 as usize].tala_id;
            let mut seen = descendants.iter().copied().collect::<BTreeSet<_>>();
            for (index, candidate) in self.nodes.iter().enumerate() {
                let candidate_id = NodeId(index as u32);
                if candidate_id != root
                    && candidate
                        .scoring_container_ancestors
                        .contains(&root_tala_id)
                    && seen.insert(candidate_id)
                {
                    descendants.push(candidate_id);
                }
            }
        }
        descendants
    }

    /// Reconstructs the hierarchy-preorder slice created by
    /// `d2transpiler.PopulateNodes`.
    pub(super) fn hierarchy_node_order(&self) -> Vec<NodeId> {
        fn append_scope(
            graph: &ArenaGraph,
            scope: Option<NodeId>,
            seen: &mut BTreeSet<NodeId>,
            ordered: &mut Vec<NodeId>,
        ) {
            for node in graph.containers.get(&scope).into_iter().flatten() {
                if !seen.insert(node) {
                    continue;
                }
                ordered.push(node);
                append_scope(graph, Some(node), seen, ordered);
            }
        }

        let mut seen = BTreeSet::new();
        let mut ordered = Vec::with_capacity(self.nodes.len());
        append_scope(self, None, &mut seen, &mut ordered);

        // Preserve malformed/orphaned input deterministically. Valid D2 graphs
        // exhaust the arena through the root scope above.
        for node in self.nodes.iter().map(|node| node.input_id) {
            if seen.insert(node) {
                ordered.push(node);
                append_scope(self, Some(node), &mut seen, &mut ordered);
            }
        }
        ordered
    }

    /// Applies the vessel insertion lifecycle used by `AddClusters` to the
    /// stable-ID arena's observable node slice. Sequence placement is already
    /// finalized by `PreprocessTrees` before clusters are added.
    pub(super) fn rebuild_active_aggregate_node_order(&mut self) {
        let mut order = self.node_order.clone();
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "REBUILD_ORDER_RUST before={:?} clusters={:?}",
                order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                self.clusters
                    .iter()
                    .map(|cluster| cluster
                        .members
                        .iter()
                        .map(|node| self.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );
        }
        // Sequence insertion happens in its own preprocessing stage.  The
        // tree stage then physically removes/re-appends the sequence vessel,
        // establishing its current Graph.Nodes position.  AddClusters runs
        // afterward and must preserve that sequence position while replacing
        // only each cluster's members with its newly appended vessel.  A few
        // in-memory recovery fixtures materialize the Sequence slice without
        // running that earlier lifecycle; apply the same remove-and-append
        // once when every member is still visible, without disturbing a live
        // sequence vessel's post-tree position.
        for sequence in &self.sequences {
            if sequence.members.len() > 1
                && sequence.members.iter().all(|member| order.contains(member))
            {
                let Some(&vessel) = sequence.members.first() else {
                    continue;
                };
                order.retain(|node| !sequence.members.contains(node));
                order.push(vessel);
            }
        }
        for members in self
            .clusters
            .iter()
            .map(|cluster| cluster.members.as_slice())
        {
            let Some(&vessel) = members.first() else {
                continue;
            };
            order.retain(|node| !members.contains(node));
            order.push(vessel);
        }
        self.node_order = order;
        self.invalidate_node_order_membership();
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "REBUILD_ORDER_RUST after={:?}",
                self.node_order
                    .iter()
                    .map(|node| self.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
    }

    /// Applies `AddSequences`' remove-and-append lifecycle. This runs before
    /// tree preprocessing; the tree stage may then move that active vessel
    /// again while restoring non-branching trees.
    pub(super) fn rebuild_sequence_node_order(&mut self) {
        let mut order = self.node_order.clone();
        for sequence in &self.sequences {
            let Some(&vessel) = sequence.members.first() else {
                continue;
            };
            order.retain(|node| !sequence.members.contains(node));
            order.push(vessel);
        }
        self.node_order = order;
        self.invalidate_node_order_membership();
    }

    // grouping/lifecycle.go129–132/167–170 append restored members before
    // fresh vessel filters at 149–157/185–192. The cluster starts with make(0);
    // the sequence starts with a nil slice.
    fn restore_bound_aggregate_container_members(
        &self,
        parent: Option<NodeId>,
        vessel: NodeId,
        members: &[NodeId],
        allocated_empty: bool,
    ) {
        for &member in members {
            self.containers.append_node(parent, member);
        }
        let current = self.containers.get_header(&parent).unwrap_or_default();
        let mut updated = if allocated_empty {
            SharedNodeSlice::allocated_empty()
        } else {
            SharedNodeSlice::nil()
        };
        for child in current.iter_slots() {
            if child != Some(vessel) {
                updated = updated.append(&[child]).expect("aggregate cleanup append");
            }
        }
        self.containers.replace_header(parent, updated);
    }

    // Cleanup uses original pointers directly, in retained record order;
    // placement RestoreEdgeAbductions instead resolves active vessels.
    fn restore_bound_group_edge_endpoints(
        &mut self,
        vessel: NodeId,
        kind: super::owner_groups::OwnerGroupKind,
    ) {
        let group = self
            .owner_graph_group_entry(OwnerGraphId(0), kind, Some(vessel))
            .expect("cleanup group map")
            .flatten();
        // Topology-only carriers have no allocated canonical group records.
        let Some(group) = group else { return };
        let header = match kind {
            super::owner_groups::OwnerGroupKind::Cluster => {
                self.owner_cluster_policy(group)
                    .expect("cleanup cluster policy")
                    .edge_abductions
            }
            super::owner_groups::OwnerGroupKind::Sequence => self
                .owner_sequence_abductions(group)
                .expect("cleanup sequence abductions"),
        };
        if let Some(header) = header {
            for id in header.iter_slots() {
                let record = *self
                    .owner_edge_abduction(id.expect("cleanup abduction pointer"))
                    .expect("cleanup abduction record");
                if let Some(from) = record.originally_from {
                    self.reconnect_owner_edge_endpoint(
                        record.source_edge.expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        ),
                        from,
                        false,
                    );
                }
                if let Some(to) = record.originally_to {
                    self.reconnect_owner_edge_endpoint(
                        record.source_edge.expect(
                            "runtime error: invalid memory address or nil pointer dereference",
                        ),
                        to,
                        true,
                    );
                }
            }
        }
    }

    // AddNewNodeToContainer restores direct member fields, not descendant
    // ownership. Cleanup keeps the group object but retires its vessel.
    fn restore_bound_group_members(&mut self, parent: Option<NodeId>, members: &[NodeId]) {
        for &member in members {
            self.node_order.push(member);
            self.containers.append_node(parent, member);
            self.nodes[member.0 as usize].container = parent;
            self.set_owner_node_graph_reference(member, Some(OwnerGraphId(0)))
                .expect("cleanup member graph");
        }
        if let Some(parent) = parent {
            self.nodes[parent.0 as usize].is_container = true;
        }
    }

    /// Commit the numerical placement/packing projection to its real vessel
    /// pointers before the Source stage callback and cleanup use those fields.
    pub(super) fn publish_aggregate_projection_geometry(&mut self) {
        let references = self
            .clusters
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                Some((TopologyVessel::Cluster(index), *group.members.first()?))
            })
            .chain(
                self.sequences
                    .iter()
                    .enumerate()
                    .filter_map(|(index, group)| {
                        Some((TopologyVessel::Sequence(index), *group.members.first()?))
                    }),
            )
            .collect::<Vec<_>>();
        for (reference, carrier) in references {
            let Some(vessel) = self.topology_vessel_node(reference) else {
                continue;
            };
            if self.topology_node_owner(vessel).is_none() {
                continue;
            }
            let position = self.active_node_position(carrier);
            let size = self.active_node_size(carrier);
            if let Some(position) = position {
                // Source node.go:2146 translates an existing TopLeft in place.
                // Retained point headers must observe publication, not keep an
                // allocation abandoned by assigning a fresh TopLeft.
                if let Ok(point) = self.owner_node_point(vessel) {
                    if point.get().is_some() {
                        self.write_owner_point(&point, position)
                            .expect("vessel point publication");
                    } else {
                        self.set_position(vessel, position);
                    }
                } else {
                    self.set_position(vessel, position);
                }
                self.nodes[vessel.0 as usize].rect.origin = position;
            }
            self.nodes[vessel.0 as usize].rect.size = size;
            // The numerical placement adapter uses the first member as its
            // carrier. Publish that slot as the actual vessel before Cleanup
            // appends the original members (lifecycle.go:129-158,167-193).
            // Otherwise the first member is visited twice by later movement.
            if !self.node_order.contains(&vessel) {
                for node in &mut self.node_order {
                    if *node == carrier {
                        *node = vessel;
                    }
                }
            }
            let parent = self.nodes[vessel.0 as usize].container;
            if let Some(header) = self.containers.get_header(&parent) {
                let children = header.require_nodes().collect::<Vec<_>>();
                if !children.contains(&vessel) {
                    for (index, child) in children.into_iter().enumerate() {
                        if child == carrier {
                            header
                                .write_capacity_slot(index, Some(vessel))
                                .expect("publish vessel container slot");
                        }
                    }
                }
            }
        }
        self.invalidate_node_order_membership();
        self.synchronize_original_owner_graph_view();
    }

    fn retire_bound_group_vessel(
        &mut self,
        parent: Option<NodeId>,
        vessel: NodeId,
        members: &[NodeId],
        allocated_empty: bool,
    ) {
        if !self.nodes[vessel.0 as usize].nears.is_empty() {
            let mut nears = self.nodes[vessel.0 as usize]
                .nears
                .raw_keys()
                .into_iter()
                .map(|near| {
                    near.expect("runtime error: invalid memory address or nil pointer dereference")
                })
                .collect::<Vec<_>>();
            nears.sort_by_key(|near| self.nodes[near.0 as usize].tala_id);
            for near in nears {
                self.nodes[near.0 as usize].nears.delete(Some(vessel));
                for &member in members {
                    self.add_owner_near(Some(near), Some(member))
                        .expect("cleanup member Near");
                }
            }
            self.nodes[vessel.0 as usize].nears = NodeNearMap::allocated();
        }
        self.node_order.retain(|node| *node != vessel);
        self.synchronize_original_owner_graph_view();
        // Members have already been appended before edge/Near restoration.
        self.restore_bound_aggregate_container_members(parent, vessel, &[], allocated_empty);
        self.nodes[vessel.0 as usize].container = None;
        self.set_owner_node_graph_reference(vessel, None)
            .expect("cleanup retired vessel graph");
    }

    /// Applies `CleanupStuffStage`'s sorted cluster-then-sequence expansion.
    pub(super) fn restore_aggregate_members_to_node_order(&mut self) {
        // In the Go graph, CleanupStuff follows the final aggregate layout
        // with the same shared-pointer `SyncNested` visit that occurred while
        // the enclosing member vessel was active. A clustered member can
        // itself be a container; its direct children therefore receive the
        // member's shape padding before the temporary vessel is discarded.
        // Rust keeps the member as the stable carrier and postpones this
        // frame publication until all parent placement is complete so those
        // hidden boxes cannot become top-level optimizer candidates.
        // CleanupStuff does not position a cluster member's container
        // children before arranging the cluster.  `ArrangeClusterNodes`
        // receives the vessel frame first; AddNewNodeToContainer only
        // restores ownership afterward.  Keeping this ordering matters for
        // half-pixel member centers (and avoids an extra child translation
        // that the temporary Go vessel never performs here).
        let mut clusters = (0..self.clusters.len()).collect::<Vec<_>>();
        clusters.sort_by_key(|cluster| self.clusters[*cluster].vessel_tala_id);
        for cluster_index in clusters {
            let members = self.clusters[cluster_index].members.clone();
            let Some(&vessel) = members.first() else {
                continue;
            };
            let bound_vessel = self.topology_vessel_node(TopologyVessel::Cluster(cluster_index));
            let parent = self.nodes[bound_vessel.unwrap_or(vessel).0 as usize].container;
            if bound_vessel.is_none()
                && let Some(mut children) = self.containers.get(&parent)
            {
                // Unbound compact aggregates still use a member carrier.
                // Preserve that numerical projection with a fresh make(len)
                // header; its allocation is not qualified as Go Cleanup.
                children.retain(|child| *child != vessel && !members.contains(child));
                children.extend(members.iter().copied());
                self.containers.insert(parent, children);
            }
            // The temporary cluster vessel is a distinct Go node. In the
            // stable arena its first member can be centered within a
            // fixed-size vessel, so that member's top-left is not necessarily
            // the vessel origin CleanupStuff passes to ArrangeClusterNodes.
            if let Some(vessel_top_left) = bound_vessel
                .map(|vessel| self.position(vessel))
                .unwrap_or_else(|| {
                    self.pending_cluster_vessel_positions
                        .get(&cluster_index)
                        .copied()
                        .or_else(|| {
                            self.transaction_external_aggregate_children
                                .get(&self.clusters[cluster_index].vessel_tala_id)
                                .and_then(|members| members.first())
                                .and_then(|member| member.position)
                        })
                        .or_else(|| {
                            self.transaction_external_container_children
                                .values()
                                .flatten()
                                .find(|member| {
                                    member.tala_id == self.clusters[cluster_index].vessel_tala_id
                                })
                                .and_then(|member| member.position)
                        })
                        .or_else(|| self.position(vessel))
                })
            {
                if crate::internal::trace_env_enabled("TALA_TRACE_CLEANUP_CLUSTER") {
                    eprintln!(
                        "CLEANUP_CLUSTER_RUST_BEFORE vessel={} pos={:?} size={:?} members={:?}",
                        self.clusters[cluster_index].vessel_tala_id,
                        vessel_top_left,
                        self.cluster_vessel_size(cluster_index),
                        members
                            .iter()
                            .map(|member| (
                                self.nodes[member.0 as usize].tala_id,
                                self.position(*member)
                            ))
                            .collect::<Vec<_>>()
                    );
                }
                // CleanupStuff invokes ArrangeClusterNodes without Resize
                // immediately before replacing the temporary vessel.
                self.arrange_cluster_members_cleanup(cluster_index, vessel_top_left);
                if crate::internal::trace_env_enabled("TALA_TRACE_CLEANUP_CLUSTER") {
                    eprintln!(
                        "CLEANUP_CLUSTER_RUST_AFTER vessel={} members={:?}",
                        self.clusters[cluster_index].vessel_tala_id,
                        members
                            .iter()
                            .map(|member| (
                                self.nodes[member.0 as usize].tala_id,
                                self.position(*member)
                            ))
                            .collect::<Vec<_>>()
                    );
                }
            }
            if let Some(bound_vessel) = bound_vessel {
                self.restore_bound_group_members(parent, &members);
                self.restore_bound_group_edge_endpoints(
                    bound_vessel,
                    super::owner_groups::OwnerGroupKind::Cluster,
                );
                self.retire_bound_group_vessel(parent, bound_vessel, &members, true);
                continue;
            }
            self.node_order
                .retain(|node| *node != bound_vessel.unwrap_or(vessel));
            self.node_order.extend(members);
        }

        let mut sequences = (0..self.sequences.len()).collect::<Vec<_>>();
        sequences.sort_by_key(|sequence| self.sequences[*sequence].vessel_tala_id);
        for sequence_index in sequences {
            let members = self.sequences[sequence_index].members.clone();
            let Some(&vessel) = members.first() else {
                continue;
            };
            let parent = self.sequences[sequence_index].container;
            let bound_vessel = self.topology_vessel_node(TopologyVessel::Sequence(sequence_index));
            if bound_vessel.is_none()
                && let Some(mut children) = self.containers.get(&parent)
            {
                // The unbound member-carrier projection has no real source
                // vessel. Keep its visible result and leave source header
                // allocation semantics unqualified at this boundary.
                children.retain(|child| !members.contains(child));
                children.extend(members.iter().copied());
                self.containers.insert(parent, children);
            }
            if let Some(vessel_top_left) = self.position(bound_vessel.unwrap_or(vessel)) {
                // Sequence.arrangeSteps expands every hidden step from the
                // vessel's current top-left during CleanupStuff.
                for member in members.iter().copied() {
                    let Some((offset, _)) = self.sequence_member_geometry(sequence_index, member)
                    else {
                        continue;
                    };
                    self.set_position(
                        member,
                        Point {
                            x: vessel_top_left.x + offset.x,
                            y: vessel_top_left.y + offset.y,
                        },
                    );
                }
            }
            if let Some(bound_vessel) = bound_vessel {
                self.restore_bound_group_members(parent, &members);
                self.restore_bound_group_edge_endpoints(
                    bound_vessel,
                    super::owner_groups::OwnerGroupKind::Sequence,
                );
                self.retire_bound_group_vessel(parent, bound_vessel, &members, false);
                continue;
            }
            self.node_order
                .retain(|node| *node != bound_vessel.unwrap_or(vessel));
            self.node_order.extend(members);
        }
        self.invalidate_node_order_membership();
    }

    /// Recovered obstruction ownership for one `Node.edgeLength` edge.
    ///
    /// TALA does not scan an arena-wide placement component. It walks the
    /// endpoint containers toward their nearest shared ancestor and appends
    /// each visited `Graph.Containers[scope]` slice in graph order.
    pub(super) fn edge_obstruction_nodes(&self, node: NodeId, adjacent: NodeId) -> Vec<NodeId> {
        self.edge_obstruction_nodes_from(node, adjacent, |graph, scope| {
            graph.obstruction_container_node_order(scope)
        })
    }

    /// Returns the live `Graph.Containers` obstruction order after a tree
    /// placement has restored the tree's retained children into its parent
    /// scope. Recovered `Node.edgeLength` reads that current slice directly;
    /// the pre-placement snapshot is only authoritative while the tree is
    /// still collapsed to its temporary root.
    pub(super) fn edge_obstruction_nodes_after_tree_restoration(
        &self,
        node: NodeId,
        adjacent: NodeId,
    ) -> Vec<NodeId> {
        self.edge_obstruction_nodes_from(node, adjacent, |graph, scope| {
            graph.obstruction_container_node_order_with_materialized(scope, true)
        })
    }

    pub(super) fn edge_obstruction_nodes_for_materialized(
        &self,
        node: NodeId,
        adjacent: NodeId,
        materialized_scopes: &BTreeSet<Option<NodeId>>,
    ) -> Vec<NodeId> {
        self.edge_obstruction_nodes_from(node, adjacent, |graph, scope| {
            graph.obstruction_container_node_order_with_materialized(
                scope,
                materialized_scopes.contains(&scope),
            )
        })
    }

    /// Source edge_length.go:869-900 appends the complete child slice for
    /// every visited key, including nil and allocated empty slices.
    pub(super) fn edge_obstruction_sets_for_materialized(
        &self,
        node: NodeId,
        adjacent: NodeId,
        materialized_scopes: &BTreeSet<Option<NodeId>>,
    ) -> Vec<(Option<NodeId>, Option<Vec<NodeId>>)> {
        let container = |node: NodeId| {
            self.topology_node(node)
                .or_else(|| self.nodes.get(node.0 as usize))
                .and_then(|node| node.container)
        };
        let chain = |endpoint: NodeId| {
            let mut scopes = Vec::new();
            let mut current = container(endpoint);
            loop {
                scopes.push(current);
                let Some(parent) = current else { break };
                current = container(parent);
            }
            scopes
        };
        let node_chain = chain(node);
        let adjacent_chain = chain(adjacent);
        let ancestor = node_chain
            .iter()
            .copied()
            .find(|scope| adjacent_chain.contains(scope))
            .flatten();
        let mut seen = BTreeSet::new();
        let mut sets = Vec::new();
        for endpoint in [node, adjacent] {
            let mut current = container(endpoint);
            loop {
                if !seen.insert(current) {
                    break;
                }
                let items = if self.shared_topology_initialized() {
                    self.topology_container_slice(current).flatten()
                } else if self.preprocessed_tree_children.contains_key(&current)
                    || self
                        .sequences
                        .iter()
                        .any(|sequence| sequence.container == current)
                    || self.clusters.iter().any(|cluster| {
                        cluster
                            .members
                            .first()
                            .is_some_and(|owner| self.nodes[owner.0 as usize].container == current)
                    })
                {
                    Some(self.obstruction_container_node_order_with_materialized(
                        current,
                        materialized_scopes.contains(&current),
                    ))
                } else {
                    self.containers.get(&current)
                };
                sets.push((current, items));
                let Some(parent) = current else { break };
                current = container(parent);
                if current == ancestor {
                    break;
                }
            }
        }
        sets
    }

    fn edge_obstruction_nodes_from(
        &self,
        node: NodeId,
        adjacent: NodeId,
        children: impl Fn(&ArenaGraph, Option<NodeId>) -> Vec<NodeId>,
    ) -> Vec<NodeId> {
        fn container_chain(graph: &ArenaGraph, node: NodeId) -> Vec<Option<NodeId>> {
            let mut chain = Vec::new();
            let mut current = graph.nodes[node.0 as usize].container;
            loop {
                chain.push(current);
                let Some(container) = current else {
                    break;
                };
                current = graph.nodes[container.0 as usize].container;
            }
            chain
        }

        let node_chain = container_chain(self, node);
        let adjacent_chain = container_chain(self, adjacent);
        let ancestor = node_chain
            .iter()
            .copied()
            .find(|scope| adjacent_chain.contains(scope))
            .flatten();

        let mut seen_scopes = BTreeSet::new();
        let mut obstructions = Vec::new();
        let trace_obstructions = crate::internal::trace_env_enabled("TALA_TRACE_EDGE_OBSTRUCTIONS");
        for endpoint in [node, adjacent] {
            let mut current = self.nodes[endpoint.0 as usize].container;
            loop {
                if !seen_scopes.insert(current) {
                    break;
                }
                // Node.edgeLength observes Graph.Containers rather than
                // Graph.Nodes. The two slices have distinct ordering after
                // aggregate insertion and SplitSubgraphs; alternate-route
                // probing is stateful, so that distinction affects scoring.
                let scope_children = children(self, current);
                if trace_obstructions {
                    eprintln!(
                        "EDGE_OBSTRUCTIONS_RUST scope={:?} endpoint={} children={:?}",
                        current.map(|scope| self.nodes[scope.0 as usize].tala_id),
                        self.nodes[endpoint.0 as usize].tala_id,
                        scope_children
                            .iter()
                            .map(|child| self.nodes[child.0 as usize].tala_id)
                            .collect::<Vec<_>>(),
                    );
                }
                obstructions.extend(scope_children);
                let Some(container) = current else {
                    break;
                };
                current = self.nodes[container.0 as usize].container;
                if current == ancestor {
                    break;
                }
            }
        }
        if trace_obstructions {
            eprintln!(
                "EDGE_OBSTRUCTIONS_RUST result={}>{} nodes={:?}",
                self.nodes[node.0 as usize].tala_id,
                self.nodes[adjacent.0 as usize].tala_id,
                obstructions
                    .iter()
                    .map(|obstruction| self.nodes[obstruction.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
            );
        }
        obstructions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAlignment, Edge, EdgeArrows, Node};

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
    fn graph_nodes_follow_source_order_not_hierarchy_preorder() {
        let mut graph = Graph::default();
        let root = graph.add_node(node("root"));
        let left = graph.add_node(node("left"));
        let right = graph.add_node(node("right"));
        let left_child = graph.add_node(node("left.child"));
        let right_child = graph.add_node(node("right.child"));
        graph.node_mut(left).unwrap().parent = Some(root);
        graph.node_mut(right).unwrap().parent = Some(root);
        graph.node_mut(left_child).unwrap().parent = Some(left);
        graph.node_mut(right_child).unwrap().parent = Some(right);

        let arena = ArenaGraph::from_input(&graph);
        assert_eq!(
            arena.graph_node_order(),
            vec![root, left, right, left_child, right_child]
        );
    }

    #[test]
    fn aggregate_cleanup_restores_members_at_the_end_of_graph_nodes() {
        let mut graph = Graph::default();
        let first = graph.add_node(node("first"));
        let clustered_first = graph.add_node(node("clustered.first"));
        let middle = graph.add_node(node("middle"));
        let clustered_second = graph.add_node(node("clustered.second"));
        let last = graph.add_node(node("last"));

        let mut arena = ArenaGraph::from_input(&graph);
        arena.clusters.push(ClusterState {
            fixed_size: false,
            members: vec![clustered_first, clustered_second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 7,
        });
        arena.rebuild_active_aggregate_node_order();
        assert_eq!(
            arena.graph_node_order(),
            vec![first, middle, last, clustered_first]
        );

        arena.restore_aggregate_members_to_node_order();
        assert_eq!(
            arena.graph_node_order(),
            vec![first, middle, last, clustered_first, clustered_second]
        );
    }

    #[test]
    fn aggregate_cleanup_arranges_sequence_steps_from_the_vessel_top_left() {
        let mut graph = Graph::default();
        let first = graph.add_node(node("first"));
        let second = graph.add_node(node("second"));
        let mut arena = ArenaGraph::from_input(&graph);
        for member in [first, second] {
            arena.nodes[member.0 as usize].sequence = Some(0);
        }
        arena.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 7,
            container: None,
            has_edge_abductions: false,
        });
        arena.set_position(first, Point { x: 10.0, y: 20.0 });
        arena.set_position(second, Point { x: 75.0, y: 77.0 });
        arena.rebuild_active_aggregate_node_order();

        arena.restore_aggregate_members_to_node_order();

        assert_eq!(arena.position(first), Some(Point { x: 10.0, y: 20.0 }));
        assert_eq!(arena.position(second), Some(Point { x: 15.0, y: 20.0 }));
    }

    #[test]
    fn cleanup_restores_sequence_steps_to_container_slice_for_routed_binpack() {
        let mut graph = Graph::default();
        let scope = graph.add_node(node("scope"));
        let mut steps = Vec::new();
        for name in ["step.1", "step.2", "step.3"] {
            let mut step = node(name);
            step.parent = Some(scope);
            steps.push(graph.add_node(step));
        }

        let mut arena = ArenaGraph::from_input(&graph);
        arena.containers.insert(Some(scope), vec![steps[0]]);
        for member in &steps {
            arena.nodes[member.0 as usize].sequence = Some(0);
        }
        arena.sequences.push(SequenceState {
            members: steps.clone(),
            vessel_tala_id: 20_001,
            container: Some(scope),
            has_edge_abductions: false,
        });
        arena.rebuild_active_aggregate_node_order();

        arena.restore_aggregate_members_to_node_order();

        assert_eq!(arena.containers.nodes(&Some(scope)), steps);
    }

    #[test]
    fn published_cluster_container_has_one_explicit_vessel_projection() {
        let mut graph = Graph::default();
        let parent = graph.add_node(node("parent"));
        let mut members = Vec::new();
        for name in ["first", "second"] {
            let mut member = node(name);
            member.parent = Some(parent);
            members.push(graph.add_node(member));
        }
        let mut vessel_node = node("allocated.vessel");
        vessel_node.parent = Some(parent);
        let vessel = graph.add_node(vessel_node);
        let mut arena = ArenaGraph::from_input(&graph);
        arena.clusters.push(ClusterState {
            members: members.clone(),
            vessel_tala_id: 501,
            fixed_size: false,
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
        });
        for member in &members {
            arena.nodes[member.0 as usize].cluster = Some(0);
        }
        arena.nodes[vessel.0 as usize].tala_id = 501;
        arena.node_order = vec![parent, members[0]];
        arena
            .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
            .unwrap();
        arena
            .containers
            .insert(Some(parent), vec![members[0], members[1]]);
        assert_eq!(arena.container_node_order(Some(parent)), vec![members[0]]);
        arena.containers.insert(Some(parent), vec![vessel]);
        assert_eq!(arena.containers.nodes(&Some(parent)), vec![vessel]);
        assert_eq!(arena.container_node_order(Some(parent)), vec![members[0]]);
        assert!(!arena.hidden_swap_aggregate_member(vessel));
        assert!(!arena.hidden_swap_aggregate_member(members[0]));
        assert!(arena.hidden_swap_aggregate_member(members[1]));
        assert_eq!(arena.gap_herd_node(members[0]), vessel);
        assert_eq!(arena.gap_herd_node(vessel), vessel);
        assert_eq!(arena.gap_herd_node(parent), parent);
        arena.nodes[vessel.0 as usize].herd_assignment = Some(crate::internal::HerdAssignment {
            orientation: crate::internal::Orientation::Top,
            value: 0.0,
            same_side_paired: Default::default(),
            opposite_side_paired: Default::default(),
        });
        assert!(arena.herd_assignment_snapshot(members[0]).is_none());
        assert_eq!(
            arena
                .herd_assignment_snapshot(arena.gap_herd_node(members[0]))
                .unwrap()
                .orientation,
            crate::internal::Orientation::Top
        );
        assert_eq!(arena.numerical_aggregate_node(vessel), members[0]);
        // Dense storage can retain both identities while the numerical
        // Graph.Nodes view exposes only the first-member carrier.
        arena.node_order = vec![parent, members[0], vessel];
        arena.refresh_node_order_membership();
        assert!(!arena.graph_node_order().contains(&vessel));
        assert_eq!(arena.numerical_aggregate_node(vessel), members[0]);
        assert_eq!(arena.container_node_order(Some(parent)), vec![members[0]]);
        arena.node_order = vec![parent, vessel];
        arena.refresh_node_order_membership();
        assert_eq!(arena.numerical_aggregate_node(vessel), vessel);
        assert_eq!(arena.container_node_order(Some(parent)), vec![vessel]);
    }

    #[test]
    fn completed_sequence_projection_updates_the_retained_vessel_geometry() {
        let mut graph = Graph::default();
        let first = graph.add_node(node("first"));
        let second = graph.add_node(node("second"));
        let vessel = graph.add_node(node("actual.vessel"));
        let mut arena = ArenaGraph::from_input(&graph);
        arena.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 501,
            container: None,
            has_edge_abductions: false,
        });
        for member in [first, second] {
            arena.nodes[member.0 as usize].sequence = Some(0);
        }
        arena.node_order = vec![first];
        arena.containers.insert(None, vec![first]);
        arena.nodes[vessel.0 as usize].tala_id = 501;
        arena
            .bind_topology_vessel(TopologyVessel::Sequence(0), vessel)
            .unwrap();
        arena.set_position(first, Point { x: 10.0, y: 20.0 });
        arena.set_position(
            vessel,
            Point {
                x: -100.0,
                y: -200.0,
            },
        );
        arena.materialize_owner_node_point(vessel).unwrap();
        let retained_point = arena.owner_node_point(vessel).unwrap();
        arena.publish_aggregate_projection_geometry();
        assert!(retained_point.same_allocation(&arena.owner_node_point(vessel).unwrap()));
        assert_eq!(retained_point.get(), Some(Point { x: 10.0, y: 20.0 }));
        assert_eq!(arena.position(vessel), Some(Point { x: 10.0, y: 20.0 }));
        assert_eq!(
            arena.nodes[vessel.0 as usize].rect.origin,
            Point { x: 10.0, y: 20.0 }
        );
        assert_eq!(arena.node_order, vec![vessel]);
        assert_eq!(arena.containers.nodes(&None), vec![vessel]);
        // Two 40-wide steps advance by 40-35, giving a 45-wide vessel.
        assert_eq!(
            arena.nodes[vessel.0 as usize].rect.size,
            Size {
                width: 45.0,
                height: 40.0
            }
        );
    }

    #[test]
    fn bound_cleanup_appends_before_fresh_vessel_filter_and_preserves_captured_tail() {
        for sequence in [false, true] {
            let mut graph = Graph::default();
            let first = graph.add_node(node("first"));
            let second = graph.add_node(node("second"));
            let survivor = graph.add_node(node("survivor"));
            let vessel = graph.add_node(node("actual.vessel"));
            let mut arena = ArenaGraph::from_input(&graph);
            arena.nodes[vessel.0 as usize].tala_id = 501;
            arena.node_order = vec![vessel, survivor];
            if sequence {
                arena.sequences.push(SequenceState {
                    members: vec![first, second],
                    vessel_tala_id: 501,
                    container: None,
                    has_edge_abductions: false,
                });
                arena
                    .bind_topology_vessel(TopologyVessel::Sequence(0), vessel)
                    .unwrap();
            } else {
                arena.clusters.push(ClusterState {
                    members: vec![first, second],
                    vessel_tala_id: 501,
                    fixed_size: false,
                    arrangement: ClusterArrangement::Row,
                    desired_arrangement: ClusterArrangement::Row,
                    padding: 0.0,
                });
                arena
                    .bind_topology_vessel(TopologyVessel::Cluster(0), vessel)
                    .unwrap();
            }
            let captured = SharedNodeSlice::with_len_cap(2, 6).unwrap();
            captured.write_capacity_slot(0, Some(vessel)).unwrap();
            captured.write_capacity_slot(1, Some(survivor)).unwrap();
            captured.write_capacity_slot(5, Some(survivor)).unwrap();
            arena.containers.replace_header(None, captured.clone());
            arena.add_owner_near(Some(vessel), Some(survivor)).unwrap();
            let captured_vessel_nears = arena.nodes[vessel.0 as usize].nears.retained();
            let captured_survivor_nears = arena.nodes[survivor.0 as usize].nears.retained();

            arena.restore_aggregate_members_to_node_order();

            let current = arena.containers.get_header(&None).unwrap();
            assert_eq!(
                current.visible_slots(),
                vec![Some(survivor), Some(first), Some(second)]
            );
            assert!(!captured.same_backing_allocation(&current));
            assert_eq!(captured.len(), 2);
            assert_eq!(
                captured.capacity_slots(),
                vec![
                    Some(vessel),
                    Some(survivor),
                    Some(first),
                    Some(second),
                    None,
                    Some(survivor)
                ]
            );
            assert_eq!(arena.node_order, vec![survivor, first, second]);
            assert_eq!(arena.topology_node_owner(vessel), None);
            assert_eq!(arena.nodes[vessel.0 as usize].container, None);
            for member in [first, second] {
                assert_eq!(arena.topology_node_owner(member), Some(OwnerGraphId(0)));
                assert_eq!(arena.nodes[member.0 as usize].container, None);
                assert!(arena.nodes[member.0 as usize].nears.contains(&survivor));
                assert!(captured_survivor_nears.contains(&member));
            }
            assert!(!captured_survivor_nears.contains(&vessel));
            assert!(captured_vessel_nears.contains(&survivor));
            assert!(arena.nodes[vessel.0 as usize].nears.is_empty());
            assert!(!captured_vessel_nears.same_allocation(&arena.nodes[vessel.0 as usize].nears));
            assert!(
                captured_survivor_nears.same_allocation(&arena.nodes[survivor.0 as usize].nears)
            );
            assert_eq!(
                arena
                    .owner_graph_member_header(OwnerGraphId(0))
                    .unwrap()
                    .iter_slots()
                    .collect::<Vec<_>>(),
                vec![Some(survivor), Some(first), Some(second)]
            );
        }
    }

    #[test]
    fn cleanup_filter_distinguishes_cluster_make_zero_from_sequence_nil_and_keeps_nil_items() {
        let arena = ArenaGraph::from_input(&Graph::default());
        let vessel = NodeId(0);
        for allocated_empty in [true, false] {
            arena
                .containers
                .replace_header(None, SharedNodeSlice::from_slots(&[Some(vessel)]).unwrap());
            arena.restore_bound_aggregate_container_members(None, vessel, &[], allocated_empty);
            let empty = arena.containers.get_header(&None).unwrap();
            assert_eq!(empty.len(), 0);
            assert_eq!(empty.capacity(), 0);
            assert_eq!(empty.is_nil(), !allocated_empty);

            arena.containers.replace_header(
                None,
                SharedNodeSlice::from_slots(&[Some(vessel), None]).unwrap(),
            );
            arena.restore_bound_aggregate_container_members(None, vessel, &[], allocated_empty);
            assert_eq!(
                arena.containers.get_header(&None).unwrap().visible_slots(),
                vec![None]
            );
        }
    }

    #[test]
    fn active_descendants_return_vessels_but_not_hidden_sequence_steps() {
        let mut graph = Graph::default();
        let root = graph.add_node(node("root"));
        let first = graph.add_node(node("first"));
        let second = graph.add_node(node("second"));
        let ordinary = graph.add_node(node("ordinary"));
        graph.node_mut(first).unwrap().parent = Some(root);
        graph.node_mut(second).unwrap().parent = Some(root);
        graph.node_mut(ordinary).unwrap().parent = Some(root);

        let mut arena = ArenaGraph::from_input(&graph);
        arena.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 10_001,
            container: Some(root),
            has_edge_abductions: false,
        });
        arena.rebuild_active_aggregate_node_order();

        assert_eq!(arena.active_descendants(root), vec![ordinary, first]);
    }

    #[test]
    fn moving_container_translates_hidden_cluster_members_with_vessel_proxy() {
        let mut graph = Graph::default();
        let root = graph.add_node(node("root"));
        let first = graph.add_node(node("first"));
        let second = graph.add_node(node("second"));
        graph.node_mut(first).unwrap().parent = Some(root);
        graph.node_mut(second).unwrap().parent = Some(root);

        let mut arena = ArenaGraph::from_input(&graph);
        arena.clusters.push(ClusterState {
            fixed_size: false,
            members: vec![first, second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10_001,
        });
        arena.set_position(root, Point { x: 0.0, y: 0.0 });
        arena.set_position(first, Point { x: 10.0, y: 20.0 });
        arena.set_position(second, Point { x: 80.0, y: 20.0 });
        arena.rebuild_active_aggregate_node_order();

        arena.move_node_abs_with_children(root, Point { x: 100.0, y: 200.0 });

        assert_eq!(arena.position(first), Some(Point { x: 110.0, y: 220.0 }));
        assert_eq!(arena.position(second), Some(Point { x: 180.0, y: 220.0 }));
    }

    #[test]
    fn moving_container_translates_hidden_sequence_steps_with_vessel_proxy() {
        let mut graph = Graph::default();
        let root = graph.add_node(node("root"));
        let first = graph.add_node(node("first"));
        let second = graph.add_node(node("second"));
        graph.node_mut(first).unwrap().parent = Some(root);
        graph.node_mut(second).unwrap().parent = Some(root);

        let mut arena = ArenaGraph::from_input(&graph);
        arena.sequences.push(SequenceState {
            members: vec![first, second],
            vessel_tala_id: 10_001,
            container: Some(root),
            has_edge_abductions: false,
        });
        arena.set_position(root, Point { x: 0.0, y: 0.0 });
        arena.set_position(first, Point { x: 10.0, y: 20.0 });
        arena.set_position(second, Point { x: 10.0, y: 90.0 });
        arena.rebuild_active_aggregate_node_order();

        arena.move_node_abs_with_children(root, Point { x: 100.0, y: 200.0 });

        assert_eq!(arena.position(first), Some(Point { x: 110.0, y: 220.0 }));
        assert_eq!(arena.position(second), Some(Point { x: 110.0, y: 290.0 }));
    }

    #[test]
    fn moving_container_translates_sequence_steps_hidden_inside_cluster() {
        let mut graph = Graph::default();
        let root = graph.add_node(node("root"));
        let first_step = graph.add_node(node("first-step"));
        let second_step = graph.add_node(node("second-step"));
        let cluster_peer = graph.add_node(node("cluster-peer"));
        for child in [first_step, second_step, cluster_peer] {
            graph.node_mut(child).unwrap().parent = Some(root);
        }

        let mut arena = ArenaGraph::from_input(&graph);
        arena.sequences.push(SequenceState {
            members: vec![first_step, second_step],
            vessel_tala_id: 10_001,
            container: Some(root),
            has_edge_abductions: false,
        });
        arena.clusters.push(ClusterState {
            fixed_size: false,
            members: vec![first_step, cluster_peer],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10_002,
        });
        arena.set_position(root, Point { x: 0.0, y: 0.0 });
        arena.set_position(first_step, Point { x: 10.0, y: 20.0 });
        arena.set_position(second_step, Point { x: 10.0, y: 90.0 });
        arena.set_position(cluster_peer, Point { x: 80.0, y: 20.0 });
        arena.rebuild_active_aggregate_node_order();

        arena.move_node_abs_with_children(root, Point { x: 100.0, y: 200.0 });

        assert_eq!(
            arena.position(first_step),
            Some(Point { x: 110.0, y: 220.0 })
        );
        assert_eq!(
            arena.position(second_step),
            Some(Point { x: 110.0, y: 290.0 })
        );
        assert_eq!(
            arena.position(cluster_peer),
            Some(Point { x: 180.0, y: 220.0 })
        );
    }

    #[test]
    fn obstruction_order_observes_aggregate_vessel_insertion_lifecycle() {
        let mut graph = Graph::default();
        let source = graph.add_node(node("source"));
        let clustered_first = graph.add_node(node("clustered.first"));
        let ordinary = graph.add_node(node("ordinary"));
        let clustered_second = graph.add_node(node("clustered.second"));
        let target = graph.add_node(node("target"));

        let mut arena = ArenaGraph::from_input(&graph);
        arena.clusters.push(ClusterState {
            fixed_size: false,
            members: vec![clustered_first, clustered_second],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 7,
        });
        arena.rebuild_active_aggregate_node_order();

        assert_eq!(
            arena.edge_obstruction_nodes(source, target),
            vec![source, ordinary, target, clustered_first]
        );
    }

    #[test]
    fn alignment_obstruction_order_observes_restored_tree_children() {
        let mut graph = Graph::default();
        let left = graph.add_node(node("left"));
        let right = graph.add_node(node("right"));

        let mut source_node = node("source");
        source_node.parent = Some(left);
        let source = graph.add_node(source_node);

        let mut target_node = node("target");
        target_node.parent = Some(right);
        let target = graph.add_node(target_node);

        let mut restored_sibling_node = node("restored-sibling");
        restored_sibling_node.parent = Some(right);
        let restored_sibling = graph.add_node(restored_sibling_node);

        let mut arena = ArenaGraph::from_input(&graph);
        arena
            .preprocessed_tree_children
            .insert(Some(right), vec![target]);

        assert!(
            !arena
                .edge_obstruction_nodes(source, target)
                .contains(&restored_sibling)
        );
        assert!(
            arena
                .edge_obstruction_nodes_after_tree_restoration(source, target)
                .contains(&restored_sibling)
        );
    }

    #[test]
    fn materialized_obstruction_order_preserves_aggregates_and_adds_restored_children() {
        let mut graph = Graph::default();
        let scope = graph.add_node(node("scope"));

        let mut target_node = node("target");
        target_node.parent = Some(scope);
        let target = graph.add_node(target_node);

        let mut restored_node = node("restored");
        restored_node.parent = Some(scope);
        let restored = graph.add_node(restored_node);

        let mut sequence_owner_node = node("sequence.owner");
        sequence_owner_node.parent = Some(scope);
        let sequence_owner = graph.add_node(sequence_owner_node);

        let mut sequence_hidden_node = node("sequence.hidden");
        sequence_hidden_node.parent = Some(scope);
        let sequence_hidden = graph.add_node(sequence_hidden_node);

        let mut cluster_owner_node = node("cluster.owner");
        cluster_owner_node.parent = Some(scope);
        let cluster_owner = graph.add_node(cluster_owner_node);

        let mut cluster_hidden_node = node("cluster.hidden");
        cluster_hidden_node.parent = Some(scope);
        let cluster_hidden = graph.add_node(cluster_hidden_node);

        let mut arena = ArenaGraph::from_input(&graph);
        arena.sequences.push(SequenceState {
            members: vec![sequence_owner, sequence_hidden],
            vessel_tala_id: 10_001,
            container: Some(scope),
            has_edge_abductions: false,
        });
        arena.clusters.push(ClusterState {
            fixed_size: false,
            members: vec![cluster_owner, cluster_hidden],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 20.0,
            vessel_tala_id: 10_002,
        });
        arena.rebuild_active_aggregate_node_order();
        arena
            .preprocessed_tree_children
            .insert(Some(scope), vec![target, sequence_owner, cluster_owner]);

        assert_eq!(
            arena.obstruction_container_node_order_with_materialized(Some(scope), false),
            vec![target, sequence_owner, cluster_owner]
        );
        assert_eq!(
            arena.obstruction_container_node_order_with_materialized(Some(scope), true),
            vec![target, sequence_owner, cluster_owner, restored]
        );
    }

    #[test]
    fn materialized_obstruction_order_keeps_every_active_sequence_step() {
        let mut graph = Graph::default();
        let scope = graph.add_node(node("scope"));
        let mut steps = Vec::new();
        for name in ["step.1", "step.2", "step.3", "step.4"] {
            let mut step = node(name);
            step.parent = Some(scope);
            steps.push(graph.add_node(step));
        }

        let mut arena = ArenaGraph::from_input(&graph);
        arena.sequences.push(SequenceState {
            members: steps.clone(),
            vessel_tala_id: 20_001,
            container: Some(scope),
            has_edge_abductions: false,
        });
        arena.rebuild_active_aggregate_node_order();

        // Graph.Nodes still contains only the first step's sequence vessel.
        assert_eq!(arena.container_node_order(Some(scope)), vec![steps[0]]);

        // A nested placement can materialize every original step pointer in
        // the shared Graph.Containers slice without expanding Graph.Nodes.
        arena
            .preprocessed_tree_children
            .insert(Some(scope), steps.clone());
        assert_eq!(arena.obstruction_container_node_order(Some(scope)), steps);
    }

    #[test]
    fn source_orientation_boundary_endpoints_follow_active_cluster_vessels() {
        let mut graph = Graph::default();
        let root = graph.add_node(node("source"));
        let first = graph.add_node(node("outside.first"));
        let second = graph.add_node(node("outside.second"));
        let third = graph.add_node(node("outside.third"));
        for target in [first, second, third] {
            let edge = graph.add_edge(Edge {
                source: root,
                target,
            });
            graph.set_edge_arrows(
                edge,
                EdgeArrows {
                    source: false,
                    target: true,
                },
            );
        }

        let mut arena = ArenaGraph::from_input(&graph);
        arena.clusters.push(ClusterState {
            fixed_size: false,
            members: vec![first, second, third],
            arrangement: ClusterArrangement::Column,
            desired_arrangement: ClusterArrangement::Column,
            padding: 20.0,
            vessel_tala_id: 30_001,
        });
        for member in [first, second, third] {
            arena.nodes[member.0 as usize].cluster = Some(0);
        }
        arena
            .node_order
            .retain(|node| *node != second && *node != third);
        arena.refresh_node_order_membership();

        let root_tala_id = arena.active_node_tala_id(root);
        assert_eq!(arena.active_node_tala_id(first), 30_001);
        assert_eq!(
            arena.source_orientation_boundary_edge_context(root).0,
            3,
            "source-orientation degree includes the full root adjacency"
        );
        assert_eq!(
            arena.source_orientation_boundary_edges(root),
            vec![
                Some((root_tala_id, 30_001)),
                Some((root_tala_id, 30_001)),
                Some((root_tala_id, 30_001)),
            ]
        );
    }
}
