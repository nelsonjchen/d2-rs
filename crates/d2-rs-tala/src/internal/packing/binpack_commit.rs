// SPDX-License-Identifier: MPL-2.0
//! layoutgraph/transaction.go::Commit validation for the live packing trial.
//! Candidate lists are actual sweep results consumed by directional validation.
use super::*;
use crate::internal::owner_groups::OwnerGroupKind;
const NIL: &str = "runtime error: invalid memory address or nil pointer dereference";
type PairSet = BTreeSet<(NodeId, NodeId)>;
#[derive(Clone)]
pub(in crate::internal) struct CommitState {
    order: Vec<PackingNodeRef>,
    geometry: crate::internal::IdHashMap<PackingNodeRef, (Option<Point>, Size)>,
    has_fixed: bool,
}
impl CommitState {
    pub(in crate::internal) fn begin_capture(graph: &ArenaGraph) -> Self {
        Self {
            order: primary(graph),
            geometry: Default::default(),
            has_fixed: false,
        }
    }
    pub(in crate::internal) fn record_geometry(
        &mut self,
        graph: &ArenaGraph,
        node: PackingNodeRef,
    ) {
        let (point, size, metadata) = graph.packing_bounds_geometry(node);
        self.geometry.insert(node, (point, size));
        self.has_fixed |= metadata.is_some_and(|node| node.fixed_top_left.is_some());
    }
    pub(super) fn capture(graph: &ArenaGraph, state: &PackingState) -> Self {
        let order = primary(graph);
        Self {
            order,
            geometry: state.commit_geometry.clone(),
            has_fixed: state.captured_has_fixed,
        }
    }
}
fn primary(graph: &ArenaGraph) -> Vec<PackingNodeRef> {
    graph
        .node_order
        .iter()
        .map(|&n| graph.packing_effective_node(n))
        .collect()
}
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
fn pair(graph: &ArenaGraph, a: PackingNodeRef, b: PackingNodeRef) -> (NodeId, NodeId) {
    let carrier = |n| match n {
        PackingNodeRef::Node(n) => n,
        PackingNodeRef::Cluster(i) => graph.clusters[i].members[0],
        PackingNodeRef::Sequence(i) => graph.sequences[i].members[0],
    };
    let (a, b) = (carrier(a), carrier(b));
    if a < b { (a, b) } else { (b, a) }
}
fn finite(graph: &ArenaGraph, n: PackingNodeRef) -> bool {
    let (p, s, _) = graph.packing_bounds_geometry(n);
    p.is_some_and(|p| {
        p.x.is_finite()
            && p.y.is_finite()
            && s.width.is_finite()
            && s.height.is_finite()
            && s.width >= 0.
            && s.height >= 0.
            && (p.x + s.width).is_finite()
            && (p.y + s.height).is_finite()
    })
}
fn candidates(
    graph: &ArenaGraph,
    nodes: &[PackingNodeRef],
    dirty: &[bool],
    dirty_only: bool,
    operation_context: Option<&dyn limits::Context>,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<Vec<Vec<usize>>, limits::Error> {
    if nodes.len() > 10000 {
        return Err(limits::Error::Invalid(
            "TALA transaction node count exceeds limit 10000".into(),
        ));
    }
    let mut result = vec![Vec::new(); nodes.len()];
    // Each node's bounds, read once; the graph is not mutated here.
    let geometry = nodes
        .iter()
        .map(|&n| {
            let (point, size, _) = graph.packing_bounds_geometry(n);
            (point, size)
        })
        .collect::<Vec<_>>();
    let interacts_at = |i: usize, j: usize| {
        if nodes[i] == nodes[j] {
            return false;
        }
        let ((pa, sa), (pb, sb)) = (geometry[i], geometry[j]);
        let (Some(pa), Some(pb)) = (pa, pb) else {
            return false;
        };
        !(pa.x > pb.x + sb.width + 500.
            || pa.x + sa.width + 500. < pb.x
            || pa.y > pb.y + sb.height + 500.
            || pa.y + sa.height + 500. < pb.y)
    };
    let mut invalid = vec![false; nodes.len()];
    let mut sweep = Vec::new();
    let mut dirty_count = 0;
    for (i, &n) in nodes.iter().enumerate() {
        guard.step()?;
        guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
        dirty_count += usize::from(dirty_only && dirty[i]);
        if graph.packing_bounds_geometry(n).0.is_none() {
            continue;
        }
        if !finite(graph, n) {
            invalid[i] = true
        } else {
            sweep.push(i)
        }
    }
    let mut retained = 0usize;
    let mut add = |a: usize, b: usize| -> Result<(), limits::Error> {
        if retained > 4000000 - 2 {
            return Err(limits::Error::Invalid(
                "TALA transaction overlap references exceed limit 4000000".into(),
            ));
        }
        result[a].push(b);
        result[b].push(a);
        retained += 2;
        Ok(())
    };
    if dirty_only && dirty_count * 4 < nodes.len() {
        for (i, &a) in nodes.iter().enumerate() {
            if !dirty[i] || geometry[i].0.is_none() {
                continue;
            }
            for (j, &b) in nodes.iter().enumerate() {
                guard.step()?;
                if i == j || a == b || geometry[j].0.is_none() || (dirty[j] && j < i) {
                    continue;
                }
                if interacts_at(i, j) {
                    add(i, j)?
                }
            }
        }
    } else {
        sweep.sort_by(|&i, &j| {
            let x = geometry[i].0.expect(NIL).x;
            let y = geometry[j].0.expect(NIL).x;
            if x != y {
                if x < y {
                    std::cmp::Ordering::Less
                } else if y < x {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            } else {
                graph
                    .packing_ref_id(nodes[i])
                    .cmp(&graph.packing_ref_id(nodes[j]))
                    .then(i.cmp(&j))
            }
        });
        let mut active = Vec::<usize>::new();
        for i in sweep {
            guard.step()?;
            let mut kept = Vec::new();
            for j in active {
                guard.step()?;
                let (p, s) = geometry[j];
                if p.expect(NIL).x + s.width + 500. < geometry[i].0.expect(NIL).x {
                    continue;
                }
                kept.push(j);
                if !interacts_at(j, i) || (dirty_only && !dirty[j] && !dirty[i]) {
                    continue;
                }
                add(j, i)?
            }
            kept.push(i);
            active = kept;
        }
        if invalid.iter().any(|&v| v) {
            for i in 0..nodes.len() {
                for j in i + 1..nodes.len() {
                    if !invalid[i] && !invalid[j] || dirty_only && !dirty[i] && !dirty[j] {
                        continue;
                    }
                    guard.step()?;
                    if interacts_at(i, j) {
                        add(i, j)?
                    }
                }
            }
        }
    }
    for list in &mut result {
        guard.step()?;
        list.sort_unstable();
    }
    guard.finish()?;
    Ok(result)
}
fn descendants(
    graph: &ArenaGraph,
    root: PackingNodeRef,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<BTreeSet<PackingNodeRef>, limits::Error> {
    // Source descendant walks retain raw member pointers. Projecting a
    // member back to its vessel collapses distinct objects into the root,
    // skips emitted descendants and undercharges the real traversal.
    let mut seen = BTreeSet::from([root]);
    let mut stack = Vec::new();
    let mut output = Vec::new();
    let push = |parent: PackingNodeRef,
                stack: &mut Vec<Option<PackingNodeRef>>,
                guard: &mut limits::WorkGuard<'_>|
     -> Result<(), limits::Error> {
        for (i, group) in graph.sequences.iter().enumerate() {
            if graph.packing_vessel(false, i) == parent {
                for &n in group.members.iter().rev() {
                    guard.step()?;
                    stack.push(Some(PackingNodeRef::Node(n)));
                }
            }
        }
        for (i, group) in graph.clusters.iter().enumerate() {
            if graph.packing_vessel(true, i) == parent {
                for &n in group.members.iter().rev() {
                    guard.step()?;
                    stack.push(Some(PackingNodeRef::Node(n)));
                }
            }
        }
        if let Some(n) = physical(parent) {
            if graph.nodes[n.0 as usize].is_container {
                for n in graph
                    .containers
                    .get(&Some(n))
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                {
                    guard.step()?;
                    stack.push(Some(PackingNodeRef::Node(n)));
                }
            }
        }
        Ok(())
    };
    push(root, &mut stack, guard)?;
    while let Some(n) = stack.pop() {
        guard.step()?;
        let Some(n) = n else { continue };
        if !seen.insert(n) {
            continue;
        }
        output.push(n);
        push(n, &mut stack, guard)?;
    }
    let mut result = BTreeSet::new();
    for n in output {
        guard.step()?;
        result.insert(n);
    }
    guard.finish()?;
    Ok(result)
}
fn exceptions(
    graph: &ArenaGraph,
    node: PackingNodeRef,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<BTreeSet<PackingNodeRef>, limits::Error> {
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
    let is_container = physical(node).is_some_and(|n| graph.nodes[n.0 as usize].is_container);
    let mut result = if is_container || is_vessel {
        descendants(graph, node, guard)?
    } else {
        BTreeSet::new()
    };
    let needs = graph.packing_raw_container(node).is_some()
        || raw_group(graph, node, OwnerGroupKind::Cluster)
        || raw_group(graph, node, OwnerGroupKind::Sequence);
    if needs {
        let mut seen = BTreeSet::new();
        let mut current = Some(node);
        let mut ancestors = Vec::new();
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
                ancestors.push(n)
            }
            current = parent(graph, n, false);
        }
        guard.finish()?;
        for n in ancestors {
            guard.step()?;
            result.insert(n);
        }
    }
    Ok(result)
}
fn delta(
    graph: &ArenaGraph,
    a: PackingNodeRef,
    b: PackingNodeRef,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<f64, limits::Error> {
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
    let (pa, sa, ma) = graph.packing_bounds_geometry(a);
    let (pb, sb, mb) = graph.packing_bounds_geometry(b);
    let pa = pa.expect(NIL);
    let pb = pb.expect(NIL);
    let d = graph.spacing_delta_for_geometry(
        pa,
        sa,
        pb,
        sb,
        ma.is_some_and(|n| n.shape == ShapeKind::SqlTable)
            || mb.is_some_and(|n| n.shape == ShapeKind::SqlTable),
        ma.map(|n| n.layout_margins).unwrap_or(Insets::uniform(0.)),
        mb.map(|n| n.layout_margins).unwrap_or(Insets::uniform(0.)),
        (connected, width, height),
    );
    Ok(ArenaGraph::spacing_delta_with_loop_geometry(
        d,
        pa,
        sa,
        pb,
        sb,
        ma.and_then(|n| n.loop_offsets.as_ref()),
        mb.and_then(|n| n.loop_offsets.as_ref()),
    ))
}
fn exact(graph: &ArenaGraph, a: PackingNodeRef, b: PackingNodeRef) -> bool {
    if a == b {
        return false;
    }
    let (pa, sa, _) = graph.packing_bounds_geometry(a);
    let (pb, sb, _) = graph.packing_bounds_geometry(b);
    let (Some(pa), Some(pb)) = (pa, pb) else {
        return false;
    };
    pa.x < pb.x + sb.width
        && pa.x + sa.width > pb.x
        && pa.y < pb.y + sb.height
        && pa.y + sa.height > pb.y
}
impl CommitState {
    pub(super) fn validate(
        &self,
        graph: &ArenaGraph,
        overlaps: &PairSet,
        exact_overlaps: &PairSet,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.validate_with_options(graph, overlaps, exact_overlaps, true, guard)
    }
    pub(super) fn original_point(&self, node: NodeId) -> Option<Point> {
        self.geometry
            .get(&PackingNodeRef::Node(node))
            .and_then(|v| v.0)
    }
    pub(super) fn validate_with_options(
        &self,
        graph: &ArenaGraph,
        overlaps: &PairSet,
        exact_overlaps: &PairSet,
        ignore_container_escape: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.validate_with_context_options(
            graph,
            overlaps,
            exact_overlaps,
            ignore_container_escape,
            None,
            guard,
        )
    }
    pub(in crate::internal) fn validate_with_context_options(
        &self,
        graph: &ArenaGraph,
        overlaps: &PairSet,
        exact_overlaps: &PairSet,
        ignore_container_escape: bool,
        operation_context: Option<&dyn limits::Context>,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(TransactionValidate);
        let nodes = primary(graph);
        let same_order = nodes == self.order;
        let mut dirty = vec![true; nodes.len()];
        // Changed canonical nodes as dense flags; vessels and out-of-range
        // refs keep the hash set. Membership is all the overlap pass asks.
        let mut changed_nodes = vec![false; graph.nodes.len()];
        let mut changed = crate::internal::IdHashSet::default();
        if same_order {
            for (i, &n) in nodes.iter().enumerate() {
                guard.step()?;
                guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
                let (p, s, _) = graph.packing_bounds_geometry(n);
                dirty[i] = self
                    .geometry
                    .get(&n)
                    .is_none_or(|&(op, os)| op != p || os != s);
                if dirty[i] {
                    match n {
                        PackingNodeRef::Node(id) if (id.0 as usize) < changed_nodes.len() => {
                            changed_nodes[id.0 as usize] = true;
                        }
                        _ => {
                            changed.insert(n);
                        }
                    }
                }
            }
            guard.finish()?;
        }
        let mut origins = BTreeMap::new();
        if self.has_fixed {
            for &n in &nodes {
                guard.step()?;
                guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
                let (p, _, meta) = graph.packing_bounds_geometry(n);
                if let (Some(p), Some(f)) = (p, meta.and_then(|m| m.fixed_top_left)) {
                    origins.entry(parent(graph, n, true)).or_insert(Point {
                        x: p.x - f.x,
                        y: p.y - f.y,
                    });
                }
            }
        }
        guard.finish()?;
        for &n in &nodes {
            guard.step()?;
            guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
            let (p, _, meta) = graph.packing_bounds_geometry(n);
            let Some(p) = p else { continue };
            guard.step()?;
            if raw_group(graph, n, OwnerGroupKind::Cluster) {
                continue;
            }
            if !ignore_container_escape && containment_bad(graph, n, guard)? {
                return Ok(false);
            }
            if origins
                .get(&parent(graph, n, true))
                .is_some_and(|o| p.x < o.x || p.y < o.y)
            {
                return Ok(false);
            }
            if meta.is_some_and(|m| m.fixed_top_left.is_some()) {
                let original = self.geometry.get(&n).and_then(|v| v.0).unwrap_or_default();
                if p.x != original.x || p.y != original.y {
                    return Ok(false);
                }
            }
        }
        let lists = candidates(graph, &nodes, &dirty, same_order, operation_context, guard)?;
        for (i, &n) in nodes.iter().enumerate() {
            if lists[i].is_empty() {
                continue;
            }
            guard.step()?;
            guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
            let (p, size, _) = graph.packing_bounds_geometry(n);
            let Some(p) = p else { continue };
            if raw_group(graph, n, OwnerGroupKind::Cluster) {
                continue;
            }
            let except = exceptions(graph, n, guard)?;
            let right = p.x + size.width;
            let bottom = p.y + size.height;
            for &j in &lists[i] {
                guard.step()?;
                let other = nodes[j];
                if overlaps.contains(&pair(graph, n, other)) || except.contains(&other) {
                    continue;
                }
                // interacts(n, other) with n's bounds already in hand; the
                // graph is not mutated in this loop.
                let (q, s, _) = graph.packing_bounds_geometry(other);
                let Some(q) = q else {
                    continue;
                };
                if n == other
                    || p.x > q.x + s.width + 500.
                    || p.x + size.width + 500. < q.x
                    || p.y > q.y + s.height + 500.
                    || p.y + size.height + 500. < q.y
                {
                    continue;
                }
                let d = delta(graph, n, other, guard)?;
                if p.x < q.x + s.width + d
                    && right + d > q.x
                    && p.y < q.y + s.height + d
                    && bottom + d > q.y
                {
                    return Ok(false);
                }
            }
            guard.finish()?;
        }
        // Each node's partners in pair order, visited by ascending node id:
        // the BTreeMap<NodeId, Vec<NodeId>> adjacency, laid out in two passes.
        let size = overlaps
            .iter()
            .map(|&(a, b)| a.0.max(b.0) as usize + 1)
            .max()
            .unwrap_or(0);
        let mut offsets = vec![0usize; size + 1];
        for &(a, b) in overlaps {
            offsets[a.0 as usize + 1] += 1;
            offsets[b.0 as usize + 1] += 1;
        }
        for index in 0..size {
            offsets[index + 1] += offsets[index];
        }
        let mut cursor = offsets.clone();
        let mut partners = vec![NodeId(0); offsets[size]];
        for &(a, b) in overlaps {
            partners[cursor[a.0 as usize]] = b;
            cursor[a.0 as usize] += 1;
            partners[cursor[b.0 as usize]] = a;
            cursor[b.0 as usize] += 1;
        }
        let is_changed = |n: NodeId| match changed_nodes.get(n.0 as usize) {
            Some(&flag) => flag,
            None => changed.contains(&PackingNodeRef::Node(n)),
        };
        for index in 0..size {
            let others = &partners[offsets[index]..offsets[index + 1]];
            if others.is_empty() {
                continue;
            }
            let a = NodeId(index as u32);
            guard.step()?;
            guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
            for &b in others {
                if same_order && !is_changed(a) && !is_changed(b) {
                    continue;
                }
                let (a, b) = (PackingNodeRef::Node(a), PackingNodeRef::Node(b));
                guard.step()?;
                guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
                if !exact_overlaps.contains(&pair(graph, a, b)) && exact(graph, a, b) {
                    return Ok(false);
                }
            }
        }
        guard.check_context_with(operation_context.unwrap_or_else(|| guard.context()))?;
        guard.finish()?;
        Ok(true)
    }
}

/// layoutgraph/graph.go::isBadStateContext with a nil GraphState. This path
/// intentionally scans Graph.Nodes rather than Commit's sweep candidates.
pub(super) fn standalone_bad_state(
    graph: &ArenaGraph,
    node: PackingNodeRef,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    if graph.packing_bounds_geometry(node).0.is_none() {
        return Err(limits::Error::Invalid(
            "layout invariant violated: transaction bad-state check received an incomplete node"
                .into(),
        ));
    }
    guard.step()?;
    if raw_group(graph, node, OwnerGroupKind::Cluster) {
        return guard.finish().map(|()| false);
    }
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
    let owning = parent(graph, node, true);
    let origin = primary(graph).into_iter().find_map(|n| {
        // Only a fixed node yields an origin, so test that before the
        // parent walk; every check here is a pure read.
        let PackingNodeRef::Node(id) = n else {
            return None;
        };
        if graph.nodes[id.0 as usize].fixed_top_left.is_none() || parent(graph, n, true) != owning {
            return None;
        }
        let (p, _, meta) = graph.packing_bounds_geometry(n);
        let fixed = meta?.fixed_top_left?;
        let p = p?;
        Some(Point {
            x: p.x - fixed.x,
            y: p.y - fixed.y,
        })
    });
    let point = graph.packing_bounds_geometry(node).0.expect(NIL);
    if origin.is_some_and(|origin| point.x < origin.x || point.y < origin.y) {
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
        graph.packing_descendants_checked(node, guard)?
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
    let (p, size, _) = graph.packing_bounds_geometry(node);
    let p = p.expect(NIL);
    let right = p.x + size.width;
    let bottom = p.y + size.height;
    let mut except = BTreeSet::new();
    for exception in exceptions {
        guard.step()?;
        except.insert(exception);
    }
    // `interacts(node, other)` against the moved node's bounds. Nothing in
    // the loop mutates the graph, so each node's geometry is read once.
    let near = |q: Point, s: Size| {
        !(p.x > q.x + s.width + 500.
            || p.x + size.width + 500. < q.x
            || p.y > q.y + s.height + 500.
            || p.y + size.height + 500. < q.y)
    };
    for other in primary(graph) {
        guard.step()?;
        if other == node {
            continue;
        }
        let (q, s, _) = graph.packing_bounds_geometry(other);
        let Some(q) = q else {
            continue;
        };
        if !near(q, s) || except.contains(&other) {
            continue;
        }
        let d = delta(graph, node, other, guard)?;
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

// Source pre-sync validation retains GraphState fixed positions and overlap exceptions.
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

pub(super) fn transaction_container_bad_state(
    graph: &ArenaGraph,
    node: PackingNodeRef,
    original_point: Option<Point>,
    overlaps: &PairSet,
    ignore_container_escape: bool,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, limits::Error> {
    if graph.packing_bounds_geometry(node).0.is_none() {
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
    // Graph.Nodes, read once for the origin search and the overlap scan.
    let primary = transaction_primary(graph);
    let origin = primary.iter().copied().find_map(|id| {
        // Only a fixed node yields an origin, so test that before the
        // parent walk; every check here is a pure read.
        let n = PackingNodeRef::Node(id);
        if graph.nodes[id.0 as usize].fixed_top_left.is_none() || parent(graph, n, true) != owning {
            return None;
        }
        let fixed = graph.nodes[id.0 as usize].fixed_top_left?;
        let p = transaction_node_geometry(graph, id).0?;
        Some(Point {
            x: p.x - fixed.x,
            y: p.y - fixed.y,
        })
    });
    let point = graph.packing_bounds_geometry(node).0.expect(NIL);
    if origin.is_some_and(|origin| point.x < origin.x || point.y < origin.y) {
        return Ok(true);
    }
    if physical(node).is_some_and(|n| graph.nodes[n.0 as usize].fixed_top_left.is_some())
        && point != original_point.unwrap_or_default()
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
        graph.packing_descendants_checked(node, guard)?
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
    let (p, size, _) = graph.packing_bounds_geometry(node);
    let p = p.expect(NIL);
    let right = p.x + size.width;
    let bottom = p.y + size.height;
    let mut except = BTreeSet::new();
    for exception in exceptions {
        guard.step()?;
        except.insert(exception);
    }
    // `interacts(node, other)` against the moved node's bounds. Nothing in
    // the loop mutates the graph, so each node's geometry is read once.
    let near = |q: Point, s: Size| {
        !(p.x > q.x + s.width + 500.
            || p.x + size.width + 500. < q.x
            || p.y > q.y + s.height + 500.
            || p.y + size.height + 500. < q.y)
    };
    for other_id in primary {
        guard.step()?;
        let other = PackingNodeRef::Node(other_id);
        if other == node {
            continue;
        }
        let (q, s) = transaction_node_geometry(graph, other_id);
        let Some(q) = q else {
            continue;
        };
        if !near(q, s) || overlaps.contains(&pair(graph, node, other)) || except.contains(&other) {
            continue;
        }
        let d = delta(graph, node, other, guard)?;
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

// transaction.go validates the actual Graph.Nodes pointers. Packing's
// primary() follows dense carrier members and substitutes virtual vessels;
// those are a different inventory after AddCluster/AddSequence publication.
fn transaction_primary(graph: &ArenaGraph) -> Vec<NodeId> {
    graph
        .owner_graph_member_header(super::shared_topology::OwnerGraphId(0))
        .expect("transaction receiver Nodes")
        .visible_slots()
        .into_iter()
        .map(|node| node.expect(NIL))
        .collect()
}

// Transaction.Graph.Nodes contains canonical node pointers, including retained
// vessels. It never contains packing's virtual cluster/sequence references.
// Read these concrete nodes directly, preserving each live owner-point read.
fn transaction_node_geometry(graph: &ArenaGraph, node: NodeId) -> (Option<Point>, Size) {
    (graph.position(node), graph.nodes[node.0 as usize].rect.size)
}
