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

/// Byte length and SHA-256 of a bundled face, precomputed from the embedded
/// TTFs so authenticating the registry needs no hashing at startup.
fn builtin_face_digest(font: Font) -> (usize, [u8; 32]) {
    use FontFamily::*;
    use FontStyle::*;
    let (size, hex) = match (font.family, font.style) {
        (SourceSansPro, Regular) => (
            248_504,
            "a4c07a3a09edc71a053d3ed78d69ff1ad42f2f1f6b9f11090bde7cf618355c78",
        ),
        (SourceSansPro, Bold) => (
            247_784,
            "f205fb64af917c37b60b64a95e0eac3a9e5bb9849fa07c2f2c2a0463d55589e8",
        ),
        (SourceSansPro, Semibold) => (
            253_832,
            "72bf9aa2e78b8c78444ae74b6be09411d5fb5bf0eb9bcdfd47082cc5d9b536df",
        ),
        (SourceSansPro, Italic) => (
            109_604,
            "bd77181e59a8f0f3ea2bae01ba13102e4cd8f3b4ce3a7fa29be4bda9b3c9fa9c",
        ),
        (SourceCodePro, Regular) => (
            138_680,
            "f144137f557805c7327fc4b14d1d730f6e1822e0124170251ff1bcd723a693f1",
        ),
        (SourceCodePro, Bold) => (
            119_648,
            "c2aabd3830bd19420670a395c4df30596e950c53f4616445107a114046b9e618",
        ),
        (SourceCodePro, Semibold) => (
            138_208,
            "1b61c980d75de8216228a2603bcabe51394013452df26a04c99326bbc035c5a5",
        ),
        (SourceCodePro, Italic) => (
            101_260,
            "c33aca4e40836061acb34f47653db900f1030c82310a2ed08a6dcbf3cec33705",
        ),
        (HandDrawn, Regular | Italic) => (
            145_008,
            "3d2e4938c48dca0484bff9b746d0935048548072ebb1a89bc3933a65540e7b98",
        ),
        (HandDrawn, Bold | Semibold) => (
            139_528,
            "92478cf4c07457181b46ea7d6a5535af984c9f05fccbefc8a7ea343ec630d711",
        ),
    };
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex digest");
    }
    (size, digest)
}

/// `registered_font_face` as an owned buffer, copying a bundled face once.
pub fn registered_font_face_vec(name: &str, style: FontStyle) -> Option<Vec<u8>> {
    if let Some(face) = registry().lock().unwrap().get(&(name.to_owned(), style)) {
        return Some(face.face.to_vec());
    }
    FONT_FAMILIES
        .iter()
        .find(|family| family.as_str() == name)
        .map(|family| font_face(family.font(style)).to_vec())
}

/// The size and SHA-256 of the face `registered_font_face` returns, without
/// copying or hashing it, when that face is an unmodified bundled one; None
/// for an explicit process registration or an unknown family.
pub fn registered_font_face_digest(name: &str, style: FontStyle) -> Option<(usize, [u8; 32])> {
    if registry()
        .lock()
        .unwrap()
        .contains_key(&(name.to_owned(), style))
    {
        return None;
    }
    FONT_FAMILIES
        .iter()
        .find(|family| family.as_str() == name)
        .map(|family| builtin_face_digest(family.font(style)))
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

#[cfg(test)]
mod builtin_face_digest_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn precomputed_face_digests_match_the_embedded_fonts() {
        for family in FONT_FAMILIES {
            for style in FONT_STYLES {
                let face = font_face(family.font(style));
                let digest: [u8; 32] = Sha256::digest(face).into();
                assert_eq!(
                    registered_font_face_digest(family.as_str(), style),
                    Some((face.len(), digest)),
                    "{family:?} {style:?}"
                );
            }
        }
        assert_eq!(
            registered_font_face_digest("NoSuchFamily", FontStyle::Regular),
            None
        );
    }
}
