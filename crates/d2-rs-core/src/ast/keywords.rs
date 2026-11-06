// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 language keyword inventories, translated from d2ast/keywords.go.
//! Slice order is significant only for the upstream arrays, not keyword sets.

use crate::LabelPosition;

/// Pinned upstream simple reserved keywords.
pub const SIMPLE_RESERVED_KEYWORDS: &[&str] = &[
    "label",
    "shape",
    "icon",
    "constraint",
    "tooltip",
    "link",
    "near",
    "width",
    "height",
    "direction",
    "top",
    "left",
    "grid-rows",
    "grid-columns",
    "grid-gap",
    "vertical-gap",
    "horizontal-gap",
    "class",
    "vars",
];

/// Pinned upstream reserved keyword holders.
pub const RESERVED_KEYWORD_HOLDERS: &[&str] = &["style"];

/// Pinned upstream composite reserved keywords.
pub const COMPOSITE_RESERVED_KEYWORDS: &[&str] = &[
    "source-arrowhead",
    "target-arrowhead",
    "classes",
    "constraint",
    "label",
    "icon",
    "tooltip",
    "style",
    "layers",
    "scenarios",
    "steps",
];

/// Pinned upstream style keywords.
pub const STYLE_KEYWORDS: &[&str] = &[
    "opacity",
    "stroke",
    "fill",
    "fill-pattern",
    "stroke-width",
    "stroke-dash",
    "border-radius",
    "font",
    "font-size",
    "font-color",
    "bold",
    "italic",
    "underline",
    "text-transform",
    "shadow",
    "multiple",
    "double-border",
    "3d",
    "animated",
    "filled",
];

/// Pinned upstream board keywords.
pub const BOARD_KEYWORDS: &[&str] = &["layers", "scenarios", "steps"];

/// Pinned upstream near constants.
pub const NEAR_CONSTANTS: &[&str] = &[
    "top-left",
    "top-center",
    "top-right",
    "center-left",
    "center-right",
    "bottom-left",
    "bottom-center",
    "bottom-right",
];

/// Pinned upstream label positions.
pub const LABEL_POSITIONS: &[&str] = &[
    "top-left",
    "top-center",
    "top-right",
    "center-left",
    "center-center",
    "center-right",
    "bottom-left",
    "bottom-center",
    "bottom-right",
    "outside-top-left",
    "outside-top-center",
    "outside-top-right",
    "outside-left-top",
    "outside-left-center",
    "outside-left-bottom",
    "outside-right-top",
    "outside-right-center",
    "outside-right-bottom",
    "outside-bottom-left",
    "outside-bottom-center",
    "outside-bottom-right",
    "border-top-left",
    "border-top-center",
    "border-top-right",
    "border-left-top",
    "border-left-center",
    "border-left-bottom",
    "border-right-top",
    "border-right-center",
    "border-right-bottom",
    "border-bottom-left",
    "border-bottom-center",
    "border-bottom-right",
];

/// Pinned upstream tooltip positions.
pub const TOOLTIP_POSITIONS: &[&str] = &[
    "top-left",
    "top-center",
    "top-right",
    "center-left",
    "center-right",
    "bottom-left",
    "bottom-center",
    "bottom-right",
];

/// Pinned upstream fill patterns.
pub const FILL_PATTERNS: &[&str] = &["none", "dots", "lines", "grain", "paper"];

/// Pinned upstream text transforms.
pub const TEXT_TRANSFORMS: &[&str] = &["none", "uppercase", "lowercase", "capitalize"];

/// Whether a name belongs to the complete reserved-keyword union.
/// Matching is exact; callers decide when language semantics require case folding.
pub fn is_reserved_keyword(value: &str) -> bool {
    SIMPLE_RESERVED_KEYWORDS.contains(&value)
        || STYLE_KEYWORDS.contains(&value)
        || COMPOSITE_RESERVED_KEYWORDS.contains(&value)
}

/// Convert a language label/icon `near` value to its layout position.
/// An unknown value is absent, as with a failed upstream map lookup.
pub fn label_position(value: &str) -> Option<LabelPosition> {
    match value {
        "top-left" => Some(LabelPosition::InsideTopLeft),
        "top-center" => Some(LabelPosition::InsideTopCenter),
        "top-right" => Some(LabelPosition::InsideTopRight),
        "center-left" => Some(LabelPosition::InsideMiddleLeft),
        "center-center" => Some(LabelPosition::InsideMiddleCenter),
        "center-right" => Some(LabelPosition::InsideMiddleRight),
        "bottom-left" => Some(LabelPosition::InsideBottomLeft),
        "bottom-center" => Some(LabelPosition::InsideBottomCenter),
        "bottom-right" => Some(LabelPosition::InsideBottomRight),
        "outside-top-left" => Some(LabelPosition::OutsideTopLeft),
        "outside-top-center" => Some(LabelPosition::OutsideTopCenter),
        "outside-top-right" => Some(LabelPosition::OutsideTopRight),
        "outside-left-top" => Some(LabelPosition::OutsideLeftTop),
        "outside-left-center" => Some(LabelPosition::OutsideLeftMiddle),
        "outside-left-bottom" => Some(LabelPosition::OutsideLeftBottom),
        "outside-right-top" => Some(LabelPosition::OutsideRightTop),
        "outside-right-center" => Some(LabelPosition::OutsideRightMiddle),
        "outside-right-bottom" => Some(LabelPosition::OutsideRightBottom),
        "outside-bottom-left" => Some(LabelPosition::OutsideBottomLeft),
        "outside-bottom-center" => Some(LabelPosition::OutsideBottomCenter),
        "outside-bottom-right" => Some(LabelPosition::OutsideBottomRight),
        "border-top-left" => Some(LabelPosition::BorderTopLeft),
        "border-top-center" => Some(LabelPosition::BorderTopCenter),
        "border-top-right" => Some(LabelPosition::BorderTopRight),
        "border-left-top" => Some(LabelPosition::BorderLeftTop),
        "border-left-center" => Some(LabelPosition::BorderLeftMiddle),
        "border-left-bottom" => Some(LabelPosition::BorderLeftBottom),
        "border-right-top" => Some(LabelPosition::BorderRightTop),
        "border-right-center" => Some(LabelPosition::BorderRightMiddle),
        "border-right-bottom" => Some(LabelPosition::BorderRightBottom),
        "border-bottom-left" => Some(LabelPosition::BorderBottomLeft),
        "border-bottom-center" => Some(LabelPosition::BorderBottomCenter),
        "border-bottom-right" => Some(LabelPosition::BorderBottomRight),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_and_holder_names_join_the_reserved_composites() {
        for name in ["layers", "scenarios", "steps", "style"] {
            assert!(COMPOSITE_RESERVED_KEYWORDS.contains(&name));
            assert!(is_reserved_keyword(name));
            assert!(!SIMPLE_RESERVED_KEYWORDS.contains(&name));
        }
        assert!(is_reserved_keyword("stroke"));
        assert!(!COMPOSITE_RESERVED_KEYWORDS.contains(&"stroke"));
        assert!(!is_reserved_keyword("Stroke"));
        assert!(!is_reserved_keyword("custom"));
    }

    #[test]
    fn placement_vocabularies_keep_their_distinct_boundaries() {
        assert!(LABEL_POSITIONS.contains(&"center-center"));
        assert!(!NEAR_CONSTANTS.contains(&"center-center"));
        assert!(!TOOLTIP_POSITIONS.contains(&"center-center"));
        assert_eq!(
            label_position("outside-left-center"),
            Some(LabelPosition::OutsideLeftMiddle)
        );
        assert_eq!(
            label_position("border-right-center"),
            Some(LabelPosition::BorderRightMiddle)
        );
        assert_eq!(label_position("left-center"), None);
        assert_eq!(label_position("OUTSIDE_LEFT_MIDDLE"), None);
        assert_eq!(label_position("outside-left-middle"), None);
    }
}
