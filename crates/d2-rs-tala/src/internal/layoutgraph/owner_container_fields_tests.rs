// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
fn fixture() -> ArenaGraph {
    let mut input = Graph::default();
    for i in 1..=5 {
        input.add_node(crate::Node::new(
            i.to_string(),
            Size {
                width: 20.0,
                height: 30.0,
            },
        ));
    }
    let mut g = ArenaGraph::from_input(&input);
    g.initialize_shared_topology();
    for i in 0..5 {
        g.nodes[i].tala_id = i as i64 + 1;
    }
    let map = g.allocate_owner_container_map(true);
    g.set_owner_graph_container_map(OwnerGraphId(0), map)
        .unwrap();
    let h = SharedNodeSlice::with_len_cap(2, 4).unwrap();
    h.write_capacity_slot(0, Some(NodeId(0))).unwrap();
    h.write_capacity_slot(1, Some(NodeId(1))).unwrap();
    h.write_capacity_slot(3, Some(NodeId(4))).unwrap();
    g.containers.replace_header(None, h);
    for _ in 0..2 {
        g.create_owner_graph_view(vec![], vec![]).unwrap();
    }
    g
}
#[test]
fn retained_container_fields_reject_foreign_arena_and_numeric_scope() {
    let mut g = fixture();
    let field = g
        .retain_owner_graph_container_map_field(OwnerGraphId(0))
        .unwrap();
    let mut foreign = fixture();
    assert!(matches!(
        field.replace(foreign.allocate_owner_container_map(true)),
        Err(OwnerTopologyError::ForeignArena { .. })
    ));
    let mut candidate = g.clone();
    let candidate_field = candidate
        .retain_owner_graph_container_map_field(OwnerGraphId(0))
        .unwrap();
    assert!(!field.same_field(&candidate_field));
    assert!(matches!(
        field.replace(candidate.allocate_owner_container_map(true)),
        Err(OwnerTopologyError::ForeignContainerMap)
    ));
    let before = field.header();
    candidate_field
        .replace(candidate.allocate_owner_container_map(false))
        .unwrap();
    assert!(candidate_field.header().is_nil());
    assert!(field.header().same_map_allocation(&before));
    // Two graphs sharing one map retain distinct field addresses, also after
    // a numerical clone; mutating one field must not replace the other field.
    g.set_owner_graph_container_map(OwnerGraphId(1), field.header())
        .unwrap();
    let mut copy = g.clone();
    let a = copy
        .retain_owner_graph_container_map_field(OwnerGraphId(0))
        .unwrap();
    let b = copy
        .retain_owner_graph_container_map_field(OwnerGraphId(1))
        .unwrap();
    assert!(!a.same_field(&b));
    assert!(a.header().same_map_allocation(&b.header()));
    assert!(!a.header().same_map_allocation(&field.header()));
    a.replace(copy.allocate_owner_container_map(false)).unwrap();
    assert!(a.header().is_nil());
    assert!(!b.header().is_nil());
}

#[test]
fn live_descendant_reader_follows_container_field_replacement_and_restoration() {
    let mut g = fixture();
    let field = g
        .retain_owner_graph_container_map_field(OwnerGraphId(0))
        .unwrap();
    let original = field.header();
    g.write_owner_container_map_entry(&original, Some(NodeId(0)), &[Some(NodeId(1))])
        .unwrap();
    let before = g.owner_topology_snapshot();
    let replacement = g.allocate_owner_container_map(true);
    g.write_owner_container_map_entry(&replacement, Some(NodeId(0)), &[Some(NodeId(4))])
        .unwrap();
    field.replace(replacement.clone()).unwrap();
    assert_eq!(g.descendants(NodeId(0)), vec![NodeId(4)]);
    g.restore_owner_topology_snapshot(before);
    assert_eq!(g.descendants(NodeId(0)), vec![NodeId(1)]);
    assert!(field.header().same_map_allocation(&original));
    assert_eq!(
        replacement
            .get(Some(NodeId(0)))
            .unwrap()
            .slice
            .visible_slots(),
        vec![Some(NodeId(4))]
    );
}
