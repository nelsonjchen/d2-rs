// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Goldmark v1.8.5 parser/parser.go delimiter context and ast/inline.go merging.
use super::{
    arena::{Arena, NodeId},
    delimiter::Delimiter,
    inline::Text,
    text::Segment,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkedDelimiter {
    #[serde(flatten)]
    pub value: Delimiter,
    pub previous_delimiter: Option<NodeId>,
    pub next_delimiter: Option<NodeId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum NodeKind {
    Container,
    List {
        marker: u8,
        start: i64,
        is_tight: bool,
    },
    ListItem {
        offset: isize,
        blank_previous_lines: bool,
    },
    TextBlock {
        lines: Vec<Segment>,
        blank_previous_lines: bool,
    },
    HtmlBlock {
        block_type: super::html_block::HtmlBlockType,
        lines: Vec<Segment>,
        closure_line: Option<Segment>,
    },
    Blockquote,
    Heading {
        level: u8,
        lines: Vec<Segment>,
        blank_previous_lines: bool,
    },
    FencedCodeBlock {
        info: Option<NodeId>,
        lines: Vec<Segment>,
    },
    CodeBlock {
        lines: Vec<Segment>,
    },
    ThematicBreak,
    Table {
        alignments: Option<super::table::Alignments>,
    },
    TableHeader {
        alignments: Option<super::table::Alignments>,
    },
    TableRow {
        alignments: Option<super::table::Alignments>,
    },
    TableCell {
        alignment: u8,
        lines: Vec<Segment>,
    },
    Document,
    ReferenceDefinition(super::paragraph::ReferenceDefinition),
    Paragraph(super::paragraph::Paragraph),
    RawHtml {
        segments: Vec<Segment>,
    },
    AutoLink(super::autolink::AutoLink),
    CodeSpan,
    Link(super::link_parser::Link),
    Image(super::link_parser::Link),
    LinkLabel(super::link_state::LinkLabelState),
    Text(Text),
    Delimiter(LinkedDelimiter),
    Emphasis {
        level: isize,
        position: isize,
    },
    Strikethrough {
        position: isize,
    },
}

/// A Go ast.Node boundary distinguishes a nil interface from a typed nil
/// *Delimiter returned by popLinkBottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DelimiterBottom {
    #[default]
    Absent,
    TypedNil,
    Node(NodeId),
}

#[derive(Default, Debug)]
pub struct Context {
    pub(crate) blank_previous_blocks: std::collections::HashSet<NodeId>,
    // Reader columns already consumed by enclosing containers at list open.
    pub(crate) list_reader_origins: std::collections::HashMap<NodeId, isize>,
    pub temporary_paragraph: Option<NodeId>,
    pub skip_list_parser: bool,
    pub empty_list_item_with_blank_lines: bool,
    pub fence: Option<super::fenced_code::FenceData>,
    pub escaped_pipe_cells: Option<Vec<super::table::EscapedPipeCell>>,
    link_bottoms: Vec<Option<NodeId>>,
    pub link_labels: Option<NodeId>,
    pub references: super::reference::References,
    pub arena: Arena<NodeKind>,
    pub first_delimiter: Option<NodeId>,
    pub last_delimiter: Option<NodeId>,
}

impl Context {
    pub(crate) fn block_has_blank_previous_lines(&self, node: NodeId) -> bool {
        self.blank_previous_blocks.contains(&node)
            || match &self.arena.node(node).data {
                NodeKind::Paragraph(p) => p.blank_previous_lines,
                NodeKind::ListItem {
                    blank_previous_lines,
                    ..
                }
                | NodeKind::TextBlock {
                    blank_previous_lines,
                    ..
                }
                | NodeKind::Heading {
                    blank_previous_lines,
                    ..
                } => *blank_previous_lines,
                _ => false,
            }
    }
    pub(crate) fn set_block_blank_previous_lines(&mut self, node: NodeId, blank: bool) {
        if blank {
            self.blank_previous_blocks.insert(node);
        } else {
            self.blank_previous_blocks.remove(&node);
        }
        match &mut self.arena.node_mut(node).data {
            NodeKind::Paragraph(p) => p.blank_previous_lines = blank,
            NodeKind::ListItem {
                blank_previous_lines,
                ..
            }
            | NodeKind::TextBlock {
                blank_previous_lines,
                ..
            }
            | NodeKind::Heading {
                blank_previous_lines,
                ..
            } => *blank_previous_lines = blank,
            _ => (),
        }
    }

    /// Parse and register an emphasis delimiter. The inline dispatcher attaches
    /// a returned node to its parent, as in Goldmark's InlineParser contract.
    pub fn parse_emphasis(
        &mut self,
        reader: &mut impl super::reader_api::ReaderCursor,
    ) -> Option<NodeId> {
        self.parse_delimiter(reader, false)
    }
    pub fn parse_strikethrough(
        &mut self,
        reader: &mut impl super::reader_api::ReaderCursor,
    ) -> Option<NodeId> {
        self.parse_delimiter(reader, true)
    }
    fn parse_delimiter(
        &mut self,
        reader: &mut impl super::reader_api::ReaderCursor,
        strike: bool,
    ) -> Option<NodeId> {
        let before = reader.preceding_character();
        let (line, segment) = reader.peek_line();
        let line = line.expect("delimiter parser reached EOF");
        let mut value = super::delimiter::scan_delimiter(
            &line.bytes(),
            before as i32,
            1,
            if strike { b"~" } else { b"*_" },
        )?;
        if strike && (value.original_length > 2 || before == '~') {
            return None;
        }
        value.segment = segment.with_stop(segment.start + value.original_length);
        reader.advance(value.original_length);
        let id = self.arena.insert(NodeKind::Delimiter(LinkedDelimiter {
            value,
            previous_delimiter: None,
            next_delimiter: None,
        }));
        self.push_delimiter(id);
        Some(id)
    }
    /// Process the emphasis and strikethrough delimiter processors used by D2.
    /// Custom third-party processors are not represented by this context yet.
    pub fn push_link_bottom(&mut self) {
        self.link_bottoms.push(self.last_delimiter);
    }
    pub fn pop_link_bottom(&mut self) -> DelimiterBottom {
        match self.link_bottoms.pop() {
            None => DelimiterBottom::Absent,
            Some(None) => DelimiterBottom::TypedNil,
            Some(Some(id)) => DelimiterBottom::Node(id),
        }
    }
    pub fn clear_link_bottoms(&mut self) {
        self.link_bottoms.clear();
    }
    pub fn process_delimiters(&mut self, bottom: Option<NodeId>) {
        self.process_delimiters_with_bottom(
            bottom.map_or(DelimiterBottom::Absent, DelimiterBottom::Node),
        );
    }
    pub fn process_delimiters_with_bottom(&mut self, boundary: DelimiterBottom) {
        let bottom = match boundary {
            DelimiterBottom::Node(id) => Some(id),
            _ => None,
        };
        let Some(last) = self.last_delimiter else {
            return;
        };
        let mut closer = None;
        if boundary != DelimiterBottom::Absent {
            if bottom != Some(last) {
                let mut current = self.arena.node(last).previous;
                while let Some(id) = current {
                    if Some(id) == bottom {
                        break;
                    }
                    if matches!(self.arena.node(id).data, NodeKind::Delimiter(_)) {
                        closer = Some(id);
                    }
                    current = self.arena.node(id).previous;
                }
            }
        } else {
            closer = self.first_delimiter;
        }
        if closer.is_none() {
            self.clear_delimiters(bottom);
            return;
        }
        while let Some(close_id) = closer {
            if !self.delimiter(close_id).value.can_close {
                closer = self.delimiter(close_id).next_delimiter;
                continue;
            }
            let mut maybe_opener = false;
            let mut found = None;
            let mut opener = self.delimiter(close_id).previous_delimiter;
            while let Some(open_id) = opener {
                if Some(open_id) == bottom {
                    break;
                }
                let open = &self.delimiter(open_id).value;
                let close = &self.delimiter(close_id).value;
                if open.can_open && open.char == close.char {
                    maybe_opener = true;
                    let consume = open.calc_consumption(close);
                    if consume > 0 {
                        found = Some((open_id, consume));
                        break;
                    }
                }
                opener = self.delimiter(open_id).previous_delimiter;
            }
            let Some((open_id, consume)) = found else {
                let next = self.delimiter(close_id).next_delimiter;
                if !maybe_opener && !self.delimiter(close_id).value.can_open {
                    self.remove_delimiter(close_id);
                }
                closer = next;
                continue;
            };
            self.delimiter_mut(open_id)
                .value
                .consume_characters(consume);
            self.delimiter_mut(close_id)
                .value
                .consume_characters(consume);
            let open = &self.delimiter(open_id).value;
            let position = open.segment.start;
            let kind = if open.char == b'~' {
                NodeKind::Strikethrough { position }
            } else {
                NodeKind::Emphasis {
                    level: consume,
                    position,
                }
            };
            let node = self.arena.insert(kind);
            let parent = self
                .arena
                .node(open_id)
                .parent
                .expect("opener has no parent");
            let mut child = self.arena.node(open_id).next;
            while let Some(id) = child {
                if id == close_id {
                    break;
                }
                let next = self.arena.node(id).next;
                self.arena.append_child(node, id);
                child = next;
            }
            self.arena.insert_after(parent, open_id, node);
            let mut current = self.delimiter(open_id).next_delimiter;
            while let Some(id) = current {
                if id == close_id {
                    break;
                }
                let next = self.delimiter(id).next_delimiter;
                self.remove_delimiter(id);
                current = next;
            }
            if self.delimiter(open_id).value.length == 0 {
                self.remove_delimiter(open_id);
            }
            if self.delimiter(close_id).value.length == 0 {
                let next = self.delimiter(close_id).next_delimiter;
                self.remove_delimiter(close_id);
                closer = next;
            }
        }
        self.clear_delimiters(bottom);
    }
    pub fn delimiter(&self, id: NodeId) -> &LinkedDelimiter {
        let NodeKind::Delimiter(d) = &self.arena.node(id).data else {
            panic!("not delimiter")
        };
        d
    }
    pub fn delimiter_mut(&mut self, id: NodeId) -> &mut LinkedDelimiter {
        let NodeKind::Delimiter(d) = &mut self.arena.node_mut(id).data else {
            panic!("not delimiter")
        };
        d
    }
    pub fn push_delimiter(&mut self, id: NodeId) {
        if self.first_delimiter.is_none() {
            self.first_delimiter = Some(id);
            self.last_delimiter = Some(id);
        } else {
            let last = self.last_delimiter.expect("missing last delimiter");
            self.last_delimiter = Some(id);
            self.delimiter_mut(last).next_delimiter = Some(id);
            self.delimiter_mut(id).previous_delimiter = Some(last);
        }
    }
    pub(super) fn merge_or_replace_text_segment(
        &mut self,
        parent: NodeId,
        id: NodeId,
        segment: Segment,
    ) {
        if let Some(previous) = self.arena.node(id).previous
            && let NodeKind::Text(text) = &mut self.arena.node_mut(previous).data
            && text.segment.stop == segment.start
            && !text.soft_line_break
        {
            text.segment = text.segment.with_stop(segment.stop);
            self.arena.remove_child(parent, id);
            return;
        }
        let new = self.arena.insert(NodeKind::Text(Text::new(segment)));
        self.arena.replace_child(parent, id, new);
    }
    pub fn remove_delimiter(&mut self, id: NodeId) {
        let previous = self.delimiter(id).previous_delimiter;
        let next = self.delimiter(id).next_delimiter;
        if let Some(previous) = previous {
            self.delimiter_mut(previous).next_delimiter = next;
            if let Some(next) = next {
                self.delimiter_mut(next).previous_delimiter = Some(previous);
            }
        } else {
            self.first_delimiter = next;
        }
        if next.is_none() {
            self.last_delimiter = previous;
        }
        if let Some(first) = self.first_delimiter {
            self.delimiter_mut(first).previous_delimiter = None;
        }
        if let Some(last) = self.last_delimiter {
            self.delimiter_mut(last).next_delimiter = None;
        }
        let d = self.delimiter_mut(id);
        d.next_delimiter = None;
        d.previous_delimiter = None;
        let length = d.value.length;
        let segment = d.value.segment;
        let parent = self.arena.node(id).parent.expect("delimiter has no parent");
        if length != 0 {
            self.merge_or_replace_text_segment(parent, id, segment);
        } else {
            self.arena.remove_child(parent, id);
        }
    }
    pub fn clear_delimiters(&mut self, bottom: Option<NodeId>) {
        let mut current = self.last_delimiter;
        while let Some(id) = current {
            if Some(id) == bottom {
                break;
            }
            let previous = self.arena.node(id).previous;
            if matches!(self.arena.node(id).data, NodeKind::Delimiter(_)) {
                self.remove_delimiter(id);
            }
            current = previous;
        }
    }
}
