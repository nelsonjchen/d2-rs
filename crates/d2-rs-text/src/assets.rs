// SPDX-License-Identifier: MPL-2.0
//! D2's bundled font files, shared with the renderer and exporter crates so
//! each file lives once, in this crate (see `assets/OFL-1.1.txt`).

pub const FUZZY_BUBBLES_BOLD_TTF: &[u8] = include_bytes!("../assets/FuzzyBubbles-Bold.ttf");
pub const FUZZY_BUBBLES_REGULAR_TTF: &[u8] = include_bytes!("../assets/FuzzyBubbles-Regular.ttf");
pub const SOURCE_CODE_PRO_BOLD_TTF: &[u8] = include_bytes!("../assets/SourceCodePro-Bold.ttf");
pub const SOURCE_CODE_PRO_BOLD_WOFF2: &[u8] = include_bytes!("../assets/SourceCodePro-Bold.woff2");
pub const SOURCE_CODE_PRO_ITALIC_TTF: &[u8] = include_bytes!("../assets/SourceCodePro-Italic.ttf");
pub const SOURCE_CODE_PRO_ITALIC_WOFF2: &[u8] =
    include_bytes!("../assets/SourceCodePro-Italic.woff2");
pub const SOURCE_CODE_PRO_REGULAR_TTF: &[u8] =
    include_bytes!("../assets/SourceCodePro-Regular.ttf");
pub const SOURCE_CODE_PRO_REGULAR_WOFF2: &[u8] =
    include_bytes!("../assets/SourceCodePro-Regular.woff2");
pub const SOURCE_CODE_PRO_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../assets/SourceCodePro-Semibold.ttf");
pub const SOURCE_CODE_PRO_SEMIBOLD_WOFF2: &[u8] =
    include_bytes!("../assets/SourceCodePro-Semibold.woff2");
pub const SOURCE_SANS_PRO_BOLD_TTF: &[u8] = include_bytes!("../assets/SourceSansPro-Bold.ttf");
pub const SOURCE_SANS_PRO_BOLD_WOFF2: &[u8] = include_bytes!("../assets/SourceSansPro-Bold.woff2");
pub const SOURCE_SANS_PRO_ITALIC_TTF: &[u8] = include_bytes!("../assets/SourceSansPro-Italic.ttf");
pub const SOURCE_SANS_PRO_ITALIC_WOFF2: &[u8] =
    include_bytes!("../assets/SourceSansPro-Italic.woff2");
pub const SOURCE_SANS_PRO_REGULAR_TTF: &[u8] =
    include_bytes!("../assets/SourceSansPro-Regular.ttf");
pub const SOURCE_SANS_PRO_REGULAR_WOFF2: &[u8] =
    include_bytes!("../assets/SourceSansPro-Regular.woff2");
pub const SOURCE_SANS_PRO_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../assets/SourceSansPro-Semibold.ttf");
pub const SOURCE_SANS_PRO_SEMIBOLD_WOFF2: &[u8] =
    include_bytes!("../assets/SourceSansPro-Semibold.woff2");
