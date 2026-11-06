// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source-reference ranges corresponding to D2's `GetRefRanges` helper.

use d2_rs_core::{ast, ir};
use d2_rs_language::{ParseOptions, compiler, parse, parse_map_key};
use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Ranges associated with one resolved field or edge reference query.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ReferenceRanges {
    /// Source ranges of the selected field or edge references; `None` preserves an upstream nil slice.
    pub references: Option<Vec<ast::Range>>,
    /// Import source ranges; `None` preserves an upstream nil slice.
    pub imports: Option<Vec<ast::Range>>,
}

/// Failure while parsing or compiling a reference query.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ReferenceError {
    /// The requested key is not a valid D2 key path.
    #[error("invalid reference key {key:?}: {message}")]
    InvalidKey { key: String, message: String },
    /// Source parsing failed.
    #[error("could not parse source: {0}")]
    Parse(String),
    /// The requested source path is absent from a virtual filesystem.
    #[error("source file {path:?} was not found")]
    SourceNotFound { path: String },
    /// A requested layer, scenario, or step board is absent.
    #[error("board {path:?} was not found")]
    BoardNotFound { path: Vec<String> },
    /// IR compilation failed before references could be selected.
    #[error("could not compile source: {0}")]
    Compile(String),
}

/// Returns source and import ranges for a resolved field or edge key.
///
/// The lookup is source-order preserving and returns absent ranges as `None`,
/// matching D2's nil slices when the key is syntactically valid but absent.
/// This mirrors D2's `GetRefRanges`
/// split between field-path traversal and edge-ID lookup. Filesystem import
/// providers remain a separate boundary; callers can still use this helper for
/// fully in-memory sources.
pub fn reference_ranges(source: &str, key: &str) -> Result<ReferenceRanges, ReferenceError> {
    let map_key = parse_map_key(key.as_bytes()).map_err(|error| ReferenceError::InvalidKey {
        key: key.to_owned(),
        message: error.to_string(),
    })?;
    if map_key.key.is_none() && map_key.edges.as_ref().is_none_or(|edges| edges.is_empty()) {
        return Err(ReferenceError::InvalidKey {
            key: key.to_owned(),
            message: "empty key path and no edges".into(),
        });
    }
    let parsed = parse(
        "-",
        source.as_bytes(),
        Some(ParseOptions {
            utf16_positions: false,
            errors: None,
        }),
    );
    if let Some(error) = parsed.error {
        return Err(ReferenceError::Parse(error.to_string()));
    }
    let (compiler, root) = compiler::compile_ir_with_options(
        ast::Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: None,
            utf16_positions: false,
        },
    )
    .map_err(|error| ReferenceError::Compile(error.to_string()))?;
    Ok(collect_reference_ranges(&compiler.arena, root, &map_key))
}

/// Returns reference ranges from a virtual filesystem and selected board.
///
/// This is the in-memory counterpart of D2's `d2lsp.GetRefRanges`: imports are
/// opened through the compiler's explicit provider, and `board_path` selects
/// nested `layers`, `scenarios`, or `steps` maps before resolving the key.
pub fn reference_ranges_with_files(
    path: &str,
    files: &HashMap<String, String>,
    board_path: &[String],
    key: &str,
) -> Result<ReferenceRanges, ReferenceError> {
    let source = files
        .get(path)
        .ok_or_else(|| ReferenceError::SourceNotFound {
            path: path.to_owned(),
        })?;
    let map_key = parse_map_key(key.as_bytes()).map_err(|error| ReferenceError::InvalidKey {
        key: key.to_owned(),
        message: error.to_string(),
    })?;
    if map_key.key.is_none() && map_key.edges.as_ref().is_none_or(|edges| edges.is_empty()) {
        return Err(ReferenceError::InvalidKey {
            key: key.to_owned(),
            message: "empty key path and no edges".into(),
        });
    }
    let parsed = parse(
        path,
        source.as_bytes(),
        Some(ParseOptions {
            utf16_positions: false,
            errors: None,
        }),
    );
    if let Some(error) = parsed.error {
        return Err(ReferenceError::Parse(error.to_string()));
    }
    let files = files.clone();
    let opener: compiler::ImportOpener = Box::new(move |requested| {
        files
            .get(requested)
            .cloned()
            .map(|bytes| {
                Box::new(std::io::Cursor::new(bytes)) as Box<dyn d2_rs_language::stream::ByteReader>
            })
            .ok_or_else(|| format!("source file {requested:?} was not found"))
    });
    let (compiler, root) = compiler::compile_ir_with_options(
        ast::Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(opener),
            utf16_positions: false,
        },
    )
    .map_err(|error| ReferenceError::Compile(error.to_string()))?;
    let map = find_board_map(&compiler.arena, root, board_path).ok_or_else(|| {
        ReferenceError::BoardNotFound {
            path: board_path.to_vec(),
        }
    })?;
    Ok(collect_reference_ranges(&compiler.arena, map, &map_key))
}

fn find_board_map(
    arena: &ir::Arena,
    mut map: ir::NodeId,
    board_path: &[String],
) -> Option<ir::NodeId> {
    for name in board_path {
        let mut found = None;
        for section in ["layers", "scenarios", "steps"] {
            let Some(section_map) = arena
                .get_field(map, &[ast::raw_string_box(section, true)])
                .and_then(|field| arena.map(field))
            else {
                continue;
            };
            if let Some(field) =
                arena.get_field(section_map, &[ast::raw_string_box(name.as_str(), true)])
            {
                found = arena.map(field);
                if found.is_some() {
                    break;
                }
            }
        }
        map = found?;
    }
    Some(map)
}

fn collect_reference_ranges(
    arena: &ir::Arena,
    mut map: ir::NodeId,
    map_key: &ast::Key,
) -> ReferenceRanges {
    let mut output = ReferenceRanges::default();
    if let Some(path) = &map_key.key {
        let names = path
            .read()
            .path
            .iter()
            .flat_map(ast::SharedSlice::iter)
            .filter_map(|slot| slot.read().clone().map(|name| name.read().clone()))
            .collect::<Vec<_>>();
        let Some(field_id) = arena.get_field(map, &names) else {
            return output;
        };
        map = arena.map(field_id).unwrap_or(field_id);
        if map_key.edges.as_ref().is_none_or(|edges| edges.is_empty()) {
            let ir::Node::Field(field) = arena.node(field_id) else {
                return output;
            };
            for reference in field.references.iter().flatten() {
                if let Some(node) = reference.read().ast() {
                    use ast::Node;
                    output
                        .references
                        .get_or_insert_with(Vec::new)
                        .push(node.range());
                }
            }
            if let Some(import) = &field.import_ast {
                use ast::Node;
                output
                    .imports
                    .get_or_insert_with(Vec::new)
                    .push(import.range());
            }
            return output;
        }
    }

    for edge_id in ir::new_edge_ids(map_key).into_iter().flatten() {
        let Some(edges) = arena.get_edges(map, &edge_id) else {
            continue;
        };
        for edge_id in edges {
            let ir::Node::Edge(edge) = arena.node(edge_id) else {
                continue;
            };
            for reference in edge.references.iter().flatten() {
                if let Some(node) = reference.read().ast() {
                    use ast::Node;
                    output
                        .references
                        .get_or_insert_with(Vec::new)
                        .push(node.range());
                }
            }
            if let Some(import) = &edge.import_ast {
                use ast::Node;
                output
                    .imports
                    .get_or_insert_with(Vec::new)
                    .push(import.range());
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{ReferenceError, reference_ranges, reference_ranges_with_files};
    use std::collections::HashMap;

    #[test]
    fn selects_field_reference_ranges_in_source_order() {
        let result = reference_ranges("x\nx.a\na.x\nx -> y\n", "x").unwrap();
        assert_eq!(
            result
                .references
                .as_ref()
                .unwrap()
                .iter()
                .map(|range| range.start.line)
                .collect::<Vec<_>>(),
            [0, 1, 3]
        );
        assert!(result.imports.is_none());
        assert!(
            result.references.as_ref().unwrap().windows(2).all(|pair| {
                pair[0].start.before(pair[1].start) || pair[0].start == pair[1].start
            })
        );
    }

    #[test]
    fn absent_valid_key_returns_nil_ranges() {
        assert_eq!(
            reference_ranges("a: hello\n", "missing").unwrap(),
            Default::default()
        );
    }

    #[test]
    fn malformed_key_is_reported() {
        assert!(matches!(
            reference_ranges("a: hello\n", ""),
            Err(ReferenceError::InvalidKey { .. })
        ));
    }

    #[test]
    fn selects_edge_reference_ranges() {
        let source = "x\nx.a\na.x\nx -> y\ny -> z\nx -> z\nb: {\n  x -> y\n}\n";
        for (key, line) in [("x -> y", 3), ("y -> z", 4), ("x -> z", 5)] {
            let result = reference_ranges(source, key).unwrap();
            let references = result.references.as_ref().unwrap();
            assert_eq!(references.len(), 1, "{key}");
            assert_eq!(references[0].start.line, line, "{key}");
        }
        let nested = reference_ranges(source, "b.(x -> y)").unwrap();
        let references = nested.references.unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].start.line, 7);
    }

    #[test]
    fn selects_nested_edge_reference_ranges() {
        let result = reference_ranges("b: {\n  x -> y\n}\n", "b.(x -> y)").unwrap();
        let references = result.references.unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].start.line, 1);
    }

    #[test]
    fn resolves_imported_reference_ranges_from_a_virtual_filesystem() {
        let files = HashMap::from([
            ("index.d2".into(), "\n...@ok\nhi\nhey: @ok\n".into()),
            ("ok.d2".into(), "\nwhat\nlala\nokay\n".into()),
        ]);
        let result = reference_ranges_with_files("index.d2", &files, &[], "okay").unwrap();
        let references = result.references.as_ref().unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].path, "ok.d2");
        assert_eq!(result.imports.as_ref().unwrap().len(), 1);
        assert_eq!(references[0].start.line, 3);

        let result = reference_ranges_with_files("index.d2", &files, &[], "hey.okay").unwrap();
        let references = result.references.as_ref().unwrap();
        let imports = result.imports.as_ref().unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].path, "ok.d2");
        assert_eq!(references[0].start.line, 3);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].start.line, 3);
        assert_eq!(imports[0].start.column, 5);
    }

    #[test]
    fn resolves_nested_layer_reference_ranges() {
        let files = HashMap::from([(
            "index.d2".into(),
            "layers: {\n  x: {\n    hello\n  }\n}\n".into(),
        )]);
        let result =
            reference_ranges_with_files("index.d2", &files, &["x".into()], "hello").unwrap();
        let references = result.references.unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].start.line, 2);
    }

    #[test]
    fn imported_board_queries_match_v090_errors_and_ranges() {
        let files = HashMap::from([(
            "index.d2".into(),
            "\nhi\nlayers: {\n  x: {\n    hello\n\n    layers: {\n      y: {\n        qwer\n      }\n    }\n  }\n}\n"
                .into(),
        )]);
        let result =
            reference_ranges_with_files("index.d2", &files, &["x".into(), "y".into()], "qwer")
                .unwrap();
        let references = result.references.unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].start.line, 8);

        let error =
            reference_ranges_with_files("index.d2", &files, &["y".into()], "hello").unwrap_err();
        assert!(matches!(error, ReferenceError::BoardNotFound { path } if path == ["y"]));
    }
}
