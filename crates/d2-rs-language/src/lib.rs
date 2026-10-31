// SPDX-License-Identifier: MPL-2.0
//! Native language parsing; compilation and editing are still being translated.
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
