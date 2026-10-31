// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reuse the projected translation closure for a stable compaction subgraph.
//! Canonical owner checkpoints remain governed by compaction.go/types.go.
use super::*;

pub(super) struct Footprint {
    nodes: Vec<NodeId>,
    containers: Vec<usize>,
    children: Vec<(i64, usize)>,
    aggregates: Vec<(i64, usize)>,
    vessels: Vec<usize>,
}
impl Footprint {
    pub(super) fn capture(graph: &ArenaGraph, node: NodeId) -> Self {
        // Same closure as translate_node_with_children_walk. Its topology is
        // stable for the lifetime of this subgraph's cancellation-only trials.
        let active_descendants = if graph.nodes[node.0 as usize].scoring_is_container
            && !graph.nodes[node.0 as usize].is_container
        {
            Vec::new()
        } else {
            graph.active_descendants(node)
        };
        // These are stable owner nodes standing in for aggregate vessels in
        // an ordinary container's recovered Containers traversal. Keep the
        // distinction from `node` itself: optimizer operations may move one
        // real cluster member directly, and that must not translate every
        // sibling merely because the member is also the Rust vessel proxy.
        // Membership sets are hashed. `moved` is applied below in ascending
        // NodeId order, exactly as the BTreeSet iterated it: nodes sharing a
        // point refresh their rect origins in that order.
        let aggregate_descendant_proxies =
            active_descendants.iter().copied().collect::<IdHashSet<_>>();
        let mut moved = active_descendants
            .into_iter()
            .chain(std::iter::once(node))
            .collect::<IdHashSet<_>>();
        // TALA's induced graphs retain Node pointers shared with their owning
        // Containers map. Rust clones those projections, so translate every
        // projected copy of the moved subtree too, including hidden aggregate
        // vessels that have no stable NodeId.
        let mut moved_tala_ids = moved
            .iter()
            .map(|moved| graph.nodes[moved.0 as usize].tala_id)
            .collect::<IdHashSet<_>>();
        // A cluster vessel is a distinct Go Node pointer. The first retained
        // member is only its stable-arena carrier; moving that member directly
        // must not move the vessel. Record vessel traversal at the point where
        // getAllDescendantNodes actually crosses the synthetic vessel instead
        // of inferring it later from the carrier appearing in `moved`.
        let mut moved_cluster_vessels = IdHashSet::default();
        // The closure below grows both sets; reserve once instead of
        // rehashing through each small size.
        moved.reserve(32);
        moved_tala_ids.reserve(32);

        loop {
            let mut changed = false;
            for (cluster_index, cluster) in graph.clusters.iter().enumerate() {
                // Recovered Node.moveNodeWithChildren calls
                // getAllDescendantNodes(node, true).  The `true` branch
                // appends every node owned by a cluster vessel.  Rust keeps
                // the vessel as its first stable member, so recognize that
                // proxy in addition to projections that carry the synthetic
                // vessel TALA ID.
                let owner_is_descendant = cluster
                    .members
                    .first()
                    .is_some_and(|owner| aggregate_descendant_proxies.contains(owner));
                // A nested scope has already materialized the Go vessel as a
                // pending position even when this stable arena still lists
                // every original member in node_order. In that state
                // cluster_is_active() is false, but the shared Go
                // Containers walk still reaches the distinct vessel pointer
                // and translates it with its containing node.
                let vessel_is_pending = graph
                    .pending_cluster_vessel_positions
                    .contains_key(&cluster_index);
                if !(moved_tala_ids.contains(&cluster.vessel_tala_id)
                    || owner_is_descendant
                        && (graph.cluster_is_active(cluster) || vessel_is_pending))
                {
                    continue;
                }
                moved_cluster_vessels.insert(cluster_index);
                for member in &cluster.members {
                    changed |= moved.insert(*member);
                    changed |= moved_tala_ids.insert(graph.nodes[member.0 as usize].tala_id);
                    // AddClusters can absorb an already-materialized sequence
                    // vessel as one cluster member. Go's independent cluster
                    // and sequence passes then recurse through that vessel as
                    // well, so carry its synthetic ID into the next closure
                    // iteration and expose every hidden sequence step.
                    if let Some(sequence_index) = graph.active_sequence_index(*member)
                        && graph.sequences[sequence_index].members.first() == Some(member)
                    {
                        changed |=
                            moved_tala_ids.insert(graph.sequences[sequence_index].vessel_tala_id);
                    }
                }
            }
            for sequence in &graph.sequences {
                // Sequences are an independent pass in
                // getAllDescendantNodes and likewise expose every step when
                // includeClusterNodes is true.
                if !(moved_tala_ids.contains(&sequence.vessel_tala_id)
                    || graph.sequence_is_active(sequence)
                        && sequence
                            .members
                            .first()
                            .is_some_and(|owner| aggregate_descendant_proxies.contains(owner)))
                {
                    continue;
                }
                for member in &sequence.members {
                    changed |= moved.insert(*member);
                    changed |= moved_tala_ids.insert(graph.nodes[member.0 as usize].tala_id);
                }
            }
            for (&container_tala_id, children) in &graph.transaction_external_container_children {
                if !moved_tala_ids.contains(&container_tala_id) {
                    continue;
                }
                for child in children {
                    changed |= moved_tala_ids.insert(child.tala_id);
                    if let Some(stable) = graph
                        .nodes
                        .iter()
                        .position(|candidate| candidate.tala_id == child.tala_id)
                    {
                        changed |= moved.insert(NodeId(stable as u32));
                    }
                }
            }
            for (&container_tala_id, children) in &graph.transaction_external_aggregate_children {
                if !moved_tala_ids.contains(&container_tala_id) {
                    continue;
                }
                for child in children {
                    changed |= moved_tala_ids.insert(child.tala_id);
                    if let Some(stable) = graph
                        .nodes
                        .iter()
                        .position(|candidate| candidate.tala_id == child.tala_id)
                    {
                        changed |= moved.insert(NodeId(stable as u32));
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let mut nodes = moved.into_iter().collect::<Vec<_>>();
        nodes.sort_unstable();
        Self {
            nodes,
            containers: graph
                .transaction_external_containers
                .iter()
                .enumerate()
                .filter_map(|(i, n)| moved_tala_ids.contains(&n.tala_id).then_some(i))
                .collect(),
            children: graph
                .transaction_external_container_children
                .iter()
                .flat_map(|(&key, ns)| {
                    ns.iter()
                        .enumerate()
                        .filter_map(|(i, n)| {
                            moved_tala_ids.contains(&n.tala_id).then_some((key, i))
                        })
                        .collect::<Vec<_>>()
                })
                .collect(),
            aggregates: graph
                .transaction_external_aggregate_children
                .iter()
                .flat_map(|(&key, ns)| {
                    ns.iter()
                        .enumerate()
                        .filter_map(|(i, n)| {
                            moved_tala_ids.contains(&n.tala_id).then_some((key, i))
                        })
                        .collect::<Vec<_>>()
                })
                .collect(),
            vessels: graph
                .pending_cluster_vessel_positions
                .keys()
                .copied()
                .filter(|i| moved_cluster_vessels.contains(i))
                .collect(),
        }
    }
    pub(super) fn apply(&self, graph: &mut ArenaGraph, delta: Point) {
        if delta.x == 0.0 && delta.y == 0.0 {
            return;
        }
        let translate = |n: &mut ArenaNode| {
            if let Some(p) = n.position.as_mut() {
                p.x += delta.x;
                p.y += delta.y;
                n.rect.origin = *p;
            }
        };
        // Preserve the existing projection and shared-point write order.
        for &(key, i) in &self.children {
            translate(
                &mut graph
                    .transaction_external_container_children
                    .get_mut(&key)
                    .unwrap()[i],
            );
        }
        for &(key, i) in &self.aggregates {
            translate(
                &mut graph
                    .transaction_external_aggregate_children
                    .get_mut(&key)
                    .unwrap()[i],
            );
        }
        for &i in &self.containers {
            translate(&mut graph.transaction_external_containers[i]);
        }
        for i in &self.vessels {
            let p = graph.pending_cluster_vessel_positions.get_mut(i).unwrap();
            p.x += delta.x;
            p.y += delta.y;
        }
        for &node in &self.nodes {
            if graph.position(node).is_some() {
                graph.translate_owner_point(node, delta.x, delta.y);
            }
        }
    }
}

pub(super) fn detached(graph: &ArenaGraph, owner: &ArenaGraph) -> bool {
    let points = (0..graph.nodes.len())
        .filter_map(|i| {
            graph
                .owner_node_point(NodeId(i as u32))
                .ok()?
                .shared_point()
                .map(|p| p.allocation_key())
        })
        .collect::<IdHashSet<_>>();
    points.is_empty()
        || !(0..owner.nodes.len()).any(|i| {
            owner
                .owner_node_point(NodeId(i as u32))
                .ok()
                .and_then(|p| p.shared_point())
                .is_some_and(|p| points.contains(&p.allocation_key()))
        })
}

pub(super) struct Journal {
    points: Vec<(NodeId, shared_topology::OwnerNodePointSnapshot)>,
    nodes: Vec<(NodeId, Option<Point>, Point)>,
    containers: Vec<(usize, Option<Point>, Point)>,
    children: Vec<(i64, usize, Option<Point>, Point)>,
    aggregates: Vec<(i64, usize, Option<Point>, Point)>,
    vessels: Vec<(usize, Point)>,
}
impl Journal {
    pub(super) fn capture(graph: &ArenaGraph, footprints: &[Footprint]) -> Self {
        let mut nodes = BTreeSet::new();
        let mut containers = BTreeSet::new();
        let mut children = BTreeSet::new();
        let mut aggregates = BTreeSet::new();
        let mut vessels = BTreeSet::new();
        for f in footprints {
            nodes.extend(f.nodes.iter().copied());
            containers.extend(f.containers.iter().copied());
            children.extend(f.children.iter().copied());
            aggregates.extend(f.aggregates.iter().copied());
            vessels.extend(f.vessels.iter().copied());
        }
        Self {
            points: nodes
                .iter()
                .filter_map(|&n| graph.owner_node_point(n).ok().map(|p| (n, p.snapshot())))
                .collect(),
            nodes: nodes
                .into_iter()
                .map(|id| {
                    let n = &graph.nodes[id.0 as usize];
                    (id, n.position, n.rect.origin)
                })
                .collect(),
            containers: containers
                .into_iter()
                .map(|i| {
                    let n = &graph.transaction_external_containers[i];
                    (i, n.position, n.rect.origin)
                })
                .collect(),
            children: children
                .into_iter()
                .map(|(k, i)| {
                    let n = &graph.transaction_external_container_children[&k][i];
                    (k, i, n.position, n.rect.origin)
                })
                .collect(),
            aggregates: aggregates
                .into_iter()
                .map(|(k, i)| {
                    let n = &graph.transaction_external_aggregate_children[&k][i];
                    (k, i, n.position, n.rect.origin)
                })
                .collect(),
            vessels: vessels
                .into_iter()
                .map(|i| (i, graph.pending_cluster_vessel_positions[&i]))
                .collect(),
        }
    }
    pub(super) fn restore(&self, graph: &mut ArenaGraph) {
        for (id, p) in &self.points {
            graph
                .restore_owner_node_point(*id, p)
                .expect("captured compaction point");
        }
        for &(id, p, o) in &self.nodes {
            let n = &mut graph.nodes[id.0 as usize];
            n.position = p;
            n.rect.origin = o;
        }
        for &(i, p, o) in &self.containers {
            let n = &mut graph.transaction_external_containers[i];
            n.position = p;
            n.rect.origin = o;
        }
        for &(k, i, p, o) in &self.children {
            let n = &mut graph
                .transaction_external_container_children
                .get_mut(&k)
                .unwrap()[i];
            n.position = p;
            n.rect.origin = o;
        }
        for &(k, i, p, o) in &self.aggregates {
            let n = &mut graph
                .transaction_external_aggregate_children
                .get_mut(&k)
                .unwrap()[i];
            n.position = p;
            n.rect.origin = o;
        }
        for &(i, p) in &self.vessels {
            *graph.pending_cluster_vessel_positions.get_mut(&i).unwrap() = p;
        }
    }
}
