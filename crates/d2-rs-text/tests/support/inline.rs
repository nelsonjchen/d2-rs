// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    arena::NodeId,
    block_reader::BlockReader,
    bytes::Bytes,
    context::{Context, NodeKind},
    dispatcher::InlineOptions,
    reference::Reference,
    text::Segment,
};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn tree(c: &Context, id: NodeId, source: &Bytes) -> Value {
    let n = c.arena.node(id);
    let mut children = Vec::new();
    let mut child = n.first;
    while let Some(id) = child {
        children.push(tree(c, id, source));
        child = c.arena.node(id).next;
    }
    let mut data = json!({"children":children,"count":n.child_count,"pos":c.node_position(id)});
    let kind = match &n.data {
        NodeKind::Container | NodeKind::Paragraph(_) => "root",
        NodeKind::CodeSpan => "CodeSpan",
        NodeKind::Text(t) => {
            data["segment"] = json!(t.segment);
            data["raw"] = json!(t.raw);
            data["soft"] = json!(t.soft_line_break);
            data["hard"] = json!(t.hard_line_break);
            "Text"
        }
        NodeKind::Emphasis { level, .. } => {
            data["level"] = json!(level);
            "Emphasis"
        }
        NodeKind::Strikethrough { .. } => "Strikethrough",
        NodeKind::Link(v) | NodeKind::Image(v) => {
            data["destination"] = json!(v.destination.as_ref().map(|v| hex(&v.bytes())));
            data["title"] = json!(v.title.as_ref().map(|v| hex(&v.bytes())));
            data["reference"] = json!(v.reference.as_ref().map(|v| {
                let v = v.borrow();
                json!({"type":v.typ,"value":hex(&v.value)})
            }));
            if matches!(n.data, NodeKind::Link(_)) {
                "Link"
            } else {
                "Image"
            }
        }
        NodeKind::AutoLink(v) => {
            data["type"] = json!(v.typ);
            data["label"] = json!(hex(&c.autolink_label(id, source).bytes()));
            data["url"] = json!(hex(&c.autolink_url(id, source).bytes()));
            "AutoLink"
        }
        NodeKind::RawHtml { segments } => {
            data["segments"] = json!(segments);
            "RawHTML"
        }
        _ => panic!("uncleaned node"),
    };
    data["kind"] = json!(kind);
    data
}
pub fn evaluate(source: &[u8], segments: &[Segment], kind: &str) -> Value {
    let mut c = Context::default();
    for label in ["x", "a", "strasse"] {
        c.references.add(Rc::new(RefCell::new(Reference {
            label: Some(Bytes::new(label.as_bytes())),
            destination: Some(Bytes::new(b"destination")),
            title: Some(Bytes::new(b"title")),
        })));
    }
    let root = c.arena.insert(if kind == "inlineraw" {
        NodeKind::Container
    } else {
        NodeKind::Paragraph(d2_rs_text::markdown::paragraph::Paragraph {
            lines: segments.to_vec(),
            blank_previous_lines: false,
        })
    });
    let mut r = BlockReader::new(source, None);
    c.parse_inline_block(
        root,
        &mut r,
        segments,
        kind == "inlineraw",
        InlineOptions {
            escaped_space: kind == "inlineescaped",
            strikethrough: kind == "inlinestrike",
        },
    );
    let (line, pos) = r.position();
    json!({"tree":tree(&c,root,&r.source()),"line":line,"position":pos,"source":hex(&r.source().bytes())})
}
