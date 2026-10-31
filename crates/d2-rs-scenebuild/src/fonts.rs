// SPDX-License-Identifier: MPL-2.0
//! d2fonts.FontFaces.Lookup over the bundled faces (d2-rs-text d2fonts),
//! keyed by Go's FontFamily and FontStyle strings. Bundled families
//! and explicit process registrations are read through owned snapshots.

use d2_rs_text::d2fonts::{self, FontStyle};

pub(crate) fn style(name: &str) -> Option<FontStyle> {
    match name {
        "regular" => Some(FontStyle::Regular),
        "bold" => Some(FontStyle::Bold),
        "semibold" => Some(FontStyle::Semibold),
        "italic" => Some(FontStyle::Italic),
        _ => None,
    }
}

/// FontFaces.Lookup(Font{Family, Style}).
pub(crate) fn lookup(family_name: &str, style_name: &str) -> Option<std::sync::Arc<[u8]>> {
    d2fonts::registered_font_face(family_name, style(style_name)?)
}
