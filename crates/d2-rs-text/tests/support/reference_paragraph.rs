// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::markdown::{
    arena::NodeId,
    block_reader::BlockReader,
    bytes::Bytes,
    context::{Context, NodeKind},
    dispatcher::InlineOptions,
    link::normalize_reference_label,
    paragraph::Paragraph,
    reader::Reader,
    text::Segment,
};
use serde_json::{Value, json};
fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}
fn tree(c: &Context, id: NodeId, source: &Bytes) -> Value {
    let n = c.arena.node(id);
    if !matches!(
        n.data,
        NodeKind::Document | NodeKind::Paragraph(_) | NodeKind::ReferenceDefinition(_)
    ) {
        return super::inline_support::tree(c, id, source);
    }
    let mut children = Vec::new();
    let mut current = n.first;
    while let Some(id) = current {
        children.push(tree(c, id, source));
        current = c.arena.node(id).next;
    }
    let mut data = json!({"children":children,"count":n.child_count,"pos":c.node_position(id)});
    match &n.data {
        NodeKind::Document => data["kind"] = json!("root"),
        NodeKind::Paragraph(p) => {
            data["kind"] = json!("Paragraph");
            data["lines"] = json!(p.lines);
            data["blank"] = json!(p.blank_previous_lines);
        }
        NodeKind::ReferenceDefinition(p) => {
            let r = p.reference.borrow();
            data["kind"] = json!("LinkReferenceDefinition");
            data["lines"] = json!(p.lines);
            data["blank"] = json!(p.blank_previous_lines);
            data["label"] = json!(r.label.as_ref().map(|v| hex(&v.bytes())));
            data["destination"] = json!(r.destination.as_ref().map(|v| hex(&v.bytes())));
            data["title"] = json!(r.title.as_ref().map(|v| hex(&v.bytes())));
        }
        _ => unreachable!(),
    }
    data
}
pub fn evaluate(source: &[u8], segments: &[Segment], mutate: bool) -> Value {
    let mut c = Context::default();
    let root = c.arena.insert(NodeKind::Document);
    let paragraph = c.arena.insert(NodeKind::Paragraph(Paragraph {
        lines: segments.to_vec(),
        blank_previous_lines: true,
    }));
    c.arena.append_child(root, paragraph);
    let source = Bytes::new(source);
    c.transform_paragraph_references(paragraph, &source);
    let mut keys = vec![
        b"x".to_vec(),
        b"a".to_vec(),
        b"renamed".to_vec(),
        b"strasse".to_vec(),
    ];
    let mut current = c.arena.node(root).first;
    while let Some(id) = current {
        if let NodeKind::ReferenceDefinition(p) = &c.arena.node(id).data {
            keys.push(normalize_reference_label(
                &p.reference.borrow().label.as_ref().unwrap().bytes(),
            ));
        }
        current = c.arena.node(id).next;
    }
    let snap = |c: &Context| {
        let lookups:Vec<_>=keys.iter().map(|key|c.references.get(key).map(|v|{
            let v=v.borrow();json!({"label":v.label.as_ref().map(|v|hex(&v.bytes())),"destination":v.destination.as_ref().map(|v|hex(&v.bytes())),"title":v.title.as_ref().map(|v|hex(&v.bytes()))})
        })).collect();
        json!({"tree":tree(c,root,&source),"lookups":lookups,"source":hex(&source.bytes())})
    };
    let mut stages = vec![snap(&c)];
    if mutate {
        let mut current = c.arena.node(root).first;
        while let Some(id) = current {
            if let NodeKind::ReferenceDefinition(p) = &c.arena.node(id).data {
                let mut r = p.reference.borrow_mut();
                r.label = Some(Bytes::new(b"renamed"));
                r.destination = Some(Bytes::new(b"changed"));
                r.title = None;
                break;
            }
            current = c.arena.node(id).next;
        }
    }
    stages.push(snap(&c));
    if c.arena.node(paragraph).parent.is_some() {
        let reader = Reader::with_source(source.clone());
        c.close_paragraph(paragraph, &reader);
        let NodeKind::Paragraph(p) = &c.arena.node(paragraph).data else {
            unreachable!()
        };
        let lines = p.lines.clone();
        let mut block = BlockReader::with_source(source.clone(), None);
        c.parse_inline_block(
            paragraph,
            &mut block,
            &lines,
            false,
            InlineOptions::default(),
        );
    }
    stages.push(snap(&c));
    json!(stages)
}
