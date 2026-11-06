// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Canonical D2 SVG fill-pattern assets.
//!
//! The dots and lines definitions are kept as source-shaped SVG. Grain and
//! paper use the exact embedded assets from the pinned D2 renderer,
//! with a stable local suffix in place of D2's per-diagram hash.

const GRAIN_PNG_BASE64: &str = include_str!("../assets/grain.base64");
const PAPER_TEMPLATE: &str = include_str!("../assets/paper.svg");
const STREAK_PATH: &str = include_str!("../assets/streak_path.txt");

const GRAIN_PREFIX: &str = r##"<pattern id="grain-%[1]s" x="0" y="0" width="300" height="300" patternUnits="userSpaceOnUse">
<g opacity="0.8" clip-path="url(#clip0_3943_51420-%[1]s)">
<path d="M300 0H0V300H300V0Z" fill="url(#pattern0-%[1]s)" fill-opacity="0.9"/>
</g>
<defs>
<pattern id="pattern0-%[1]s" patternContentUnits="objectBoundingBox" width="1" height="1">
<use xlink:href="#image0_3943_51420-%[1]s" transform="scale(0.00214592 0.00286533)"/>
</pattern>
<clipPath id="clip0_3943_51420-%[1]s">
<rect width="300" height="300" fill="white"/>
</clipPath>
<image id="image0_3943_51420-%[1]s" width="466" height="349" xlink:href="data:image/png;base64,"##;
const GRAIN_SUFFIX: &str = r##""/>
</defs>
</pattern>
"##;

const DOTS: &str = r##"<pattern id="d2-rs-pattern-dots" x="0" y="0" width="15" height="15" patternUnits="userSpaceOnUse">
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="2" y="2" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="12" y="2" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="12" y="12" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="2" y="12" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="2" y="7" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="12" y="7" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="7" y="2" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="7" y="12" width="1" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="7" y="7" width="1" height="1" fill="#0A0F25"/></g>
</pattern>
"##;

const LINES: &str = r##"<pattern id="d2-rs-pattern-lines" x="0" y="0" width="15" height="15" patternUnits="userSpaceOnUse">
<g style="mix-blend-mode:multiply">
<g style="mix-blend-mode:multiply" opacity="0.05"><rect y="2" width="15" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.05"><rect y="7" width="15" height="1" fill="#0A0F25"/></g>
<g style="mix-blend-mode:multiply" opacity="0.05"><rect y="12" width="15" height="1" fill="#0A0F25"/></g>
</g>
</pattern>
"##;

/// Return the stable SVG ID used by a fill-pattern value.
pub(crate) fn id(pattern: &str) -> Option<&'static str> {
    match pattern {
        "dots" => Some("d2-rs-pattern-dots"),
        "lines" => Some("d2-rs-pattern-lines"),
        "grain" => Some("grain-d2-rs"),
        "paper" => Some("paper-d2-rs"),
        _ => None,
    }
}

/// patternassets.GrainSVG(): the Go format template (`%[1]s` = diagram hash).
pub(crate) fn grain_svg_template() -> String {
    format!("{GRAIN_PREFIX}{GRAIN_PNG_BASE64}{GRAIN_SUFFIX}")
}

/// patternassets.PaperSVG(): the Go format template (`%[1]s` = diagram hash).
pub(crate) fn paper_svg_template() -> &'static str {
    PAPER_TEMPLATE
}

/// Return only the built-in pattern definitions required by this target.
///
/// D2 emits texture definitions on demand. Keeping that behavior matters for
/// ordinary SVG size: the paper and grain assets are intentionally large and
/// must not be embedded in diagrams that do not use them.
pub(crate) fn definitions_for(patterns: &std::collections::BTreeSet<String>) -> String {
    let mut definitions = String::new();
    if patterns.contains("dots") {
        definitions.push_str(DOTS);
    }
    if patterns.contains("lines") {
        definitions.push_str(LINES);
    }
    if patterns.contains("grain") {
        definitions.push_str(
            &format!("{GRAIN_PREFIX}{GRAIN_PNG_BASE64}{GRAIN_SUFFIX}").replace("%[1]s", "d2-rs"),
        );
    }
    if patterns.contains("paper") {
        definitions.push_str(&PAPER_TEMPLATE.replace("%[1]s", "d2-rs"));
    }
    definitions
}

/// Source d2scenebuild patterns.go places the canonical grain PNG directly
/// in a300x300 image box. Its SVG target uses a nested object-box wrapper;
/// the private native raster target transports the actual scene placement.
pub(crate) fn definitions_for_raster(patterns: &std::collections::BTreeSet<String>) -> String {
    let mut ordinary = patterns.clone();
    ordinary.remove("grain");
    ordinary.remove("dots");
    ordinary.remove("lines");
    ordinary.remove("paper");
    let mut definitions = definitions_for(&ordinary);
    if patterns.contains("paper") {
        // Original pattern_sources.go drops the initial white mask path, then
        // newPaperPatternAsset paints every remaining path in one opacity group.
        // The native scene has no legacy SVG mask or nested viewport wrapper.
        let document = roxmltree::Document::parse(PAPER_TEMPLATE).expect("pinned paper asset");
        definitions.push_str(r##"<pattern id="paper-d2-rs" x="0" y="0" width="75" height="75" patternUnits="userSpaceOnUse"><g style="mix-blend-mode:multiply" opacity="0.3">"##);
        for path in document
            .descendants()
            .filter(|node| node.has_tag_name("path"))
            .skip(1)
        {
            definitions.push_str(&PAPER_TEMPLATE[path.range()]);
        }
        definitions.push_str("</g></pattern>");
    }
    if patterns.contains("dots") {
        definitions.push_str(r##"<pattern id="d2-rs-pattern-dots" x="0" y="0" width="15" height="15" patternUnits="userSpaceOnUse"><g style="mix-blend-mode:multiply" opacity="0.1">"##);
        for (x, y) in [
            (2, 2),
            (12, 2),
            (12, 12),
            (2, 12),
            (2, 7),
            (12, 7),
            (7, 2),
            (7, 12),
            (7, 7),
        ] {
            definitions.push_str(&format!(
                r##"<rect x="{x}" y="{y}" width="1" height="1" fill="#0A0F25"/>"##
            ));
        }
        definitions.push_str("</g></pattern>");
    }
    if patterns.contains("lines") {
        definitions.push_str(r##"<pattern id="d2-rs-pattern-lines" x="0" y="0" width="15" height="15" patternUnits="userSpaceOnUse"><g style="mix-blend-mode:multiply" opacity="0.05">"##);
        for y in [2, 7, 12] {
            definitions.push_str(&format!(
                r##"<rect y="{y}" width="15" height="1" fill="#0A0F25"/>"##
            ));
        }
        definitions.push_str("</g></pattern>");
    }
    if patterns.contains("grain") {
        definitions.push_str(&format!(
            r#"<pattern id="grain-d2-rs" x="0" y="0" width="300" height="300" patternUnits="userSpaceOnUse"><g opacity="0.8"><g opacity="0.9"><image width="300" height="300" preserveAspectRatio="none" xlink:href="data:image/png;base64,{GRAIN_PNG_BASE64}"/></g></g></pattern>"#
        ));
    }
    definitions
}

#[cfg(test)]
fn definitions() -> String {
    definitions_for(
        &["dots", "lines", "grain", "paper"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
}

/// The pinned sketch streak path, exposed for the future rough target.
#[allow(dead_code)]
pub(crate) fn streak_path() -> &'static str {
    STREAK_PATH
}

#[cfg(test)]
mod tests {
    use super::{definitions, id};

    #[test]
    fn canonical_asset_wrappers_are_present() {
        let defs = definitions();
        assert!(defs.contains("id=\"grain-d2-rs\""));
        assert!(defs.contains("id=\"paper-d2-rs\""));
        assert!(defs.contains("xlink:href=\"data:image/png;base64,iVBOR"));
        assert!(defs.contains("<rect x=\"2\" y=\"7\" width=\"1\" height=\"1\""));
        assert_eq!(id("dots"), Some("d2-rs-pattern-dots"));
        assert_eq!(id("grain"), Some("grain-d2-rs"));
    }
}
