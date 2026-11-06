// SPDX-License-Identifier: MPL-2.0
//! Library source admission, before measurement, layout and rendering.
//!
//! Maps d2lib.Parse and the parser/IR portion of d2lib.compileInput. Imports
//! require a caller-supplied opener; this API never falls back to host files.

use d2_rs_core::{CancellationToken, LayoutError, ast, ir::NodeId};
use d2_rs_language::{ParseContextError, ParseContextOutput, ParseOptions, compiler};

/// Source options forwarded independently of appearance configuration.
#[derive(Default)]
pub struct SourceOptions {
    /// Source path used in diagnostics and relative imports.
    pub input_path: String,
    /// Count source positions in UTF-16 code units.
    pub utf16_positions: bool,
    /// Compilation-wide expansion budgets; zero selects secure defaults.
    pub limits: compiler::CompileLimits,
    /// Explicit import capability. Absence disables imports.
    pub import_opener: Option<compiler::ImportOpener>,
}

/// Source admission failure, retaining cancellation as a typed cause.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// Parser or compiler context cause.
    #[error("{0}")]
    Context(#[from] LayoutError),
    /// Ordered parser or IR diagnostics.
    #[error(transparent)]
    Diagnostics(#[from] d2_rs_language::parser::ParseError),
    /// A recovered upstream parser runtime fault.
    #[error(transparent)]
    Parser(#[from] d2_rs_language::parser::ParserFault),
    /// An explicit untranslated native feature.
    #[error(transparent)]
    Unsupported(#[from] compiler::UntranslatedFeature),
    /// A recovered upstream compiler runtime fault.
    #[error(transparent)]
    Compiler(compiler::CompileError),
}

impl From<compiler::CompileError> for SourceError {
    fn from(error: compiler::CompileError) -> Self {
        match error {
            compiler::CompileError::Untranslated(error) => Self::Unsupported(error),
            error => Self::Compiler(error),
        }
    }
}

/// Parse library input, retaining partial syntax and the original context cause.
pub fn parse_source(
    input: &[u8],
    options: &SourceOptions,
    cancellation: Option<&CancellationToken>,
) -> ParseContextOutput {
    let context = cancellation.map(|token| {
        let token = token.clone();
        Box::new(move || token.error()) as d2_rs_language::parser::ParseContext
    });
    d2_rs_language::parse_with_context(
        &options.input_path,
        input,
        Some(ParseOptions {
            utf16_positions: options.utf16_positions,
            ..Default::default()
        }),
        context,
    )
}

/// Parse and compile library input to IR with the same source options and
/// context in both stages. Graph lowering, layout and rendering follow this
/// boundary separately. No partial IR is returned on failure.
pub fn compile_source_ir(
    input: &[u8],
    options: SourceOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<(compiler::Compiler, NodeId), SourceError> {
    let parsed = parse_source(input, &options, cancellation);
    if let Some(error) = parsed.error {
        return Err(match error {
            ParseContextError::Context(error) => SourceError::Context(error),
            ParseContextError::Parse(error) => match error.fault {
                Some(fault) => SourceError::Parser(fault),
                None => SourceError::Diagnostics(error),
            },
        });
    }
    let context = cancellation.map(|token| {
        let token = token.clone();
        Box::new(move || token.error()) as compiler::CompileContext
    });
    let (mut compiler, root) = compiler::compile_ir_with_context(
        ast::Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: options.import_opener,
            utf16_positions: options.utf16_positions,
        },
        options.limits,
        context,
    )?;
    if let Some(error) = compiler.context_error.take() {
        return Err(SourceError::Context(error));
    }
    if !compiler.errors.is_empty() {
        return Err(SourceError::Diagnostics(
            d2_rs_language::parser::ParseError {
                errors: Some(compiler.errors),
                ..Default::default()
            },
        ));
    }
    Ok((compiler, root))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nil_board_base_is_a_typed_error() {
        let result =
            compile_source_ir(b"steps: { a: 1; b: { x } }", SourceOptions::default(), None);
        assert!(matches!(
            result,
            Err(SourceError::Compiler(compiler::CompileError::NilBoardBase))
        ));
        let result = compile_source_ir(b"a: x*${x}a*", SourceOptions::default(), None);
        assert!(matches!(
            result,
            Err(SourceError::Parser(
                d2_rs_language::parser::ParserFault::SliceBounds { start: 2, end: 1 }
            ))
        ));
        assert!(compile_source_ir(b"a -> b", SourceOptions::default(), None).is_ok());
    }
}
