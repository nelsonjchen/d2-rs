// SPDX-License-Identifier: MPL-2.0
//! Shared native Markdown ruler; production implementation lives in text.
#[cfg(all(test, feature = "oracle-fixtures"))]
use d2_rs_text::markdown::html::render_html;
pub use d2_rs_text::markdown_measure::{MarkdownMeasurer, legacy_inline};
#[cfg(test)]
#[path = "markdown_measure/legacy_inline.rs"]
mod legacy_inline_tests;
