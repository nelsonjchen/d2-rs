// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::super::ArenaGraph;
use super::super::shared_topology::{
    OwnerGraphId, OwnerTopologyError, OwnerTreeRootsHeader, OwnerTreeRootsMap,
};
use super::*;
use crate::{Graph, Node, Size};
use serde_json::{Value, json};

fn fixture() -> (ArenaGraph, [NodeId; 4], [OwnerTreeId; 3], OwnerGraphId) {
    let mut input = Graph::default();
    let nodes = ["A", "B", "C", "D"].map(|name| {
        input.add_node(Node::new(
            name,
            Size {
                width: 10.0,
                height: 20.0,
            },
        ))
    });
    let mut owner = ArenaGraph::from_input(&input);
    owner.cell_size = 10.0;
    owner.initialize_shared_topology();
    let other = owner
        .create_owner_graph_view(Vec::new(), Vec::new())
        .unwrap();
    owner.set_owner_graph_cell_size(other, 20.0).unwrap();
    let trees = nodes[..3]
        .iter()
        .map(|&node| owner.new_owner_tree(Some(node)).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    (owner, nodes, trees, other)
}

fn header(
    owner: &mut ArenaGraph,
    slots: &[Option<OwnerTreeId>],
    capacity: usize,
) -> OwnerTreeRootsHeader {
    owner
        .allocate_owner_tree_roots_header(slots, capacity, true)
        .unwrap()
}

fn observed_header(h: Option<&OwnerTreeRootsHeader>, trees: &[OwnerTreeId; 3]) -> Value {
    let Some(h) = h else {
        return json!({"present":false,"nil":true,"len":0,"cap":0,"slots":[]});
    };
    let slots: Vec<_> = (0..h.capacity())
        .map(|i| {
            h.read_capacity_slot(i)
                .unwrap()
                .map(|tree| trees.iter().position(|&known| known == tree).unwrap())
        })
        .collect();
    json!({"present":true,"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":slots})
}

#[test]
fn owner_tree_roots_actual_source_controls() {
    for name in [
        "new-graph",
        "nullable-values",
        "copy-live",
        "retained-header",
        "restore-overlap",
        "restore-nil-map",
        "restore-empty-map",
    ] {
        let (mut owner, nodes, trees, other) = fixture();
        let main = OwnerGraphId(0);
        let original = owner.owner_graph_tree_roots_map(main).unwrap();
        let mut retained = None;
        let mut replacement: Option<OwnerTreeRootsMap> = None;
        if name == "nullable-values" {
            let nil = owner
                .allocate_owner_tree_roots_header(&[], 0, false)
                .unwrap();
            let empty = header(&mut owner, &[], 0);
            let roots = header(&mut owner, &[Some(trees[1]), None], 3);
            owner
                .write_owner_tree_roots_slot(&roots, 2, Some(trees[2]))
                .unwrap();
            owner
                .write_owner_tree_roots_entry(&original, None, &nil)
                .unwrap();
            owner
                .write_owner_tree_roots_entry(&original, Some(nodes[0]), &empty)
                .unwrap();
            owner
                .write_owner_tree_roots_entry(&original, Some(nodes[1]), &roots)
                .unwrap();
        }
        if name == "copy-live" {
            let roots = header(&mut owner, &[Some(trees[0])], 2);
            owner
                .write_owner_tree_roots_entry(&original, Some(nodes[0]), &roots)
                .unwrap();
            owner.copy_owner_graph_tree_roots_from(other, main).unwrap();
            owner
                .set_owner_node_graph_reference(nodes[0], Some(other))
                .unwrap();
            let roots = header(&mut owner, &[Some(trees[1])], 1);
            let copied = owner.owner_graph_tree_roots_map(other).unwrap();
            owner
                .write_owner_tree_roots_entry(&copied, Some(nodes[1]), &roots)
                .unwrap();
        }
        if name == "retained-header" || name == "restore-overlap" {
            let roots = header(
                &mut owner,
                &[Some(trees[0]), Some(trees[1]), Some(trees[2])],
                4,
            );
            let a = roots.reslice(0, 1, 4).unwrap();
            let b = roots.reslice(1, 2, 3).unwrap();
            owner
                .write_owner_tree_roots_entry(&original, Some(nodes[0]), &a)
                .unwrap();
            owner
                .write_owner_tree_roots_entry(&original, Some(nodes[1]), &b)
                .unwrap();
            retained = Some(roots.reslice(0, 2, 4).unwrap());
            if name == "retained-header" {
                let empty = header(&mut owner, &[], 0);
                owner
                    .write_owner_tree_roots_entry(&original, Some(nodes[0]), &empty)
                    .unwrap();
                owner.write_owner_tree_roots_slot(&b, 0, None).unwrap();
            } else {
                let nil = owner
                    .allocate_owner_tree_roots_header(&[], 0, false)
                    .unwrap();
                let empty = header(&mut owner, &[], 0);
                owner
                    .write_owner_tree_roots_entry(&original, Some(nodes[2]), &nil)
                    .unwrap();
                owner
                    .write_owner_tree_roots_entry(&original, None, &empty)
                    .unwrap();
                owner
                    .set_owner_tree_children_header(trees[0], &roots.reslice(1, 3, 4).unwrap())
                    .unwrap();
                owner.copy_owner_graph_tree_roots_from(other, main).unwrap();
                let snapshot = owner.owner_topology_snapshot();
                for i in 0..4 {
                    owner.write_owner_tree_roots_slot(&roots, i, None).unwrap()
                }
                owner
                    .write_owner_tree_roots_slot(&roots, 0, Some(trees[2]))
                    .unwrap();
                owner
                    .write_owner_tree_roots_entry(&original, Some(nodes[0]), &nil)
                    .unwrap();
                owner
                    .write_owner_tree_roots_entry(&original, Some(nodes[1]), &empty)
                    .unwrap();
                let changed = owner.allocate_owner_tree_roots_map(true);
                let c = header(&mut owner, &[Some(trees[2])], 1);
                owner
                    .write_owner_tree_roots_entry(&changed, Some(nodes[2]), &c)
                    .unwrap();
                owner
                    .set_owner_graph_tree_roots_map(main, changed.clone())
                    .unwrap();
                let mut fields = owner.owner_tree(trees[0]).unwrap().fields();
                fields.parent = Some(trees[1]);
                owner.set_owner_tree_fields(trees[0], fields).unwrap();
                replacement = Some(changed);
                owner.restore_owner_topology_snapshot(snapshot);
            }
        }
        if name == "restore-nil-map" || name == "restore-empty-map" {
            if name == "restore-nil-map" {
                let nil = owner.allocate_owner_tree_roots_map(false);
                owner.set_owner_graph_tree_roots_map(main, nil).unwrap()
            } else {
                owner.copy_owner_graph_tree_roots_from(other, main).unwrap()
            }
            let snapshot = owner.owner_topology_snapshot();
            if name == "restore-empty-map" {
                let a = header(&mut owner, &[Some(trees[1])], 1);
                owner
                    .write_owner_tree_roots_entry(&original, Some(nodes[0]), &a)
                    .unwrap()
            }
            let changed = owner.allocate_owner_tree_roots_map(true);
            let c = header(&mut owner, &[Some(trees[2])], 1);
            owner
                .write_owner_tree_roots_entry(&changed, Some(nodes[2]), &c)
                .unwrap();
            owner
                .set_owner_graph_tree_roots_map(main, changed.clone())
                .unwrap();
            replacement = Some(changed);
            owner.restore_owner_topology_snapshot(snapshot);
        }
        let main_map = owner.owner_graph_tree_roots_map(main).unwrap();
        let other_map = owner.owner_graph_tree_roots_map(other).unwrap();
        // Behavioral map alias probe with an otherwise unused canonical key.
        let shares_map = if main_map.is_nil() {
            false
        } else {
            let nil = owner
                .allocate_owner_tree_roots_header(&[], 0, false)
                .unwrap();
            owner
                .write_owner_tree_roots_entry(&main_map, Some(nodes[3]), &nil)
                .unwrap();
            let visible = owner
                .owner_graph_tree_roots_entry(other, Some(nodes[3]))
                .unwrap()
                .is_some();
            owner
                .delete_owner_tree_roots_entry(&main_map, Some(nodes[3]))
                .unwrap();
            visible
        };
        let mut headers: Vec<_> = [None, Some(nodes[0]), Some(nodes[1]), Some(nodes[2])]
            .into_iter()
            .map(|key| owner.owner_graph_tree_roots_entry(main, key).unwrap())
            .collect();
        headers.push(Some(owner.owner_tree_children_header(trees[0]).unwrap()));
        headers.push(retained);
        // Retain a replacement map independently of its original Graph field.
        let replacement_header = replacement.as_ref().and_then(|map| {
            owner
                .owner_tree_roots_map_entry(map, Some(nodes[2]))
                .unwrap()
        });
        headers.push(replacement_header);
        let mut overlaps = Vec::new();
        for left in 0..headers.len() {
            if let Some(a) = &headers[left] {
                for ai in 0..a.capacity() {
                    for right in left + 1..headers.len() {
                        if let Some(b) = &headers[right] {
                            for bi in 0..b.capacity() {
                                if a.slot_address(ai) == b.slot_address(bi) {
                                    overlaps.push([left, ai, right, bi])
                                }
                            }
                        }
                    }
                }
            }
        }
        let node_owners: Vec<_> = nodes
            .iter()
            .map(|&node| owner.topology_node_owner(node).map(|id| id.0))
            .collect();
        let row = json!({"case":name,"map_nil":main_map.is_nil(),"map_len":main_map.len(),"other_nil":other_map.is_nil(),"other_len":other_map.len(),"shares_map":shares_map,"headers":headers.iter().map(|h|observed_header(h.as_ref(),&trees)).collect::<Vec<_>>(),"overlaps":overlaps,"node_owners":node_owners,"cells":[owner.owner_graph_cell_size(main).unwrap(),owner.owner_graph_cell_size(other).unwrap()],"tree_parent":owner.owner_tree(trees[0]).unwrap().fields().parent.map(|tree|trees.iter().position(|&t|t==tree).unwrap()),"replacement_nil":replacement.as_ref().map(|m|m.is_nil()),"replacement_len":replacement.as_ref().map(|m|m.len())});
        println!("OWNER_TREE_ROOTS_CONTROL {row}");
    }
}

#[test]
fn tree_roots_scopes_and_nil_assignment_preserve_real_headers() {
    let (mut owner, nodes, trees, other) = fixture();
    let nil = owner.allocate_owner_tree_roots_map(false);
    let roots = header(&mut owner, &[Some(trees[0])], 2);
    assert_eq!(
        owner.write_owner_tree_roots_entry(&nil, Some(nodes[0]), &roots),
        Err(OwnerTopologyError::NilTreeRootsMapAssignment)
    );
    let mut fork = owner.clone();
    let map = fork.owner_graph_tree_roots_map(other).unwrap();
    assert_eq!(
        fork.write_owner_tree_roots_entry(&map, None, &roots),
        Err(OwnerTopologyError::ForeignTreeRootsScope)
    );
    assert_eq!(
        fork.set_owner_graph_tree_roots_map(other, nil),
        Err(OwnerTopologyError::ForeignTreeRootsScope)
    );
    assert_eq!(roots.get(0), Some(Some(trees[0])));
    assert_eq!(roots.data_address(), roots.slot_address(0));
}

#[test]
fn numerical_fork_preserves_root_child_overlaps_and_detaches_from_parent() {
    let (mut owner, nodes, trees, other) = fixture();
    let roots = header(&mut owner, &[Some(trees[0]), Some(trees[1])], 4);
    owner
        .set_owner_tree_children_header(trees[0], &roots.reslice(1, 2, 4).unwrap())
        .unwrap();
    let map = owner.owner_graph_tree_roots_map(OwnerGraphId(0)).unwrap();
    owner
        .write_owner_tree_roots_entry(&map, Some(nodes[0]), &roots)
        .unwrap();
    owner
        .copy_owner_graph_tree_roots_from(other, OwnerGraphId(0))
        .unwrap();
    let mut fork = owner.clone();
    let fork_roots = fork
        .owner_graph_tree_roots_entry(other, Some(nodes[0]))
        .unwrap()
        .unwrap();
    let fork_children = fork.owner_tree_children_header(trees[0]).unwrap();
    assert_eq!(fork_roots.slot_address(1), fork_children.slot_address(0));
    assert_ne!(fork_roots.slot_address(1), roots.slot_address(1));
    assert!(
        fork.owner_graph_tree_roots_map(other)
            .unwrap()
            .same_map_allocation(&fork.owner_graph_tree_roots_map(OwnerGraphId(0)).unwrap())
    );
    assert!(!map.same_map_allocation(&fork.owner_graph_tree_roots_map(other).unwrap()));
    fork.write_owner_tree_roots_slot(&fork_children, 0, None)
        .unwrap();
    assert_eq!(fork_roots.get(1), Some(None));
    assert_eq!(roots.get(1), Some(Some(trees[1])));
    assert_eq!(
        owner.owner_tree_children_header(trees[0]).unwrap().get(0),
        Some(Some(trees[1]))
    );
}
