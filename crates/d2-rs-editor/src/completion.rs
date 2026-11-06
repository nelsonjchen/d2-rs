// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! D2 completion vocabularies and v0.9.0 d2lsp source-context resolution.

use d2_rs_core::ast::{self, keywords};
use serde::Serialize;

/// Completion category used by D2's editor helper.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum CompletionKind {
    /// A language keyword or value.
    Keyword = 0,
    /// A style property.
    Style = 1,
    /// A shape or arrowhead shape.
    Shape = 2,
}

/// One deterministic completion candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompletionItem {
    /// Display label.
    pub label: String,
    /// Candidate category.
    pub kind: CompletionKind,
    /// Short explanatory detail.
    pub detail: String,
    /// Text inserted by an editor, possibly empty for prose placeholders.
    pub insert_text: String,
}

const SHAPES: &[&str] = &[
    "rectangle",
    "square",
    "page",
    "parallelogram",
    "document",
    "cylinder",
    "queue",
    "package",
    "step",
    "callout",
    "stored_data",
    "person",
    "c4-person",
    "diamond",
    "oval",
    "circle",
    "hexagon",
    "cloud",
    "text",
    "code",
    "class",
    "sql_table",
    "image",
    "sequence_diagram",
    "hierarchy",
];
/// Returns completions for the source context at a zero-based position.
///
/// Ports v0.9.0 `d2lsp/completion.go` getCompletionItems: parse the whole
/// document, fall back to the text before the cursor when that fails, resolve
/// the keyword context from the AST, then select the vocabulary. It is
/// transport-free; callers serialize the records into LSP or WASM responses.
pub fn completion_items(
    source: &str,
    line: usize,
    column: usize,
    encoding: super::PositionEncoding,
) -> Vec<CompletionItem> {
    let utf16 = encoding == super::PositionEncoding::Utf16;
    let parse = |input: &[u8]| {
        d2_rs_language::parse(
            "",
            input,
            Some(d2_rs_language::ParseOptions {
                utf16_positions: utf16,
                errors: None,
            }),
        )
    };
    let parsed = parse(source.as_bytes());
    let map = if parsed.error.is_some() {
        // Go keeps this second AST even when it also reports errors.
        parse(&text_until_position(source.as_bytes(), line, column, utf16)).map
    } else {
        parsed.map
    };
    let (line, column) = (line as i64, column as i64);
    items_for(&keyword_context(
        source.as_bytes(),
        &map,
        line,
        column,
        utf16,
    ))
}

/// completion.go getCompletionItems' keyword switch.
fn items_for(keyword: &str) -> Vec<CompletionItem> {
    match keyword {
        "style" | "style." => style_items(),
        "shape" | "shape:" => value_items(SHAPES, CompletionKind::Shape, "shape", ""),
        "shadow"
        | "3d"
        | "multiple"
        | "animated"
        | "bold"
        | "italic"
        | "underline"
        | "filled"
        | "double-border"
        | "shadow:"
        | "3d:"
        | "multiple:"
        | "animated:"
        | "bold:"
        | "italic:"
        | "underline:"
        | "filled:"
        | "double-border:"
        | "style.shadow:"
        | "style.3d:"
        | "style.multiple:"
        | "style.animated:"
        | "style.bold:"
        | "style.italic:"
        | "style.underline:"
        | "style.filled:"
        | "style.double-border:" => boolean_items(),
        "fill-pattern" | "fill-pattern:" | "style.fill-pattern:" => value_items(
            keywords::FILL_PATTERNS,
            CompletionKind::Keyword,
            "fill pattern",
            "",
        ),
        "text-transform" | "text-transform:" | "style.text-transform:" => value_items(
            keywords::TEXT_TRANSFORMS,
            CompletionKind::Keyword,
            "text transform",
            "",
        ),
        "opacity" | "stroke-width" | "stroke-dash" | "border-radius" | "font-size" | "stroke"
        | "fill" | "font-color" | "width" | "height" | "top" | "left" => value_completions(keyword),
        "opacity:"
        | "stroke-width:"
        | "stroke-dash:"
        | "border-radius:"
        | "font-size:"
        | "stroke:"
        | "fill:"
        | "font-color:"
        | "style.opacity:"
        | "style.stroke-width:"
        | "style.stroke-dash:"
        | "style.border-radius:"
        | "style.font-size:"
        | "style.stroke:"
        | "style.fill:"
        | "style.font-color:"
        | "width:"
        | "height:"
        | "top:"
        | "left:" => value_completions(
            keyword
                .strip_prefix("style.")
                .unwrap_or(keyword)
                .trim_end_matches(':'),
        ),
        "source-arrowhead" | "target-arrowhead" => arrowhead_items(),
        "source-arrowhead.shape:" | "target-arrowhead.shape:" => arrowhead_shape_items(),
        "label" | "label." | "icon." => vec![CompletionItem {
            label: "near".into(),
            kind: CompletionKind::Style,
            detail: "label position".into(),
            insert_text: "near: ".into(),
        }],
        "icon" | "icon:" => vec![CompletionItem {
            label: "(URL, e.g. https://icons.d2lang.com/xyz.svg)".into(),
            kind: CompletionKind::Keyword,
            detail: "icon URL".into(),
            insert_text: "https://icons.d2lang.com/essentials%2F073-add.svg".into(),
        }],
        "near" | "near:" => {
            // getNearCompletions: an object-ID hint, then LabelPositionsArray.
            let mut items = vec![CompletionItem {
                label: "(object ID)".into(),
                kind: CompletionKind::Keyword,
                detail: "e.g. container.inner_shape".into(),
                insert_text: String::new(),
            }];
            items.extend(value_items(
                keywords::LABEL_POSITIONS,
                CompletionKind::Keyword,
                "label position",
                "",
            ));
            items
        }
        "tooltip" | "tooltip:" => vec![CompletionItem {
            label: "(markdown)".into(),
            kind: CompletionKind::Keyword,
            detail: "markdown formatted text".into(),
            insert_text: "|md\n  # Tooltip\n  Hello world\n|".into(),
        }],
        "direction" | "direction:" => value_items(
            &["up", "down", "right", "left"],
            CompletionKind::Keyword,
            "direction",
            "",
        ),
        _ => Vec::new(),
    }
}

/// completion.go getValueCompletions.
fn value_completions(property: &str) -> Vec<CompletionItem> {
    let (label, detail) = match property {
        "opacity" => ("(number between 0.0 and 1.0)", "e.g. 0.4"),
        "stroke-width" => ("(number between 0 and 15)", "e.g. 2"),
        "font-size" => ("(number between 8 and 100)", "e.g. 14"),
        "stroke-dash" => ("(number between 0 and 10)", "e.g. 5"),
        "border-radius" => ("(number greater than or equal to 0)", "e.g. 4"),
        "font-color" | "stroke" | "fill" => ("(color name or hex code)", "e.g. blue, #ff0000"),
        "width" | "height" | "top" | "left" => ("(pixels)", "e.g. 400"),
        _ => return Vec::new(),
    };
    value_placeholder(label, detail)
}

/// completion.go getTextUntilPosition. Byte-based like Go: a UTF-8 column may
/// end inside a rune, and the parser then sees that truncated byte string.
fn text_until_position(text: &[u8], line: usize, column: usize, utf16: bool) -> Vec<u8> {
    let lines = text.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    if line >= lines.len() {
        return text.to_vec();
    }
    let mut result = lines[..line].join(&b'\n');
    if !result.is_empty() {
        result.push(b'\n');
    }
    let column = column_byte_offset(lines[line], column as i64, utf16);
    result.extend_from_slice(&lines[line][..column.min(lines[line].len())]);
    result
}

/// completion.go getKeywordContext.
fn keyword_context(text: &[u8], map: &ast::Map, line: i64, column: i64, utf16: bool) -> String {
    let lines = text.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let line_index = usize::try_from(line)
        .ok()
        .filter(|index| *index < lines.len());
    let line_column = line_index.map_or(column, |index| {
        column_byte_offset(lines[index], column, utf16) as i64
    });
    let holder = |part: &str| {
        keywords::RESERVED_KEYWORD_HOLDERS.contains(&part)
            || keywords::COMPOSITE_RESERVED_KEYWORDS.contains(&part)
    };
    let line_text = |index: i64| {
        usize::try_from(index)
            .ok()
            .and_then(|index| lines.get(index).copied())
            .unwrap_or_default()
    };
    let Some(nodes) = &map.nodes else {
        return String::new();
    };
    for node in nodes.iter() {
        let node = node.read();
        let Some(map_key) = &node.map_key else {
            continue;
        };
        let map_key = map_key.read();
        let has_edges = map_key
            .edges
            .as_ref()
            .is_some_and(|edges| !edges.is_empty());
        let key = if has_edges {
            &map_key.edge_key
        } else {
            &map_key.key
        };
        let key_parts = key.as_ref().and_then(|key| {
            let key = key.read();
            key.path.as_ref().map(|path| {
                path.iter()
                    .map(|slot| slot.read().as_ref().map(|part| part.read().clone()))
                    .collect::<Vec<_>>()
            })
        });
        let key_range = map_key.range.clone();
        let mut first_part = String::new();
        let mut last_part = String::new();
        let path_len = key_parts.as_ref().map_or(0, Vec::len);
        if let Some(parts) = key_parts.as_ref().filter(|parts| !parts.is_empty()) {
            let Some(first) = parts[0]
                .as_ref()
                .filter(|part| part.unquoted_string.is_some())
            else {
                continue;
            };
            first_part = first.scalar_string();
            if path_len > 1
                && let Some(last) = parts[path_len - 1]
                    .as_ref()
                    .filter(|part| part.unquoted_string.is_some())
            {
                last_part = last.scalar_string();
                let text = line_text(key_range.end.line);
                let dot_column = column_byte_offset(text, column, utf16);
                if holder(&last_part) && is_after_dot(text, dot_column) {
                    return format!("{last_part}.");
                }
            }
        }
        if keywords::BOARD_KEYWORDS.contains(&first_part.as_str()) || first_part == "classes" {
            first_part.clear();
        }
        let is_holder = holder(&first_part);
        if let Some(value) = &map_key.value.map {
            let value = value.read();
            if is_position_in_map(line, column, &value.range) {
                let nested = keyword_context(text, &value, line, column, utf16);
                if nested.is_empty() {
                    return first_part;
                }
                // A direct key completion inside a holder's map is prefixed
                // with the holder's name.
                if is_holder && nested.ends_with(':') && !nested.contains('.') {
                    return format!("{first_part}.{nested}");
                }
                return nested;
            }
        }
        if line != key_range.end.line
            || line < key_range.start.line
            || (line == key_range.start.line && column < key_range.start.column)
            || (line == key_range.end.line && column > key_range.end.column + 1)
        {
            continue;
        }
        let text = line_text(key_range.end.line);
        if is_after_colon(text, line_column) {
            if path_len > 1 {
                if is_holder
                    && matches!(first_part.as_str(), "source-arrowhead" | "target-arrowhead")
                {
                    return format!("{first_part}.{last_part}:");
                }
                if !keywords::RESERVED_KEYWORD_HOLDERS.contains(&last_part.as_str()) {
                    return last_part;
                }
            }
            return format!("{first_part}:");
        }
        if is_after_dot(text, line_column as usize) && is_holder {
            return first_part;
        }
    }
    String::new()
}

/// completion.go columnByteOffset: a UTF-16 column becomes a byte offset.
fn column_byte_offset(text: &[u8], column: i64, utf16: bool) -> usize {
    if column <= 0 {
        return 0;
    }
    if !utf16 {
        return column as usize;
    }
    let mut units = 0;
    for (offset, rune) in String::from_utf8_lossy(text).char_indices() {
        let rune_units = rune.len_utf16() as i64;
        if units >= column || units + rune_units > column {
            return offset;
        }
        units += rune_units;
    }
    text.len()
}

fn is_after_dot(text: &[u8], position: usize) -> bool {
    position > 0 && position <= text.len() && text[position - 1] == b'.'
}

/// completion.go isAfterColon, including Go's unicode.IsSpace(rune(byte)).
fn is_after_colon(text: &[u8], position: i64) -> bool {
    if position < 1 || position as usize > text.len() {
        return false;
    }
    let mut index = position as usize;
    while index > 0
        && matches!(
            text[index - 1],
            b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ' | 0x85 | 0xa0
        )
    {
        index -= 1;
    }
    index > 0 && text[index - 1] == b':'
}

fn is_position_in_map(line: i64, column: i64, range: &d2_rs_core::ast::Range) -> bool {
    !(line < range.start.line
        || line > range.end.line
        || (line == range.start.line && column < range.start.column)
        || (line == range.end.line && column > range.end.column))
}

fn value_items(
    values: &[&str],
    kind: CompletionKind,
    detail: &str,
    suffix: &str,
) -> Vec<CompletionItem> {
    values
        .iter()
        .map(|value| CompletionItem {
            label: (*value).into(),
            kind,
            detail: detail.into(),
            insert_text: format!("{value}{suffix}"),
        })
        .collect()
}

fn style_items() -> Vec<CompletionItem> {
    value_items(
        keywords::STYLE_KEYWORDS,
        CompletionKind::Style,
        "style property",
        ": ",
    )
}

fn arrowhead_items() -> Vec<CompletionItem> {
    value_items(
        &["shape", "label", "style.filled"],
        CompletionKind::Shape,
        "",
        "",
    )
}

fn arrowhead_shape_items() -> Vec<CompletionItem> {
    let details = [
        ("triangle", "default"),
        ("arrow", "like triangle but pointier"),
        ("diamond", "arrowhead shape"),
        ("circle", "arrowhead shape"),
        ("cf-one", "crows foot one"),
        ("cf-one-required", "crows foot one (required)"),
        ("cf-many", "crows foot many"),
        ("cf-many-required", "crows foot many (required)"),
    ];
    details
        .into_iter()
        .map(|(label, detail)| CompletionItem {
            label: label.into(),
            kind: CompletionKind::Shape,
            detail: detail.into(),
            insert_text: label.into(),
        })
        .collect()
}

fn boolean_items() -> Vec<CompletionItem> {
    value_items(&["true", "false"], CompletionKind::Keyword, "boolean", "")
}

fn value_placeholder(label: &str, detail: &str) -> Vec<CompletionItem> {
    vec![CompletionItem {
        label: label.into(),
        kind: CompletionKind::Keyword,
        detail: detail.into(),
        insert_text: String::new(),
    }]
}

#[cfg(test)]
mod tests {
    use super::{CompletionKind, completion_items, text_until_position};
    use crate::PositionEncoding;

    #[test]
    fn completes_style_and_shape_values() {
        let styles = completion_items("node.style.", 0, 11, PositionEncoding::Utf8);
        assert!(styles.iter().any(|item| item.label == "font-size"));
        assert!(styles.iter().all(|item| item.kind == CompletionKind::Style));

        let shapes = completion_items("node: {shape: }", 0, 14, PositionEncoding::Utf8);
        assert!(shapes.iter().any(|item| item.label == "rectangle"));
    }

    #[test]
    fn completion_positions_honor_utf16_columns() {
        let items = completion_items("😀.shape:", 0, 9, PositionEncoding::Utf16);
        assert!(items.iter().any(|item| item.label == "diamond"));

        assert_eq!(
            text_until_position("node: {shape: }".as_bytes(), 0, 14, true),
            b"node: {shape: "
        );
        let ascii_items = completion_items("node: {shape: }", 0, 14, PositionEncoding::Utf16);
        assert!(ascii_items.iter().any(|item| item.label == "rectangle"));
    }

    #[test]
    fn malformed_or_unknown_context_is_empty() {
        assert!(completion_items("node: [", 0, 7, PositionEncoding::Utf8).is_empty());
        assert!(completion_items("node: {unknown: }", 0, 17, PositionEncoding::Utf8).is_empty());
    }

    #[test]
    fn upstream_map_contexts_and_nested_arrowheads() {
        for (source, line, column, expected) in [
            ("a: {\n  3d:\n}\n", 1, 5, "true"),
            ("a.style: {\n  fill-pattern:\n}\n", 1, 15, "dots"),
            (
                "a -> b: { style.opacity: }",
                0,
                24,
                "(number between 0.0 and 1.0)",
            ),
            ("a -> b: { source-arrowhead.shape: }", 0, 33, "triangle"),
            (
                "a -> b: { source-arrowhead: { shape: } }",
                0,
                36,
                "triangle",
            ),
            ("a -> b: { icon.near: }", 0, 20, "top-left"),
        ] {
            let items = completion_items(source, line, column, PositionEncoding::Utf8);
            assert!(
                items.iter().any(|item| item.label == expected),
                "{source:?} at {line}:{column} did not contain {expected:?}: {items:?}"
            );
        }
        assert!(
            completion_items(
                "a -> b: { source-arrowhead: }",
                0,
                27,
                PositionEncoding::Utf8
            )
            .is_empty()
        );
    }

    #[test]
    fn arrowhead_shape_details_match_upstream() {
        let items = completion_items(
            "a -> b: { source-arrowhead.shape: }",
            0,
            33,
            PositionEncoding::Utf8,
        );
        let triangle = items.iter().find(|item| item.label == "triangle").unwrap();
        assert_eq!(triangle.detail, "default");
        let cf_many = items.iter().find(|item| item.label == "cf-many").unwrap();
        assert_eq!(cf_many.detail, "crows foot many");
    }

    #[test]
    fn shape_completion_order_and_vocab_match_v090() {
        let labels = completion_items("a.shape:", 0, 8, PositionEncoding::Utf8)
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "rectangle",
                "square",
                "page",
                "parallelogram",
                "document",
                "cylinder",
                "queue",
                "package",
                "step",
                "callout",
                "stored_data",
                "person",
                "c4-person",
                "diamond",
                "oval",
                "circle",
                "hexagon",
                "cloud",
                "text",
                "code",
                "class",
                "sql_table",
                "image",
                "sequence_diagram",
                "hierarchy",
            ]
        );
    }

    #[test]
    fn upstream_completion_context_matrix_has_the_same_boundaries() {
        let cases = [
            ("a.style.", 0, 8, "font-size"),
            ("a: {\n  style.\n}\n", 1, 8, "font-size"),
            (
                "classes: {\n  goal: {\n    shape:\n  }\n}\n",
                2,
                10,
                "rectangle",
            ),
            ("a: {\n\tstyle: {\n    3d:\n  }\n}\n", 2, 7, "true"),
            ("a.style: {\n  3d:\n}\n", 1, 5, "true"),
            ("a.style: {\n  fill-pattern:\n}\n", 1, 15, "dots"),
            (
                "a.style: {\n  opacity:\n}\n",
                1,
                10,
                "(number between 0.0 and 1.0)",
            ),
            ("a.width:", 0, 8, "(pixels)"),
            (
                "a\n\nlayers: {\n  hey: {\n    go: {\n      shape:\n    }\n  }\n}\n",
                5,
                12,
                "rectangle",
            ),
            ("a -> b: { style. }", 0, 16, "font-size"),
            (
                "a -> b: { style.opacity: }",
                0,
                24,
                "(number between 0.0 and 1.0)",
            ),
            ("a -> b: { style.fill-pattern: }", 0, 29, "dots"),
            ("a -> b: { style.text-transform: }", 0, 31, "uppercase"),
            ("a -> b: { style.shadow: }", 0, 23, "true"),
            ("a -> b: { label.near: }", 0, 21, "top-left"),
            ("a -> b: { direction: }", 0, 20, "up"),
            (
                "a -> b: { icon: }",
                0,
                15,
                "(URL, e.g. https://icons.d2lang.com/xyz.svg)",
            ),
            (
                "a.icon:",
                0,
                7,
                "(URL, e.g. https://icons.d2lang.com/xyz.svg)",
            ),
            ("a -> b: { icon.near: }", 0, 20, "top-left"),
            ("a.icon.", 0, 7, "near"),
            ("a.label.", 0, 8, "near"),
            ("a -> b: { source-arrowhead. }", 0, 27, "shape"),
            ("(a -> b)[0].source-arrowhead.shape:", 0, 35, "triangle"),
            ("a -> b: { source-arrowhead.shape: }", 0, 33, "triangle"),
            (
                "a -> b: { source-arrowhead: { shape: } }",
                0,
                36,
                "triangle",
            ),
            ("a -> b: { width: }", 0, 16, "(pixels)"),
            ("a -> b: { height: }", 0, 17, "(pixels)"),
            ("a -> b: { tooltip: }", 0, 18, "(markdown)"),
            ("a.tooltip:", 0, 10, "(markdown)"),
            ("a.shape:", 0, 8, "rectangle"),
            ("a -> b: { shape: }", 0, 16, "rectangle"),
            ("a: {\n  shape:\n}\n", 1, 8, "rectangle"),
        ];
        for (source, line, column, expected) in cases {
            let items = completion_items(source, line, column, PositionEncoding::Utf8);
            assert!(
                items.iter().any(|item| item.label == expected),
                "{source:?} at {line}:{column} did not contain {expected:?}: {items:?}"
            );
        }

        for (source, line, column) in [
            ("a.style.stroke-width: 1", 0, 23),
            ("a.style:\n", 0, 8),
            ("a -> b: { source-arrowhead: }", 0, 27),
            ("a.icon: {\n  # here\n}", 1, 2),
        ] {
            assert!(
                completion_items(source, line, column, PositionEncoding::Utf8).is_empty(),
                "{source:?} at {line}:{column} unexpectedly returned completions"
            );
        }
    }
}
