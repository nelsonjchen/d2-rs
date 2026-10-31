// SPDX-License-Identifier: MPL-2.0
//! d2themescatalog.Find + Theme.ApplyOverrides + d2themes.ResolveThemeColor,
//! over the shared catalog in d2-rs-core.

use crate::target::ThemeOverrides;
use d2_rs_core::theme::{self, SpecialRules};

/// Theme color codes in d2themes.ResolveThemeColor order.
const CODES: [&str; 18] = [
    "N1", "N2", "N3", "N4", "N5", "N6", "N7", "B1", "B2", "B3", "B4", "B5", "B6", "AA2", "AA4",
    "AA5", "AB4", "AB5",
];

/// lib/color.IsThemeColor: `^(N[1-7]|B[1-6]|AA[245]|AB[45])$`.
pub fn is_theme_color(value: &str) -> bool {
    CODES.contains(&value)
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: i64,
    pub name: &'static str,
    pub dark: bool,
    pub special_rules: SpecialRules,
    colors: [String; 18],
}

impl Theme {
    /// d2themescatalog.Find; None for an unknown ID (Go returns the zero Theme).
    pub fn find(id: i64) -> Option<Self> {
        let entry = theme::find(id)?;
        let palette = theme::palette(id)?;
        Some(Self {
            id,
            name: entry.name,
            dark: entry.dark,
            special_rules: palette.special_rules,
            colors: CODES.map(|code| {
                theme::resolve_color(palette, code)
                    .expect("catalog code")
                    .to_owned()
            }),
        })
    }

    pub fn apply_overrides(&mut self, overrides: Option<&ThemeOverrides>) {
        let Some(o) = overrides else { return };
        let values = [
            &o.n1, &o.n2, &o.n3, &o.n4, &o.n5, &o.n6, &o.n7, &o.b1, &o.b2, &o.b3, &o.b4, &o.b5,
            &o.b6, &o.aa2, &o.aa4, &o.aa5, &o.ab4, &o.ab5,
        ];
        for (slot, value) in self.colors.iter_mut().zip(values) {
            if let Some(value) = value {
                slot.clone_from(value);
            }
        }
    }

    /// d2themes.ResolveThemeColor: theme codes map to the palette, anything
    /// else is returned unchanged.
    pub fn resolve(&self, code: &str) -> String {
        match CODES.iter().position(|candidate| *candidate == code) {
            Some(index) => self.colors[index].clone(),
            None => code.to_owned(),
        }
    }
}
