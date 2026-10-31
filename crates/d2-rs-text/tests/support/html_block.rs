// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    context::{Context, NodeKind},
    paragraph::BlockState,
    reader::Reader,
    text::Segment,
};
use serde_json::{Value, json};

fn snapshot(
    context: &Context,
    reader: &Reader,
    node: Option<d2_rs_text::markdown::arena::NodeId>,
    state: BlockState,
) -> Value {
    let (line, position) = reader.position();
    let (typ, lines, closure) = node.map_or((None, None, None), |id| {
        let NodeKind::HtmlBlock {
            block_type,
            lines,
            closure_line,
        } = &context.arena.node(id).data
        else {
            panic!()
        };
        let typ = Some(match block_type {
            d2_rs_text::markdown::html_block::HtmlBlockType::Type1 => 1,
            d2_rs_text::markdown::html_block::HtmlBlockType::Type2 => 2,
            d2_rs_text::markdown::html_block::HtmlBlockType::Type3 => 3,
            d2_rs_text::markdown::html_block::HtmlBlockType::Type4 => 4,
            d2_rs_text::markdown::html_block::HtmlBlockType::Type5 => 5,
            d2_rs_text::markdown::html_block::HtmlBlockType::Type6 => 6,
            d2_rs_text::markdown::html_block::HtmlBlockType::Type7 => 7,
        });
        (typ, Some(lines.clone()), *closure_line)
    });
    json!({"type":typ,"lines":lines,"closure":closure,"state":state.0,"line":line,"position":position})
}

pub fn evaluate(source: &[u8], segments: &[Segment]) -> Value {
    let mut context = Context::default();
    let mut reader = Reader::new(source);
    let mut steps = Vec::new();
    if let Some(&segment) = segments.first() {
        reader.set_position(0, segment);
        let (node, state) = context.open_html_block(&mut reader);
        steps.push(snapshot(&context, &reader, node, state));
        if let Some(id) = node {
            for (index, &segment) in segments[1..].iter().enumerate() {
                reader.set_position(index as isize + 1, segment);
                let state = context.continue_html_block(id, &mut reader);
                steps.push(snapshot(&context, &reader, node, state));
                if state == BlockState::CLOSE {
                    break;
                }
            }
        }
    }
    json!({"steps":steps,"source":super::hex(&reader.source().bytes())})
}
