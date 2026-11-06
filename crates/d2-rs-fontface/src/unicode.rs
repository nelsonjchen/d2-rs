// SPDX-License-Identifier: MPL-2.0
//! Source d2renderers/internal/fontface/unicode.go.

/// `IsDefaultIgnorableRune`: whether the pinned HarfBuzz-compatible shaper
/// treats `value` as default-ignorable (HarfBuzz's exact range list, not
/// Unicode's broader Default_Ignorable_Code_Point property).
pub fn is_default_ignorable_rune(value: char) -> bool {
    matches!(u32::from(value),
        0x00ad
        | 0x034f
        | 0x061c
        | 0x17b4..=0x17b5
        | 0x180b..=0x180e
        | 0x200b..=0x200f
        | 0x202a..=0x202e
        | 0x2060..=0x206f
        | 0xfe00..=0xfe0f
        | 0xfeff
        | 0xfff0..=0xfff8
        | 0x1d173..=0x1d17a
        | 0xe0000..=0xe0fff)
}
