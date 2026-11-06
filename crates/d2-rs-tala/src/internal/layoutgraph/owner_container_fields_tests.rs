// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use serde_json::{Value, json};
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
fn node(v: &Value) -> Option<NodeId> {
    v.as_u64().filter(|&v| v != 0).map(|v| NodeId(v as u32 - 1))
}
fn capture(
    fields: &BTreeMap<String, OwnerContainerMapField>,
    held: &BTreeMap<String, OwnerContainerMap>,
) -> Value {
    let mut maps: BTreeMap<_, _> = fields
        .iter()
        .map(|(k, f)| (k.clone(), f.header()))
        .collect();
    for (name, map) in held {
        maps.insert(format!("held:{name}"), map.clone());
    }
    let pairs: Vec<_> = maps.iter().collect();
    let mut aliases = Vec::new();
    let mut state = serde_json::Map::new();
    for (i, (name, map)) in pairs.iter().enumerate() {
        let entries:Vec<_>=map.slices.entries().iter().map(|(key,h)|json!({"key":key.map(|n|n.0+1),"nil":h.is_nil(),"len":h.len(),"cap":h.capacity(),"slots":h.capacity_slots().iter().map(|n|n.map(|n|n.0+1)).collect::<Vec<_>>()})).collect();
        state.insert(
            (*name).clone(),
            json!({"nil":map.is_nil(),"entries":entries}),
        );
        for (other, b) in pairs.iter().skip(i + 1) {
            aliases.push(json!({"a":name,"b":other,"same":map.same_map_allocation(b)||(map.is_nil()&&b.is_nil())}));
        }
    }
    let fields: Vec<_> = fields.iter().collect();
    let mut field_aliases = Vec::new();
    for (i, (a, f)) in fields.iter().enumerate() {
        for (b, g) in fields.iter().skip(i + 1) {
            field_aliases.push(json!({"a":a,"b":b,"same":f.same_field(g)}));
        }
    }
    json!({"maps":state,"map_aliases":aliases,"field_aliases":field_aliases,"panic":null})
}
fn run(row: &Value) -> Value {
    let mut g = fixture();
    if row["copy_views"] != true {
        for id in [OwnerGraphId(1), OwnerGraphId(2)] {
            let map = g.allocate_owner_container_map(true);
            g.set_owner_graph_container_map(id, map).unwrap();
        }
    }
    let root = g
        .retain_owner_graph_container_map_field(OwnerGraphId(0))
        .unwrap();
    let fields = BTreeMap::from([
        ("root".into(), root.clone()),
        ("root-alias".into(), root),
        (
            "other".into(),
            g.retain_owner_graph_container_map_field(OwnerGraphId(1))
                .unwrap(),
        ),
        (
            "third".into(),
            g.retain_owner_graph_container_map_field(OwnerGraphId(2))
                .unwrap(),
        ),
    ]);
    let mut held = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    let mut states = vec![capture(&fields, &held)];
    for op in row["ops"].as_array().unwrap() {
        let target = op["target"].as_str().unwrap_or("");
        let name = op["name"].as_str().unwrap_or("");
        let map = fields
            .get(target)
            .map(OwnerContainerMapField::header)
            .or_else(|| held.get(target).cloned());
        match op["kind"].as_str().unwrap() {
            "retain" => {
                held.insert(name.to_string(), map.unwrap());
            }
            "assign" => {
                fields[target]
                    .replace(fields[op["source"].as_str().unwrap()].header())
                    .unwrap();
            }
            "new" => {
                let map = g.allocate_owner_container_map(op["allocated"] == true);
                fields[target].replace(map).unwrap();
            }
            "put" => {
                let h = SharedNodeSlice::with_len_cap(
                    op["length"].as_u64().unwrap() as usize,
                    op["capacity"].as_u64().unwrap() as usize,
                )
                .unwrap();
                for (i, v) in op["slots"].as_array().unwrap().iter().enumerate() {
                    h.write_capacity_slot(i, node(v)).unwrap();
                }
                map.unwrap().slices.replace(node(&op["key"]), h).unwrap();
            }
            "nil-entry" => {
                map.unwrap()
                    .slices
                    .replace(node(&op["key"]), SharedNodeSlice::nil())
                    .unwrap();
            }
            "append" => {
                let map = map.unwrap();
                let key = node(&op["key"]);
                let mut h = map.slices.get(&key).unwrap_or_else(SharedNodeSlice::nil);
                for v in op["slots"].as_array().unwrap() {
                    h = h.append(&[node(v)]).unwrap();
                }
                map.slices.replace(key, h).unwrap();
            }
            "write" => {
                map.unwrap()
                    .slices
                    .get(&node(&op["key"]))
                    .unwrap()
                    .write_capacity_slot(op["index"].as_u64().unwrap() as usize, node(&op["value"]))
                    .unwrap();
            }
            "delete" => {
                map.unwrap().slices.remove(&node(&op["key"]));
            }
            "clear" => {
                map.unwrap().slices.clear();
            }
            "snapshot" => {
                snapshots.insert(name.to_string(), g.owner_topology_snapshot());
            }
            "rollback" => {
                g.restore_owner_topology_snapshot(snapshots.remove(name).unwrap());
            }
            kind => panic!("unknown operation {kind}"),
        }
        states.push(capture(&fields, &held));
    }
    json!({"name":row["name"],"states":states})
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
