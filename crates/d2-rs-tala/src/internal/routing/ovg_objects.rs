// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Root-owned routing objects. Internal references never own the root itself.
use super::ovg_ports::{PortEdge, PortNode, SharedPortNode};
use crate::internal::go_cell::GoCell;
use std::sync::{Arc, Mutex, Weak};

#[derive(Default, Debug)]
struct Objects {
    parent: Option<Arc<ObjectRoot>>,
    rank: usize,
    nodes: Vec<Arc<GoCell<PortNode>>>,
    edges: Vec<Arc<GoCell<PortEdge>>>,
}
#[derive(Default, Debug)]
pub(super) struct ObjectRoot(Mutex<Objects>);
// Union and insertion share one short lock; no caller callbacks run under it.
static OBJECT_UNION: Mutex<()> = Mutex::new(());
impl ObjectRoot {
    fn representative(root: &Arc<Self>) -> Arc<Self> {
        let mut current = root.clone();
        loop {
            let parent = current
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .parent
                .clone();
            match parent {
                Some(parent) => current = parent,
                None => return current,
            }
        }
    }
    pub(super) fn add_node(root: &Arc<Self>, node: Arc<GoCell<PortNode>>) {
        let _union = OBJECT_UNION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = Self::representative(root);
        node.write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .edges
            .set_owner(Arc::downgrade(&root));
        root.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .nodes
            .push(node);
    }
    fn join_locked(a: &Arc<Self>, b: &Arc<Self>) -> Arc<Self> {
        let same_root = Arc::ptr_eq(a, b);
        let mut a = Self::representative(a);
        // One root has one representative; skip the second walk.
        let mut b = if same_root {
            a.clone()
        } else {
            Self::representative(b)
        };
        if !Arc::ptr_eq(&a, &b) {
            let rank_a =
                a.0.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .rank;
            let rank_b =
                b.0.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .rank;
            if rank_a < rank_b {
                std::mem::swap(&mut a, &mut b);
            }
            let mut donor =
                b.0.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut owner =
                a.0.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            if rank_a == rank_b {
                owner.rank += 1;
            }
            for node in &donor.nodes {
                node.write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .edges
                    .set_owner(Arc::downgrade(&a));
            }
            owner.nodes.append(&mut donor.nodes);
            owner.edges.append(&mut donor.edges);
            donor.parent = Some(a.clone());
        }
        a
    }
    fn adopt(root: &Arc<Self>, other: &Arc<Self>) {
        let _union = OBJECT_UNION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::join_locked(root, other);
    }
    pub(super) fn add_edge(a: &Arc<Self>, b: &Arc<Self>, edge: Arc<GoCell<PortEdge>>) -> Arc<Self> {
        let _union = OBJECT_UNION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let a = Self::join_locked(a, b);
        a.0.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .edges
            .push(edge);
        a
    }
}

#[derive(Clone, Debug)]
pub(super) struct SharedPortEdge {
    pub(super) record: Arc<GoCell<PortEdge>>,
    pub(super) root: Arc<ObjectRoot>,
}
impl SharedPortEdge {
    pub(super) fn new(
        from: &SharedPortNode,
        to: &SharedPortNode,
        from_slot: usize,
        to_slot: usize,
    ) -> Self {
        let record = Arc::new(GoCell::new(PortEdge {
            from: u32::try_from(from_slot).expect("OVG node slot"),
            to: u32::try_from(to_slot).expect("OVG node slot"),
            from_object: from.0.clone(),
            to_object: to.0.clone(),
            distance: 0.0,
        }));
        let root = ObjectRoot::add_edge(&from.1, &to.1, record.clone());
        Self { record, root }
    }
    pub(super) fn allocation_key(&self) -> usize {
        Arc::as_ptr(&self.record) as usize
    }
    pub(super) fn read(&self) -> crate::internal::go_cell::GoCellRef<'_, PortEdge> {
        self.record
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn write(&self) -> EdgeWrite<'_> {
        let guard = self
            .record
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = (
            Arc::as_ptr(&guard.from_object),
            Arc::as_ptr(&guard.to_object),
        );
        EdgeWrite {
            guard: Some(guard),
            root: &self.root,
            before,
        }
    }
    pub(super) fn incident(&self, local_index: usize) -> IncidentEdge {
        #[cfg(not(any(test, feature = "diagnostic-traces")))]
        let _ = local_index;
        IncidentEdge {
            record: Arc::downgrade(&self.record),
            #[cfg(any(test, feature = "diagnostic-traces"))]
            local_index,
            retained_owner: None,
        }
    }
}
// Endpoint pointer mutation changes reachability, not cached distance or any
// node's incident slice. Adopt the new allocation domains after releasing the
// edge lock; ordinary distance writes never enter ownership union.
pub(super) struct EdgeWrite<'a> {
    guard: Option<crate::internal::go_cell::GoCellMut<'a, PortEdge>>,
    root: &'a Arc<ObjectRoot>,
    before: (*const GoCell<PortNode>, *const GoCell<PortNode>),
}
impl std::ops::Deref for EdgeWrite<'_> {
    type Target = PortEdge;
    fn deref(&self) -> &PortEdge {
        self.guard.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for EdgeWrite<'_> {
    fn deref_mut(&mut self) -> &mut PortEdge {
        self.guard.as_mut().unwrap()
    }
}
impl Drop for EdgeWrite<'_> {
    fn drop(&mut self) {
        let guard = self.guard.take().unwrap();
        let after = (
            Arc::as_ptr(&guard.from_object),
            Arc::as_ptr(&guard.to_object),
        );
        if after == self.before {
            return;
        }
        let endpoints = [guard.from_object.clone(), guard.to_object.clone()];
        drop(guard);
        for node in endpoints {
            let owner = node
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .edges
                .owner_root()
                .expect("live OVG endpoint allocation domain");
            ObjectRoot::adopt(self.root, &owner);
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct IncidentEdge {
    record: Weak<GoCell<PortEdge>>,
    // Source adjacency slots hold only an edge pointer. Local numbers are
    // diagnostic labels, not identity: a hierarchy import keeps the original
    // pointer even when the receiver has different edge indices.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    local_index: usize,
    retained_owner: Option<Arc<ObjectRoot>>,
}
impl IncidentEdge {
    #[cfg(test)]
    pub(super) fn allocation_key(&self) -> usize {
        self.record.as_ptr() as usize
    }
    pub(super) fn record(&self) -> SharedPortEdge {
        SharedPortEdge {
            record: self.record.upgrade().expect("retained OVG edge object"),
            root: self
                .retained_owner
                .as_ref()
                .expect("retained OVG edge root")
                .clone(),
        }
    }
    pub(super) fn adjacent(&self, node: &SharedPortNode) -> SharedPortNode {
        let record = self.record();
        let edge = record.read();
        let other = if Arc::ptr_eq(&node.0, &edge.from_object) {
            &edge.to_object
        } else {
            &edge.from_object
        };
        SharedPortNode(other.clone(), node.1.clone())
    }
}
/// An incident slot in an OVG node's edge backing array. Stored slots never
/// carried an owner root; the reading header supplies it.
#[derive(Clone, Debug)]
pub(super) struct StoredIncident {
    record: Weak<GoCell<PortEdge>>,
    #[cfg(any(test, feature = "diagnostic-traces"))]
    local_index: usize,
}
impl StoredIncident {
    pub(super) fn allocation_key(&self) -> usize {
        self.record.as_ptr() as usize
    }
    /// Read an incident while its containing OVG node keeps the root alive.
    /// No retained edge or endpoint handle escapes this callback-free walk.
    pub(super) fn with_record<R>(&self, read: impl FnOnce(&PortEdge) -> R) -> R {
        let record = self.record.upgrade().expect("retained OVG edge object");
        let edge = record
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read(&edge)
    }
}
impl super::ovg_edge_header::SlotValue for IncidentEdge {
    type Stored = StoredIncident;
    fn retained(stored: &StoredIncident, owner: Option<Arc<ObjectRoot>>) -> Self {
        Self {
            record: stored.record.clone(),
            #[cfg(any(test, feature = "diagnostic-traces"))]
            local_index: stored.local_index,
            retained_owner: owner,
        }
    }
    fn stored(&self) -> StoredIncident {
        StoredIncident {
            record: self.record.clone(),
            #[cfg(any(test, feature = "diagnostic-traces"))]
            local_index: self.local_index,
        }
    }
    fn into_stored(self) -> StoredIncident {
        StoredIncident {
            record: self.record,
            #[cfg(any(test, feature = "diagnostic-traces"))]
            local_index: self.local_index,
        }
    }
}
#[cfg(any(test, feature = "diagnostic-traces"))]
impl PartialEq<usize> for IncidentEdge {
    fn eq(&self, other: &usize) -> bool {
        self.local_index == *other
    }
}
#[cfg(any(test, feature = "diagnostic-traces"))]
impl serde::Serialize for IncidentEdge {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.local_index.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Point, ovg_ports::PortBuild};
    use super::*;
    #[test]
    fn rewritten_endpoint_domains_retain_incidents_and_release_without_cycles() {
        for side in [0, 1, 2] {
            let mut original = PortBuild::default();
            let a = original
                .add_point(
                    Point { x: 1., y: 2. },
                    &super::super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap();
            let b = original
                .add_point(
                    Point { x: 11., y: 2. },
                    &super::super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap();
            original
                .connect(
                    a,
                    b,
                    &super::super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap();
            let edge = original.edges[0].clone();
            let mut foreign = PortBuild::default();
            let c = foreign
                .add_point(
                    Point { x: 101., y: 102. },
                    &super::super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap();
            let d = foreign
                .add_point(
                    Point { x: 111., y: 102. },
                    &super::super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap();
            foreign
                .connect(
                    c,
                    d,
                    &super::super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap();
            let nodes = [
                Arc::downgrade(&original.nodes[a].0),
                Arc::downgrade(&original.nodes[b].0),
                Arc::downgrade(&foreign.nodes[c].0),
                Arc::downgrade(&foreign.nodes[d].0),
            ];
            let edges = [
                Arc::downgrade(&edge.record),
                Arc::downgrade(&foreign.edges[0].record),
            ];
            let roots = [
                Arc::downgrade(&original.nodes[a].1),
                Arc::downgrade(&foreign.nodes[c].1),
            ];
            if side != 1 {
                edge.write().from_object = foreign.nodes[c].0.clone();
            }
            if side != 0 {
                edge.write().to_object = foreign.nodes[d].0.clone();
            }
            drop(original);
            drop(foreign);
            assert!(nodes.iter().all(|n| n.upgrade().is_some()));
            assert!(edges.iter().all(|e| e.upgrade().is_some()));
            drop(edge);
            assert!(nodes.iter().all(|n| n.upgrade().is_none()));
            assert!(edges.iter().all(|e| e.upgrade().is_none()));
            assert!(roots.iter().all(|r| r.upgrade().is_none()));
        }
    }

    #[test]
    fn retained_nodes_edges_and_headers_keep_objects_then_release_every_allocation() {
        let mut source = PortBuild::default();
        let a = source
            .add_point(
                Point { x: 1.0, y: 2.0 },
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        let b = source.allocate_node(Point { x: 11.0, y: 2.0 });
        source
            .connect(
                a,
                b,
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        let node = source.nodes[a].clone();
        let captured = node.read().edges.clone();
        let retained_edge = source.edges[0].clone();
        let source_root = Arc::downgrade(&node.1);
        let from = Arc::downgrade(&node.0);
        let foreign = Arc::downgrade(&source.nodes[b].0);
        let old_edge = Arc::downgrade(&retained_edge.record);
        let mut receiver = PortBuild::default();
        let local = receiver.import_object(node.clone());
        let c = receiver
            .add_point(
                Point { x: 1.0, y: 12.0 },
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        let receiver_root = Arc::downgrade(&receiver.nodes[c].1);
        let receiver_node = Arc::downgrade(&receiver.nodes[c].0);
        receiver
            .connect(
                local,
                c,
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        let late = source.allocate_node(Point { x: 1.0, y: 22.0 });
        let late_node = Arc::downgrade(&source.nodes[late].0);
        source
            .connect(
                a,
                late,
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        let late_edge = Arc::downgrade(&source.edges[1].record);
        drop(source);
        drop(receiver);
        assert_eq!(node.read().edges.len(), 3);
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured.get(0).unwrap().adjacent(&node).point(),
            Point { x: 11.0, y: 2.0 }
        );
        let extracted = captured.get(0).unwrap();
        let extracted_record = extracted.record();
        drop(node);
        assert!(old_edge.upgrade().is_some() && foreign.upgrade().is_some());
        drop(captured);
        // An edge alone roots its endpoints and their current incidences.
        assert!(late_edge.upgrade().is_some() && receiver_node.upgrade().is_some());
        drop(retained_edge);
        assert!(old_edge.upgrade().is_some() && foreign.upgrade().is_some());
        drop(extracted);
        assert!(late_edge.upgrade().is_some());
        drop(extracted_record);
        assert!(from.upgrade().is_none() && foreign.upgrade().is_none());
        assert!(late_node.upgrade().is_none() && receiver_node.upgrade().is_none());
        assert!(old_edge.upgrade().is_none() && late_edge.upgrade().is_none());
        assert!(source_root.upgrade().is_none() && receiver_root.upgrade().is_none());
    }
}
