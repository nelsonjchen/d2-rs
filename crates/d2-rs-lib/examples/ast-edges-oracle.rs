// SPDX-License-Identifier: MPL-2.0
use d2_tala_rs::ast::*;
use serde_json::json;
fn main() {
    let edge = Edge {
        src: Some(Shared::new(KeyPath::from_strings(["a"]))),
        dst: Some(Shared::new(KeyPath::from_strings(["b.c"]))),
        src_arrow: "<".into(),
        dst_arrow: ">".into(),
        ..Default::default()
    };
    let mut same = edge.clone();
    same.range.path = "different.d2".into();
    let mut changed = edge.clone();
    changed.dst_arrow = "*".into();
    let mut full = Vec::new();
    let mut pruned = Vec::new();
    let mut paths = 0;
    walk(Some(&edge), &mut |node| {
        full.push(node.node_type().into_owned());
        true
    });
    walk(Some(&edge), &mut |node| {
        pruned.push(node.node_type().into_owned());
        if node.node_type() == "key path" {
            paths += 1;
            return paths != 1;
        }
        true
    });
    let shared = Shared::new(3);
    let indices = vec![
        EdgeIndex::default(),
        EdgeIndex::default(),
        EdgeIndex {
            glob: true,
            ..Default::default()
        },
        EdgeIndex {
            int: Some(shared.clone()),
            ..Default::default()
        },
        EdgeIndex {
            int: Some(shared.clone()),
            ..Default::default()
        },
        EdgeIndex {
            int: Some(Shared::new(3)),
            ..Default::default()
        },
        EdgeIndex {
            int: Some(shared),
            glob: true,
            ..Default::default()
        },
    ];
    let matrix = indices
        .iter()
        .map(|a| indices.iter().map(|b| a.equals(b)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    println!(
        "{}",
        json!({"edge": edge, "empty": Edge::default(), "same": edge.equals(&same), "changed": edge.equals(&changed), "walk": full, "pruned": pruned, "indices": indices, "index_equality": matrix})
    );
}
