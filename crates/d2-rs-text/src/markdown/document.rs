// SPDX-License-Identifier: MPL-2.0 AND MIT
// Copyright (c) 2019 Yusuke Inuzuka
// Rust translation Copyright 2026 Nelson Chen.
// Native document wiring for the currently translated Goldmark block stages.

use super::{
    arena::NodeId,
    block::is_thematic_break,
    block_reader::BlockReader,
    context::{Context, NodeKind},
    dispatcher::InlineOptions,
    inline::Text,
    paragraph::Paragraph,
    reader::Reader,
    reader_api::BlockCursor,
    text::{Segment, is_space},
};

fn first_non_space(line: &[u8]) -> Option<usize> {
    line.iter().position(|&byte| !matches!(byte, b' ' | b'\t'))
}

/// Goldmark lets an inline code span consume following physical lines before
/// the enclosing list item is closed. The block dispatcher therefore needs a
/// small amount of delimiter state while it is collecting list paragraph
/// lines; otherwise an unindented line after an opening backtick is detached
/// from the item and substitutions are applied inside what should be code.
fn has_unclosed_code_span(lines: &[Segment], source: &[u8]) -> bool {
    let mut opener = None;
    for segment in lines {
        let bytes = segment.value(source);
        let bytes = bytes.as_ref();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'`' {
                index += 1;
                continue;
            }
            let mut escaped = 0;
            let mut before = index;
            while before > 0 && bytes[before - 1] == b'\\' {
                escaped += 1;
                before -= 1;
            }
            let start = index;
            while index < bytes.len() && bytes[index] == b'`' {
                index += 1;
            }
            let length = index - start;
            if escaped % 2 != 0 {
                continue;
            }
            if opener == Some(length) {
                opener = None;
            } else if opener.is_none() {
                opener = Some(length);
            }
        }
    }
    opener.is_some()
}

fn list_interrupts_paragraph(line: &[u8]) -> bool {
    let (matched, kind) = super::list::parse_list_item(line);
    if kind == super::list::ListItemType::NotList || matched[4] < 0 {
        return false;
    }
    if line[matched[4] as usize..matched[5] as usize]
        .iter()
        .all(|&byte| is_space(byte))
    {
        return false;
    }
    kind != super::list::ListItemType::Ordered
        || std::str::from_utf8(&line[matched[2] as usize..matched[3] as usize - 1])
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            == Some(1)
}

fn atx_heading_interrupts_paragraph(line: &[u8]) -> bool {
    let Some(start) = first_non_space(line) else {
        return false;
    };
    let mut end = start;
    while end < line.len() && line[end] == b'#' {
        end += 1;
    }
    let level = end - start;
    (1..=6).contains(&level) && (end == line.len() || line[end].is_ascii_whitespace())
}

fn fenced_code_interrupts_paragraph(line: &[u8]) -> bool {
    let Some(start) = first_non_space(line) else {
        return false;
    };
    if start > 3 || !matches!(line[start], b'`' | b'~') {
        return false;
    }
    let character = line[start];
    let mut end = start;
    while end < line.len() && line[end] == character {
        end += 1;
    }
    if end - start < 3 {
        return false;
    }
    if character == b'`' {
        let info = &line[end..];
        let first = info.iter().position(|&byte| !is_space(byte));
        if let Some(first) = first
            && info[first..].contains(&b'`')
        {
            return false;
        }
    }
    true
}

fn quote_allows_lazy_continuation(
    quoted_lines: &[Segment],
    line: &[u8],
    line_offset: isize,
    source: &[u8],
) -> bool {
    if line.iter().all(|&byte| is_space(byte)) {
        return false;
    }
    let Some(last) = quoted_lines
        .iter()
        .rev()
        .find(|segment| !quote_segment_is_blank(**segment, source))
    else {
        return false;
    };
    let last_bytes = last.value(source);
    if atx_heading_interrupts_paragraph(last_bytes.as_ref())
        || fenced_code_interrupts_paragraph(last_bytes.as_ref())
        || is_thematic_break(last_bytes.as_ref(), line_offset)
        || list_interrupts_paragraph(last_bytes.as_ref())
        || super::html_block::can_interrupt_paragraph(last_bytes.as_ref())
    {
        return false;
    }
    !atx_heading_interrupts_paragraph(line)
        && !fenced_code_interrupts_paragraph(line)
        && !is_thematic_break(line, line_offset)
        && !list_interrupts_paragraph(line)
        && !super::html_block::can_interrupt_paragraph(line)
}

fn append_inline(context: &mut Context, node: NodeId, source: &[u8]) {
    let lines = match &context.arena.node(node).data {
        NodeKind::Paragraph(paragraph) => paragraph.lines.clone(),
        NodeKind::Heading { lines, .. } => lines.clone(),
        NodeKind::TextBlock { lines, .. } => lines.clone(),
        NodeKind::TableCell { lines, .. } => lines.clone(),
        _ => return,
    };
    let mut reader = BlockReader::new(source, Some(&lines));
    // D2 enables Goldmark's strikethrough extension for every rendered
    // Markdown label. Keep the lower-level dispatcher option explicit so its
    // direct tests can still model a caller that has disabled the extension.
    context.parse_inline_block(
        node,
        &mut reader,
        &lines,
        false,
        InlineOptions {
            strikethrough: true,
            ..InlineOptions::default()
        },
    );
}

fn finish_paragraph(context: &mut Context, node: NodeId, reader: &impl BlockCursor, source: &[u8]) {
    let source_bytes = super::bytes::Bytes::new(source);
    context.transform_paragraph_references(node, &source_bytes);
    if context.arena.node(node).parent.is_some() {
        context.transform_paragraph_table(node, &source_bytes);
    }
    if context.arena.node(node).parent.is_some() {
        context.close_paragraph(node, reader);
    }
}

// Goldmark parseBlocks continues open outer containers before inner items.
// The recursive collector can revisit a line after an inner block closes;
// already-consumed columns must not be stripped a second time.
fn continue_item_containers(
    context: &mut Context,
    item: NodeId,
    reader: &mut impl BlockCursor,
) -> super::paragraph::BlockState {
    let mut items = Vec::new();
    let mut node = Some(item);
    while let Some(id) = node {
        if matches!(context.arena.node(id).data, NodeKind::Blockquote) {
            break;
        }
        if matches!(context.arena.node(id).data, NodeKind::ListItem { .. }) {
            items.push(id);
        }
        node = context.arena.node(id).parent;
    }
    for id in items.into_iter().rev() {
        let list = context.arena.node(id).parent.expect("unattached item");
        let origin = context.list_reader_origins.get(&list).copied().unwrap_or(0);
        let offset = match context.arena.node(id).data {
            NodeKind::ListItem { offset, .. } => offset,
            _ => unreachable!(),
        };
        if reader.line_offset() < origin + offset
            && context.continue_list_item(id, reader) == super::paragraph::BlockState::CLOSE
        {
            return super::paragraph::BlockState::CLOSE;
        }
    }
    super::paragraph::BlockState(10)
}

fn append_list_item_content(
    context: &mut Context,
    item: NodeId,
    reader: &mut impl BlockCursor,
    source: &[u8],
    blank_before: bool,
) -> bool {
    let (line, segment) = reader.peek_line();
    let Some(line) = line else { return false };
    if line.bytes().iter().all(|&byte| is_space(byte)) {
        reader.advance_to_eol();
        reader.advance_line();
        return true;
    }
    let bytes = line.bytes();
    let offset = first_non_space(&bytes).unwrap_or(bytes.len()) as isize;
    if let (Some(node), _) = context.open_atx_heading(reader, offset) {
        context.set_node_position(node, segment.start + offset);
        context.set_block_blank_previous_lines(node, blank_before);
        context.arena.append_child(item, node);
        append_inline(context, node, source);
        return append_list_continuation(context, item, reader, source, false).1;
    }
    if super::block::indent_width(&bytes, reader.line_offset()).0 <= 3
        && let (Some(node), _) = context.open_fenced_code(reader, offset)
    {
        context.set_node_position(node, segment.start + offset);
        context.set_block_blank_previous_lines(node, blank_before);
        context.arena.append_child(item, node);
        reader.advance_line();
        loop {
            // A block inside a list item may only consume continuation lines
            // that still belong to the item.  Goldmark closes the list before
            // an unindented line; continuing the HTML/fence node blindly
            // would swallow that line (and any substitutions it contains).
            if continue_item_containers(context, item, reader)
                == super::paragraph::BlockState::CLOSE
            {
                break;
            }
            let before = reader.position();
            if context.continue_fenced_code(node, reader) == super::paragraph::BlockState::CLOSE {
                reader.advance_line();
                break;
            }
            reader.advance_line();
            if reader.position() == before {
                break;
            }
        }
        context.close_fenced_code(node);
        return append_list_continuation(context, item, reader, source, false).1;
    }
    if let (Some(node), _) = context.open_code_block(reader) {
        context.set_node_position(node, segment.start + offset);
        context.set_block_blank_previous_lines(node, blank_before);
        context.arena.append_child(item, node);
        reader.advance_line();
        loop {
            if continue_item_containers(context, item, reader)
                == super::paragraph::BlockState::CLOSE
            {
                break;
            }
            let before = reader.position();
            if context.continue_code_block(node, reader) == super::paragraph::BlockState::CLOSE {
                break;
            }
            reader.advance_line();
            if reader.position() == before {
                break;
            }
        }
        context.close_code_block(node, reader);
        return append_list_continuation(context, item, reader, source, false).1;
    }
    if let (Some(node), _) = context.open_thematic_break(reader) {
        context.set_node_position(node, segment.start + offset);
        context.set_block_blank_previous_lines(node, blank_before);
        context.arena.append_child(item, node);
        reader.advance_line();
        return append_list_continuation(context, item, reader, source, false).1;
    }
    if let (Some(node), _) = context.open_html_block(reader) {
        context.set_node_position(node, segment.start + offset);
        context.set_block_blank_previous_lines(node, blank_before);
        context.arena.append_child(item, node);
        reader.advance_line();
        loop {
            if continue_item_containers(context, item, reader)
                == super::paragraph::BlockState::CLOSE
            {
                break;
            }
            let before = reader.position();
            if context.continue_html_block(node, reader) == super::paragraph::BlockState::CLOSE {
                if reader.position() != before {
                    reader.advance_line();
                }
                break;
            }
            reader.advance_line();
        }
        return append_list_continuation(context, item, reader, source, false).1;
    }
    if let (Some(node), _) = context.open_blockquote(reader) {
        context.set_node_position(node, segment.start + offset);
        context.set_block_blank_previous_lines(node, blank_before);
        context.arena.append_child(item, node);
        let mut quoted_lines = Vec::new();
        if reader.peek_line().0.is_some() {
            quoted_lines.push(reader.peek_line().1);
        }
        reader.advance_to_eol();
        reader.advance_line();
        loop {
            if !super::block::process_blockquote(reader) {
                let (next, next_segment) = reader.peek_line();
                if let Some(next) = next
                    && quote_allows_lazy_continuation(
                        &quoted_lines,
                        &next.bytes(),
                        reader.line_offset(),
                        source,
                    )
                {
                    quoted_lines.push(next_segment);
                    reader.advance_to_eol();
                    reader.advance_line();
                    continue;
                }
                break;
            }
            if reader.peek_line().0.is_some() {
                quoted_lines.push(reader.peek_line().1);
            }
            reader.advance_to_eol();
            reader.advance_line();
        }
        append_quote_contents(context, node, quoted_lines, source);
        return append_list_continuation(context, item, reader, source, false).1;
    }
    let paragraph = context.arena.insert(NodeKind::Paragraph(Paragraph {
        lines: vec![segment],
        blank_previous_lines: blank_before,
    }));
    context.arena.append_child(item, paragraph);
    reader.advance_to_eol();
    reader.advance_line();
    loop {
        let before = reader.position();
        let state = continue_item_containers(context, item, reader);
        let closed = state == super::paragraph::BlockState::CLOSE;
        if closed {
            let keep_inline_span = matches!(context.arena.node(paragraph).data, NodeKind::Paragraph(ref value) if has_unclosed_code_span(&value.lines, source));
            if !keep_inline_span {
                break;
            }
        }
        let (next, next_segment) = reader.peek_line();
        let Some(next) = next else { break };
        if next.bytes().iter().all(|&byte| is_space(byte)) {
            break;
        }
        if !closed && reader.position() == before {
            break;
        }
        if !closed {
            let (heading, _) = context.open_setext_heading(item, Some(paragraph), reader);
            if let Some(heading) = heading {
                context.arena.append_child(item, heading);
                context.close_paragraph(paragraph, reader);
                context.close_setext_heading(heading, reader);
                append_inline(context, heading, source);
                reader.advance_to_eol();
                reader.advance_line();
                return append_list_continuation(context, item, reader, source, false).1;
            }
            let bytes = next.bytes();
            let quote = first_non_space(&bytes).is_some_and(|i| {
                bytes[i] == b'>' && super::block::indent_width(&bytes, reader.line_offset()).0 <= 3
            });
            if atx_heading_interrupts_paragraph(&bytes)
                || fenced_code_interrupts_paragraph(&bytes)
                || quote
                || is_thematic_break(&bytes, reader.line_offset())
                || super::html_block::can_interrupt_paragraph(&bytes)
            {
                finish_paragraph(context, paragraph, reader, source);
                return append_list_item_content(context, item, reader, source, false);
            }
        }
        if super::list::parse_list_item(&next.bytes()).1 != super::list::ListItemType::NotList {
            finish_paragraph(context, paragraph, reader, source);
            return parse_list(context, item, reader, source, false)
                .map(|(_, blank)| blank)
                .unwrap_or(false);
        }
        if let NodeKind::Paragraph(value) = &mut context.arena.node_mut(paragraph).data {
            value.lines.push(next_segment);
        }
        reader.advance_to_eol();
        reader.advance_line();
    }
    finish_paragraph(context, paragraph, reader, source);
    false
}

fn append_list_continuation(
    context: &mut Context,
    item: NodeId,
    reader: &mut impl BlockCursor,
    source: &[u8],
    mut blank_before: bool,
) -> (bool, bool) {
    let mut added = false;
    loop {
        if continue_item_containers(context, item, reader) == super::paragraph::BlockState::CLOSE {
            break;
        }
        let (line, _) = reader.peek_line();
        let Some(line) = line else {
            break;
        };
        if line.bytes().iter().all(|&byte| is_space(byte)) {
            reader.advance_line();
            blank_before = true;
            continue;
        }
        if super::list::parse_list_item(&line.bytes()).1 != super::list::ListItemType::NotList {
            let trailing = parse_list(context, item, reader, source, blank_before)
                .map(|(_, blank)| blank)
                .unwrap_or(false);
            return (true, trailing);
        }
        // A continuation can open any block parser, not just a paragraph.
        // Keep the separator state owned by this container across recursion.
        let before = reader.position();
        blank_before = append_list_item_content(context, item, reader, source, blank_before);
        added = true;
        if reader.position() == before {
            break;
        }
    }
    (added, blank_before)
}

fn parse_list(
    context: &mut Context,
    parent: NodeId,
    reader: &mut impl BlockCursor,
    source: &[u8],
    blank_previous: bool,
) -> Option<(NodeId, bool)> {
    let (list, _) = context.open_list(parent, None, reader);
    let list = list?;
    context.set_block_blank_previous_lines(list, blank_previous);
    let list_position = reader.peek_line().1.start;
    let list_indent = reader.line_offset();
    context.list_reader_origins.insert(list, list_indent);
    context.set_node_position(list, list_position);
    context.arena.append_child(parent, list);
    let mut blank_before_item = false;
    let mut last_item = None;
    loop {
        if matches!(context.arena.node(parent).data, NodeKind::ListItem { .. })
            && continue_item_containers(context, parent, reader)
                == super::paragraph::BlockState::CLOSE
        {
            break;
        }
        // Goldmark keeps blank lines in the list so Continue can mark the
        // current item as loose before the next item is opened.  The
        // item-content helper intentionally leaves the reader on the blank
        // line; consume the run here and let the list parser inspect the
        // following marker at the same indentation.
        while reader
            .peek_line()
            .0
            .is_some_and(|line| line.bytes().iter().all(|&byte| is_space(byte)))
        {
            if last_item.is_some_and(|id| context.arena.node(id).child_count == 0) {
                context.empty_list_item_with_blank_lines = true;
            }
            reader.advance_line();
            blank_before_item = true;
        }
        if let Some(item) = last_item {
            if context.continue_list(list, reader) == super::paragraph::BlockState::CLOSE {
                break;
            }
            if continue_item_containers(context, item, reader)
                != super::paragraph::BlockState::CLOSE
            {
                let (added, trailing_blank) =
                    append_list_continuation(context, item, reader, source, blank_before_item);
                if added {
                    blank_before_item = trailing_blank;
                    continue;
                }
            }
        }
        if reader.line_offset() < list_indent {
            break;
        }
        let before = reader.position();
        let (item, _) = context.open_list_item(list, reader);
        let Some(item) = item else { break };
        context.arena.append_child(list, item);
        last_item = Some(item);
        if blank_before_item {
            if let NodeKind::ListItem {
                blank_previous_lines,
                ..
            } = &mut context.arena.node_mut(item).data
            {
                *blank_previous_lines = true;
            }
            blank_before_item = false;
        }
        if reader.position() == before {
            reader.advance_to_eol();
            reader.advance_line();
        } else {
            blank_before_item = append_list_item_content(context, item, reader, source, false);
        }
    }
    context.close_list(list);
    append_list_inlines(context, list, source);
    context.skip_list_parser = false;
    Some((list, blank_before_item))
}

fn append_list_inlines(context: &mut Context, list: NodeId, source: &[u8]) {
    let mut items = Vec::new();
    let mut item = context.arena.node(list).first;
    while let Some(item_id) = item {
        items.push(item_id);
        item = context.arena.node(item_id).next;
    }
    for item_id in items {
        let mut child = context.arena.node(item_id).first;
        let mut children = Vec::new();
        while let Some(child_id) = child {
            children.push(child_id);
            child = context.arena.node(child_id).next;
        }
        for child_id in children {
            // Blocks opened eagerly (such as headings) already own their
            // inline nodes. Tight paragraph replacement creates a new block.
            if context.arena.node(child_id).child_count == 0 {
                append_inline(context, child_id, source);
            }
        }
    }
}

fn append_table_inlines(context: &mut Context, node: NodeId, source: &[u8]) {
    if matches!(context.arena.node(node).data, NodeKind::TableCell { .. }) {
        append_inline(context, node, source);
    }
    let mut child = context.arena.node(node).first;
    let mut children = Vec::new();
    while let Some(child_id) = child {
        children.push(child_id);
        child = context.arena.node(child_id).next;
    }
    for child_id in children {
        append_table_inlines(context, child_id, source);
    }
}

fn nested_blockquote_segment(
    segment: super::text::Segment,
    source: &[u8],
) -> Option<super::text::Segment> {
    let value = segment.value(source);
    let bytes = value.as_ref();
    let position = first_non_space(bytes)?;
    if position > 3 || bytes.get(position) != Some(&b'>') {
        return None;
    }
    let mut skip = position + 1;
    if bytes
        .get(skip)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        skip += 1;
    }
    let physical_skip = skip as isize - segment.padding;
    if physical_skip <= 0 {
        Some(super::text::Segment::with_padding(
            segment.start,
            segment.stop,
            segment.padding - skip as isize,
        ))
    } else {
        Some(super::text::Segment::new(
            segment.start + physical_skip,
            segment.stop,
        ))
    }
}

fn quote_fence_open(
    segment: Segment,
    source: &[u8],
) -> Option<(u8, usize, Option<Segment>, isize)> {
    let value = segment.value(source);
    let bytes = value.as_ref();
    let position = first_non_space(bytes)?;
    if position > 3 {
        return None;
    }
    let character = *bytes.get(position)?;
    if !matches!(character, b'~' | b'`') {
        return None;
    }
    let mut end = position;
    while end < bytes.len() && bytes[end] == character {
        end += 1;
    }
    let length = end - position;
    if length < 3 {
        return None;
    }
    let rest = &bytes[end..];
    let left = rest.iter().take_while(|&&byte| is_space(byte)).count();
    let right = rest
        .iter()
        .rev()
        .take_while(|&&byte| is_space(byte))
        .count();
    if left >= rest.len().saturating_sub(right) {
        return Some((character, length, None, position as isize));
    }
    let info_start = end + left;
    let info_stop = rest.len() + end - right;
    if character == b'`' && rest[left..rest.len() - right].contains(&b'`') {
        return None;
    }
    let base = segment.start - segment.padding;
    Some((
        character,
        length,
        Some(Segment::new(
            base + info_start as isize,
            base + info_stop as isize,
        )),
        position as isize,
    ))
}

fn quote_fence_close(segment: Segment, source: &[u8], character: u8, length: usize) -> bool {
    let value = segment.value(source);
    let bytes = value.as_ref();
    let position = first_non_space(bytes).unwrap_or(bytes.len());
    if position > 3 {
        return false;
    }
    let mut end = position;
    while end < bytes.len() && bytes[end] == character {
        end += 1;
    }
    end - position >= length && bytes[end..].iter().all(|&byte| is_space(byte))
}

fn quote_segment_is_blank(segment: Segment, source: &[u8]) -> bool {
    segment.value(source).iter().all(|&byte| is_space(byte))
}

fn quote_indent_width(segment: Segment, source: &[u8]) -> usize {
    let bytes = segment.value(source);
    let mut width = 0;
    for &byte in bytes.as_ref() {
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4 - width % 4,
            _ => break,
        }
    }
    width
}

fn append_quote_contents(
    context: &mut Context,
    blockquote: NodeId,
    quoted_lines: Vec<super::text::Segment>,
    source: &[u8],
) {
    // A quote marker can introduce another quote after the outer marker has
    // already been consumed. Split those lines and recurse so the AST retains
    // Goldmark's nested Blockquote containers instead of flattening them into
    // one paragraph.
    if quoted_lines
        .iter()
        .any(|segment| nested_blockquote_segment(*segment, source).is_some())
    {
        let mut index = 0;
        while index < quoted_lines.len() {
            if nested_blockquote_segment(quoted_lines[index], source).is_some() {
                let position = quoted_lines[index].start;
                let mut nested = Vec::new();
                while index < quoted_lines.len() {
                    let Some(segment) = nested_blockquote_segment(quoted_lines[index], source)
                    else {
                        break;
                    };
                    nested.push(segment);
                    index += 1;
                }
                let child = context.arena.insert(NodeKind::Blockquote);
                context.set_node_position(child, position);
                context.arena.append_child(blockquote, child);
                append_quote_contents(context, child, nested, source);
            } else {
                let start = index;
                while index < quoted_lines.len()
                    && nested_blockquote_segment(quoted_lines[index], source).is_none()
                {
                    index += 1;
                }
                append_quote_contents(
                    context,
                    blockquote,
                    quoted_lines[start..index].to_vec(),
                    source,
                );
            }
        }
        return;
    }

    if quoted_lines.is_empty() {
        return;
    }
    let first_nonblank = quoted_lines
        .iter()
        .find(|segment| !quote_segment_is_blank(**segment, source));
    let first_is_list = first_nonblank.is_some_and(|segment| {
        let value = segment.value(source);
        super::list::parse_list_item(value.as_ref()).1 != super::list::ListItemType::NotList
    });
    if quoted_lines
        .iter()
        .any(|segment| quote_segment_is_blank(*segment, source))
        && !first_is_list
    {
        let mut group = Vec::new();
        for segment in quoted_lines {
            if quote_segment_is_blank(segment, source) {
                if !group.is_empty() {
                    append_quote_contents(context, blockquote, std::mem::take(&mut group), source);
                }
            } else {
                group.push(segment);
            }
        }
        if !group.is_empty() {
            append_quote_contents(context, blockquote, group, source);
        }
        return;
    }
    let first_bytes = quoted_lines[0].value(source);
    if is_thematic_break(first_bytes.as_ref(), 0) {
        let node = context.arena.insert(NodeKind::ThematicBreak);
        context.set_node_position(node, quoted_lines[0].start);
        context.arena.append_child(blockquote, node);
        if quoted_lines.len() > 1 {
            append_quote_contents(context, blockquote, quoted_lines[1..].to_vec(), source);
        }
        return;
    }
    if quote_indent_width(quoted_lines[0], source) >= 4 {
        let node = context
            .arena
            .insert(NodeKind::CodeBlock { lines: Vec::new() });
        context.set_node_position(node, quoted_lines[0].start);
        context.arena.append_child(blockquote, node);
        let mut close_at = quoted_lines.len();
        for (index, segment) in quoted_lines.iter().enumerate() {
            if index > 0
                && !quote_segment_is_blank(*segment, source)
                && quote_indent_width(*segment, source) < 4
            {
                close_at = index;
                break;
            }
            let mut line = segment.trim_left_space_width(4, source);
            line.force_newline = true;
            if let NodeKind::CodeBlock { lines } = &mut context.arena.node_mut(node).data {
                lines.push(line);
            }
        }
        if close_at < quoted_lines.len() {
            append_quote_contents(
                context,
                blockquote,
                quoted_lines[close_at..].to_vec(),
                source,
            );
        }
        return;
    }
    let mut html_reader = Reader::new(source);
    html_reader.set_position(0, quoted_lines[0]);
    if let (Some(node), _) = context.open_html_block(&mut html_reader) {
        context.set_node_position(node, quoted_lines[0].start);
        context.arena.append_child(blockquote, node);
        let mut close_at = quoted_lines.len();
        for (index, segment) in quoted_lines.iter().enumerate().skip(1) {
            html_reader.set_position(index as isize, *segment);
            let before = html_reader.position();
            if context.continue_html_block(node, &mut html_reader)
                == super::paragraph::BlockState::CLOSE
            {
                close_at = if html_reader.position() == before {
                    index
                } else {
                    index + 1
                };
                break;
            }
        }
        if close_at < quoted_lines.len() {
            append_quote_contents(
                context,
                blockquote,
                quoted_lines[close_at..].to_vec(),
                source,
            );
        }
        return;
    }
    if quoted_lines.len() > 1
        && let Some(mark) =
            super::heading::matches_setext_heading_bar(quoted_lines[1].value(source).as_ref())
    {
        let heading = context.arena.insert(NodeKind::Heading {
            level: if mark == b'-' { 2 } else { 1 },
            lines: vec![quoted_lines[0]],
            blank_previous_lines: false,
        });
        context.set_node_position(heading, quoted_lines[0].start);
        context.arena.append_child(blockquote, heading);
        append_inline(context, heading, source);
        if quoted_lines.len() > 2 {
            append_quote_contents(context, blockquote, quoted_lines[2..].to_vec(), source);
        }
        return;
    }
    if let Some((character, length, info_segment, position)) =
        quote_fence_open(quoted_lines[0], source)
    {
        let info =
            info_segment.map(|segment| context.arena.insert(NodeKind::Text(Text::new(segment))));
        let node = context.arena.insert(NodeKind::FencedCodeBlock {
            info,
            lines: Vec::new(),
        });
        context.set_node_position(node, quoted_lines[0].start + position);
        context.arena.append_child(blockquote, node);
        let mut close_at = quoted_lines.len();
        if quoted_lines.len() > 1 {
            for (index, segment) in quoted_lines.iter().enumerate().skip(1) {
                if quote_fence_close(*segment, source, character, length) {
                    close_at = index;
                    break;
                }
                let mut line = *segment;
                line.force_newline = true;
                if let NodeKind::FencedCodeBlock { lines, .. } =
                    &mut context.arena.node_mut(node).data
                {
                    lines.push(line);
                }
            }
        }
        if close_at < quoted_lines.len() && close_at + 1 < quoted_lines.len() {
            append_quote_contents(
                context,
                blockquote,
                quoted_lines[close_at + 1..].to_vec(),
                source,
            );
        }
        return;
    }
    let first_value = quoted_lines[0].value(source);
    let first_offset = first_non_space(first_value.as_ref()).unwrap_or(first_value.len()) as isize;
    if first_offset >= 0 {
        let mut quote_reader = Reader::new(source);
        quote_reader.set_position(0, quoted_lines[0]);
        if let (Some(heading), _) = context.open_atx_heading(&mut quote_reader, first_offset) {
            context.set_node_position(heading, quoted_lines[0].start + first_offset);
            context.arena.append_child(blockquote, heading);
            append_inline(context, heading, source);
            if quoted_lines.len() > 1 {
                append_quote_contents(context, blockquote, quoted_lines[1..].to_vec(), source);
            }
            return;
        }
    }
    let first = quoted_lines[0].value(source).into_owned();
    let (_, kind) = super::list::parse_list_item(&first);
    if kind != super::list::ListItemType::NotList {
        let mut quote_reader = BlockReader::new(source, Some(&quoted_lines));
        let _ = parse_list(context, blockquote, &mut quote_reader, source, false);
        let consumed = usize::try_from(quote_reader.position().0)
            .unwrap_or(quoted_lines.len())
            .min(quoted_lines.len());
        if consumed < quoted_lines.len() {
            append_quote_contents(
                context,
                blockquote,
                quoted_lines[consumed..].to_vec(),
                source,
            );
        }
    } else {
        let paragraph = context.arena.insert(NodeKind::Paragraph(Paragraph {
            lines: quoted_lines,
            blank_previous_lines: false,
        }));
        context.arena.append_child(blockquote, paragraph);
        let reader = Reader::new(source);
        finish_paragraph(context, paragraph, &reader, source);
        append_inline(context, paragraph, source);
    }
}

/// Parse the subset of document blocks whose individual stages are currently
/// translated. The return keeps the arena alive with the root NodeId so callers
/// can inspect the native AST and run later transforms.
pub fn parse_document(source: &[u8]) -> (Context, NodeId) {
    let mut context = Context::default();
    let root = context.arena.insert(NodeKind::Document);
    let mut reader = Reader::new(source);
    loop {
        let (line, segment) = reader.peek_line();
        let Some(line) = line else { break };
        let bytes = line.bytes();
        if bytes.iter().all(|&byte| is_space(byte)) {
            reader.advance_line();
            continue;
        }
        let offset = first_non_space(&bytes).unwrap_or(bytes.len()) as isize;
        if let (Some(node), _) = context.open_atx_heading(&mut reader, offset) {
            context.set_node_position(node, segment.start + offset);
            context.arena.append_child(root, node);
            reader.advance_line();
            append_inline(&mut context, node, source);
            continue;
        }
        // A blockquote marker is only a block opener when its indentation is
        // below the four-space code-block threshold.  Looking only at the
        // first non-space byte misclassifies `    >text` and, because the
        // marker processor correctly rejects that indentation, can leave the
        // document reader unchanged forever.
        if bytes.get(offset as usize) == Some(&b'>')
            && super::block::indent_width(&bytes, reader.line_offset()).0 <= 3
        {
            let blockquote = context.arena.insert(NodeKind::Blockquote);
            context.set_node_position(blockquote, segment.start + offset);
            context.arena.append_child(root, blockquote);
            let mut quoted_lines = Vec::new();
            loop {
                if !super::block::process_blockquote(&mut reader) {
                    let (next, next_segment) = reader.peek_line();
                    if let Some(next) = next
                        && quote_allows_lazy_continuation(
                            &quoted_lines,
                            &next.bytes(),
                            reader.line_offset(),
                            source,
                        )
                    {
                        quoted_lines.push(next_segment);
                        reader.advance_to_eol();
                        reader.advance_line();
                        continue;
                    }
                    break;
                }
                let (quoted, quoted_segment) = reader.peek_line();
                if quoted.is_some() {
                    quoted_lines.push(quoted_segment);
                }
                reader.advance_to_eol();
                reader.advance_line();
            }
            append_quote_contents(&mut context, blockquote, quoted_lines, source);
            continue;
        }
        if bytes.get(offset as usize) == Some(&b'<')
            && let (Some(node), _) = context.open_html_block(&mut reader)
        {
            context.set_node_position(node, segment.start + offset);
            context.arena.append_child(root, node);
            reader.advance_line();
            loop {
                let before = reader.position();
                let state = context.continue_html_block(node, &mut reader);
                if state == super::paragraph::BlockState::CLOSE {
                    if reader.position() != before {
                        reader.advance_line();
                    }
                    break;
                }
                reader.advance_line();
            }
            continue;
        }
        if is_thematic_break(&bytes, reader.line_offset()) {
            let (node, _) = context.open_thematic_break(&mut reader);
            if let Some(node) = node {
                context.set_node_position(node, segment.start + offset);
                context.arena.append_child(root, node);
            }
            reader.advance_line();
            continue;
        }
        if super::block::indent_width(&bytes, reader.line_offset()).0 <= 3
            && let (Some(node), _) = context.open_fenced_code(&mut reader, offset)
        {
            context.set_node_position(node, segment.start + offset);
            context.arena.append_child(root, node);
            reader.advance_line();
            loop {
                let before = reader.position();
                let state = context.continue_fenced_code(node, &mut reader);
                if state == super::paragraph::BlockState::CLOSE {
                    reader.advance_line();
                    break;
                }
                reader.advance_line();
                if reader.position() == before {
                    break;
                }
            }
            context.close_fenced_code(node);
            continue;
        }
        if let (Some(node), _) = context.open_code_block(&mut reader) {
            context.set_node_position(node, segment.start + offset);
            context.arena.append_child(root, node);
            reader.advance_line();
            loop {
                let before = reader.position();
                let state = context.continue_code_block(node, &mut reader);
                if state == super::paragraph::BlockState::CLOSE {
                    break;
                }
                // At EOF the Go reader returns a blank synthetic line while
                // leaving its position unchanged. Advance once before the
                // no-progress check so real blank lines are retained.
                reader.advance_line();
                if reader.position() == before {
                    break;
                }
            }
            context.close_code_block(node, &reader);
            continue;
        }
        if super::list::parse_list_item(&bytes).1 != super::list::ListItemType::NotList {
            let _ = parse_list(&mut context, root, &mut reader, source, false);
            continue;
        }
        let paragraph = context.arena.insert(NodeKind::Paragraph(Paragraph {
            lines: vec![segment],
            blank_previous_lines: false,
        }));
        context.set_node_position(paragraph, segment.start);
        context.arena.append_child(root, paragraph);
        reader.advance_to_eol();
        reader.advance_line();
        let mut is_setext_bar = false;
        loop {
            let (next, next_segment) = reader.peek_line();
            let Some(next) = next else { break };
            let next_bytes = next.bytes();
            if next_bytes.iter().all(|&byte| is_space(byte)) {
                break;
            }
            let NodeKind::Paragraph(value) = &context.arena.node(paragraph).data else {
                unreachable!()
            };
            let prior = value.lines.last().copied().unwrap_or(segment);
            let current_is_setext_bar =
                super::heading::matches_setext_heading_bar(&next_bytes).is_some();
            let current_interrupts_with_list = list_interrupts_paragraph(&next_bytes);
            if current_is_setext_bar
                || atx_heading_interrupts_paragraph(&next_bytes)
                || fenced_code_interrupts_paragraph(&next_bytes)
                || first_non_space(&next_bytes).is_some_and(|position| {
                    next_bytes[position] == b'>'
                        && super::block::indent_width(&next_bytes, 0).0 <= 3
                })
                || is_thematic_break(&next_bytes, reader.line_offset())
                || current_interrupts_with_list
                || first_non_space(&next_bytes).is_some_and(|position| {
                    next_bytes[position] == b'<'
                        && super::html_block::can_interrupt_paragraph(&next_bytes)
                })
            {
                // Preserve the line for the next block parser. Setext and table
                // transforms run after the paragraph is complete below.
                is_setext_bar = current_is_setext_bar;
                let _ = prior;
                break;
            }
            if let NodeKind::Paragraph(value) = &mut context.arena.node_mut(paragraph).data {
                value.lines.push(next_segment);
            }
            reader.advance_to_eol();
            reader.advance_line();
        }
        let source_bytes = super::bytes::Bytes::new(source);
        context.transform_paragraph_references(paragraph, &source_bytes);
        if context.arena.node(paragraph).parent.is_some() {
            context.transform_paragraph_table(paragraph, &source_bytes);
        }
        if is_setext_bar
            && context.arena.node(paragraph).parent.is_some()
            && let (Some(heading), _) =
                context.open_setext_heading(root, Some(paragraph), &mut reader)
        {
            context.arena.append_child(root, heading);
            reader.advance_to_eol();
            context.close_setext_heading(heading, &reader);
            reader.advance_line();
            append_inline(&mut context, heading, source);
            continue;
        }
        if context.arena.node(paragraph).parent.is_some() {
            context.close_paragraph(paragraph, &reader);
        }
        append_inline(&mut context, paragraph, source);
        if reader.peek_line().0.is_none() {
            break;
        }
    }
    append_table_inlines(&mut context, root, source);
    context.transform_table_escaped_pipes();
    (context, root)
}
