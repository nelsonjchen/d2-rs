// SPDX-License-Identifier: MPL-2.0
//! Source d2renderers/internal/fontface/noto_color_emoji_coverage.go,
//! noto_color_emoji_digest.go and noto_color_emoji_native.go (wasm builds
//! follow noto_color_emoji_wasm.go).

use crate::colrv1::{COLRv1Plan, TrustedCOLRv1Limits};
use crate::error::Error;
use crate::face::{BundledNotoColorEmojiSource, Digest32};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use {
    crate::colrv1::compile_colrv1_plan,
    crate::face::{parse_face_collection_with_digest, sha256},
    std::sync::OnceLock,
};

pub(crate) const BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE: usize = 4_991_984;

/// `bundledNotoColorEmojiCOLRv1SHA256`: the decoded immutable resource.
pub(crate) const BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256: Digest32 = [
    0x0a, 0xe5, 0x7f, 0xe5, 0x86, 0x45, 0x63, 0x85, 0x23, 0xba, 0x35, 0xf3, 0x88, 0xd9, 0x37, 0x39,
    0xd2, 0x92, 0x53, 0x9a, 0x9a, 0xcb, 0x84, 0xdf, 0x57, 0x00, 0xc8, 0x1b, 0x1e, 0x1a, 0x28, 0xd2,
];

/// `bundledNotoColorEmojiCmap`: the pinned font's cmap ranges.
const BUNDLED_NOTO_COLOR_EMOJI_CMAP: &[(u32, u32)] = &[
    (0x0020, 0x0020),
    (0x0023, 0x0023),
    (0x002a, 0x002a),
    (0x0030, 0x0039),
    (0x00a9, 0x00a9),
    (0x00ae, 0x00ae),
    (0x200d, 0x200d),
    (0x203c, 0x203c),
    (0x2049, 0x2049),
    (0x20e3, 0x20e3),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x2199),
    (0x21a9, 0x21aa),
    (0x231a, 0x231b),
    (0x2328, 0x2328),
    (0x23cf, 0x23cf),
    (0x23e9, 0x23f3),
    (0x23f8, 0x23fa),
    (0x24c2, 0x24c2),
    (0x25aa, 0x25ab),
    (0x25b6, 0x25b6),
    (0x25c0, 0x25c0),
    (0x25fb, 0x25fe),
    (0x2600, 0x2604),
    (0x260e, 0x260e),
    (0x2611, 0x2611),
    (0x2614, 0x2615),
    (0x2618, 0x2618),
    (0x261d, 0x261d),
    (0x2620, 0x2620),
    (0x2622, 0x2623),
    (0x2626, 0x2626),
    (0x262a, 0x262a),
    (0x262e, 0x262f),
    (0x2638, 0x263a),
    (0x2640, 0x2640),
    (0x2642, 0x2642),
    (0x2648, 0x2653),
    (0x265f, 0x2660),
    (0x2663, 0x2663),
    (0x2665, 0x2666),
    (0x2668, 0x2668),
    (0x267b, 0x267b),
    (0x267e, 0x267f),
    (0x2692, 0x2697),
    (0x2699, 0x2699),
    (0x269b, 0x269c),
    (0x26a0, 0x26a1),
    (0x26a7, 0x26a7),
    (0x26aa, 0x26ab),
    (0x26b0, 0x26b1),
    (0x26bd, 0x26be),
    (0x26c4, 0x26c5),
    (0x26c8, 0x26c8),
    (0x26ce, 0x26cf),
    (0x26d1, 0x26d1),
    (0x26d3, 0x26d4),
    (0x26e9, 0x26ea),
    (0x26f0, 0x26f5),
    (0x26f7, 0x26fa),
    (0x26fd, 0x26fd),
    (0x2702, 0x2702),
    (0x2705, 0x2705),
    (0x2708, 0x270d),
    (0x270f, 0x270f),
    (0x2712, 0x2712),
    (0x2714, 0x2714),
    (0x2716, 0x2716),
    (0x271d, 0x271d),
    (0x2721, 0x2721),
    (0x2728, 0x2728),
    (0x2733, 0x2734),
    (0x2744, 0x2744),
    (0x2747, 0x2747),
    (0x274c, 0x274c),
    (0x274e, 0x274e),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2763, 0x2764),
    (0x2795, 0x2797),
    (0x27a1, 0x27a1),
    (0x27b0, 0x27b0),
    (0x27bf, 0x27bf),
    (0x2934, 0x2935),
    (0x2b05, 0x2b07),
    (0x2b1b, 0x2b1c),
    (0x2b50, 0x2b50),
    (0x2b55, 0x2b55),
    (0x3030, 0x3030),
    (0x303d, 0x303d),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1f004, 0x1f004),
    (0x1f0cf, 0x1f0cf),
    (0x1f170, 0x1f171),
    (0x1f17e, 0x1f17f),
    (0x1f18e, 0x1f18e),
    (0x1f191, 0x1f19a),
    (0x1f1e6, 0x1f1ff),
    (0x1f201, 0x1f202),
    (0x1f21a, 0x1f21a),
    (0x1f22f, 0x1f22f),
    (0x1f232, 0x1f23a),
    (0x1f250, 0x1f251),
    (0x1f300, 0x1f321),
    (0x1f324, 0x1f393),
    (0x1f396, 0x1f397),
    (0x1f399, 0x1f39b),
    (0x1f39e, 0x1f3f0),
    (0x1f3f3, 0x1f3f5),
    (0x1f3f7, 0x1f4fd),
    (0x1f4ff, 0x1f53d),
    (0x1f549, 0x1f54e),
    (0x1f550, 0x1f567),
    (0x1f56f, 0x1f570),
    (0x1f573, 0x1f57a),
    (0x1f587, 0x1f587),
    (0x1f58a, 0x1f58d),
    (0x1f590, 0x1f590),
    (0x1f595, 0x1f596),
    (0x1f5a4, 0x1f5a5),
    (0x1f5a8, 0x1f5a8),
    (0x1f5b1, 0x1f5b2),
    (0x1f5bc, 0x1f5bc),
    (0x1f5c2, 0x1f5c4),
    (0x1f5d1, 0x1f5d3),
    (0x1f5dc, 0x1f5de),
    (0x1f5e1, 0x1f5e1),
    (0x1f5e3, 0x1f5e3),
    (0x1f5e8, 0x1f5e8),
    (0x1f5ef, 0x1f5ef),
    (0x1f5f3, 0x1f5f3),
    (0x1f5fa, 0x1f64f),
    (0x1f680, 0x1f6c5),
    (0x1f6cb, 0x1f6d2),
    (0x1f6d5, 0x1f6d8),
    (0x1f6dc, 0x1f6e5),
    (0x1f6e9, 0x1f6e9),
    (0x1f6eb, 0x1f6ec),
    (0x1f6f0, 0x1f6f0),
    (0x1f6f3, 0x1f6fc),
    (0x1f7e0, 0x1f7eb),
    (0x1f7f0, 0x1f7f0),
    (0x1f90c, 0x1f93a),
    (0x1f93c, 0x1f945),
    (0x1f947, 0x1f9ff),
    (0x1fa70, 0x1fa7c),
    (0x1fa80, 0x1fa8a),
    (0x1fa8e, 0x1fac6),
    (0x1fac8, 0x1fac8),
    (0x1facd, 0x1fadc),
    (0x1fadf, 0x1faea),
    (0x1faef, 0x1faf8),
    (0xe0030, 0xe0039),
    (0xe0061, 0xe007a),
    (0xe007f, 0xe007f),
    (0xfe4e5, 0xfe4ee),
    (0xfe82c, 0xfe82c),
    (0xfe82e, 0xfe837),
];

/// `BundledNotoColorEmojiCoversRune`.
pub fn bundled_noto_color_emoji_covers_rune(value: char) -> bool {
    let value = u32::from(value);
    let (mut left, mut right) = (0usize, BUNDLED_NOTO_COLOR_EMOJI_CMAP.len());
    while left < right {
        let middle = (left + right) >> 1;
        let (first, last) = BUNDLED_NOTO_COLOR_EMOJI_CMAP[middle];
        if value < first {
            right = middle;
        } else if value > last {
            left = middle + 1;
        } else {
            return true;
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
struct Trusted {
    data: Arc<[u8]>,
    source: Arc<BundledNotoColorEmojiSource>,
}

#[cfg(not(target_arch = "wasm32"))]
static TRUSTED: OnceLock<Result<Trusted, Error>> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn register_authenticated_bundled_noto_color_emoji(
    data: Arc<[u8]>,
    digest: Digest32,
) -> Result<Arc<[u8]>, Error> {
    let trusted = TRUSTED.get_or_init(|| {
        let collection = parse_face_collection_with_digest(data, 1, digest)
            .map_err(|error| Error::wrap("d2fonts: parse bundled Noto Color Emoji", error))?;
        if collection.num_faces() != 1 {
            return Err(Error::msg(format!(
                "d2fonts: bundled Noto Color Emoji has {} faces, want 1",
                collection.num_faces()
            )));
        }
        let face = collection
            .face(0)
            .map_err(|error| Error::wrap("d2fonts: load bundled Noto Color Emoji", error))?;
        Ok(Trusted {
            data: collection.data().clone(),
            source: Arc::new(BundledNotoColorEmojiSource { face }),
        })
    });
    match trusted {
        Ok(trusted) => Ok(trusted.data.clone()),
        Err(error) => Err(error.clone()),
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn register_authenticated_bundled_noto_color_emoji(
    data: Arc<[u8]>,
    _digest: Digest32,
) -> Result<Arc<[u8]>, Error> {
    Ok(data)
}

/// The decoded bundled face. build.rs decodes the pinned Brotli asset and
/// fails the build unless its compressed and decoded sizes and SHA-256
/// digests match, so the embedded bytes are the authenticated resource.
#[cfg(not(target_arch = "wasm32"))]
static BUNDLED_NOTO_COLOR_EMOJI_DECODED: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/NotoColorEmoji-COLRv1-v2.051.ttf"
));

/// The build-authenticated bundled face, borrowed without registering or
/// copying it. Font readers that accept static bytes can use this backing.
#[cfg(not(target_arch = "wasm32"))]
pub fn bundled_noto_color_emoji_bytes() -> Result<&'static [u8], Error> {
    if BUNDLED_NOTO_COLOR_EMOJI_DECODED.len() != BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE {
        return Err(Error::msg(
            "d2fonts: bundled Noto Color Emoji decoded resource is not authenticated",
        ));
    }
    Ok(BUNDLED_NOTO_COLOR_EMOJI_DECODED)
}

/// The registered bundled Noto Color Emoji backing, shared by every caller
/// in the process; the first call registers the build-authenticated face.
#[cfg(not(target_arch = "wasm32"))]
pub fn bundled_noto_color_emoji() -> Result<Arc<[u8]>, Error> {
    if let Some(trusted) = TRUSTED.get() {
        return match trusted {
            Ok(trusted) => Ok(trusted.data.clone()),
            Err(error) => Err(error.clone()),
        };
    }
    register_authenticated_bundled_noto_color_emoji(
        Arc::from(bundled_noto_color_emoji_bytes()?),
        BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256,
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod bundled_tests {
    #[test]
    fn embedded_face_is_the_pinned_resource() {
        let data = super::bundled_noto_color_emoji().unwrap();
        assert_eq!(data.len(), super::BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE);
        assert_eq!(
            data.as_ref(),
            super::bundled_noto_color_emoji_bytes().unwrap()
        );
        assert_eq!(
            crate::face::sha256(&data),
            super::BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256
        );
        assert!(std::sync::Arc::ptr_eq(
            &data,
            &super::bundled_noto_color_emoji().unwrap()
        ));
    }
}

/// `RegisterOwnedBundledNotoColorEmoji`: authenticate and take ownership of
/// a freshly decoded resource; returns the canonical registered backing.
#[cfg(not(target_arch = "wasm32"))]
pub fn register_owned_bundled_noto_color_emoji(data: Vec<u8>) -> Result<Arc<[u8]>, Error> {
    let unauthenticated =
        || Error::msg("d2fonts: bundled Noto Color Emoji decoded resource is not authenticated");
    if data.len() != BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE {
        return Err(unauthenticated());
    }
    let digest = sha256(&data);
    if digest != BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256 {
        return Err(unauthenticated());
    }
    register_authenticated_bundled_noto_color_emoji(Arc::from(data), digest)
}

#[cfg(target_arch = "wasm32")]
pub fn register_owned_bundled_noto_color_emoji(data: Vec<u8>) -> Result<Arc<[u8]>, Error> {
    Ok(Arc::from(data))
}

/// `RegisteredBundledNotoColorEmoji`: Ok(None) when `data` is not the exact
/// bundled resource (callers continue through the ordinary parser path).
#[cfg(not(target_arch = "wasm32"))]
pub fn registered_bundled_noto_color_emoji(
    data: &[u8],
    face_index: u16,
) -> Result<Option<Arc<BundledNotoColorEmojiSource>>, Error> {
    if data.len() != BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE {
        return Ok(None);
    }
    // This exact static backing was authenticated by build.rs; unrelated
    // candidate slices still require the ordinary byte or digest checks.
    let embedded = std::ptr::eq(data.as_ptr(), BUNDLED_NOTO_COLOR_EMOJI_DECODED.as_ptr());
    let digest = match TRUSTED.get() {
        Some(Ok(trusted)) => {
            if !embedded
                && !std::ptr::eq(data.as_ptr(), trusted.data.as_ptr())
                && data != &trusted.data[..]
            {
                return Ok(None);
            }
            BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256
        }
        _ => {
            let digest = if embedded {
                BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256
            } else {
                sha256(data)
            };
            if digest != BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256 {
                return Ok(None);
            }
            digest
        }
    };
    if face_index != 0 {
        return Err(Error::msg(format!(
            "load font face {face_index}: collection has 1 faces"
        )));
    }
    // The ownership wrapper in noto_color_emoji_native.go copies bytes only
    // inside its once initializer. A registered face already owns its backing.
    if TRUSTED.get().is_none() {
        register_authenticated_bundled_noto_color_emoji(Arc::from(data), digest)?;
    }
    match TRUSTED.get() {
        Some(Ok(trusted)) => Ok(Some(trusted.source.clone())),
        Some(Err(error)) => Err(error.clone()),
        None => Err(Error::msg(
            "d2fonts: bundled Noto Color Emoji is not registered",
        )),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn registered_bundled_noto_color_emoji(
    _data: &[u8],
    _face_index: u16,
) -> Result<Option<Arc<BundledNotoColorEmojiSource>>, Error> {
    Ok(None)
}

/// `RegisteredBundledNotoColorEmojiBackingDigest`: identity-only match of
/// the registered private backing.
pub fn registered_bundled_noto_color_emoji_backing_digest(data: &[u8]) -> Option<Digest32> {
    if data.len() != BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE {
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(Ok(trusted)) = TRUSTED.get()
        && std::ptr::eq(data.as_ptr(), trusted.data.as_ptr())
    {
        return Some(BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256);
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn compile_private_bundled_noto_color_emoji_colrv1_plan(
    glyph_id: u32,
    limits: TrustedCOLRv1Limits,
) -> Result<Option<COLRv1Plan>, Error> {
    match TRUSTED.get() {
        Some(Ok(trusted)) => compile_colrv1_plan(&trusted.source.face, glyph_id, limits),
        Some(Err(error)) => Err(error.clone()),
        None => Err(Error::msg(
            "d2fonts: bundled Noto Color Emoji is not registered",
        )),
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn compile_private_bundled_noto_color_emoji_colrv1_plan(
    _glyph_id: u32,
    _limits: TrustedCOLRv1Limits,
) -> Result<Option<COLRv1Plan>, Error> {
    Ok(None)
}
