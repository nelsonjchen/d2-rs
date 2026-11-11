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

/// patternassets.GrainSVG(): the Go format template (`%[1]s` = diagram hash).
pub(crate) fn grain_svg_template() -> String {
    format!("{GRAIN_PREFIX}{GRAIN_PNG_BASE64}{GRAIN_SUFFIX}")
}

/// patternassets.PaperSVG(): the Go format template (`%[1]s` = diagram hash).
pub(crate) fn paper_svg_template() -> &'static str {
    PAPER_TEMPLATE
}

/// The pinned sketch streak path, exposed for the future rough target.
#[allow(dead_code)]
pub(crate) fn streak_path() -> &'static str {
    STREAK_PATH
}
