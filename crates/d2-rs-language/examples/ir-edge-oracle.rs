// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    ast::*,
    ir::{EdgeId, new_edge_ids},
};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
pub struct Request {
    #[serde(default)]
    hex: String,
    mode: String,
    parts: Vec<String>,
}
fn decode(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect()
}
pub fn evaluate(r: Request) -> serde_json::Value {
    if r.mode == "ir_edge_match" {
        let nodes: Vec<_> = r
            .parts
            .iter()
            .map(|p| {
                Some(Child::Block(Shared::new(BlockString {
                    value: decode(p).into(),
                    ..Default::default()
                })))
            })
            .collect();
        let a = EdgeId {
            src_path: Some(vec![nodes[0].clone()]),
            dst_path: Some(vec![nodes[1].clone()]),
            ..Default::default()
        };
        let b = EdgeId {
            src_path: Some(vec![nodes[2].clone()]),
            dst_path: Some(vec![nodes[3].clone()]),
            ..Default::default()
        };
        return json!(a.matches(&b));
    }
    let key = d2_rs_language::parse_map_key(&decode(&r.hex)).unwrap();
    let ids = new_edge_ids(&key);
    let copies: Vec<_> = ids.iter().flatten().cloned().collect();
    let aliases: Vec<_> = ids
        .iter()
        .flatten()
        .zip(&copies)
        .map(|(id, copy)| {
            let path_alias = id
                .src_path
                .as_ref()
                .and_then(|p| p.first())
                .and_then(Option::as_ref)
                .is_some_and(|n| n.ptr_eq(copy.src_path.as_ref().unwrap()[0].as_ref().unwrap()));
            let index_alias = id
                .index
                .as_ref()
                .is_some_and(|i| i.ptr_eq(copy.index.as_ref().unwrap()));
            let slots_alias = id
                .src_path
                .as_ref()
                .and_then(|p| p.first())
                .is_some_and(|p| std::ptr::eq(p, &copy.src_path.as_ref().unwrap()[0]));
            let ast_index_alias = id.index.as_ref().is_some_and(|i| {
                key.edge_index
                    .as_ref()
                    .is_some_and(|ast| ast.read().int.as_ref().is_some_and(|j| i.ptr_eq(j)))
            });
            [path_alias, slots_alias, index_alias, ast_index_alias]
        })
        .collect();
    let peers: Vec<_> = r
        .parts
        .iter()
        .flat_map(|p| {
            new_edge_ids(&d2_rs_language::parse_map_key(&decode(p)).unwrap()).unwrap_or_default()
        })
        .collect();
    let matches = ids.as_ref().map(|ids| {
        ids.iter()
            .map(|id| peers.iter().map(|p| id.matches(p)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    });
    json!({"ids":ids,"copies":copies,"aliases":aliases,"matches":matches})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&requests.into_iter().map(evaluate).collect::<Vec<_>>()).unwrap()
    );
}
