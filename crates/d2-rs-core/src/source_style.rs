// SPDX-License-Identifier: MPL-2.0
//! Typed source-target styles shared by compiler and renderers.
use crate::{Size, ast};
use serde::Serialize;

/// Original d2target.Text defaults in an SQL column. Row font/face are
/// measurement inputs, not exported text styles; only label dimensions change.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSqlText {
    pub label: String,
    pub font_size: u32,
    pub font_family: String,
    pub language: String,
    pub color: String,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    pub label_width: u32,
    pub label_height: u32,
}

impl SourceSqlText {
    pub fn measured(label: String, size: Size) -> Self {
        Self {
            label,
            label_width: size.width.ceil().max(0.0) as u32,
            label_height: size.height.ceil().max(0.0) as u32,
            ..Self::default()
        }
    }
}

/// Source-order d2target.SQLColumn, retaining raw constraints and nil slices.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SourceSqlColumn {
    pub name: SourceSqlText,
    #[serde(rename = "type")]
    pub type_text: SourceSqlText,
    pub constraint: Option<Vec<String>>,
    pub reference: String,
}

/// Structured-shape accents assigned by original d2exporter.applyTheme.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTargetAccents {
    pub primary_accent_color: String,
    pub secondary_accent_color: String,
    pub neutral_accent_color: String,
}

impl SourceTargetAccents {
    pub fn structured() -> Self {
        Self {
            primary_accent_color: "B2".into(),
            secondary_accent_color: "AA2".into(),
            neutral_accent_color: "N2".into(),
        }
    }
}

/// First-party node style fields retained for source-target rendering.
///
/// Layout does not consult these values; they belong to the later scene/target
/// stages and are kept beside the measured graph until those stages are
/// translated.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SourceNodeStyle {
    pub fill: Option<String>,
    pub fill_pattern: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: Option<String>,
    pub opacity: Option<String>,
    pub stroke_dash: Option<String>,
    pub animated: Option<String>,
    pub border_radius: Option<String>,
    pub shadow: Option<String>,
    pub three_dee: Option<String>,
    pub multiple: Option<String>,
    pub double_border: Option<String>,
    pub font_color: Option<String>,
    /// Effective source style for ordinary labels. D2 defaults non-container
    /// labels to bold and permits explicit bold/italic overrides.
    pub bold: bool,
    /// d2exporter's Shape.Bold when it differs from the measurement face:
    /// Object.Text's default ignores Markdown/LaTeX, then applyStyles parses
    /// an explicit style.bold. `None` means the same as `bold`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_bold: Option<bool>,
    pub italic: bool,
    pub underline: bool,
    pub text_transform: Option<String>,
    pub font: Option<String>,
    pub filled: Option<String>,
    /// Explicit source font size before structured-shape header adjustment.
    pub font_size: Option<u32>,
    /// Exported structured-shape base font, before the renderer adds its header increment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_font_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_accents: Option<SourceTargetAccents>,
    /// Normalized D2 label language (`markdown`, `latex`, or a source tag).
    pub language: Option<String>,
    /// Source-order SQL-table column names for semantic table rendering.
    pub table_columns: Vec<String>,
    /// Source-order SQL-table rows: column name, declared type, constraints.
    pub table_rows: Vec<(String, String, String)>,
    /// Complete original SQL target columns, measured with the table rows.
    pub table_target_columns: Vec<SourceSqlColumn>,
    /// Widest SQL-table column name in D2 target pixels. This is an internal
    /// rendering measurement and is omitted from serialized source metadata.
    #[serde(skip)]
    pub table_max_name_width: u32,
    /// Source-order class member names and their declared types/values.
    pub class_fields: Vec<(String, String)>,
    /// Source-order class methods rendered in the lower compartment.
    pub class_methods: Vec<(String, String)>,
    /// Original class member underline roles, parallel to their row arrays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class_field_underlines: Vec<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class_method_underlines: Vec<bool>,
    pub markdown: bool,
    /// Optional hyperlink attached to the rendered node.
    pub link: Option<String>,
    /// Optional display text derived from an internal board link. `Some("")`
    /// means D2 does not add an appendix row for this link; `None` leaves the
    /// renderer's normal URL display fallback in effect.
    #[serde(skip)]
    pub pretty_link: Option<String>,
    /// Raw source PrettyLink bytes, before Go JSON replaces invalid UTF-8.
    #[serde(skip)]
    pub pretty_link_bytes: Option<ast::ByteString>,
    /// Cloud content-box aspect ratio retained before layout, as exact bits.
    #[serde(skip)]
    pub content_aspect_ratio_bits: Option<u64>,
    /// Optional tooltip rendered as an SVG title for the node hyperlink.
    pub tooltip: Option<String>,
    /// Authored tooltip.near position, retained independently of tooltip text.
    pub tooltip_position: Option<String>,
    /// Optional icon URL attached to the node.
    pub icon: Option<String>,
    /// Authored icon clipping radius, retained for d2exporter target metadata.
    pub icon_border_radius: Option<String>,
}
