// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Programmatic D2 source editing: a Rust port of upstream D2's `d2oracle`
//! (create, set, delete, move, rename, reconnect and import updates) and
//! `d2lsp` (references, board lookup and completions) packages.
//!
//! It returns source edits and completion data without an LSP transport, so
//! native and WASM callers share it. The `oracle-fixtures` feature compiles
//! tests that read regenerated Go-oracle artifacts.

mod boards;
mod completion;
mod edit;
mod graph_edit;
mod imports;
mod references;

pub use boards::{BoardError, board_at_position};
pub use completion::{CompletionItem, CompletionKind, completion_items};
pub use edit::{EditError, PositionEncoding, SourceEdit, SourcePosition, apply_edits};
pub use graph_edit::{
    CreateObjectResult, CreateRootResult, DeleteObjectResult, DeleteRootResult, GraphEditError,
    MoveObjectResult, MoveOptions, ReconnectObjectResult, ReconnectResult, RenameObjectResult,
    RenameRootResult, SetObjectResult, SetRootResult, create_object, create_object_with_options,
    create_root_field, delete_object, delete_object_with_options, delete_root_field, move_object,
    move_object_with_options, reconnect_edge, reconnect_edge_with_options, rename_object,
    rename_object_with_options, rename_root_field, set_object, set_object_with_options,
    set_root_field,
};
pub use imports::{UpdateImportError, update_import};
pub use references::{
    ReferenceError, ReferenceRanges, reference_ranges, reference_ranges_with_files,
};
