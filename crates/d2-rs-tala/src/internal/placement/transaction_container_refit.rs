// SPDX-License-Identifier: MPL-2.0
//! transaction.go repositionContainers and Commit's immediately following validation.
use super::shared_topology::OwnerGraphId;
use super::*;
use d2_rs_core::stable_sort::StableSort;

impl ArenaGraph {
    fn transaction_wrap_container(&mut self, container: NodeId) {
        if !self.nodes[container.0 as usize].is_container {
            return;
        }
        // Node.wrapChildren reads the node's Graph, not the transaction receiver.
        let receiver = self
            .topology_node_owner(container)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let children = self
            .owner_container_map(receiver)
            .expect("container Graph")
            .get(Some(container));
        let children = children
            .map(|h| {
                h.iter_slots()
                    .map(|n| {
                        n.expect("runtime error: invalid memory address or nil pointer dereference")
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let padding = self.shape_fit_padding(container);
        let (mut low, mut high) = self
            .fixed_node_bounds(&children)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        for &child in &children {
            let node = &self.nodes[child.0 as usize];
            let position = self.position(child).expect("positioned child");
            if let Some(label) = node.label_size {
                if (position.x == low.x || position.x + node.rect.size.width == high.x)
                    && label.width > node.rect.size.width
                {
                    low.x = low
                        .x
                        .min((position.x + node.rect.size.width / 2.0 - label.width / 2.0).floor());
                    high.x = high
                        .x
                        .max((position.x + node.rect.size.width / 2.0 + label.width / 2.0).ceil());
                }
            }
        }
        let content = Size {
            width: high.x - low.x,
            height: high.y - low.y,
        };
        self.nodes[container.0 as usize].rect.size =
            self.shape_dimensions_to_fit(container, content, padding);
        let inside = self
            .shape_inside_placement_absolute(container, content, padding)
            .expect("positioned container");
        let delta = Point {
            x: low.x - inside.x,
            y: low.y - inside.y,
        };
        self.adjust_projected_offsets_for_refit(container, delta);
        self.translate_owner_point(container, delta.x, delta.y);
    }

    pub(super) fn canonical_transaction_overlap_pairs(
        &self,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
    ) -> BTreeSet<(NodeId, NodeId)> {
        let canonical = self
            .node_order
            .iter()
            .copied()
            .zip(self.original_owner_member_nodes())
            .collect::<BTreeMap<_, _>>();
        overlaps
            .iter()
            .map(|&(a, b)| {
                let a = canonical.get(&a).copied().unwrap_or(a);
                let b = canonical.get(&b).copied().unwrap_or(b);
                if a < b { (a, b) } else { (b, a) }
            })
            .collect()
    }

    pub(super) fn numeric_transaction_overlap_pairs(
        &self,
        pairs: &BTreeSet<(NodeId, NodeId)>,
    ) -> BTreeSet<(NodeId, NodeId)> {
        let numeric = self
            .original_owner_member_nodes()
            .into_iter()
            .zip(self.node_order.iter().copied())
            .collect::<BTreeMap<_, _>>();
        pairs
            .iter()
            .map(|&(a, b)| {
                let a = numeric.get(&a).copied().unwrap_or(a);
                let b = numeric.get(&b).copied().unwrap_or(b);
                if a < b { (a, b) } else { (b, a) }
            })
            .collect()
    }

    pub(super) fn transaction_refit_containers_checked(
        &mut self,
        checkpoint: &GeometryCheckpoint,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.transaction_refit_containers_with_options_checked(
            |node| checkpoint.original_position(node),
            overlaps,
            false,
            Some(context),
            guard,
        )
    }

    pub(super) fn transaction_refit_containers_with_options_checked(
        &mut self,
        original_point: impl Fn(NodeId) -> Option<Point>,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        ignore_container_escape: bool,
        context: Option<&dyn limits::Context>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(RefitContainers);
        let map = self
            .owner_container_map(OwnerGraphId(0))
            .expect("transaction receiver Containers");
        let mut containers = Vec::new();
        for (container, _) in map.live_entries() {
            guard.step()?;
            if let Some(container) = container {
                if self.position(container).is_some() {
                    containers.push(container);
                }
            }
        }
        containers.stable_sort_by(|&a, &b| {
            let a_node = &self.nodes[a.0 as usize];
            let b_node = &self.nodes[b.0 as usize];
            let a_area = a_node.rect.size.width * a_node.rect.size.height;
            let b_area = b_node.rect.size.width * b_node.rect.size.height;
            if a_area == b_area {
                a_node.tala_id.cmp(&b_node.tala_id)
            } else if a_area < b_area {
                std::cmp::Ordering::Less
            } else if b_area < a_area {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        for container in containers {
            if let Some(context) = context {
                guard.check_context_with(context)?;
            } else {
                guard.check_context()?;
            }
            self.transaction_wrap_container(container);
            guard.step()?;
        }
        guard.finish()?;
        // Commit rereads the Containers field after repositionContainers.
        let map = self
            .owner_container_map(OwnerGraphId(0))
            .expect("transaction receiver Containers");
        let mut overlap_index = None;
        let mut primary = Vec::new();
        // Only a FixedTopLeft node yields an origin. Plain node fields are
        // unchanged until this read-only scan returns; resolve them once,
        // including a dense copy of every Box size for the overlap scans.
        let mut any_fixed = None;
        let mut sizes = None;
        for (container, _) in map.live_entries() {
            guard.step()?;
            if let Some(context) = context {
                guard.check_context_with(context)?;
            } else {
                guard.check_context()?;
            }
            if let Some(container) = container {
                if self.position(container).is_none() {
                    continue;
                }
                if refit_container_bad_state(
                    self,
                    container,
                    || original_point(container),
                    overlap_index
                        .get_or_insert_with(|| RefitOverlapIndex::new(overlaps, self.nodes.len())),
                    *any_fixed.get_or_insert_with(|| {
                        self.nodes.iter().any(|node| node.fixed_top_left.is_some())
                    }),
                    sizes.get_or_insert_with(|| {
                        self.nodes
                            .iter()
                            .map(|node| node.rect.size)
                            .collect::<Vec<_>>()
                    }),
                    ignore_container_escape,
                    &mut primary,
                    guard,
                )? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

// Concrete Graph.Nodes validation at the pre-sync container boundary.
use super::binpack_traversal::PackingNodeRef;
use super::owner_groups::OwnerGroupKind;
const NIL: &str = "runtime error: invalid memory address or nil pointer dereference";
type PairSet = BTreeSet<(NodeId, NodeId)>;
fn physical(node: PackingNodeRef) -> Option<NodeId> {
    if let PackingNodeRef::Node(n) = node {
        Some(n)
    } else {
        None
    }
}
fn raw_group(graph: &ArenaGraph, node: PackingNodeRef, kind: OwnerGroupKind) -> bool {
    if let Some(n) = physical(node) {
        match graph.owner_node_group_vessel(n, kind) {
            Ok(p) => p.is_some(),
            Err(_) => match kind {
                OwnerGroupKind::Cluster => graph.nodes[n.0 as usize].cluster.is_some(),
                OwnerGroupKind::Sequence => graph.nodes[n.0 as usize].sequence.is_some(),
            },
        }
    } else {
        false
    }
}
fn parent(graph: &ArenaGraph, node: PackingNodeRef, active: bool) -> Option<PackingNodeRef> {
    let Some(n) = physical(node) else {
        return graph.packing_raw_container(node);
    };
    if !active {
        if let Some(c) = graph.nodes[n.0 as usize].container {
            return Some(PackingNodeRef::Node(c));
        }
    }
    for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
        match graph.owner_node_group_vessel(n, kind) {
            Ok(Some(v)) => {
                if active {
                    let v = v.expect(NIL);
                    if graph.topology_node_owner(v).is_some() {
                        return graph.nodes[v.0 as usize]
                            .container
                            .map(PackingNodeRef::Node);
                    }
                } else {
                    return v.map(PackingNodeRef::Node);
                }
            }
            Ok(None) => {}
            Err(_) => {
                let index = match kind {
                    OwnerGroupKind::Cluster => graph.nodes[n.0 as usize].cluster,
                    OwnerGroupKind::Sequence => graph.nodes[n.0 as usize].sequence,
                };
                if let Some(i) = index {
                    let vessel = graph.packing_vessel(kind == OwnerGroupKind::Cluster, i);
                    if !active {
                        return Some(vessel);
                    }
                    if graph.packing_group_active(kind == OwnerGroupKind::Cluster, i) {
                        return graph.packing_raw_container(vessel);
                    }
                }
            }
        }
    }
    graph.nodes[n.0 as usize]
        .container
        .map(PackingNodeRef::Node)
}
fn refit_delta(
    graph: &ArenaGraph,
    a_id: NodeId,
    b_id: NodeId,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<f64, limits::Error> {
    let a = PackingNodeRef::Node(a_id);
    let b = PackingNodeRef::Node(b_id);
    let mut connected = false;
    let mut width = f64::NEG_INFINITY;
    let mut height = f64::NEG_INFINITY;
    for edge in graph.packing_segment_incidents(a) {
        guard.step()?;
        let edge = edge.ok_or_else(|| {
            limits::Error::Invalid(
                "layout invariant violated: spacing check encountered an incomplete edge".into(),
            )
        })?;
        let from = graph.owner_edge_endpoint(edge, false).ok().flatten();
        let to = graph.owner_edge_endpoint(edge, true).ok().flatten();
        let (Some(from), Some(to)) = (from, to) else {
            return Err(limits::Error::Invalid(
                "layout invariant violated: spacing check encountered an incomplete edge".into(),
            ));
        };
        let from = graph.packing_effective_node(from);
        let to = graph.packing_effective_node(to);
        if (if from == a { to } else { from }) == b {
            connected = true;
            let e = &graph.edge_records[edge.0 as usize];
            width = width.max(e.min_width);
            height = height.max(e.min_height);
        }
    }
    let ma = &graph.nodes[a_id.0 as usize];
    let mb = &graph.nodes[b_id.0 as usize];
    let (pa, sa) = (graph.position(a_id), ma.rect.size);
    let (pb, sb) = (graph.position(b_id), mb.rect.size);
    let pa = pa.expect(NIL);
    let pb = pb.expect(NIL);
    let d = graph.spacing_delta_for_geometry(
        pa,
        sa,
        pb,
        sb,
        ma.shape == ShapeKind::SqlTable || mb.shape == ShapeKind::SqlTable,
        ma.layout_margins,
        mb.layout_margins,
        (connected, width, height),
    );
    Ok(ArenaGraph::spacing_delta_with_loop_geometry(
        d,
        pa,
        sa,
        pb,
        sb,
        ma.loop_offsets.as_ref(),
        mb.loop_offsets.as_ref(),
    ))
}
fn containment_bad(
    graph: &ArenaGraph,
    node: PackingNodeRef,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    let surrounds = |outer: PackingNodeRef, inner: PackingNodeRef| {
        let (a, sa, _) = graph.packing_bounds_geometry(outer);
        let (b, sb, _) = graph.packing_bounds_geometry(inner);
        let a = a.expect(NIL);
        let b = b.expect(NIL);
        b.x > a.x
            && b.y > a.y
            && b.x + sb.width < a.x + sa.width
            && b.y + sb.height < a.y + sa.height
    };
    if let Some(container) = parent(graph, node, true) {
        if graph.packing_bounds_geometry(container).0.is_some() && !surrounds(container, node) {
            return Ok(true);
        }
    }
    let container = physical(node).is_some_and(|n| graph.nodes[n.0 as usize].is_container);
    if container {
        for child in graph.packing_direct_refs(physical(node)) {
            guard.step()?;
            if graph.packing_bounds_geometry(child).0.is_none() {
                return Err(limits::Error::Invalid(
                    "layout invariant violated: transaction container has an incomplete child"
                        .into(),
                ));
            }
            if !surrounds(node, child) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn refit_container_bad_state(
    graph: &ArenaGraph,
    node_id: NodeId,
    original_point: impl FnOnce() -> Option<Point>,
    overlaps: &RefitOverlapIndex<'_>,
    any_fixed: bool,
    sizes: &[Size],
    ignore_container_escape: bool,
    primary: &mut Vec<NodeId>,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    let node = PackingNodeRef::Node(node_id);
    if graph.position(node_id).is_none() {
        return Err(limits::Error::Invalid(
            "layout invariant violated: transaction bad-state check received an incomplete node"
                .into(),
        ));
    }
    guard.step()?;
    if raw_group(graph, node, OwnerGroupKind::Cluster) {
        return guard.finish().map(|()| false);
    }
    if !ignore_container_escape && containment_bad(graph, node, guard)? {
        return Ok(true);
    }
    let container = physical(node).is_some_and(|n| graph.nodes[n.0 as usize].is_container);
    let owning = parent(graph, node, true);
    // Graph.Nodes, read once for the origin search and the overlap scan,
    // into the caller's buffer.
    transaction_primary_into(graph, primary);
    let primary = &*primary;
    // Only a fixed node yields an origin: without any FixedTopLeft node in
    // the graph, every candidate of the search below returns None.
    let origin = if any_fixed {
        primary.iter().copied().find_map(|id| {
            // Only a fixed node yields an origin, so test that before the
            // parent walk; every check here is a pure read.
            let n = PackingNodeRef::Node(id);
            if graph.nodes[id.0 as usize].fixed_top_left.is_none()
                || parent(graph, n, true) != owning
            {
                return None;
            }
            let fixed = graph.nodes[id.0 as usize].fixed_top_left?;
            let p = transaction_node_geometry(graph, id).0?;
            Some(Point {
                x: p.x - fixed.x,
                y: p.y - fixed.y,
            })
        })
    } else {
        None
    };
    let point = graph.position(node_id).expect(NIL);
    if origin.is_some_and(|origin| point.x < origin.x || point.y < origin.y) {
        return Ok(true);
    }
    if physical(node).is_some_and(|n| graph.nodes[n.0 as usize].fixed_top_left.is_some())
        && point != original_point().unwrap_or_default()
    {
        return Ok(true);
    }
    let is_vessel = graph
        .sequences
        .iter()
        .enumerate()
        .any(|(i, _)| graph.packing_vessel(false, i) == node)
        || graph
            .clusters
            .iter()
            .enumerate()
            .any(|(i, _)| graph.packing_vessel(true, i) == node);
    let mut exceptions = if container || is_vessel {
        refit_descendants_checked(graph, node, guard)?
    } else {
        Vec::new()
    };
    if graph.packing_raw_container(node).is_some()
        || raw_group(graph, node, OwnerGroupKind::Cluster)
        || raw_group(graph, node, OwnerGroupKind::Sequence)
    {
        let mut seen = BTreeSet::new();
        let mut current = Some(node);
        while let Some(n) = current {
            guard.step()?;
            if !seen.insert(n) {
                return Err(limits::Error::Invalid(
                    "layout invariant violated: cycle in transaction ancestry".into(),
                ));
            }
            if seen.len() > 10000 {
                return Err(limits::Error::Invalid(
                    "layout invariant violated: transaction ancestry exceeds node limit".into(),
                ));
            }
            if n != node {
                exceptions.push(n);
            }
            current = parent(graph, n, false);
        }
        guard.finish()?;
    }
    let p = graph.position(node_id).expect(NIL);
    let size = graph.nodes[node_id.0 as usize].rect.size;
    let right = p.x + size.width;
    let bottom = p.y + size.height;
    let mut except = RefitNodeSet::new(graph);
    for exception in exceptions {
        guard.step()?;
        except.insert(exception);
    }
    // Both exception sets are immutable during this check. Go resolves the
    // moved node's existing-overlap row once (graph.go:367). Merge that row
    // into the already allocated membership flags without charging work:
    // pairwise map reads never charged work in the original loop.
    for &other in overlaps.row(node_id) {
        except.insert(PackingNodeRef::Node(other));
    }
    // `interacts(node, other)` against the moved node's bounds. Nothing in
    // the loop mutates the graph, so each node's geometry is read once.
    let near = |q: Point, s: Size| {
        !(p.x > q.x + s.width + 500.
            || p.x + size.width + 500. < q.x
            || p.y > q.y + s.height + 500.
            || p.y + size.height + 500. < q.y)
    };
    for &other_id in primary {
        guard.step()?;
        let other = PackingNodeRef::Node(other_id);
        if other == node {
            continue;
        }
        // `sizes[i]` is `graph.nodes[i].rect.size`; the point stays live.
        let (q, s) = (graph.position(other_id), sizes[other_id.0 as usize]);
        let Some(q) = q else {
            continue;
        };
        if !near(q, s)
            || overlaps.out_of_range_contains(node_id, other_id)
            || except.contains(&other)
        {
            continue;
        }
        let d = refit_delta(graph, node_id, other_id, guard)?;
        if p.x < q.x + s.width + d
            && right + d > q.x
            && p.y < q.y + s.height + d
            && bottom + d > q.y
        {
            return Ok(true);
        }
    }
    guard.finish().map(|()| false)
}

fn transaction_primary_into(graph: &ArenaGraph, primary: &mut Vec<NodeId>) {
    primary.clear();
    primary.extend(
        graph
            .owner_graph_member_header(super::shared_topology::OwnerGraphId(0))
            .expect("transaction receiver Nodes")
            .iter_slots()
            .map(|node| node.expect(NIL)),
    );
}

// Transaction.Graph.Nodes contains canonical node pointers, including retained
// vessels. It never contains packing's virtual cluster/sequence references.
// Read these concrete nodes directly, preserving each live owner-point read.
#[inline(always)]
fn transaction_node_geometry(graph: &ArenaGraph, node: NodeId) -> (Option<Point>, Size) {
    (graph.position(node), graph.nodes[node.0 as usize].rect.size)
}

// allDescendantNodesGuarded (graph.go:955–1032) keeps a seen set and a
// reusable child stack. NodeId is a dense canonical allocation here; retain
// the generic set for virtual references and out-of-range invariant witnesses.
struct RefitNodeSet {
    nodes: Vec<bool>,
    other: BTreeSet<PackingNodeRef>,
}
impl RefitNodeSet {
    fn new(graph: &ArenaGraph) -> Self {
        Self {
            nodes: vec![false; graph.nodes.len()],
            other: BTreeSet::new(),
        }
    }
    fn insert(&mut self, node: PackingNodeRef) -> bool {
        if let PackingNodeRef::Node(id) = node {
            if let Some(seen) = self.nodes.get_mut(id.0 as usize) {
                return !std::mem::replace(seen, true);
            }
        }
        self.other.insert(node)
    }
    fn contains(&self, node: &PackingNodeRef) -> bool {
        if let PackingNodeRef::Node(id) = *node {
            if let Some(&seen) = self.nodes.get(id.0 as usize) {
                return seen;
            }
        }
        self.other.contains(node)
    }
}

// Preserve packing's current concrete/virtual member projection and snapshot
// each complete child list before charging work, just as the generic walk does.
// The buffer is local to this read-only walk; nothing is cached across calls.
fn refit_descendant_children(
    graph: &ArenaGraph,
    node: PackingNodeRef,
    children: &mut Vec<PackingNodeRef>,
) {
    children.clear();
    for (i, sequence) in graph.sequences.iter().enumerate() {
        if graph.packing_vessel(false, i) == node {
            children.extend(
                sequence
                    .members
                    .iter()
                    .rev()
                    .map(|&n| PackingNodeRef::Node(n)),
            );
        }
    }
    if let Some(cluster) = graph
        .clusters
        .iter()
        .enumerate()
        .find_map(|(i, cluster)| (graph.packing_vessel(true, i) == node).then_some(cluster))
    {
        // The original helper projects in forward order before reversing.
        let start = children.len();
        children.extend(cluster.members.iter().map(|&member| {
            if graph.bin_pack_node_is_vessel_carrier(member) {
                if let Some(sequence) = graph.active_sequence_index(member) {
                    if graph.sequences[sequence].members.first() == Some(&member) {
                        return graph.packing_vessel(false, sequence);
                    }
                }
            }
            PackingNodeRef::Node(member)
        }));
        children[start..].reverse();
    }
    if let PackingNodeRef::Node(node) = node {
        if graph.nodes[node.0 as usize].is_container {
            children.extend(
                graph
                    .containers
                    .get(&Some(node))
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                    .map(|n| graph.packing_effective_node(n)),
            );
        }
    }
}

fn refit_descendants_checked(
    graph: &ArenaGraph,
    root: PackingNodeRef,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<Vec<PackingNodeRef>, limits::Error> {
    let mut seen = RefitNodeSet::new(graph);
    seen.insert(root);
    let mut stack = Vec::new();
    let mut children = Vec::new();
    let mut result = Vec::new();
    refit_descendant_children(graph, root, &mut children);
    for &child in &children {
        guard.step()?;
        stack.push(child);
    }
    while let Some(node) = stack.pop() {
        guard.step()?;
        if !seen.insert(node) {
            continue;
        }
        result.push(node);
        refit_descendant_children(graph, node, &mut children);
        for &child in &children {
            guard.step()?;
            stack.push(child);
        }
    }
    guard.finish()?;
    Ok(result)
}

#[cfg(test)]
mod refit_walk_tests {
    use super::*;

    #[test]
    fn dense_refit_exceptions_keep_virtual_and_out_of_range_identities() {
        let graph = ArenaGraph::from_input(&Graph::default());
        let mut set = RefitNodeSet::new(&graph);
        for node in [
            PackingNodeRef::Node(NodeId(100)),
            PackingNodeRef::Cluster(100),
            PackingNodeRef::Sequence(100),
        ] {
            assert!(set.insert(node));
            assert!(!set.insert(node));
            assert!(set.contains(&node));
        }
        assert!(!set.contains(&PackingNodeRef::Node(NodeId(101))));
    }

    #[test]
    fn refit_descendants_keep_preorder_and_suppress_duplicate_cycles() {
        let mut input = Graph::default();
        for index in 0..4 {
            input.add_node(crate::Node::new(
                index.to_string(),
                Size {
                    width: 10.,
                    height: 10.,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[0].is_container = true;
        graph.nodes[1].is_container = true;
        graph
            .containers
            .insert(Some(NodeId(0)), vec![NodeId(1), NodeId(2), NodeId(1)]);
        graph
            .containers
            .insert(Some(NodeId(1)), vec![NodeId(3), NodeId(0)]);
        struct Background;
        impl limits::Context for Background {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                None
            }
        }
        let mut guard = limits::WorkGuard::new(Some(&Background), "RefitWalkTest", 100).unwrap();
        assert_eq!(
            refit_descendants_checked(&graph, PackingNodeRef::Node(NodeId(0)), &mut guard).unwrap(),
            vec![
                PackingNodeRef::Node(NodeId(1)),
                PackingNodeRef::Node(NodeId(3)),
                PackingNodeRef::Node(NodeId(2))
            ]
        );
        assert_eq!(guard.used(), 10);
    }
}

// An immutable overlap snapshot, indexed only for this refit invocation.
// Two passes produce sorted adjacency rows without per-node allocations.
// Canonical pairs have a < b; reversed and self entries were never looked up.
struct RefitOverlapIndex<'a> {
    source: &'a PairSet,
    offsets: Vec<usize>,
    members: Vec<NodeId>,
    node_count: usize,
}
impl<'a> RefitOverlapIndex<'a> {
    fn new(source: &'a PairSet, node_count: usize) -> Self {
        if source.is_empty() {
            return Self {
                source,
                offsets: Vec::new(),
                members: Vec::new(),
                node_count,
            };
        }
        let mut offsets = vec![0; node_count + 1];
        for &(a, b) in source {
            if a < b && (b.0 as usize) < node_count {
                offsets[a.0 as usize] += 1;
                offsets[b.0 as usize] += 1;
            }
        }
        let mut total = 0;
        for offset in &mut offsets {
            total += *offset;
            *offset = total;
        }
        let mut members = vec![NodeId(0); total];
        // Fill backwards; offsets are left at the beginning of every row.
        for &(a, b) in source.iter().rev() {
            if a < b && (b.0 as usize) < node_count {
                offsets[a.0 as usize] -= 1;
                members[offsets[a.0 as usize]] = b;
                offsets[b.0 as usize] -= 1;
                members[offsets[b.0 as usize]] = a;
            }
        }
        Self {
            source,
            offsets,
            members,
            node_count,
        }
    }
    fn row(&self, node: NodeId) -> &[NodeId] {
        let i = node.0 as usize;
        if self.offsets.is_empty() || i >= self.node_count {
            return &[];
        }
        &self.members[self.offsets[i]..self.offsets[i + 1]]
    }
    fn out_of_range_contains(&self, a: NodeId, b: NodeId) -> bool {
        ((a.0 as usize) >= self.node_count || (b.0 as usize) >= self.node_count)
            && self.source.contains(&if a < b { (a, b) } else { (b, a) })
    }
}

#[cfg(test)]
mod refit_pair_tests {
    use super::*;

    #[test]
    fn refit_pair_rows_preserve_canonical_identity_and_sparse_membership() {
        let source: PairSet = [
            (0, 1),
            (0, 4),
            (1, 3),
            (1, 4),
            (3, 4),
            (2, 2),
            (4, 1),
            (1, 100),
        ]
        .into_iter()
        .map(|(a, b)| (NodeId(a), NodeId(b)))
        .collect();
        let index = RefitOverlapIndex::new(&source, 6);
        // Self is rejected by the overlap scan before consulting exceptions.
        // Reversed input entries must not create a new canonical exception.
        for a in [0, 1, 2, 3, 4, 5, 6, 100] {
            for b in [0, 1, 2, 3, 4, 5, 6, 100] {
                if a == b {
                    continue;
                }
                let (a, b) = (NodeId(a), NodeId(b));
                assert_eq!(
                    index.row(a).contains(&b) || index.out_of_range_contains(a, b),
                    source.contains(&if a < b { (a, b) } else { (b, a) })
                );
            }
        }
        let reversed_only = PairSet::from([(NodeId(4), NodeId(2))]);
        let index = RefitOverlapIndex::new(&reversed_only, 6);
        assert!(index.row(NodeId(2)).is_empty());
        assert!(index.row(NodeId(4)).is_empty());
        let empty = PairSet::new();
        let index = RefitOverlapIndex::new(&empty, 0);
        assert!(index.row(NodeId(0)).is_empty());
        assert!(!index.out_of_range_contains(NodeId(0), NodeId(1)));
    }
}
