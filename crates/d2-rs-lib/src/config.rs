// SPDX-License-Identifier: MPL-2.0
//! d2lib/d2.go:108–140,289–354: caller/source/default precedence.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Source palette slots. A missing palette differs from an explicitly empty one.
pub type Palette = BTreeMap<String, Option<String>>;

/// Appearance and layout configuration retained on a compiled diagram.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LibraryConfig {
    /// Sketch mode authored in source.
    pub sketch: Option<bool>,
    /// Effective true mode contributes to the appearance hash; false is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isometric: Option<bool>,
    /// Light palette identifier.
    #[serde(rename = "themeID")]
    pub theme_id: Option<i64>,
    /// Optional dark palette identifier.
    #[serde(rename = "darkThemeID")]
    pub dark_theme_id: Option<i64>,
    /// Source padding, before defaults.
    pub pad: Option<i64>,
    /// Source centering, before defaults.
    pub center: Option<bool>,
    /// Source layout, before defaults.
    pub layout_engine: Option<String>,
    /// Source light overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_overrides: Option<Palette>,
    /// Source dark overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_theme_overrides: Option<Palette>,
    /// Owned custom layout data.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, serde_json::Value>,
}

/// Caller-owned rendering options. Optional values preserve explicit false/zero.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct LibraryRenderOptions {
    /// Outer padding.
    pub pad: Option<i64>,
    /// Sketch rendering selection.
    pub sketch: Option<bool>,
    /// Isometric selection, resolved independently of sketch/font selection.
    pub isometric: Option<bool>,
    /// Center the SVG.
    pub center: Option<bool>,
    /// Light theme selection.
    #[serde(rename = "ThemeID")]
    pub theme_id: Option<i64>,
    /// Dark theme selection.
    #[serde(rename = "DarkThemeID")]
    pub dark_theme_id: Option<i64>,
    /// Caller light overrides; Some(empty) still takes precedence.
    pub theme_overrides: Option<Palette>,
    /// Caller dark overrides; Some(empty) still takes precedence.
    pub dark_theme_overrides: Option<Palette>,
    /// Output font name.
    pub font: String,
    /// Optional SVG scale.
    pub scale: Option<f64>,
    /// Master board hash.
    #[serde(rename = "MasterID")]
    pub master_id: String,
    /// XML declaration selection.
    #[serde(rename = "NoXMLTag")]
    pub no_xml_tag: Option<bool>,
    /// Hash salt.
    pub salt: Option<String>,
    /// Version attribute selection.
    pub omit_version: Option<bool>,
}

/// Configuration-facing compile options; resolver execution is a separate API.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct LibraryCompileOptions {
    /// Layout engine name.
    pub layout: Option<String>,
    /// Measurement font family.
    pub font_family: Option<String>,
}

/// Fill absent caller options from source configuration (d2lib.applyConfigs).
pub fn apply_configs(
    config: Option<&LibraryConfig>,
    compile: &mut LibraryCompileOptions,
    render: &mut LibraryRenderOptions,
) {
    let Some(config) = config else { return };
    if compile.layout.is_none() {
        compile.layout.clone_from(&config.layout_engine);
    }
    macro_rules! fill { ($($field:ident),*) => {$(
        if render.$field.is_none() { render.$field.clone_from(&config.$field); }
    )*}; }
    fill!(
        theme_id,
        dark_theme_id,
        sketch,
        isometric,
        pad,
        center,
        theme_overrides,
        dark_theme_overrides
    );
}

/// Apply upstream defaults after source precedence (d2lib.applyDefaults).
pub fn apply_defaults(compile: &mut LibraryCompileOptions, render: &mut LibraryRenderOptions) {
    compile.layout.get_or_insert_with(|| "dagre".into());
    render.theme_id.get_or_insert(0);
    render.sketch.get_or_insert(false);
    render.isometric.get_or_insert(false);
    if render.sketch == Some(true) && compile.font_family.is_none() {
        compile.font_family = Some("HandDrawn".into());
    }
    render.pad.get_or_insert(100);
    render.center.get_or_insert(false);
}

/// Resolve a library compile's options and retain only appearance-changing fields.
/// Source padding/centering/layout stay authored, while theme/sketch reflect defaults.
/// Disabled isometric never changes an existing diagram's ordinary appearance hash.
pub fn resolve_config(
    config: Option<&LibraryConfig>,
    compile: &mut LibraryCompileOptions,
    render: &mut LibraryRenderOptions,
) -> Result<LibraryConfig, String> {
    apply_configs(config, compile, render);
    let isometric = render.isometric.filter(|enabled| *enabled);
    apply_defaults(compile, render);
    if render.sketch == Some(true) && render.isometric == Some(true) {
        return Err("sketch cannot be combined with isometric rendering".into());
    }
    let mut result = config.cloned().unwrap_or_default();
    result.theme_id = render.theme_id;
    result.dark_theme_id = render.dark_theme_id;
    result.sketch = render.sketch;
    result.isometric = isometric;
    Ok(result)
}

impl From<&d2_rs_language::compiler::DiagramConfig> for LibraryConfig {
    fn from(config: &d2_rs_language::compiler::DiagramConfig) -> Self {
        fn palette(values: &BTreeMap<String, String>) -> Option<Palette> {
            if values.is_empty() {
                return None;
            }
            let mut result: Palette = "n1 n2 n3 n4 n5 n6 n7 b1 b2 b3 b4 b5 b6 aa2 aa4 aa5 ab4 ab5"
                .split_whitespace()
                .map(|key| (key.to_owned(), None))
                .collect();
            for (key, value) in values {
                result.insert(key.to_lowercase(), Some(value.clone()));
            }
            Some(result)
        }
        Self {
            sketch: config.sketch,
            isometric: config.isometric,
            theme_id: config.theme_id,
            dark_theme_id: config.dark_theme_id,
            pad: config.pad,
            center: config.center,
            layout_engine: config.layout_engine.clone(),
            theme_overrides: palette(&config.theme_overrides),
            dark_theme_overrides: palette(&config.dark_theme_overrides),
            data: config.data.clone(),
        }
    }
}
