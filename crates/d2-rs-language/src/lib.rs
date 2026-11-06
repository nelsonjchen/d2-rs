// SPDX-License-Identifier: MPL-2.0
//! The D2 language: a Rust port of upstream D2's `d2parser`, `d2ast`, `d2ir`,
//! `d2compiler` and `d2format` packages, covering parsing, compilation and
//! formatting.
//!
//! The `compiler-trace` feature enables compiler diagnostics; `oracle-fixtures`
//! compiles tests that read regenerated Go-oracle artifacts.
mod parse;
pub mod parser;
pub use parse::{
    ParseContextError, ParseContextOutput, ParseHelperError, ParseOptions, ParseOutput, parse,
    parse_key, parse_map_key, parse_value, parse_with_context,
};

pub mod format;

pub mod ir;

pub mod compiler;

pub mod stream;
pub use stream::{parse_stream, parse_stream_with_context};
