// SPDX-License-Identifier: MPL-2.0
//! D2 v0.9.0 d2fonts/fallback.go path and collection-face ranking.
//!
//! The raster adapter supplies one missing character at a time. Whole-scene
//! missing-rune buckets, bundled emoji precedence, bounded discovery and
//! cumulative work/cancellation contracts still require their source APIs.

use resvg::usvg::fontdb::{Database, FaceInfo, ID, Source, Style};
use std::path::{Path, PathBuf};

#[path = "font_cmap.rs"]
pub(super) mod source_cmap;

#[derive(Clone, Debug)]
struct Profile {
    family: String,
    weight: i32,
    italic: bool,
    mono: bool,
    cjk: bool,
    arabic: bool,
    hebrew: bool,
    symbol: bool,
}

fn normalize_family(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

impl Profile {
    fn new(
        family: &str,
        weight: i32,
        italic: bool,
        values: impl IntoIterator<Item = char>,
    ) -> Self {
        let family = normalize_family(family);
        let mut profile = Self {
            mono: ["mono", "code", "console"]
                .iter()
                .any(|part| family.contains(part)),
            family,
            weight: weight.clamp(100, 900),
            italic,
            cjk: false,
            arabic: false,
            hebrew: false,
            symbol: false,
        };
        for value in values {
            let value = u32::from(value);
            profile.cjk |= matches!(value, 0x2e80..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af);
            profile.arabic |= matches!(value, 0x0600..=0x08ff | 0xfb50..=0xfdff | 0xfe70..=0xfeff);
            profile.hebrew |= matches!(value, 0x0590..=0x05ff | 0xfb1d..=0xfb4f);
            profile.symbol |= matches!(value, 0x2190..=0x2bff | 0x1f000..=0x1faff);
        }
        profile
    }

    fn penalty(&self, family: &str, weight: i32, italic: bool, mono: bool) -> i32 {
        let mut penalty = (weight - self.weight).abs();
        if italic != self.italic {
            penalty += 1000;
        }
        if mono != self.mono {
            penalty += if self.mono { 1000 } else { 100 };
        }
        if !self.family.is_empty() && !family.is_empty() {
            if family == self.family {
                penalty -= 40;
            } else if ["sans", "serif", "mono", "code", "console"]
                .iter()
                .any(|part| family.contains(part) && self.family.contains(part))
            {
                penalty -= 10;
            }
        }
        penalty
    }

    fn path_priority(&self, path: &Path) -> i32 {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let contains = |parts: &[&str]| parts.iter().any(|part| name.contains(part));
        let mut semantic: i32 = if contains(&["arial unicode", "unifont"]) {
            -10
        } else {
            80
        };
        if (self.cjk
            && contains(&[
                "cjk", "hiragino", "pingfang", "heiti", "gothic", "songti", "mingliu", "simsun",
                "malgun",
            ]))
            || (self.arabic && contains(&["arab", "amiri", "naskh", "kufi"]))
            || (self.hebrew && contains(&["hebrew", "david", "frank"]))
            || (self.symbol
                && contains(&["symbol", "emoji", "dingbat", "dejavu", "unifont", "unicode"]))
        {
            semantic = semantic.min(0);
        }
        if contains(&["noto", "dejavu", "arial unicode", "unifont"]) {
            semantic = semantic.min(20);
        }
        if contains(&["emoji"]) {
            semantic += 10;
        }
        if self.mono && contains(&["mono", "code", "console"]) {
            semantic -= 6;
        }
        semantic
            + self.penalty(
                &normalize_family(&name),
                filename_weight(&name),
                contains(&["italic", "oblique"]),
                contains(&["mono", "code", "console"]),
            )
    }

    fn face_priority(&self, face: &FaceInfo) -> i32 {
        self.penalty(
            &normalize_family(face.families.first().map_or("", |family| family.0.as_str())),
            i32::from(face.weight.0).clamp(100, 900),
            face.style != Style::Normal,
            face.monospaced,
        )
    }
}

fn filename_weight(name: &str) -> i32 {
    // Source order matters: semibold includes bold, and extralight includes light.
    for (parts, weight) in [
        (&["extrablack", "ultrablack", "superblack"][..], 900),
        (&["black", "heavy"][..], 900),
        (
            &["extrabold", "extra-bold", "ultrabold", "ultra-bold"][..],
            800,
        ),
        (&["semibold", "semi-bold", "demibold", "demi-bold"][..], 600),
        (&["bold"][..], 700),
        (&["medium"][..], 500),
        (
            &["extralight", "extra-light", "ultralight", "ultra-light"][..],
            200,
        ),
        (
            &["semilight", "semi-light", "demilight", "demi-light"][..],
            350,
        ),
        (&["thin"][..], 100),
        (&["light"][..], 300),
    ] {
        if parts.iter().any(|part| name.contains(part)) {
            return weight;
        }
    }
    400
}

fn system_path(face: &FaceInfo) -> Option<&Path> {
    let path = match &face.source {
        Source::File(path) => path.as_path(),
        Source::Binary(_) => return None,
    };
    // Source excludes user font roots. Primary/custom faces are selected by
    // the ordinary font selector, before this fallback boundary.
    let in_root = if cfg!(target_os = "macos") {
        ["/System/Library/Fonts", "/Library/Fonts"]
            .iter()
            .any(|root| path.starts_with(root))
    } else if cfg!(target_os = "windows") {
        path.starts_with(
            PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
                .join("Fonts"),
        )
    } else {
        ["/usr/share/fonts", "/usr/local/share/fonts"]
            .iter()
            .any(|root| path.starts_with(root))
    };
    // Source raster.go's per-file search budget and fallback.go's Lstat
    // admission precede cmap probing. Large fonts must not win merely because
    // the preloaded font database already contains their faces.
    let metadata = std::fs::symlink_metadata(path).ok()?;
    (in_root && metadata.is_file() && metadata.len() > 0 && metadata.len() <= 32 * 1024 * 1024)
        .then_some(path)
}

pub(super) fn select_fallback(value: char, excluded: &[ID], database: &Database) -> Option<ID> {
    let base = database.face(*excluded.first()?)?;
    let profile = Profile::new(
        base.families.first().map_or("", |family| family.0.as_str()),
        i32::from(base.weight.0),
        base.style != Style::Normal,
        [value],
    );
    // Source sorts paths first, then considers faces within each collection.
    // Global face sorting would change which file supplies a fallback.
    let mut candidates: Vec<_> = database
        .faces()
        .filter(|face| !excluded.contains(&face.id))
        .filter_map(|face| system_path(face).map(|path| (path, face)))
        .collect();
    candidates.sort_by(|(a_path, a), (b_path, b)| {
        profile
            .path_priority(a_path)
            .cmp(&profile.path_priority(b_path))
            .then_with(|| a_path.cmp(b_path))
            .then_with(|| profile.face_priority(a).cmp(&profile.face_priority(b)))
            .then_with(|| a.index.cmp(&b.index))
    });
    for (path, face) in candidates {
        let covered = database
            .with_face_data(face.id, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .ok()
                    .and_then(|face| {
                        let glyph = source_cmap::glyph_index(&face, value)?;
                        (face.glyph_index(value) == Some(glyph)).then_some(glyph)
                    })
                    .is_some_and(|glyph| glyph.0 != 0)
            })
            .unwrap_or(false);
        if covered {
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var_os("D2_RS_TRACE_FONT_FALLBACK_SELECTION").is_some() {
                eprintln!(
                    "FONT_FALLBACK_RUST value={value:?} request={profile:?} path={path:?} face={} family={:?} weight={}",
                    face.index, face.families, face.weight.0
                );
            }
            #[cfg(not(feature = "diagnostic-traces"))]
            let _ = path;
            return Some(face.id);
        }
    }
    None
}

pub(super) fn face_covers(database: &Database, face: ID, value: char) -> bool {
    database
        .with_face_data(face, |data, index| {
            ttf_parser::Face::parse(data, index)
                .ok()
                .and_then(|face| {
                    let glyph = source_cmap::glyph_index(&face, value)?;
                    (face.glyph_index(value) == Some(glyph)).then_some(glyph)
                })
                .is_some_and(|glyph| glyph.0 != 0)
        })
        .unwrap_or(false)
}

/// d2fonts SystemFallbackResolver.ResolveFallbacks (fallback.go:121–282) for
/// one scene bucket: one request profile from every missing rune, font paths
/// in priority order, and within each path the unselected face covering the
/// remaining runes with the best priority (ties: more coverage, then the
/// earlier face). Returns the faces in resolution order.
pub(super) fn select_fallbacks(values: &[char], excluded: &[ID], database: &Database) -> Vec<ID> {
    let Some(base) = excluded.first().and_then(|id| database.face(*id)) else {
        return Vec::new();
    };
    let profile = Profile::new(
        base.families.first().map_or("", |family| family.0.as_str()),
        i32::from(base.weight.0),
        base.style != Style::Normal,
        values.iter().copied(),
    );
    let mut paths: Vec<(&Path, Vec<&FaceInfo>)> = Vec::new();
    for face in database.faces().filter(|face| !excluded.contains(&face.id)) {
        let Some(path) = system_path(face) else {
            continue;
        };
        match paths.iter_mut().find(|(candidate, _)| *candidate == path) {
            Some((_, faces)) => faces.push(face),
            None => paths.push((path, vec![face])),
        }
    }
    paths.sort_by(|(a, _), (b, _)| {
        profile
            .path_priority(a)
            .cmp(&profile.path_priority(b))
            .then_with(|| a.cmp(b))
    });
    let mut remaining: Vec<char> = Vec::new();
    for value in values {
        if !remaining.contains(value) {
            remaining.push(*value);
        }
    }
    let mut resolved = Vec::new();
    for (_, mut faces) in paths {
        faces.sort_by_key(|face| face.index);
        let mut selected = Vec::new();
        while !remaining.is_empty() {
            let mut best: Option<(ID, i32, Vec<char>)> = None;
            for face in &faces {
                if selected.contains(&face.id) {
                    continue;
                }
                let covered: Vec<char> = remaining
                    .iter()
                    .copied()
                    .filter(|value| face_covers(database, face.id, *value))
                    .collect();
                if covered.is_empty() {
                    continue;
                }
                let priority = profile.face_priority(face);
                if best.as_ref().is_none_or(|(_, best_priority, best_covered)| {
                    priority < *best_priority
                        || (priority == *best_priority && covered.len() > best_covered.len())
                }) {
                    best = Some((face.id, priority, covered));
                }
            }
            let Some((id, _, covered)) = best else {
                break;
            };
            selected.push(id);
            resolved.push(id);
            remaining.retain(|value| !covered.contains(value));
        }
        if remaining.is_empty() {
            break;
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_style_priorities_preserve_bold_italic_semibold_and_mono() {
        // Source fallback_test.go:TestFallbackPathPriorityPreservesRequestedStyle.
        let italic = Profile::new("SourceSansPro", 400, true, ['A']);
        assert!(
            italic.path_priority(Path::new("Custom-Italic.ttf"))
                < italic.path_priority(Path::new("Custom-Regular.ttf"))
        );
        let bold_mono = Profile::new("SourceCodePro", 700, false, ['A']);
        assert!(
            bold_mono.path_priority(Path::new("CustomMono-Bold.ttf"))
                < bold_mono.path_priority(Path::new("Custom-Regular.ttf"))
        );
        let bold = Profile::new("Source Sans Pro", 700, false, ['A']);
        assert!(
            bold.path_priority(Path::new("NotoSansArabic-Bold.ttf"))
                < bold.path_priority(Path::new("Arial Unicode.ttf"))
        );
        assert_eq!(filename_weight("custom-semibold.ttf"), 600);
        assert_eq!(filename_weight("custom-extralight.ttf"), 200);
    }

    #[test]
    fn source_script_priority_and_family_class_apply_before_path_ties() {
        let japanese = Profile::new("Source Sans Pro", 700, false, ['あ']);
        assert!(
            japanese.path_priority(Path::new("Hiragino.ttc"))
                < japanese.path_priority(Path::new("Custom.ttf"))
        );
        assert_eq!(japanese.penalty("sourcesanspro", 700, false, false), -40);
        assert_eq!(japanese.penalty("notosans", 700, false, false), -10);
        assert_eq!(japanese.penalty("serif", 400, true, true), 1400);
    }
}
