// SPDX-License-Identifier: MPL-2.0
//! Native text dependencies. Markdown parsing and font measurement are being
//! connected to the full D2 target pipeline.
//!
//! [`measure`] contains the dependency-free TrueType/OpenType advance and line
//! metric reader used by the native migration. [`markdown::html`] provides a
//! deterministic HTML projection for the translated Markdown tree; shaping,
//! fallback families, styled dimensions, and ink bounds remain separate work.
pub mod code;
pub mod d2fonts;
pub mod font_subset;
mod go_quote;
mod go_xml_names;
pub mod markdown;
pub mod measure;
pub mod subset_font;
#[doc(hidden)]
pub mod uniseg;
pub mod woff;
pub mod xml;

pub mod markdown_measure;
