// SPDX-License-Identifier: MPL-2.0
//! Text for d2-rs: font measurement (`lib/textmeasure`), D2's bundled fonts
//! (`d2renderers/d2fonts`), font subsetting and WOFF encoding, Markdown
//! (`goldmark`) and code highlighting (`chroma`), ported from the Go packages
//! upstream D2 uses.
//!
//! The `oracle-fixtures` feature compiles tests that read regenerated Go-oracle
//! artifacts.
pub mod assets;
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
