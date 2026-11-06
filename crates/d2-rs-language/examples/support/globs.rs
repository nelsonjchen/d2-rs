// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast, ir::*};
use serde_json::{Value, json};
pub fn evaluate(arena: &Arena, nodes: &[NodeId]) -> Value {
    let index = |id: NodeId| nodes.iter().position(|n| *n == id);
    let ids =
        |ids: Option<Vec<NodeId>>| ids.map(|ids| ids.into_iter().map(index).collect::<Vec<_>>());
    let patterns: Vec<Vec<ast::ByteString>> = vec![
        vec!["*".into()],
        vec!["*".into(), "".into(), "*".into()],
        vec!["*".into(), "".into(), "*".into(), "".into(), "*".into()],
    ];
    let globs:Vec<_>=nodes.iter().filter(|n|matches!(arena.node(**n),Node::Map(_))).map(|m|{let rows:Vec<_>=patterns.iter().map(|p|{let(matches,multi)=arena.multi_glob(*m,p);let toward:Vec<_>=nodes.iter().filter(|n|matches!(arena.node(**n),Node::Field(_))).map(|f|json!({"field":index(*f),"path":ids(arena.path_to_field(*m,Some(*f))),"matches":ids(arena.multi_glob_matches_toward(*m,Some(*f),p))})).collect();json!({"matches":ids(matches),"multi":multi,"toward":toward})}).collect();json!({"map":index(*m),"patterns":rows})}).collect();
    let describe = |path: Option<Vec<Option<ast::Child>>>| {
        path.map(|parts|parts.into_iter().map(|part|{let owner=nodes.iter().position(|n|match arena.node(*n){Node::Field(f)=>f.name.unbox().is_some_and(|n|part.as_ref().is_some_and(|p|p.ptr_eq(&n))),_=>false});let bytes=ast::ScalarBox::from_child(part.clone()).scalar_bytes();json!({"ast":part,"raw":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>(),"owner":owner})}).collect::<Vec<_>>())
    };
    let paths:Vec<_>=nodes.iter().map(|n|{let relative:Vec<_>=std::iter::once(None).chain(nodes.iter().copied().map(Some)).map(|p|describe(d2_rs_language::ir::relative_ida(arena,p,*n))).collect();json!({"ida":describe(d2_rs_language::ir::ida(arena,*n)),"board":describe(d2_rs_language::ir::board_ida(arena,*n)),"relative":relative})}).collect();
    json!({"globs":globs,"paths":paths})
}
