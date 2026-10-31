// SPDX-License-Identifier: MPL-2.0
//! Bundled faces and embedded-font encodings of D2 v0.9.0
//! `d2renderers/d2fonts` (`d2fonts_common.go`, `d2fonts_embed.go`).
//!
//! The TTF assets are byte-identical to `d2renderers/d2fonts/ttf/*` (their
//! SHA-256 digests are `bundled_faces.go`'s `bundledFaceSpecs`, checked by
//! this crate's tests).

use base64::Engine as _;

use crate::subset_font::utf8_cut_font;
use crate::woff::sfnt2woff;

/// `FontFamily` (d2fonts_common.go) for the bundled families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontFamily {
    SourceSansPro,
    SourceCodePro,
    HandDrawn,
}

impl FontFamily {
    /// The Go `FontFamily` string value.
    pub fn as_str(self) -> &'static str {
        match self {
            FontFamily::SourceSansPro => "SourceSansPro",
            FontFamily::SourceCodePro => "SourceCodePro",
            FontFamily::HandDrawn => "HandDrawn",
        }
    }

    /// `FontFamily.Font` without the size, which does not select a face.
    pub fn font(self, style: FontStyle) -> Font {
        Font {
            family: self,
            style,
        }
    }
}

/// `FontStyle` (d2fonts_common.go).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontStyle {
    Regular,
    Bold,
    Semibold,
    Italic,
}

impl FontStyle {
    /// The Go `FontStyle` string value.
    pub fn as_str(self) -> &'static str {
        match self {
            FontStyle::Regular => "regular",
            FontStyle::Bold => "bold",
            FontStyle::Semibold => "semibold",
            FontStyle::Italic => "italic",
        }
    }
}

/// `FontFamilies` (d2fonts_common.go).
pub const FONT_FAMILIES: [FontFamily; 3] = [
    FontFamily::SourceSansPro,
    FontFamily::SourceCodePro,
    FontFamily::HandDrawn,
];

/// `FontStyles` (d2fonts_common.go).
pub const FONT_STYLES: [FontStyle; 4] = [
    FontStyle::Regular,
    FontStyle::Bold,
    FontStyle::Semibold,
    FontStyle::Italic,
];

/// `Font` (d2fonts_common.go) restricted to the face-selecting fields.
/// d2svg always looks fonts up with `Size` 0, the registry key size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Font {
    pub family: FontFamily,
    pub style: FontStyle,
}

const DATA_URI_PREFIX: &str = "data:application/font-woff;base64,";

static SOURCE_SANS_PRO_REGULAR: &[u8] = include_bytes!("../assets/SourceSansPro-Regular.ttf");
static SOURCE_SANS_PRO_BOLD: &[u8] = include_bytes!("../assets/SourceSansPro-Bold.ttf");
static SOURCE_SANS_PRO_SEMIBOLD: &[u8] = include_bytes!("../assets/SourceSansPro-Semibold.ttf");
static SOURCE_SANS_PRO_ITALIC: &[u8] = include_bytes!("../assets/SourceSansPro-Italic.ttf");
static SOURCE_CODE_PRO_REGULAR: &[u8] = include_bytes!("../assets/SourceCodePro-Regular.ttf");
static SOURCE_CODE_PRO_BOLD: &[u8] = include_bytes!("../assets/SourceCodePro-Bold.ttf");
static SOURCE_CODE_PRO_SEMIBOLD: &[u8] = include_bytes!("../assets/SourceCodePro-Semibold.ttf");
static SOURCE_CODE_PRO_ITALIC: &[u8] = include_bytes!("../assets/SourceCodePro-Italic.ttf");
static FUZZY_BUBBLES_REGULAR: &[u8] = include_bytes!("../assets/FuzzyBubbles-Regular.ttf");
static FUZZY_BUBBLES_BOLD: &[u8] = include_bytes!("../assets/FuzzyBubbles-Bold.ttf");

/// `FontFaces.Get(font)` as registered by `d2fonts_embed.go`'s `init`.
/// HandDrawn has no italic or semibold; Go reuses Regular and Bold.
pub fn font_face(font: Font) -> &'static [u8] {
    use FontFamily::*;
    use FontStyle::*;
    match (font.family, font.style) {
        (SourceSansPro, Regular) => SOURCE_SANS_PRO_REGULAR,
        (SourceSansPro, Bold) => SOURCE_SANS_PRO_BOLD,
        (SourceSansPro, Semibold) => SOURCE_SANS_PRO_SEMIBOLD,
        (SourceSansPro, Italic) => SOURCE_SANS_PRO_ITALIC,
        (SourceCodePro, Regular) => SOURCE_CODE_PRO_REGULAR,
        (SourceCodePro, Bold) => SOURCE_CODE_PRO_BOLD,
        (SourceCodePro, Semibold) => SOURCE_CODE_PRO_SEMIBOLD,
        (SourceCodePro, Italic) => SOURCE_CODE_PRO_ITALIC,
        (HandDrawn, Regular | Italic) => FUZZY_BUBBLES_REGULAR,
        (HandDrawn, Bold | Semibold) => FUZZY_BUBBLES_BOLD,
    }
}

/// `FontEncodings.Get(font)` as registered by `d2fonts_embed.go`.
///
/// For eight of the twelve registrations the embedded
/// `encoded/*.txt` file is exactly the data-URI prefix followed by the
/// standard base64 of the bundled TTF (verified against the pinned files by
/// `tools/font-subset-oracle encodings`), so it is derived here. SourceSansPro
/// Semibold and SourceCodePro Bold/Semibold/Italic instead embed real WOFF
/// files produced by an external converter that cannot be reproduced from
/// the TTF; they are not carried and this returns `None` for them. Those
/// encodings are only reachable through `GetEncodedSubset`'s fallback, which
/// the bundled faces never take (see the crate tests), and the appendix
/// renderer uses only SourceSansPro Regular and Bold.
pub fn font_encoding(font: Font) -> Option<String> {
    use FontFamily::*;
    use FontStyle::*;
    match (font.family, font.style) {
        (SourceSansPro, Semibold) | (SourceCodePro, Bold | Semibold | Italic) => None,
        _ => Some(encode_data_uri(font_face(font))),
    }
}

fn encode_data_uri(woff: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(woff);
    let mut uri = String::with_capacity(DATA_URI_PREFIX.len() + encoded.len());
    uri.push_str(DATA_URI_PREFIX);
    uri.push_str(&encoded);
    uri
}

#[derive(Clone)]
struct RegisteredFace {
    face: std::sync::Arc<[u8]>,
    encoding: Option<String>,
}

type FontRegistry = std::collections::BTreeMap<(String, FontStyle), RegisteredFace>;

fn registry() -> &'static std::sync::Mutex<FontRegistry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<FontRegistry>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// d2fonts.AddFontFamily. Each missing style inherits SourceSansPro, including
/// when the registered family is used as a diagram's monospace family.
pub fn add_font_family(
    name: &str,
    regular: Option<&[u8]>,
    italic: Option<&[u8]>,
    bold: Option<&[u8]>,
    semibold: Option<&[u8]>,
) -> Result<String, String> {
    let mut registered = registry().lock().unwrap();
    for (style, supplied) in [
        (FontStyle::Regular, regular),
        (FontStyle::Italic, italic),
        (FontStyle::Bold, bold),
        (FontStyle::Semibold, semibold),
    ] {
        let key = (name.to_owned(), style);
        if let Some(face) = supplied {
            // AddFontStyle installs the face before attempting WOFF encoding.
            // Preserve that partial registration if encoding fails.
            registered
                .entry(key.clone())
                .or_insert_with(|| RegisteredFace {
                    face: face.into(),
                    encoding: None,
                })
                .face = face.into();
            let encoded = sfnt2woff(face)
                .map_err(|error| format!("failed to encode ttf to woff: {error}"))?;
            registered.get_mut(&key).unwrap().encoding = Some(encode_data_uri(&encoded));
        } else {
            let fallback = FontFamily::SourceSansPro.font(style);
            let fallback = registered
                .get(&(fallback.family.as_str().to_owned(), style))
                .cloned()
                .unwrap_or_else(|| RegisteredFace {
                    face: font_face(fallback).into(),
                    encoding: font_encoding(fallback),
                });
            registered.insert(key, fallback);
        }
    }
    Ok(name.to_owned())
}

/// d2target.validateRenderFontFamily checks all four face registrations.
pub fn is_registered_family(name: &str) -> bool {
    let registered = registry().lock().unwrap();
    FONT_STYLES.iter().all(|&style| {
        registered.get(&(name.to_owned(), style)).map_or_else(
            || FONT_FAMILIES.iter().any(|family| family.as_str() == name),
            |face| !face.face.is_empty(),
        )
    })
}

/// FontFaces.Lookup for string families. Own the snapshot so re-registration
/// cannot invalidate a face while a renderer parses it.
pub fn registered_font_face(name: &str, style: FontStyle) -> Option<std::sync::Arc<[u8]>> {
    if let Some(face) = registry().lock().unwrap().get(&(name.to_owned(), style)) {
        return Some(face.face.clone());
    }
    FONT_FAMILIES
        .iter()
        .find(|family| family.as_str() == name)
        .map(|family| font_face(family.font(style)).into())
}

/// Font.GetEncodedSubset for a string family, including registered families.
/// The registry lock also keeps a re-registration from changing a face midway
/// through the source-defined subsetting/fallback operation.
pub fn registered_encoded_subset(name: &str, style: FontStyle, corpus: &str) -> Option<String> {
    let registered = registry().lock().unwrap();
    if let Some(registered) = registered.get(&(name.to_owned(), style)) {
        let subset = utf8_cut_font(&registered.face, &unique_chars(corpus))?;
        return match sfnt2woff(&subset) {
            Ok(woff) => Some(encode_data_uri(&woff)),
            Err(crate::woff::Sfnt2WoffError::Checksum { .. }) => registered.encoding.clone(),
            Err(crate::woff::Sfnt2WoffError::Malformed) => None,
        };
    }
    FONT_FAMILIES
        .iter()
        .find(|family| family.as_str() == name)?
        .font(style)
        .get_encoded_subset(corpus)
}

/// Per-render custom families. An absent style in a registered family uses
/// SourceSansPro, including customMono (d2fonts.AddFontFamily).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomFonts {
    pub regular: std::collections::BTreeMap<FontStyle, Vec<u8>>,
    pub mono: std::collections::BTreeMap<FontStyle, Vec<u8>>,
}

impl CustomFonts {
    pub fn get_encoded_subset(
        &self,
        mono: bool,
        style: FontStyle,
        corpus: &str,
    ) -> Result<Option<String>, String> {
        let faces = if mono { &self.mono } else { &self.regular };
        if faces.is_empty() {
            return Ok(None);
        }
        let fallback = FontFamily::SourceSansPro.font(style);
        let encoded = if let Some(face) = faces.get(&style) {
            utf8_cut_font(face, &unique_chars(corpus)).and_then(|subset| {
                let woff = match sfnt2woff(&subset) {
                    Ok(woff) => woff,
                    Err(crate::woff::Sfnt2WoffError::Checksum { .. }) => sfnt2woff(face).ok()?,
                    Err(crate::woff::Sfnt2WoffError::Malformed) => return None,
                };
                Some(encode_data_uri(&woff))
            })
        } else {
            fallback.get_encoded_subset(corpus)
        };
        encoded
            .map(Some)
            .ok_or_else(|| format!("custom font subset failed for {style:?}"))
    }
}

/// The deduplicated characters of `corpus` in first-appearance order, as
/// built by `GetEncodedSubset`.
pub fn unique_chars(corpus: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    corpus.chars().filter(|c| seen.insert(*c)).collect()
}

impl Font {
    /// `Font.GetEncodedSubset` (d2fonts_common.go): a WOFF data URI of the
    /// face subset to the characters of `corpus`.
    ///
    /// Returns `None` only where Go panics (a face without a Unicode
    /// format-4 cmap or otherwise malformed) or where Go's checksum fallback
    /// would emit an encoding this port does not carry (see
    /// [`font_encoding`]); neither occurs for the bundled faces.
    pub fn get_encoded_subset(&self, corpus: &str) -> Option<String> {
        let unique_chars = unique_chars(corpus);
        let font_buf = utf8_cut_font(font_face(*self), &unique_chars)?;
        match sfnt2woff(&font_buf) {
            Ok(woff) => Some(encode_data_uri(&woff)),
            // If subset fails, return full encoding
            Err(crate::woff::Sfnt2WoffError::Checksum { .. }) => font_encoding(*self),
            // Go panics indexing the malformed buffer.
            Err(crate::woff::Sfnt2WoffError::Malformed) => None,
        }
    }
}
