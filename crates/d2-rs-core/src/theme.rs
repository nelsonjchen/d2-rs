// SPDX-License-Identifier: MPL-2.0
//! The pinned D2 theme catalog.
//!
//! Theme rendering is owned by the target layer, but the catalog is shared by
//! configuration validation, CLI discovery, and future renderers. Keeping the
//! IDs and order in one source-shaped table prevents those boundaries from
//! drifting apart.

/// A catalog entry from D2's light or dark theme list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    /// Numeric ID accepted by `theme-id` or `dark-theme-id`.
    pub id: i64,
    /// Human-readable catalog name.
    pub name: &'static str,
    /// Whether the entry belongs to the dark catalog.
    pub dark: bool,
}

/// Neutral colors used by a D2 theme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Neutral {
    /// Darkest foreground color.
    pub n1: &'static str,
    /// Secondary foreground color.
    pub n2: &'static str,
    /// Tertiary foreground color.
    pub n3: &'static str,
    /// Mid light neutral.
    pub n4: &'static str,
    /// Light neutral.
    pub n5: &'static str,
    /// Very light neutral.
    pub n6: &'static str,
    /// Paper/background neutral.
    pub n7: &'static str,
}

/// Theme color palette as defined by D2 v0.9.0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    /// Neutral colors.
    pub neutrals: Neutral,
    /// Primary/base colors.
    pub b1: &'static str,
    /// Primary/base colors.
    pub b2: &'static str,
    /// Primary/base colors.
    pub b3: &'static str,
    /// Primary/base colors.
    pub b4: &'static str,
    /// Primary/base colors.
    pub b5: &'static str,
    /// Primary/base colors.
    pub b6: &'static str,
    /// Accent color.
    pub aa2: &'static str,
    /// Accent color.
    pub aa4: &'static str,
    /// Accent color.
    pub aa5: &'static str,
    /// Alternate accent color.
    pub ab4: &'static str,
    /// Alternate accent color.
    pub ab5: &'static str,
    /// Non-color rendering rules selected by the theme.
    pub special_rules: SpecialRules,
}

/// Theme rules that alter shape and text rendering beyond colors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SpecialRules {
    /// Use monospace text.
    pub mono: bool,
    /// Disable corner radii.
    pub no_corner_radius: bool,
    /// Double the outer container border.
    pub outer_container_double_border: bool,
    /// Draw container dots.
    pub container_dots: bool,
    /// Uppercase text where D2 applies the caps-lock rule.
    pub caps_lock: bool,
    /// Use C4-specific shape rules.
    pub c4: bool,
    /// Render all objects as paper.
    pub all_paper: bool,
}

/// Light themes in the exact order used by D2 v0.9.0's CLI catalog.
pub const LIGHT_CATALOG: &[Theme] = &[
    Theme {
        id: 0,
        name: "Neutral Default",
        dark: false,
    },
    Theme {
        id: 1,
        name: "Neutral Grey",
        dark: false,
    },
    Theme {
        id: 3,
        name: "Flagship Terrastruct",
        dark: false,
    },
    Theme {
        id: 4,
        name: "Cool Classics",
        dark: false,
    },
    Theme {
        id: 5,
        name: "Mixed Berry Blue",
        dark: false,
    },
    Theme {
        id: 6,
        name: "Grape Soda",
        dark: false,
    },
    Theme {
        id: 7,
        name: "Aubergine",
        dark: false,
    },
    Theme {
        id: 8,
        name: "Colorblind Clear",
        dark: false,
    },
    Theme {
        id: 100,
        name: "Vanilla Nitro Cola",
        dark: false,
    },
    Theme {
        id: 101,
        name: "Orange Creamsicle",
        dark: false,
    },
    Theme {
        id: 102,
        name: "Shirley Temple",
        dark: false,
    },
    Theme {
        id: 103,
        name: "Earth Tones",
        dark: false,
    },
    Theme {
        id: 104,
        name: "Everglade Green",
        dark: false,
    },
    Theme {
        id: 105,
        name: "Buttered Toast",
        dark: false,
    },
    Theme {
        id: 300,
        name: "Terminal",
        dark: false,
    },
    Theme {
        id: 301,
        name: "Terminal Grayscale",
        dark: false,
    },
    Theme {
        id: 302,
        name: "Origami",
        dark: false,
    },
    Theme {
        id: 303,
        name: "C4",
        dark: false,
    },
];

/// Dark themes in the exact order used by D2 v0.9.0's CLI catalog.
pub const DARK_CATALOG: &[Theme] = &[
    Theme {
        id: 200,
        name: "Dark Mauve",
        dark: true,
    },
    Theme {
        id: 201,
        name: "Dark Flagship Terrastruct",
        dark: true,
    },
];

/// Returns the complete pinned palette for a theme ID.
///
/// The catalog IDs are intentionally kept separate from the palette data so
/// callers that only need discovery do not pay for a second representation.
/// This table is copied from D2 v0.9.0's `d2themescatalog` package and uses
/// static strings, making palette lookup allocation-free in render hot paths.
#[must_use]
pub fn palette(id: i64) -> Option<Palette> {
    Some(match id {
        0 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#0D32B2",
            b2: "#0D32B2",
            b3: "#E3E9FD",
            b4: "#E3E9FD",
            b5: "#EDF0FD",
            b6: "#F7F8FE",
            aa2: "#4A6FF3",
            aa4: "#EDF0FD",
            aa5: "#F7F8FE",
            ab4: "#EDF0FD",
            ab5: "#F7F8FE",
            special_rules: SpecialRules::default(),
        },
        1 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#0A0F25",
            b2: "#676C7E",
            b3: "#9499AB",
            b4: "#CFD2DD",
            b5: "#DEE1EB",
            b6: "#EEF1F8",
            aa2: "#676C7E",
            aa4: "#CFD2DD",
            aa5: "#DEE1EB",
            ab4: "#CFD2DD",
            ab5: "#DEE1EB",
            special_rules: SpecialRules::default(),
        },
        3 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#000E3D",
            b2: "#234CDA",
            b3: "#6B8AFB",
            b4: "#A6B8F8",
            b5: "#D2DBFD",
            b6: "#E7EAFF",
            aa2: "#5829DC",
            aa4: "#B4AEF8",
            aa5: "#E4DBFF",
            ab4: "#7FDBF8",
            ab5: "#C3F0FF",
            special_rules: SpecialRules::default(),
        },
        4 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#000536",
            b2: "#0F66B7",
            b3: "#4393DD",
            b4: "#87BFF3",
            b5: "#BCDDFB",
            b6: "#E5F3FF",
            aa2: "#076F6F",
            aa4: "#77DEDE",
            aa5: "#C3F8F8",
            ab4: "#C1A2F3",
            ab5: "#DACEFB",
            special_rules: SpecialRules::default(),
        },
        5 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#000536",
            b2: "#0F66B7",
            b3: "#4393DD",
            b4: "#87BFF3",
            b5: "#BCDDFB",
            b6: "#E5F3FF",
            aa2: "#7639C5",
            aa4: "#C1A2F3",
            aa5: "#DACEFB",
            ab4: "#EA99C6",
            ab5: "#FFDEF1",
            special_rules: SpecialRules::default(),
        },
        6 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#170034",
            b2: "#7639C5",
            b3: "#8F70D1",
            b4: "#C1A2F3",
            b5: "#DACEFB",
            b6: "#F2EDFF",
            aa2: "#0F66B7",
            aa4: "#87BFF3",
            aa5: "#BCDDFB",
            ab4: "#EA99C6",
            ab5: "#FFDAEF",
            special_rules: SpecialRules::default(),
        },
        7 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#170034",
            b2: "#7639C5",
            b3: "#8F70D1",
            b4: "#D0B9F5",
            b5: "#E7DEFF",
            b6: "#F4F0FF",
            aa2: "#0F66B7",
            aa4: "#87BFF3",
            aa5: "#BCDDFB",
            ab4: "#92E3E3",
            ab5: "#D7F5F5",
            special_rules: SpecialRules::default(),
        },
        8 => Palette {
            neutrals: Neutral {
                n1: "#0A0F25",
                n2: "#676C7E",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#DEE1EB",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#010E31",
            b2: "#173688",
            b3: "#5679D4",
            b4: "#84A1EC",
            b5: "#C8D6F9",
            b6: "#E5EDFF",
            aa2: "#048E63",
            aa4: "#A6E2D0",
            aa5: "#CAF2E6",
            ab4: "#FFDA90",
            ab5: "#FFF0D1",
            special_rules: SpecialRules::default(),
        },
        100 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#535152",
                n3: "#787777",
                n4: "#CCCACA",
                n5: "#DFDCDC",
                n6: "#ECEBEB",
                n7: "#FFFFFF",
            },
            b1: "#1E1303",
            b2: "#55452F",
            b3: "#9A876C",
            b4: "#C9B9A1",
            b5: "#E9DBCA",
            b6: "#FAF1E6",
            aa2: "#D35F0A",
            aa4: "#FABA8A",
            aa5: "#FFE0C7",
            ab4: "#84A1EC",
            ab5: "#D5E0FD",
            special_rules: SpecialRules::default(),
        },
        101 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#535152",
                n3: "#787777",
                n4: "#CCCACA",
                n5: "#DFDCDC",
                n6: "#ECEBEB",
                n7: "#FFFFFF",
            },
            b1: "#311602",
            b2: "#D35F0A",
            b3: "#F18F47",
            b4: "#FABA8A",
            b5: "#FFE0C7",
            b6: "#FFF6EF",
            aa2: "#13A477",
            aa4: "#A6E2D0",
            aa5: "#CAF2E6",
            ab4: "#FEEC8C",
            ab5: "#FFF8CF",
            special_rules: SpecialRules::default(),
        },
        102 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#535152",
                n3: "#787777",
                n4: "#CCCACA",
                n5: "#DFDCDC",
                n6: "#ECEBEB",
                n7: "#FFFFFF",
            },
            b1: "#31021D",
            b2: "#9B1A48",
            b3: "#D2517F",
            b4: "#EA99B6",
            b5: "#FFDAE7",
            b6: "#FCEDF2",
            aa2: "#D35F0A",
            aa4: "#FABA8A",
            aa5: "#FFE0C7",
            ab4: "#FFE767",
            ab5: "#FFF2AA",
            special_rules: SpecialRules::default(),
        },
        103 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#535152",
                n3: "#787777",
                n4: "#CCCACA",
                n5: "#DFDCDC",
                n6: "#ECEBEB",
                n7: "#FFFFFF",
            },
            b1: "#1E1303",
            b2: "#55452F",
            b3: "#9A876C",
            b4: "#C9B9A1",
            b5: "#E9DBCA",
            b6: "#FAF1E6",
            aa2: "#D35F0A",
            aa4: "#FABA8A",
            aa5: "#FFE0C7",
            ab4: "#FFE767",
            ab5: "#FFF2AA",
            special_rules: SpecialRules::default(),
        },
        104 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#535152",
                n3: "#787777",
                n4: "#CCCACA",
                n5: "#DFDCDC",
                n6: "#ECEBEB",
                n7: "#FFFFFF",
            },
            b1: "#023324",
            b2: "#048E63",
            b3: "#49BC99",
            b4: "#A6E2D0",
            b5: "#CAF2E6",
            b6: "#EBFDF7",
            aa2: "#D35F0A",
            aa4: "#FABA8A",
            aa5: "#FFE0C7",
            ab4: "#C9B9A1",
            ab5: "#E9DBCA",
            special_rules: SpecialRules::default(),
        },
        105 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#535152",
                n3: "#787777",
                n4: "#CCCACA",
                n5: "#DFDCDC",
                n6: "#ECEBEB",
                n7: "#FFFFFF",
            },
            b1: "#312102",
            b2: "#DF9C18",
            b3: "#FDC659",
            b4: "#FFDA90",
            b5: "#FFF0D1",
            b6: "#FFF7E7",
            aa2: "#55452F",
            aa4: "#C9B9A1",
            aa5: "#E9DBCA",
            ab4: "#FABA8A",
            ab5: "#FFE0C7",
            special_rules: SpecialRules::default(),
        },
        200 => Palette {
            neutrals: Neutral {
                n1: "#CDD6F4",
                n2: "#BAC2DE",
                n3: "#A6ADC8",
                n4: "#585B70",
                n5: "#45475A",
                n6: "#313244",
                n7: "#1E1E2E",
            },
            b1: "#CBA6f7",
            b2: "#CBA6f7",
            b3: "#6C7086",
            b4: "#585B70",
            b5: "#45475A",
            b6: "#313244",
            aa2: "#f38BA8",
            aa4: "#45475A",
            aa5: "#313244",
            ab4: "#45475A",
            ab5: "#313244",
            special_rules: SpecialRules::default(),
        },
        201 => Palette {
            neutrals: Neutral {
                n1: "#F4F6FA",
                n2: "#BBBEC9",
                n3: "#868A96",
                n4: "#676D7D",
                n5: "#3A3D49",
                n6: "#191C28",
                n7: "#000410",
            },
            b1: "#F4F6FA",
            b2: "#6B8AFB",
            b3: "#3733E9",
            b4: "#070B67",
            b5: "#0B1197",
            b6: "#3733E9",
            aa2: "#8B5DEE",
            aa4: "#4918B1",
            aa5: "#7240DD",
            ab4: "#00607C",
            ab5: "#01799D",
            special_rules: SpecialRules::default(),
        },
        300 => Palette {
            neutrals: Neutral {
                n1: "#000410",
                n2: "#0000B8",
                n3: "#9499AB",
                n4: "#CFD2DD",
                n5: "#C3DEF3",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#000410",
            b2: "#0000E4",
            b3: "#5AA4DC",
            b4: "#E7E9EE",
            b5: "#F5F6F9",
            b6: "#FFFFFF",
            aa2: "#008566",
            aa4: "#45BBA5",
            aa5: "#7ACCBD",
            ab4: "#F1C759",
            ab5: "#F9E088",
            special_rules: SpecialRules {
                mono: true,
                no_corner_radius: true,
                outer_container_double_border: true,
                container_dots: true,
                caps_lock: true,
                ..SpecialRules::default()
            },
        },
        301 => Palette {
            neutrals: Neutral {
                n1: "#000410",
                n2: "#000410",
                n3: "#9499AB",
                n4: "#FFFFFF",
                n5: "#FFFFFF",
                n6: "#EEF1F8",
                n7: "#FFFFFF",
            },
            b1: "#000410",
            b2: "#000410",
            b3: "#FFFFFF",
            b4: "#E7E9EE",
            b5: "#F5F6F9",
            b6: "#FFFFFF",
            aa2: "#6D7284",
            aa4: "#F5F6F9",
            aa5: "#FFFFFF",
            ab4: "#F5F6F9",
            ab5: "#FFFFFF",
            special_rules: SpecialRules {
                mono: true,
                no_corner_radius: true,
                outer_container_double_border: true,
                container_dots: true,
                caps_lock: true,
                ..SpecialRules::default()
            },
        },
        302 => Palette {
            neutrals: Neutral {
                n1: "#170206",
                n2: "#6F0019",
                n3: "#FFFFFF",
                n4: "#E07088",
                n5: "#D2B098",
                n6: "#FFFFFF",
                n7: "#FFFFFF",
            },
            b1: "#170206",
            b2: "#A62543",
            b3: "#E07088",
            b4: "#F3E0D2",
            b5: "#FAF1E6",
            b6: "#FFFBF8",
            aa2: "#0A4EA6",
            aa4: "#3182CD",
            aa5: "#68A8E4",
            ab4: "#E07088",
            ab5: "#F19CAE",
            special_rules: SpecialRules {
                no_corner_radius: true,
                outer_container_double_border: true,
                all_paper: true,
                ..SpecialRules::default()
            },
        },
        303 => Palette {
            neutrals: Neutral {
                n1: "#0f5eaa",
                n2: "#707070",
                n3: "#FFFFFF",
                n4: "#073b6f",
                n5: "#999999",
                n6: "#FFFFFF",
                n7: "#FFFFFF",
            },
            b1: "#073b6f",
            b2: "#08427b",
            b3: "#3c7fc0",
            b4: "#438dd5",
            b5: "#8a8a8a",
            b6: "#999999",
            aa2: "#0f5eaa",
            aa4: "#707070",
            aa5: "#f5f5f5",
            ab4: "#e1e1e1",
            ab5: "#f0f0f0",
            special_rules: SpecialRules {
                c4: true,
                ..SpecialRules::default()
            },
        },
        _ => return None,
    })
}

/// Resolves a D2 theme color code to its palette value.
#[must_use]
pub fn resolve_color(palette: Palette, code: &str) -> Option<&'static str> {
    Some(match code {
        "N1" => palette.neutrals.n1,
        "N2" => palette.neutrals.n2,
        "N3" => palette.neutrals.n3,
        "N4" => palette.neutrals.n4,
        "N5" => palette.neutrals.n5,
        "N6" => palette.neutrals.n6,
        "N7" => palette.neutrals.n7,
        "B1" => palette.b1,
        "B2" => palette.b2,
        "B3" => palette.b3,
        "B4" => palette.b4,
        "B5" => palette.b5,
        "B6" => palette.b6,
        "AA2" => palette.aa2,
        "AA4" => palette.aa4,
        "AA5" => palette.aa5,
        "AB4" => palette.ab4,
        "AB5" => palette.ab5,
        _ => return None,
    })
}

/// Finds a theme by its numeric ID across both catalogs.
#[must_use]
pub fn find(id: i64) -> Option<Theme> {
    LIGHT_CATALOG
        .iter()
        .chain(DARK_CATALOG)
        .copied()
        .find(|theme| theme.id == id)
}

/// Formats the catalog exactly as D2's `themes` command does.
#[must_use]
pub fn catalog_text() -> String {
    let mut output = String::from("Available themes:\nLight:\n");
    for theme in LIGHT_CATALOG {
        output.push_str(&format!("- {}: {}\n", theme.name, theme.id));
    }
    output.push_str("Dark:\n");
    for theme in DARK_CATALOG {
        output.push_str(&format!("- {}: {}\n", theme.name, theme.id));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{DARK_CATALOG, LIGHT_CATALOG, catalog_text, find, palette, resolve_color};

    #[test]
    fn catalog_ids_and_order_match_v090() {
        let light_ids: Vec<_> = LIGHT_CATALOG.iter().map(|theme| theme.id).collect();
        assert_eq!(
            light_ids,
            [
                0, 1, 3, 4, 5, 6, 7, 8, 100, 101, 102, 103, 104, 105, 300, 301, 302, 303
            ]
        );
        let dark_ids: Vec<_> = DARK_CATALOG.iter().map(|theme| theme.id).collect();
        assert_eq!(dark_ids, [200, 201]);
        assert_eq!(find(303).map(|theme| theme.name), Some("C4"));
        assert_eq!(find(999), None);
    }

    #[test]
    fn catalog_text_is_stable() {
        let text = catalog_text();
        assert!(text.starts_with("Available themes:\nLight:\n- Neutral Default: 0\n"));
        assert!(text.ends_with("- Dark Flagship Terrastruct: 201\n"));
    }

    #[test]
    fn palettes_match_default_and_special_theme_rules() {
        let neutral = palette(0).expect("default palette");
        assert_eq!(resolve_color(neutral, "B1"), Some("#0D32B2"));
        assert_eq!(resolve_color(neutral, "N7"), Some("#FFFFFF"));
        assert!(palette(300).expect("terminal palette").special_rules.mono);
        assert!(palette(999).is_none());
    }
}
