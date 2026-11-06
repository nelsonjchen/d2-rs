// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    arena::NodeId,
    bytes::Bytes,
    context::{Context, NodeKind},
    paragraph::Paragraph,
    text::Segment,
};
use serde_json::{Value, json};
fn snapshot(c: &Context, id: NodeId) -> Value {
    let mut children = Vec::new();
    let mut child = c.arena.node(id).first;
    while let Some(n) = child {
        children.push(snapshot(c, n));
        child = c.arena.node(n).next;
    }
    let mut result =
        json!({"pos":c.node_position(id),"count":c.arena.node(id).child_count,"children":children});
    let kind = match &c.arena.node(id).data {
        NodeKind::CodeSpan => "CodeSpan",
        NodeKind::Text(t) => {
            result["segment"] = json!(t.segment);
            result["raw"] = json!(t.raw);
            result["soft"] = json!(t.soft_line_break);
            result["hard"] = json!(t.hard_line_break);
            "Text"
        }
        NodeKind::Document => "Document",
        NodeKind::Paragraph(p) => {
            result["lines"] = json!(p.lines);
            "Paragraph"
        }
        NodeKind::Table { alignments } => {
            result["alignments"] = json!(alignments);
            "Table"
        }
        NodeKind::TableHeader { alignments } => {
            result["alignments"] = json!(alignments);
            "TableHeader"
        }
        NodeKind::TableRow { alignments } => {
            result["alignments"] = json!(alignments);
            "TableRow"
        }
        NodeKind::TableCell { alignment, lines } => {
            result["alignment"] = json!(alignment);
            result["lines"] = json!(lines);
            "TableCell"
        }
        _ => panic!("unexpected node"),
    };
    result["kind"] = json!(kind);
    result
}
pub fn evaluate(source: &[u8], segments: &[Segment], inline: bool) -> Value {
    let source = Bytes::new(source);
    let mut c = Context::default();
    let root = c.arena.insert(NodeKind::Document);
    let p = c.arena.insert(NodeKind::Paragraph(Paragraph {
        lines: segments.to_vec(),
        blank_previous_lines: false,
    }));
    c.arena.append_child(root, p);
    c.transform_paragraph_table(p, &source);
    if inline {
        fn parse(c: &mut Context, id: NodeId, source: &Bytes) {
            if let NodeKind::TableCell { lines, .. } = &c.arena.node(id).data {
                let lines = lines.clone();
                let mut reader = d2_rs_text::markdown::block_reader::BlockReader::new(
                    &source.bytes(),
                    Some(&lines),
                );
                c.parse_inline_block(id, &mut reader, &lines, false, Default::default());
            }
            let mut child = c.arena.node(id).first;
            while let Some(n) = child {
                parse(c, n, source);
                child = c.arena.node(n).next;
            }
        }
        parse(&mut c, root, &source);
        c.transform_table_escaped_pipes();
    }
    let escaped=c.escaped_pipe_cells.as_ref().map(|entries|entries.iter().map(|e|json!({"cell":snapshot(&c,e.cell),"positions":e.positions,"transformed":e.transformed})).collect::<Vec<_>>()).unwrap_or_default();
    json!({"tree":snapshot(&c,root),"paragraph":snapshot(&c,p),"attached":c.arena.node(p).parent.is_some(),"escaped":escaped,"source":super::hex(&source.bytes())})
}
