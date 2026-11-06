// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    arena::NodeId,
    context::{Context, NodeKind},
    paragraph::BlockState,
    reader::Reader,
    text::Segment,
};
use serde_json::{Value, json};
fn snap(c: &Context, r: &Reader, node: Option<NodeId>, state: BlockState, fenced: bool) -> Value {
    let (line, position) = r.position();
    let lines = node.map(|n| {
        let (NodeKind::CodeBlock { lines } | NodeKind::FencedCodeBlock { lines, .. }) =
            &c.arena.node(n).data
        else {
            panic!()
        };
        lines.clone()
    });
    let mut result = json!({"state":state.0,"line":line,"position":position,"lines":lines,"nodepos":node.map(|n|c.node_position(n))});
    if fenced {
        result["context"] = json!(c.fence.is_some());
        result["info"] = json!(node.and_then(|n| {
            let NodeKind::FencedCodeBlock { info, .. } = &c.arena.node(n).data else {
                panic!()
            };
            info.map(|id| {
                let NodeKind::Text(t) = &c.arena.node(id).data else {
                    panic!()
                };
                t.segment
            })
        }));
    }
    result
}
pub fn evaluate(source: &[u8], segments: &[Segment], fenced: bool) -> Value {
    let mut r = Reader::new(source);
    let mut c = Context::default();
    r.set_position(0, segments[0]);
    let (node, mut state) = if fenced {
        let (line, _) = r.peek_line();
        let offset = line
            .map(|v| {
                v.bytes()
                    .iter()
                    .position(|&b| b != b' ' && b != b'\t')
                    .map_or(-1, |n| n as isize)
            })
            .unwrap_or(-1);
        c.open_fenced_code(&mut r, offset)
    } else {
        c.open_code_block(&mut r)
    };
    let mut steps = vec![snap(&c, &r, node, state, fenced)];
    if let Some(n) = node {
        for (i, &segment) in segments[1..].iter().enumerate() {
            r.set_position(i as isize + 1, segment);
            state = if fenced {
                c.continue_fenced_code(n, &mut r)
            } else {
                c.continue_code_block(n, &mut r)
            };
            steps.push(snap(&c, &r, node, state, fenced));
            if state == BlockState::CLOSE {
                break;
            }
        }
        if fenced {
            c.close_fenced_code(n)
        } else {
            c.close_code_block(n, &r)
        };
        steps.push(snap(&c, &r, node, state, fenced));
    }
    json!({"steps":steps,"source":super::hex(&r.source().bytes())})
}
