// SPDX-License-Identifier: MPL-2.0
//! packing/guard.go guarded postorder and cluster/sequence geometry work.
use super::*;
use binpack_traversal::PackingNodeRef as Node;
use owner_groups::{OwnerGroupId, OwnerGroupKind as Kind};
use shared_topology::{OwnerContainerHeader, OwnerMemberHeader};
#[derive(Clone, Copy)]
enum Group {
    Owner(OwnerGroupId),
    Cluster(usize),
    Sequence(usize),
}
#[derive(Clone)]
enum Members {
    Owner(OwnerMemberHeader),
    Children(OwnerContainerHeader),
    Compact(Vec<Option<Node>>),
}
impl Members {
    fn len(&self) -> usize {
        match self {
            Self::Owner(h) => h.len(),
            Self::Children(h) => h.len(),
            Self::Compact(v) => v.len(),
        }
    }
    fn get(&self, i: usize) -> Option<Node> {
        match self {
            Self::Owner(h) => h.get(i).expect("group member").map(Node::Node),
            Self::Children(h) => h.get(i).expect("container child").map(Node::Node),
            Self::Compact(v) => v[i],
        }
    }
}
fn invalid(s: &str) -> limits::Error {
    limits::Error::Invalid(format!("layout invariant violated: {s}"))
}
fn maximum(a: f64, b: f64) -> f64 {
    routing::channel_solver::maximum(a, b)
}
fn minimum(a: f64, b: f64) -> f64 {
    routing::channel_solver::minimum(a, b)
}
fn advance(w: f64) -> f64 {
    if w <= 0. {
        0.
    } else if w <= 35. {
        w - w / 2.
    } else {
        w - 35.
    }
}
impl ArenaGraph {
    fn packing_group_compact(&self) -> bool {
        self.clusters.iter().enumerate().any(|(i, _)| {
            self.topology_vessel_reference(TopologyVessel::Cluster(i))
                .is_none()
        }) || self.sequences.iter().enumerate().any(|(i, _)| {
            self.topology_vessel_reference(TopologyVessel::Sequence(i))
                .is_none()
        })
    }
    fn work_graph(&self, n: Node) -> Option<OwnerGraphId> {
        match n {
            Node::Node(n) => self
                .topology_node_owner(n)
                // Once the actual heap exists, Node.Graph=None is a source
                // nil pointer, even if another group has a compact carrier.
                .or_else(|| {
                    (self.topology_arena_id().is_none() && self.packing_group_compact())
                        .then_some(OwnerGraphId(0))
                }),
            _ => Some(OwnerGraphId(0)),
        }
    }
    fn work_vessel(&self, cluster: bool, i: usize) -> Node {
        self.topology_vessel_node(if cluster {
            TopologyVessel::Cluster(i)
        } else {
            TopologyVessel::Sequence(i)
        })
        .map(Node::Node)
        .unwrap_or(if cluster {
            Node::Cluster(i)
        } else {
            Node::Sequence(i)
        })
    }
    fn work_reference(&self, n: NodeId, within: Option<usize>) -> Node {
        for (i, c) in self.clusters.iter().enumerate() {
            if Some(i) != within
                && c.members.first() == Some(&n)
                && self
                    .topology_vessel_reference(TopologyVessel::Cluster(i))
                    .is_none()
                && self.cluster_is_active(c)
            {
                return Node::Cluster(i);
            }
        }
        for (i, s) in self.sequences.iter().enumerate() {
            if s.members.first() == Some(&n)
                && self
                    .topology_vessel_reference(TopologyVessel::Sequence(i))
                    .is_none()
                && self.sequence_is_active(s)
            {
                return Node::Sequence(i);
            }
        }
        Node::Node(n)
    }
    fn work_group(&self, receiver: OwnerGraphId, kind: Kind, n: Node) -> Option<Group> {
        if let Node::Node(n) = n {
            if let Ok(Some(group)) = self.owner_graph_group_entry(receiver, kind, Some(n)) {
                return group.map(Group::Owner);
            }
        }
        match kind {
            Kind::Cluster => {
                self.clusters.iter().enumerate().find_map(|(i, _)| {
                    // A bound carrier mirrors an actual group allocation;
                    // it cannot recreate an absent entry in the selected map.
                    (self
                        .owner_group_binding(TopologyVessel::Cluster(i))
                        .is_none()
                        && self.work_vessel(true, i) == n)
                        .then_some(Group::Cluster(i))
                })
            }
            Kind::Sequence => self.sequences.iter().enumerate().find_map(|(i, _)| {
                (self
                    .owner_group_binding(TopologyVessel::Sequence(i))
                    .is_none()
                    && self.work_vessel(false, i) == n)
                    .then_some(Group::Sequence(i))
            }),
        }
    }
    fn work_group_count(&self, kind: Kind) -> usize {
        let count = self
            .owner_graph_group_map(OwnerGraphId(0), kind)
            .map(|m| m.len())
            .unwrap_or(0);
        count.max(match kind {
            // Only unbound compact groups lack an authoritative map record.
            Kind::Cluster => (0..self.clusters.len())
                .filter(|&i| {
                    self.owner_group_binding(TopologyVessel::Cluster(i))
                        .is_none()
                })
                .count(),
            Kind::Sequence => (0..self.sequences.len())
                .filter(|&i| {
                    self.owner_group_binding(TopologyVessel::Sequence(i))
                        .is_none()
                })
                .count(),
        })
    }
    fn work_is_cluster(&self, n: Node) -> bool {
        match n {
            Node::Cluster(_) => true,
            Node::Sequence(_) => false,
            Node::Node(n) => self.owner_is_cluster_vessel(n),
        }
    }
    fn work_is_container(&self, n: Node) -> bool {
        matches!(n,Node::Node(n)if self.nodes[n.0 as usize].is_container)
    }
    fn work_members(&self, group: Group) -> Members {
        match group {
            Group::Owner(g) => {
                Members::Owner(self.owner_group_fields(g).expect("known group").nodes)
            }
            Group::Sequence(i) => Members::Compact(
                self.sequences[i]
                    .members
                    .iter()
                    .map(|&n| Some(Node::Node(n)))
                    .collect(),
            ),
            Group::Cluster(i) => Members::Compact(
                self.clusters[i]
                    .members
                    .iter()
                    .map(|&n| Some(self.work_reference(n, Some(i))))
                    .collect(),
            ),
        }
    }
    fn work_group_vessel(&self, group: Group) -> Option<Node> {
        match group {
            Group::Owner(g) => self
                .owner_group_fields(g)
                .expect("known group")
                .vessel
                .map(Node::Node),
            Group::Cluster(i) => Some(self.work_vessel(true, i)),
            Group::Sequence(i) => Some(self.work_vessel(false, i)),
        }
    }
    fn work_children(&self, n: Node) -> Members {
        if let Node::Node(node) = n {
            if let Some(owner) = self.work_graph(n) {
                if let Some(map) = self.owner_container_map(owner) {
                    if let Some(header) = map.get(Some(node)) {
                        if !self.packing_group_compact() {
                            return Members::Children(header);
                        }
                    }
                }
            }
            return Members::Compact(
                self.containers
                    .get(&Some(node))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|n| Some(self.work_reference(n, None)))
                    .collect(),
            );
        }
        Members::Compact(Vec::new())
    }
    fn work_size(&self, n: Node) -> Size {
        self.packing_bounds_geometry(n).1
    }
    fn work_point(&self, n: Node) -> Option<Point> {
        self.packing_bounds_geometry(n).0
    }
    fn work_set_size(&mut self, n: Node, size: Size) {
        if let Node::Node(node) = n {
            self.nodes[node.0 as usize].rect.size = size
        }
        for i in 0..self.clusters.len() {
            if self.work_vessel(true, i) == n {
                self.cluster_vessel_sizes
                    .resize(self.clusters.len(), Size::default());
                self.cluster_vessel_sizes[i] = size;
                self.serialized_vessel_sizes_by_tala
                    .insert(self.clusters[i].vessel_tala_id, size);
            }
        }
        for i in 0..self.sequences.len() {
            if self.work_vessel(false, i) == n {
                self.serialized_vessel_sizes_by_tala
                    .insert(self.sequences[i].vessel_tala_id, size);
            }
        }
    }
    fn work_policy(&self, g: Group) -> (String, f64, bool) {
        match g {
            Group::Owner(g) => {
                let p = self.owner_cluster_policy(g).expect("known cluster");
                (p.arrangement, p.padding, p.fixed_size)
            }
            Group::Cluster(i) => {
                let c = &self.clusters[i];
                (
                    (match c.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    })
                    .into(),
                    c.padding,
                    c.fixed_size,
                )
            }
            _ => unreachable!(),
        }
    }
    /// guard.go::binPackSyncClusters/Sequences, including the empty-map return.
    pub(super) fn packing_sync_groups_checked(
        &mut self,
        kind: Kind,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        if self.work_group_count(kind) == 0 {
            return guard.finish();
        }
        // Go range retains Graph.Nodes' header, loading each slot before
        // Step. Keep the backing live so callbacks can edit future roots.
        let roots = self
            .owner_graph_member_header(OwnerGraphId(0))
            .map(Members::Owner)
            .unwrap_or_else(|_| {
                Members::Compact(
                    self.node_order
                        .iter()
                        .map(|&n| Some(Node::Node(n)))
                        .collect(),
                )
            });
        for index in 0..roots.len() {
            let root = roots.get(index).map(|n| match n {
                Node::Node(n) => self.work_reference(n, None),
                other => other,
            });
            guard.step()?;
            self.work_postorder(root, kind, guard)?;
        }
        guard.finish()
    }
    fn work_postorder(
        &mut self,
        root: Option<Node>,
        kind: Kind,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let root = root.ok_or_else(|| invalid("BinPack reverse traversal contains a nil node"))?;
        let mut stack = vec![(root, 0u8, 0usize)];
        let mut active = BTreeSet::from([root]);
        while let Some(&(n, phase, index)) = stack.last() {
            guard.step()?;
            let owner = self
                .work_graph(n)
                .ok_or_else(|| invalid("BinPack reverse traversal found a node without a graph"))?;
            let children = match phase {
                0 => {
                    if self.work_is_container(n) {
                        self.work_children(n)
                    } else {
                        Members::Compact(Vec::new())
                    }
                }
                1 => {
                    if self.work_is_cluster(n) {
                        let g=self.work_group(owner,Kind::Cluster,n).ok_or_else(||invalid("BinPack reverse traversal found a cluster vessel without a cluster"))?;
                        self.work_members(g)
                    } else if let Some(g) = self.work_group(owner, Kind::Sequence, n) {
                        self.work_members(g)
                    } else {
                        Members::Compact(Vec::new())
                    }
                }
                _ => {
                    let apply = match kind {
                        Kind::Cluster => self.work_is_cluster(n),
                        Kind::Sequence => self
                            .work_group(OwnerGraphId(0), Kind::Sequence, n)
                            .is_some(),
                    };
                    if apply {
                        let group = self.work_group(OwnerGraphId(0), kind, n);
                        self.work_sync_geometry(group, kind, guard)?;
                    }
                    active.remove(&n);
                    stack.pop();
                    continue;
                }
            };
            if index >= children.len() {
                let frame = stack.last_mut().unwrap();
                frame.1 += 1;
                frame.2 = 0;
                continue;
            }
            let child = children
                .get(index)
                .ok_or_else(|| invalid("BinPack reverse traversal contains a nil child"))?;
            stack.last_mut().unwrap().2 += 1;
            if !active.insert(child) {
                return Err(invalid("BinPack reverse traversal found a topology cycle"));
            }
            stack.push((child, 0, 0));
        }
        guard.finish()
    }
    fn work_sync_geometry(
        &mut self,
        group: Option<Group>,
        kind: Kind,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        guard.step()?;
        let name = if kind == Kind::Cluster {
            "cluster"
        } else {
            "sequence"
        };
        let group = group.ok_or_else(|| invalid(&format!("{name} is missing its vessel")))?;
        let vessel = self
            .work_group_vessel(group)
            .ok_or_else(|| invalid(&format!("{name} is missing its vessel")))?;
        if kind == Kind::Sequence {
            let members = self.work_members(group);
            let (mut width, mut offset, mut height) = (0., 0., 0.);
            for i in 0..members.len() {
                let n = members.get(i);
                guard.step()?;
                let n = n.ok_or_else(|| invalid("sequence contains a nil step"))?;
                let size = self.work_size(n);
                width = maximum(width, offset + maximum(0., size.width));
                offset += advance(size.width);
                height = maximum(height, maximum(0., size.height));
            }
            // sequence.go writes through the live vessel after the retained
            // resize range. A Step callback may have changed that pointer.
            let vessel = self
                .work_group_vessel(group)
                .expect("runtime error: invalid memory address or nil pointer dereference");
            self.work_set_size(vessel, Size { width, height });
            let Some(mut p) = self.work_point(vessel) else {
                return guard.finish();
            };
            let members = self.work_members(group);
            for i in 0..members.len() {
                let n = members.get(i);
                guard.step()?;
                let n =
                    n.expect("runtime error: invalid memory address or nil pointer dereference");
                self.packing_set_ref_position(n, Some(p));
                p.x += advance(self.work_size(n).width);
            }
            return guard.finish();
        }
        let (_, _, fixed) = self.work_policy(group);
        if !fixed {
            let size = self.work_maximums(group, guard)?;
            let members = self.work_members(group);
            for i in 0..members.len() {
                let n = members.get(i);
                guard.step()?;
                self.work_set_size(
                    n.expect("runtime error: invalid memory address or nil pointer dereference"),
                    size,
                );
            }
        }
        let size = self.work_maximums(group, guard)?;
        let (arrangement, padding, _) = self.work_policy(group);
        let count = self.work_members(group).len() as f64;
        match arrangement.as_str() {
            "Row" => self.work_set_size(
                vessel,
                Size {
                    width: size.width * count + padding * (count - 1.),
                    height: size.height,
                },
            ),
            "Column" => self.work_set_size(
                vessel,
                Size {
                    width: size.width,
                    height: size.height * count + padding * (count - 1.),
                },
            ),
            _ => {}
        }
        // resizeWithWork retains its vessel argument, but arrangeNodesWithWork
        // reads c.Vessel again after the resize's final callback.
        let vessel = self
            .work_group_vessel(group)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        if self.work_point(vessel).is_none() {
            return guard.finish();
        }
        // These are separate source if-statements. A movement callback can
        // turn a Row into a Column, so the second branch must read live policy.
        if self.work_policy(group).0 == "Row" {
            self.work_arrange_cluster_axis(group, true, guard)?;
        }
        if self.work_policy(group).0 == "Column" {
            self.work_arrange_cluster_axis(group, false, guard)?;
        }
        guard.finish()
    }
    fn work_arrange_cluster_axis(
        &mut self,
        group: Group,
        row: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let vessel = self
            .work_group_vessel(group)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let origin = self
            .work_point(vessel)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let vessel_size = self.work_size(vessel);
        let mut position = if row { origin.x } else { origin.y };
        let center = if row {
            origin.y + vessel_size.height / 2.
        } else {
            origin.x + vessel_size.width / 2.
        };
        // A Go range copies the header once, but loads each slot before Step.
        let members = self.work_members(group);
        for i in 0..members.len() {
            let n = members.get(i);
            guard.step()?;
            let n = n.expect("runtime error: invalid memory address or nil pointer dereference");
            let size = self.work_size(n);
            if let Some(p) = self.work_point(n) {
                let delta = if row {
                    Point {
                        x: position - p.x,
                        y: (center - (p.y + size.height / 2.)).round(),
                    }
                } else {
                    Point {
                        x: (center - (p.x + size.width / 2.)).round(),
                        y: position - p.y,
                    }
                };
                self.work_move(n, delta, guard)?;
            } else {
                let p = if row {
                    Point {
                        x: position,
                        y: (center - size.height / 2.).round(),
                    }
                } else {
                    Point {
                        x: (center - size.width / 2.).round(),
                        y: position,
                    }
                };
                self.packing_set_ref_position(n, Some(p));
                self.work_position_children(n, guard)?;
            }
            // Both source loops read Padding after guarded movement returns.
            let padding = self.work_policy(group).1;
            position += if row {
                self.work_size(n).width + padding
            } else {
                self.work_size(n).height + padding
            };
        }
        Ok(())
    }
    fn work_maximums(
        &self,
        group: Group,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Size, limits::Error> {
        let mut size = Size::default();
        let members = self.work_members(group);
        for i in 0..members.len() {
            let n = members.get(i);
            guard.step()?;
            let n = n.ok_or_else(|| invalid("cluster contains a nil node"))?;
            size.width = maximum(size.width, self.work_size(n).width);
        }
        let members = self.work_members(group);
        for i in 0..members.len() {
            let n = members.get(i);
            guard.step()?;
            size.height = maximum(
                size.height,
                self.work_size(
                    n.expect("runtime error: invalid memory address or nil pointer dereference"),
                )
                .height,
            );
        }
        guard.finish().map(|()| size)
    }
    fn work_descendants(
        &self,
        root: Node,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Vec<Node>, limits::Error> {
        let mut stack = Vec::new();
        let mut result = Vec::new();
        let mut seen = BTreeSet::from([root]);
        let push = |n: Node,
                    stack: &mut Vec<Option<Node>>,
                    guard: &mut limits::WorkGuard<'_>|
         -> Result<(), limits::Error> {
            let owner = self
                .work_graph(n)
                .ok_or_else(|| invalid("BinPack reverse traversal found a node without a graph"))?;
            for kind in [Kind::Sequence, Kind::Cluster] {
                if kind == Kind::Cluster && !self.work_is_cluster(n) {
                    continue;
                }
                if let Some(group) = self.work_group(owner, kind, n) {
                    let nodes = self.work_members(group);
                    for i in (0..nodes.len()).rev() {
                        let child = nodes.get(i);
                        guard.step()?;
                        stack.push(child);
                    }
                }
            }
            if self.work_is_container(n) {
                let children = self.work_children(n);
                for i in (0..children.len()).rev() {
                    let child = children.get(i);
                    guard.step()?;
                    stack.push(child);
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
            result.push(n);
            push(n, &mut stack, guard)?;
        }
        guard.finish().map(|()| result)
    }
    fn work_move(
        &mut self,
        n: Node,
        delta: Point,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        guard.step()?;
        if delta.x == 0. && delta.y == 0. {
            return Ok(());
        }
        if self.work_point(n).is_none() || self.work_graph(n).is_none() {
            return Err(invalid(
                "BinPack cannot move an unplaced node or a node without a graph",
            ));
        }
        self.packing_translate_ref(n, delta);
        let descendants = self.work_descendants(n, guard)?;
        for child in descendants {
            guard.step()?;
            if self.work_point(child).is_none() {
                return Err(invalid("BinPack cannot move an unplaced descendant"));
            }
            self.packing_translate_ref(child, delta);
        }
        guard.finish()
    }
    fn work_position_children(
        &mut self,
        n: Node,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        guard.step()?;
        if !self.work_is_container(n) {
            return Ok(());
        }
        if self.work_graph(n).is_none() {
            return Err(invalid("BinPack container has no graph"));
        }
        let children = self.work_children(n);
        let refs = (0..children.len())
            .map(|i| children.get(i))
            .collect::<Vec<_>>();
        let (mut low, mut high) = self
            .packing_fixed_bounds_checked(&refs, guard)
            .map_err(|f| f.error)?;
        for i in 0..children.len() {
            let child = children.get(i);
            guard.step()?;
            let Some(child) = child else {
                return Err(invalid("BinPack container contains an unplaced child"));
            };
            let Some(p) = self.work_point(child) else {
                return Err(invalid("BinPack container contains an unplaced child"));
            };
            let (_, size, meta) = self.packing_bounds_geometry(child);
            if let Some(label) = meta.and_then(|m| m.label_size) {
                if (p.x == low.x || p.x + size.width == high.x) && label.width > size.width {
                    low.x = minimum(low.x, (p.x + (size.width / 2. - label.width / 2.)).floor());
                    high.x = maximum(
                        high.x,
                        (p.x + size.width - (size.width / 2. - label.width / 2.)).ceil(),
                    );
                }
            }
        }
        let Node::Node(node) = n else { unreachable!() };
        let inside = self
            .shape_inside_placement_absolute(
                node,
                Size {
                    width: high.x - low.x,
                    height: high.y - low.y,
                },
                Insets::uniform(0.),
            )
            .expect("positioned container");
        let delta = Point {
            x: inside.x - low.x,
            y: inside.y - low.y,
        };
        for i in 0..children.len() {
            self.work_move(
                children
                    .get(i)
                    .expect("runtime error: invalid memory address or nil pointer dereference"),
                delta,
                guard,
            )?;
        }
        guard.finish()
    }
}
