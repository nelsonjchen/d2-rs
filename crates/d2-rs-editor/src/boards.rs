// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Board-at-position queries corresponding to D2's editor helper.

use crate::{PositionEncoding, SourcePosition};
use d2_rs_core::ast::{KeyPath, Map, MapNodeBox, Position, SharedSlice};
use d2_rs_language::{ParseOptions, parse};
use thiserror::Error;

/// A parse diagnostic returned together with the best partial board path.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("could not parse D2 source while finding board: {message}")]
pub struct BoardError {
    /// Board path recovered from the partial AST, or `None` when no board was found.
    pub path: Option<Vec<String>>,
    /// Parser diagnostic text.
    pub message: String,
}

fn ast_position(position: SourcePosition) -> Position {
    Position {
        line: position.line as i64,
        column: position.column as i64,
        byte: -1,
    }
}

fn in_range(map: &Map, position: Position) -> bool {
    !position.before(map.range.start) && position.before(map.range.end)
}

fn first_key(key: &KeyPath) -> Option<String> {
    key.path
        .as_ref()?
        .get(0)?
        .read()
        .as_ref()?
        .read()
        .scalar_string()
        .into()
}

fn node_key(node: &MapNodeBox) -> Option<String> {
    first_key(&node.map_key.as_ref()?.read().key.as_ref()?.read())
}

fn child_map(node: &MapNodeBox) -> Option<Map> {
    node.map_key
        .as_ref()?
        .read()
        .value
        .map
        .as_ref()
        .map(|map| map.read().clone())
}

fn walk(map: &Map, mut current: Vec<String>, position: Position) -> Option<Vec<String>> {
    if !in_range(map, position) {
        return None;
    }
    for node in map.nodes.iter().flat_map(SharedSlice::iter) {
        let node = node.read();
        let Some(name) = node_key(&node) else {
            continue;
        };
        if current.len().is_multiple_of(2)
            && !matches!(name.as_str(), "layers" | "scenarios" | "steps")
        {
            continue;
        }
        let Some(child) = child_map(&node) else {
            continue;
        };
        if !in_range(&child, position) {
            continue;
        }
        current.push(name);
        if let Some(deeper) = walk(&child, current.clone(), position) {
            return Some(deeper);
        }
        // An odd-length path names the board collection (`layers`, `steps`,
        // ...), so a cursor between that collection and its board has no
        // active board. An even-length path is itself the active board.
        return current.len().is_multiple_of(2).then_some(current);
    }
    None
}

/// Returns the board path containing a source position.
///
/// A path has the form `layers, name, scenarios, name, ...`. Positions in a
/// collection between two boards return no path. Malformed source is
/// still searched through its partial AST; the returned [`BoardError`] carries
/// the recovered path and parser text.
pub fn board_at_position(
    source: &str,
    position: SourcePosition,
    encoding: PositionEncoding,
) -> Result<Option<Vec<String>>, BoardError> {
    let parsed = parse(
        "-",
        source.as_bytes(),
        Some(ParseOptions {
            utf16_positions: matches!(encoding, PositionEncoding::Utf16),
            errors: None,
        }),
    );
    let path = walk(&parsed.map, Vec::new(), ast_position(position));
    if let Some(error) = parsed.error {
        return Err(BoardError {
            path,
            message: error.to_string(),
        });
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{PositionEncoding, SourcePosition, board_at_position};

    #[test]
    fn finds_nested_layer_board() {
        let source = "layers: {\n  one: {\n    service: api\n  }\n}\n";
        let path = board_at_position(
            source,
            SourcePosition {
                line: 2,
                column: 12,
            },
            PositionEncoding::Utf8,
        )
        .unwrap();
        assert_eq!(path, Some(vec!["layers".to_owned(), "one".to_owned()]));
    }

    #[test]
    fn returns_nil_between_board_collection_and_board() {
        let source = "layers: {\n  one: { service: api }\n}\n";
        let path = board_at_position(
            source,
            SourcePosition { line: 1, column: 1 },
            PositionEncoding::Utf8,
        )
        .unwrap();
        assert_eq!(path, None);
    }

    #[test]
    fn malformed_source_returns_partial_path_and_diagnostic() {
        let source = "layers: {\n  one: {\n    service: api\n";
        let error = board_at_position(
            source,
            SourcePosition {
                line: 2,
                column: 10,
            },
            PositionEncoding::Utf8,
        )
        .unwrap_err();
        assert_eq!(
            error.path,
            Some(vec!["layers".to_owned(), "one".to_owned()])
        );
        assert!(!error.message.is_empty());
    }

    #[test]
    fn upstream_board_position_matrix_matches_v090() {
        let cases = [
            (
                "x\nlayers: {\n  basic: {\n    x -> y\n  }\n}",
                SourcePosition { line: 3, column: 4 },
                Some(vec!["layers", "basic"]),
                false,
            ),
            (
                "\nlayers: {\n  outer: {\n    layers: {\n      inner: {\n        x -> y\n      }\n    }\n  }\n}",
                SourcePosition { line: 5, column: 4 },
                Some(vec!["layers", "outer", "layers", "inner"]),
                false,
            ),
            (
                "\nlayers: {\n\touter: {\n\t\tlayers: {\n\t\t\tfirst: {\n\t\t\t\ta -> b\n\t\t\t}\n\t\t\tsecond: {\n\t\t\t\tx -> y\n\t\t\t}\n\t\t}\n\t}\n}",
                SourcePosition { line: 8, column: 4 },
                Some(vec!["layers", "outer", "layers", "second"]),
                false,
            ),
            (
                "\nwumbo: {\n  car\n}",
                SourcePosition { line: 2, column: 4 },
                None,
                false,
            ),
            (
                "\nlayers: {\n\tx: {\n    wumbo: {\n      car\n    }\n  }\n}",
                SourcePosition { line: 4, column: 4 },
                Some(vec!["layers", "x"]),
                false,
            ),
            (
                "\n\tscenarios: {\n\t\thappy: {\n\t\t\tx -> y\n\t\t}\n\t}",
                SourcePosition { line: 3, column: 4 },
                Some(vec!["scenarios", "happy"]),
                false,
            ),
            (
                "\n\tsteps: {\n\t\tfirst: {\n\t\t\tx -> y\n\t\t}\n\t}",
                SourcePosition { line: 3, column: 4 },
                Some(vec!["steps", "first"]),
                false,
            ),
            (
                "\n\tsteps: {\n\t\tfirst: {\n\t\t\t(x -> y)[]\n\t\t}\n\t}",
                SourcePosition { line: 3, column: 4 },
                Some(vec!["steps", "first"]),
                true,
            ),
            (
                "\n\tx -> y\n\tlayers: {\n\t\tbasic: {\n\t\t\ta -> b\n\t\t}\n\t}",
                SourcePosition { line: 1, column: 1 },
                None,
                false,
            ),
            (
                "\n\tlayers: {\n\t\tbasic: {\n\t\t}\n\t}",
                SourcePosition { line: 3, column: 2 },
                Some(vec!["layers", "basic"]),
                false,
            ),
            (
                "\n\tlayers: {\n\t\tbasic: {\n\t\t}\n\t}",
                SourcePosition { line: 2, column: 2 },
                None,
                false,
            ),
        ];
        for (source, position, expected, has_error) in cases {
            let result = board_at_position(source, position, PositionEncoding::Utf8);
            if has_error {
                let error = result.expect_err(source);
                assert_eq!(
                    error.path,
                    expected.map(|path| path.into_iter().map(str::to_owned).collect()),
                    "{source:?}"
                );
            } else {
                assert_eq!(
                    result.expect(source),
                    expected.map(|path| path.into_iter().map(str::to_owned).collect()),
                    "{source:?}"
                );
            }
        }
    }
}
