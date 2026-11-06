// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 util/util.go, parser/auto_link.go, ast/inline.go.
use super::{
    arena::NodeId,
    bytes::Bytes,
    context::{Context, NodeKind},
    inline::Text,
    reader_api::ReaderCursor,
    text::Segment,
};
use serde::{Deserialize, Serialize};

pub fn find_url_index(bytes: &[u8]) -> Option<usize> {
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'+' | b'-' | b'.'))
    {
        i += 1;
    }
    if i == 1 || i > 33 || bytes.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    while i < bytes.len() && bytes[i] > 32 && !matches!(bytes[i], b'<' | b'>') {
        i += 1;
    }
    Some(i)
}
pub fn find_email_index(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || b"!#$%&'*+-./=?^_`{|}~".contains(&bytes[i]))
    {
        i += 1;
    }
    if i == 0 || bytes.get(i) != Some(&b'@') {
        return None;
    }
    i += 1;
    if !bytes.get(i).is_some_and(u8::is_ascii_alphanumeric) {
        return None;
    }
    loop {
        let start = i;
        i += 1;
        // Greedy bounded domain component, backing off trailing hyphens just
        // like the original regexp's required final alphanumeric character.
        let mut scan = i;
        while scan < bytes.len()
            && scan - start < 63
            && (bytes[scan].is_ascii_alphanumeric() || bytes[scan] == b'-')
        {
            scan += 1;
        }
        while scan > i && !bytes[scan - 1].is_ascii_alphanumeric() {
            scan -= 1;
        }
        i = scan;
        if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_alphanumeric) {
            i += 1;
        } else {
            break;
        }
    }
    Some(i)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutoLink {
    pub typ: u8,
    pub value: NodeId,
    pub protocol: Option<Bytes>,
}
impl Context {
    pub fn parse_autolink(&mut self, block: &mut impl ReaderCursor) -> Option<NodeId> {
        let (line, segment) = block.peek_line();
        let line = line.expect("autolink parser reached EOF");
        let bytes = line.bytes();
        let (stop, typ) = if let Some(stop) = find_email_index(&bytes[1..]) {
            (stop, 1)
        } else {
            (find_url_index(&bytes[1..])?, 2)
        };
        let stop = stop + 1;
        if bytes.get(stop) != Some(&b'>') {
            return None;
        }
        let value = self.arena.insert(NodeKind::Text(Text::new(Segment::new(
            segment.start + 1,
            segment.start + stop as isize,
        ))));
        block.advance(stop as isize + 1);
        Some(self.arena.insert(NodeKind::AutoLink(AutoLink {
            typ,
            value,
            protocol: None,
        })))
    }
    pub fn autolink_label(&self, id: NodeId, source: &Bytes) -> Bytes {
        let NodeKind::AutoLink(link) = &self.arena.node(id).data else {
            panic!("not an autolink")
        };
        let NodeKind::Text(text) = &self.arena.node(link.value).data else {
            panic!("not autolink text")
        };
        text.segment.value_shared(source)
    }
    pub fn autolink_url(&self, id: NodeId, source: &Bytes) -> Bytes {
        let NodeKind::AutoLink(link) = &self.arena.node(id).data else {
            panic!("not an autolink")
        };
        let label = self.autolink_label(id, source);
        if let Some(protocol) = &link.protocol {
            let mut value = protocol.bytes().to_vec();
            value.extend_from_slice(b"://");
            value.extend_from_slice(&label.bytes());
            Bytes::new(&value)
        } else {
            label
        }
    }
}
