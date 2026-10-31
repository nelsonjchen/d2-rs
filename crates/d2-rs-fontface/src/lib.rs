// SPDX-License-Identifier: MPL-2.0
//! Native port of D2's internal fontface package (parsing, shaping, glyph
//! outlines, COLRv1): d2renderers/internal/fontface at D2 master 6a7b48378.
//! See SOURCE.md for the Go-to-Rust mapping.

// Index loops and repeated `false` arms mirror the Go control flow.
// wasm builds omit the private bundled-emoji face (noto_color_emoji_wasm.go),
// leaving the COLRv1 table reader unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
#![allow(
    clippy::needless_range_loop,
    clippy::if_same_then_else,
    clippy::large_enum_variant
)]

mod bidi;
mod bundled_faces;
mod colr;
mod colrv1;
mod error;
mod face;
pub mod fixed;
mod noto_color_emoji;
mod segmenter;
pub mod sfnt;
mod shaping;
mod shaping_tables;
mod unicode;

pub use bundled_faces::{
    register_bundled_face, registered_bundled_face, registered_bundled_face_backing_digest,
    registered_bundled_face_digest,
};
pub use colrv1::{
    COLRv1Affine, COLRv1ClipBox, COLRv1ColorLine, COLRv1ColorStop, COLRv1Composite,
    COLRv1CompositeMode, COLRv1Glyph, COLRv1Layers, COLRv1LinearGradient, COLRv1Paint, COLRv1Plan,
    COLRv1PlanUsage, COLRv1RadialGradient, COLRv1Solid, COLRv1Transform,
};
pub use error::Error;
pub use face::{
    BundledFaceSource, BundledNotoColorEmojiSource, Collection, ColorGlyphLayer, Digest32, Nrgba,
    ParsedFace, ShapingFace, ShapingFont, parse_face, parse_face_collection_with_limit,
};
pub use noto_color_emoji::{
    bundled_noto_color_emoji_covers_rune, register_owned_bundled_noto_color_emoji,
    registered_bundled_noto_color_emoji, registered_bundled_noto_color_emoji_backing_digest,
};
pub use shaping::{Context, ShapeFace, ShapeLimits, ShapedGlyph, ShapedText, ShapingWorkspace};
pub use unicode::is_default_ignorable_rune;
