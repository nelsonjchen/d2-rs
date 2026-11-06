// SPDX-License-Identifier: MPL-2.0
//! Small development oracle for the d2ast.Import path helpers.
//!
//! This executable deliberately mirrors `tools/oracle/main.go import-paths`.
//! It is a boundary harness, not a production API or a replacement for the
//! compiler's filesystem import resolution.

use d2_rs_core::ast::{Import, KeyPath, Node};
use serde::Serialize;

#[derive(Serialize)]
struct Row {
    record: Import,
    path: String,
    dir: String,
    ids: usize,
    children: usize,
}

fn main() -> Result<(), serde_json::Error> {
    let pairs = [
        ("a", "/b"),
        ("../a", "../../b"),
        ("/a", "../../b"),
        ("", ""),
        ("", "a\\b"),
        ("a//", "./b/../c"),
    ];
    let rows = pairs
        .into_iter()
        .map(|(pre, file)| {
            let import = Import {
                pre: pre.to_owned(),
                path: KeyPath::from_strings([file, "identifier"]).path,
                ..Import::default()
            };
            Row {
                path: import.path_with_pre(),
                dir: import.dir(),
                ids: import.ida().expect("constructed import path").len(),
                children: import.children().len(),
                record: import,
            }
        })
        .collect::<Vec<_>>();
    serde_json::to_writer(std::io::stdout(), &rows)
}
