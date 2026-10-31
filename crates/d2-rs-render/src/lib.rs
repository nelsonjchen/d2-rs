// SPDX-License-Identifier: MPL-2.0
//! Native SVG target for the source-to-layout vertical path.
//!
//! This is intentionally a semantic target rather than a byte-compatible
//! reproduction of D2's production renderer. It preserves the layout result,
//! labels, supported shape identity, routes, and arrows so the Rust pipeline
//! can be exercised end to end while the full scene and asset renderer is
//! translated.

mod image_mime;

/// Go net/http.DetectContentType without filename-extension precedence.
/// Shared by imageasset's malformed-resource diagnostics.
pub fn detect_image_mime(bytes: &[u8]) -> &'static str {
    image_mime::detect(bytes)
}
mod legend_connection;
mod legend_layout;
mod legend_shape;
mod markdown_flow;
pub mod markdown_scene;
mod markdown_table;
mod markdown_wrap;
pub mod target;
/// Direct d2svg document options. Mode dispatch is provided by
/// d2-rs-isometric::svg; this crate also exposes the flat-only primitive.
pub use target::d2svg::render::RenderOpts as SvgRenderOptions;

/// d2svg.ThemeCSS, including direct light and dark override admission.
pub fn svg_theme_css(hash: &str, options: &SvgRenderOptions) -> Result<String, String> {
    target::d2svg::theme_css::theme_css(
        hash,
        options.theme_id,
        options.dark_theme_id,
        options.theme_overrides.as_ref(),
        options.dark_theme_overrides.as_ref(),
    )
}
/// Pinned lib/color gradient parsing and SVG serialization.
pub use target::d2svg::color;
/// Pinned lib/svg typed markup and local-resource APIs.
pub use target::d2svg::markup as svg;
/// Pinned d2themes element and palette APIs.
pub mod themes {
    pub use crate::target::d2svg::element::{Colors, ThemableElement, Theme};
}
/// Pinned Go float semantics (math package, ARM64 FMA contraction) shared by
/// ports outside this crate, such as d2-rs-isometric.
pub mod go_float {
    pub use crate::target::d2svg::geo::gomath;
    pub use crate::target::d2svg::geo::{go_fmadd, go_fmsub, go_fnmsub};
}

use base64::Engine as _;
use d2_rs_core::theme::{Palette, resolve_color};
use d2_rs_core::{
    EdgeId, EdgeStyle, Graph, LabelPosition, Legend, Node, NodeId, Point, Rect, ShapeKind, Size,
    theme,
};
use d2_rs_language::compiler::{SourceNodeStyle, SourceSqlColumn, SourceTargetAccents};
use d2_rs_text::markdown::{
    html::{render_markdown_html as render_html, unescape_html},
    html_names::normalize_html_names,
    plain_text::plain_text,
};
use d2_rs_text::measure::{
    TrueTypeFont, bundled_source_code_pro_regular, bundled_source_sans_pro_bold,
    bundled_source_sans_pro_italic, bundled_source_sans_pro_regular,
    bundled_source_sans_pro_semibold,
};
use d2_tala_rs::LayoutResult;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};
use std::sync::OnceLock;

mod shapes;
use shapes::{
    draw_interleaved_multi_path_pattern, draw_native_three_dee_shape, draw_shape,
    draw_shape_with_id, draw_sql_table_shape_with_sketch, gradient_definition, gradient_id,
};
mod code_admission;
mod code_budget;
mod code_shape;
mod latex_admission;
pub mod markdown_measure;
mod patternassets;
mod sketch_connection;
mod sketch_shapes;
mod sketch_svg;

// D2 v0.9.0's default `--pad` is 100px. Keep the source renderer's default
// aligned with that contract. Explicit output options are carried separately
// from the layout graph so changing the canvas never changes TALA geometry.
const PADDING: f64 = 100.0;
const D2_TARGET_SHADOW_SIZE_X: f64 = 3.0;
const D2_TARGET_SHADOW_SIZE_Y: f64 = 5.0;

/// Options that affect SVG/raster output without changing layout geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    /// Pixels around the diagram on each side. D2 defaults to 100.
    pub pad: f64,
    /// Explicit output scale applied after padding. Absent scale leaves the
    /// public SVG root responsive; raster output uses a scale of one.
    pub scale: Option<f64>,
    /// Exact D2 terminal charset name; only `standard` selects basic ASCII.
    pub ascii_mode: Option<String>,
    /// Light theme ID from D2's v0.9.0 catalog.
    pub theme_id: Option<i64>,
    /// Center the SVG in its containing viewport, matching D2's `--center`.
    /// This changes only the SVG viewport alignment; layout coordinates stay
    /// untouched.
    pub center: bool,
    /// Light-theme color overrides keyed by D2 color codes (`N1`, `B1`, …).
    /// Values are emitted as supplied after palette-code resolution.
    pub theme_overrides: BTreeMap<String, String>,
    /// Optional dark-theme palette applied through a prefers-color-scheme CSS
    /// rule. When absent, the SVG remains light-only like D2's default.
    pub dark_theme_id: Option<i64>,
    /// Dark-theme color overrides keyed by D2 color codes.
    pub dark_theme_overrides: BTreeMap<String, String>,
    /// Optional salt used to isolate SVG IDs when multiple copies of the same
    /// diagram are embedded in one document.  D2 applies the salt at the
    /// target boundary; keeping the rewrite here covers every generated ID
    /// and its URL reference, including markers, masks, gradients, and
    /// pattern definitions.
    pub salt: Option<String>,
    /// Optional source asset bytes keyed by the icon URL or path from the
    /// graph. When present, image references are embedded as deterministic
    /// data URIs in SVG output. Native callers can populate this from a
    /// filesystem or network provider; WASM callers can supply browser-fetched
    /// bytes without giving the renderer filesystem access.
    pub asset_data: BTreeMap<String, Vec<u8>>,
    /// Sources for which a configured raster resolver returned ErrUnavailable.
    /// Native raster scenes share D2's placeholder; SVG bundling is independent.
    pub unavailable_images: std::collections::BTreeSet<String>,
    /// Add D2's static-export appendix for node links and tooltips.
    /// Interactive SVG output leaves this disabled unless the caller passes
    /// the v0.9.0 `--force-appendix` option.
    pub force_appendix: bool,
    /// Emit a transparent top-level canvas for PDF/PPTX page composition.
    /// D2 v0.9.0 clears each paged board's root fill before rasterizing it.
    pub transparent_background: bool,
    /// Label carried by a board's synthetic target root. Root diagrams use an
    /// empty label; child boards use their board name unless an explicit
    /// `label` overrides it. This is part of D2's Diagram.Bytes hash input.
    pub target_root_label: Option<String>,
    /// Child `d2target.Diagram` values do not carry the root Config pointer;
    /// the document root does. Keep that distinction in the hash boundary.
    pub include_config_in_hash: bool,
    /// d2lib CompileOptions.FontFamily = HandDrawn without RenderOpts.Sketch
    /// (d2svg dark_theme_test.go): the exported font family is HandDrawn while
    /// the hashed config keeps `sketch: false`. Sketch exports imply it.
    pub hand_drawn_font: bool,
    /// d2svg only: the board's complete Diagram.Bytes (itself plus its
    /// descendant boards) when it is rendered alone, so its HashID covers
    /// its children as in Go.
    pub d2svg_hash_input: Option<Vec<u8>>,
    /// Custom face bytes used by d2svg's subset embedding. They are scoped to
    /// this render rather than a process-global font registry.
    pub custom_fonts: d2_rs_text::d2fonts::CustomFonts,
}

/// A high-precision path command retained for the native raster boundary.
///
/// The SVG text target intentionally formats coordinates to six decimals, and
/// SVG path consumers commonly store those coordinates as `f32`. D2's native
/// raster path instead receives the original scene geometry as `f64`. This
/// private-to-the-render-pipeline side channel preserves connection and
/// arrowhead geometry across that serialization boundary.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RasterPathCommand {
    MoveTo(Point),
    LineTo(Point),
    CubicTo(Point, Point, Point),
    Close,
}

/// Original route geometry associated with an internal SVG path identifier.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct RasterPathSource {
    pub id: String,
    pub commands: Vec<RasterPathCommand>,
    /// Optional scene transform in `[sx, kx, ky, sy, tx, ty]` order. Native
    /// arrowheads keep their geometry local so the rasterizer can construct
    /// strokes before applying D2's scene transform.
    pub transform: Option<[f64; 6]>,
}

#[derive(Clone, Debug, PartialEq)]
struct RasterInlineArrow {
    paths: Vec<RasterInlineArrowPath>,
}

#[derive(Clone, Debug, PartialEq)]
struct RasterInlineArrowPath {
    id: String,
    path_data: String,
    commands: Vec<RasterPathCommand>,
    source_transform: Option<[f64; 6]>,
    fill: String,
    stroke: Option<String>,
    stroke_width: Option<f64>,
    stroke_style: Option<&'static str>,
}

/// Errors raised by the checked source-render path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// A source renderer dereferences a nil connection label point.
    SourceNilPointer { source_site: &'static str },
    /// A source renderer indexes an incomplete connection route.
    SourceRouteIndex { index: usize, length: usize },
    /// Source legend metadata rejected before icon construction.
    LegendMetadata {
        object: String,
        reason: &'static str,
    },
    /// Original scene rejects a zero exported-route arrow tangent.
    SceneRouteTangent {
        connection: String,
        endpoint: &'static str,
    },
    /// Original scene metadata admission or native label transport failed.
    MarkdownScene(String),
    /// D2's native Markdown SVG renderer rejects this raw HTML element.
    UnsupportedMarkdownHtml { element: String },
    /// Original scene builder rejects these effects on a synthetic root.
    UnsupportedRootEffect { effect: &'static str },
    /// Original native scene math source admission.
    LatexInput { object: String, bytes: usize },
    /// Original native scene math import rejected this generated SVG.
    LatexScene {
        object: String,
        stage: &'static str,
        error: String,
    },
    /// Original scene rejects rich labels on structured/code shapes.
    UnsupportedLabelLanguage {
        object: String,
        language: String,
        includes_code: bool,
    },
    /// Cooperative native code/render context cause.
    CodeContext { deadline: bool },
    /// Original code metric failure at a line/token location.
    CodeGlyph {
        object: String,
        line: usize,
        token: usize,
        error: String,
    },
    /// Original code collector error, with its scene owner and language.
    CodeWork {
        object: String,
        language: String,
        error: d2_rs_text::code::CodeWorkError,
    },
    /// Original scene code input admission before lexer/asset work.
    CodeInput {
        object: String,
        field: &'static str,
        value: i64,
        requirement: &'static str,
    },
    /// Original rich-label scene validation precedes asset import.
    InvalidLabelUtf8 { object: String },
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNilPointer { source_site } => fmt::Display::fmt(
                &d2_rs_core::LayoutError::D2NilPointerFault { source_site },
                formatter,
            ),
            Self::SourceRouteIndex { index, length } => write!(
                formatter,
                "panic recovered: runtime error: index out of range [{index}] with length {length}\nnative source context: d2svg.getArrowheadAdjustments:897"
            ),
            Self::LegendMetadata { object, reason } => {
                write!(formatter, "scene: {object} uses unsupported {reason}")
            }
            Self::CodeContext { deadline } => formatter.write_str(if *deadline {
                "context deadline exceeded"
            } else {
                "context canceled"
            }),
            Self::CodeGlyph {
                object,
                line,
                token,
                error,
            } => write!(
                formatter,
                "scene: {object} code token {line}:{token}: {error}"
            ),
            Self::CodeWork {
                object,
                language,
                error,
            } => write!(
                formatter,
                "scene: {object} tokenize {language} code: {error}"
            ),
            Self::CodeInput {
                object,
                field,
                value,
                requirement,
            } => write!(
                formatter,
                "scene: {object} field \"{field}\" has invalid value {value}: {requirement}"
            ),
            Self::SceneRouteTangent {
                connection,
                endpoint,
            } => write!(
                formatter,
                "scene: connection {connection:?} field \"route\" must have a non-zero {endpoint} arrowhead tangent"
            ),
            Self::MarkdownScene(error) => formatter.write_str(error),
            Self::UnsupportedLabelLanguage {
                object,
                language,
                includes_code,
            } => write!(
                formatter,
                "scene: {object} uses unsupported label language {language} on a {} shape",
                if *includes_code {
                    "structured or code"
                } else {
                    "structured"
                }
            ),
            Self::InvalidLabelUtf8 { object } => write!(
                formatter,
                "scene: {object} field \"label\" must be valid UTF-8"
            ),
            Self::LatexInput { object, bytes } => write!(
                formatter,
                "scene: {object} latex input is {bytes} bytes, exceeding limit 4096"
            ),
            Self::LatexScene {
                object,
                stage,
                error,
            } => {
                if *stage == "SVG" {
                    write!(
                        formatter,
                        "scene: {object} latex SVG: d2svgimport: {object} latex: {error}"
                    )
                } else {
                    write!(formatter, "scene: {object} latex render: {error}")
                }
            }
            Self::UnsupportedRootEffect { effect } => {
                write!(formatter, "scene: root uses unsupported {effect}")
            }
            Self::UnsupportedMarkdownHtml { element } => write!(
                formatter,
                "native Markdown SVG does not support HTML element <{element}>"
            ),
        }
    }
}

impl std::error::Error for RenderError {}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            pad: PADDING,
            scale: None,
            ascii_mode: None,
            theme_id: None,
            center: false,
            theme_overrides: BTreeMap::new(),
            dark_theme_id: None,
            dark_theme_overrides: BTreeMap::new(),
            salt: None,
            asset_data: BTreeMap::new(),
            unavailable_images: Default::default(),
            force_appendix: false,
            transparent_background: false,
            target_root_label: None,
            include_config_in_hash: true,
            hand_drawn_font: false,
            d2svg_hash_input: None,
            custom_fonts: Default::default(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PaletteView<'a> {
    pub(crate) base: Palette,
    pub(crate) overrides: &'a BTreeMap<String, String>,
}

impl<'a> PaletteView<'a> {
    fn new(base: Palette, overrides: &'a BTreeMap<String, String>) -> Self {
        Self { base, overrides }
    }

    pub(crate) fn color(&self, code: &str, fallback: &str) -> String {
        let code = code.to_ascii_uppercase();
        self.overrides
            .get(&code)
            .cloned()
            .or_else(|| resolve_color(self.base, &code).map(str::to_owned))
            .unwrap_or_else(|| fallback.to_owned())
    }

    pub(crate) fn explicit(&self, raw: &str, fallback: &str) -> String {
        let code = raw.to_ascii_uppercase();
        self.overrides
            .get(&code)
            .cloned()
            .or_else(|| resolve_color(self.base, &code).map(str::to_owned))
            .unwrap_or_else(|| {
                if raw.is_empty() {
                    fallback.to_owned()
                } else {
                    raw.to_owned()
                }
            })
    }

    pub(crate) fn semantic_code(&self, raw: Option<&str>, fallback: &str) -> Option<String> {
        let code = raw.unwrap_or(fallback).to_ascii_uppercase();
        (self.overrides.contains_key(&code) || resolve_color(self.base, &code).is_some())
            .then_some(code)
    }
}

const THEME_CODES: &[&str] = &[
    "N1", "N2", "N3", "N4", "N5", "N6", "N7", "B1", "B2", "B3", "B4", "B5", "B6", "AA2", "AA4",
    "AA5", "AB4", "AB5",
];

fn safe_css_color(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '#' | '(' | ')' | ',' | '.' | '%' | ' ' | '+' | '-'
                )
        })
}

/// Matches D2's `svg.GetStrokeDashAttributes`: the source value is a dash/gap
/// scale, while SVG receives the width-adjusted dash and gap lengths.
fn stroke_dash_values(stroke_width: &str, dash_gap: Option<&str>) -> Option<(f64, f64)> {
    let width = stroke_width.parse::<f64>().ok()?;
    let gap = dash_gap?.parse::<f64>().ok()?;
    if !width.is_finite() || !gap.is_finite() || gap <= 0.0 {
        return None;
    }
    // Keep the sign inside the affine term. Go evaluates
    // `math.Log10(-0.6*strokeWidth + 10.6)`; negating the completed term
    // would make ordinary widths non-finite and silently collapse the gap.
    // Keep the multiply and addition separate. Go's source expression is
    // `-0.6*strokeWidth + 10.6`; contracting it into an FMA changes the final
    // f64 gap even though the six-decimal SVG serialization looks identical.
    let scale = (-0.6_f64 * width + 10.6).log10() * 0.5 + 0.5;
    let dash_size = width * gap;
    let gap_size = scale * dash_size;
    (dash_size.is_finite() && gap_size.is_finite()).then_some((dash_size, gap_size))
}

fn stroke_dasharray(stroke_width: &str, dash_gap: Option<&str>) -> Option<String> {
    stroke_dash_values(stroke_width, dash_gap)
        .map(|(dash_size, gap_size)| format!("{},{}", number(dash_size), number(gap_size)))
}

fn raster_dash_attributes(dash_size: f64, gap_size: f64) -> String {
    format!(
        " data-d2-rs-dash-size-bits=\"{:016x}\" data-d2-rs-dash-gap-bits=\"{:016x}\" data-d2-rs-dash-offset-bits=\"{:016x}\"",
        dash_size.to_bits(),
        gap_size.to_bits(),
        0.0_f64.to_bits(),
    )
}

/// Match each generated dashed shape primitive to its original scene stroke.
/// A page or double border has several primitives sharing one source tuple;
/// give them distinct identities without changing existing route/path IDs.
fn tag_raster_dashed_primitive_ids(svg: &mut String) {
    if !svg.contains("data-d2-rs-dash-owner-id") {
        return;
    }
    let Ok(document) = roxmltree::Document::parse(svg) else {
        return;
    };
    let mut identities = document
        .descendants()
        .filter_map(|node| node.attribute("id"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut ordinals = BTreeMap::<String, usize>::new();
    let mut insertions = Vec::new();
    for primitive in document.descendants().filter(|node| {
        node.is_element()
            && matches!(
                node.tag_name().name(),
                "path" | "rect" | "circle" | "ellipse" | "line" | "polygon" | "polyline"
            )
            && node.attribute("stroke-dasharray").is_some()
            && node.attribute("id").is_none()
    }) {
        let Some(owner) = primitive
            .ancestors()
            .find_map(|node| node.attribute("data-d2-rs-dash-owner-id"))
        else {
            continue;
        };
        let ordinal = ordinals.entry(owner.to_owned()).or_default();
        let id = loop {
            let candidate = format!("{owner}-primitive-{ordinal}");
            *ordinal += 1;
            if identities.insert(candidate.clone()) {
                break candidate;
            }
        };
        insertions.push((
            primitive.range().start + 1 + primitive.tag_name().name().len(),
            format!(" id=\"{}\"", escape(&id)),
        ));
    }
    drop(document);
    for (position, attribute) in insertions.into_iter().rev() {
        svg.insert_str(position, &attribute);
    }
}

#[derive(Clone, Debug, Default)]
struct FontNeeds {
    sans_regular: bool,
    sans_semibold: bool,
    sans_bold: bool,
    sans_italic: bool,
    mono_regular: bool,
    mono_semibold: bool,
    mono_bold: bool,
    mono_italic: bool,
    ordinary_corpora: [String; 8],
    markdown_corpora: [String; 8],
    subset_corpus_valid: bool,
}

fn rendered_font_needs(
    graph: &Graph,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> FontNeeds {
    let mut needs = FontNeeds::default();
    for (node_id, text) in labels {
        if text.is_empty() {
            continue;
        }
        let style = styles.get(node_id);
        let node = graph.node(*node_id);
        let mono = style
            .and_then(|style| style.font.as_deref())
            .is_some_and(|font| font.eq_ignore_ascii_case("mono"))
            || node.is_some_and(|node| matches!(node.shape, ShapeKind::Code | ShapeKind::Class));
        let markdown = style.is_some_and(|style| style.markdown);
        needs.sans_regular = true;
        if mono {
            needs.mono_regular = true;
            if style.is_some_and(|style| style.bold) {
                needs.mono_bold = true;
            }
            if style.is_some_and(|style| style.italic) {
                needs.mono_italic = true;
            }
            if markdown && text.contains('`') {
                needs.mono_regular = true;
            }
        } else if markdown {
            // Markdown can introduce any inline face after parsing (including
            // semibold headings and code spans), so retain the complete Sans
            // set and add Code only when a code delimiter is present.
            needs.sans_semibold = true;
            needs.sans_bold = true;
            needs.sans_italic = true;
            if text.contains('`') {
                needs.mono_regular = true;
                needs.mono_bold = true;
                needs.mono_italic = true;
            }
        } else {
            if style.is_some_and(|style| style.bold) {
                needs.sans_bold = true;
            }
            if style.is_some_and(|style| style.italic) {
                needs.sans_italic = true;
            }
        }
    }
    for (edge_id, _) in graph.edges() {
        let Some(_label) = graph
            .edge_label(edge_id)
            .filter(|label| !label.text.is_empty())
        else {
            continue;
        };
        let style = graph.edge_style(edge_id);
        let mono = style
            .font
            .as_deref()
            .is_some_and(|font| font.eq_ignore_ascii_case("mono"));
        needs.sans_regular = true;
        if mono {
            needs.mono_regular = true;
            // Connection labels use D2's italic face by default, including
            // labels whose font family is explicitly mono.
            needs.mono_italic = true;
            if graph.edge_label_bold(edge_id) {
                needs.mono_bold = true;
            }
        } else if graph.edge_label_bold(edge_id) {
            needs.sans_bold = true;
            needs.sans_italic = true;
        } else {
            needs.sans_italic = true;
        }
    }
    for (edge_id, _) in graph.edges() {
        let labels = graph.edge_arrowhead_labels(edge_id);
        if labels
            .source
            .as_ref()
            .is_some_and(|label| !label.text.is_empty())
            || labels
                .target
                .as_ref()
                .is_some_and(|label| !label.text.is_empty())
        {
            needs.sans_italic = true;
        }
    }
    if graph.legend.as_ref().is_some_and(|legend| {
        !legend.label.is_empty()
            || legend.shapes.iter().any(|shape| !shape.label.is_empty())
            || legend
                .connections
                .iter()
                .any(|connection| !connection.label.is_empty())
    }) {
        needs.sans_regular = true;
    }
    needs
}

#[derive(Clone, Copy, Debug, Default)]
struct SvgFontSelection {
    mono: bool,
    semibold: bool,
    bold: bool,
    italic: bool,
    markdown: bool,
    markdown_face: Option<usize>,
}

fn font_needs_from_svg(svg: &str, mut fallback: FontNeeds) -> FontNeeds {
    let Ok(document) = roxmltree::Document::parse(svg) else {
        fallback.subset_corpus_valid = false;
        return fallback;
    };
    fallback.ordinary_corpora = std::array::from_fn(|_| String::new());
    fallback.markdown_corpora = std::array::from_fn(|_| String::new());
    fallback.subset_corpus_valid = true;
    for text in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "text")
    {
        let selection = svg_font_selection(text, SvgFontSelection::default());
        for child in text.children() {
            collect_svg_font_text(child, selection, &mut fallback);
        }
    }
    fallback
}

fn collect_svg_font_text(
    node: roxmltree::Node<'_, '_>,
    inherited: SvgFontSelection,
    needs: &mut FontNeeds,
) {
    if node.is_text() {
        if let Some(text) = node.text() {
            let face = svg_font_face_index(inherited);
            let corpora = if inherited.markdown {
                &mut needs.markdown_corpora
            } else {
                &mut needs.ordinary_corpora
            };
            corpora[face].push_str(text);
        }
        return;
    }
    if !node.is_element()
        || matches!(
            node.tag_name().name(),
            "style" | "script" | "title" | "desc" | "metadata"
        )
    {
        return;
    }
    let selection = svg_font_selection(node, inherited);
    for child in node.children() {
        collect_svg_font_text(child, selection, needs);
    }
}

fn svg_font_selection(
    node: roxmltree::Node<'_, '_>,
    mut selection: SvgFontSelection,
) -> SvgFontSelection {
    let classes = node
        .attribute("class")
        .unwrap_or_default()
        .split_ascii_whitespace()
        .collect::<BTreeSet<_>>();
    let family = node.attribute("font-family").unwrap_or_default();
    if !family.is_empty() {
        selection.mono = family.contains("Code Pro") || family.contains("font-mono");
        if family.ends_with("-semibold") || family.ends_with("-mono-semibold") {
            selection.semibold = true;
            selection.bold = false;
        } else if family.ends_with("-bold") || family.ends_with("-mono-bold") {
            selection.bold = true;
            selection.semibold = false;
        } else if family.ends_with("-italic") || family.ends_with("-mono-italic") {
            selection.italic = true;
        }
        selection.markdown |= family.starts_with("d2-rs-font-");
        if let Some(face) = markdown_font_family_index(family) {
            selection.markdown_face = Some(face);
        }
    }
    if classes.contains("text-mono")
        || classes.contains("text-mono-semibold")
        || classes.contains("text-mono-bold")
        || classes.contains("text-mono-italic")
    {
        selection.mono = true;
    }
    if classes.contains("text-semibold") || classes.contains("text-mono-semibold") {
        selection.semibold = true;
        selection.bold = false;
    }
    if classes.contains("text-bold") || classes.contains("text-mono-bold") {
        selection.bold = true;
        selection.semibold = false;
    }
    if classes.contains("text-italic") || classes.contains("text-mono-italic") {
        selection.italic = true;
    }
    if classes.contains("md-text") {
        selection.markdown = true;
    }
    // Markdown's bold/semibold/italic classes choose a concrete embedded face;
    // `font-weight` and `font-style` attributes can then synthesize a second
    // emphasis axis on that face. Follow the CSS family selector here instead
    // of mistaking synthetic italic on `text-bold` for the separate italic
    // family and omitting glyphs from the bold subset.
    for (class, face) in [
        ("text-semibold", 1),
        ("text-bold", 2),
        ("text-italic", 3),
        ("text-mono", 4),
        ("text-mono-semibold", 5),
        ("text-mono-bold", 6),
        ("text-mono-italic", 7),
    ] {
        if classes.contains(class) {
            selection.markdown_face = Some(face);
        }
    }
    if let Some(weight) = node.attribute("font-weight") {
        match weight.to_ascii_lowercase().as_str() {
            "600" | "semibold" => {
                selection.semibold = true;
                selection.bold = false;
            }
            "700" | "bold" => {
                selection.bold = true;
                selection.semibold = false;
            }
            "400" | "normal" => {
                selection.semibold = false;
                selection.bold = false;
            }
            _ => {}
        }
    }
    if let Some(style) = node.attribute("font-style") {
        selection.italic = style.eq_ignore_ascii_case("italic");
    }
    if node.ancestors().any(|ancestor| {
        ancestor.attribute("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|class| class == "md-primary-mono")
        })
    }) {
        selection.mono = true;
        if let Some(face) = selection.markdown_face.as_mut() {
            if *face < 4 {
                *face += 4;
            }
        }
    }
    selection
}

fn svg_font_face_index(selection: SvgFontSelection) -> usize {
    if selection.markdown {
        return selection
            .markdown_face
            .unwrap_or(if selection.mono { 4 } else { 0 });
    }
    match (
        selection.mono,
        selection.italic,
        selection.bold,
        selection.semibold,
    ) {
        (false, true, _, _) => 3,
        (true, true, _, _) => 7,
        (false, false, true, _) => 2,
        (true, false, true, _) => 6,
        (false, false, false, true) => 1,
        (true, false, false, true) => 5,
        (false, false, false, false) => 0,
        (true, false, false, false) => 4,
    }
}

fn markdown_font_family_index(family: &str) -> Option<usize> {
    Some(match family {
        "d2-rs-font-regular" => 0,
        "d2-rs-font-semibold" => 1,
        "d2-rs-font-bold" => 2,
        "d2-rs-font-italic" => 3,
        "d2-rs-font-mono-regular" => 4,
        "d2-rs-font-mono-semibold" => 5,
        "d2-rs-font-mono-bold" => 6,
        "d2-rs-font-mono-italic" => 7,
        _ => return None,
    })
}

#[cfg(test)]
fn theme_css(
    light: PaletteView<'_>,
    dark: Option<PaletteView<'_>>,
    font_needs: FontNeeds,
    sketch: bool,
) -> String {
    theme_css_with_primary(light, dark, font_needs, sketch, false, BundledFamily::Mono)
}

fn theme_css_with_primary(
    light: PaletteView<'_>,
    dark: Option<PaletteView<'_>>,
    font_needs: FontNeeds,
    hand_drawn: bool,
    mono_primary: bool,
    mono_family: BundledFamily,
) -> String {
    // These target-level rules are part of D2's SVG contract. Keep them in
    // the root stylesheet so every shape/connection, including markers and
    // nested overlays, receives the same antialiasing and joins.
    let mut css = font_face_css_with_primary(font_needs, hand_drawn, mono_primary, mono_family);
    css.push_str(
        ".d2-rs-theme .shape{shape-rendering:geometricPrecision;stroke-linejoin:round;}.d2-rs-theme .connection{stroke-linecap:round;stroke-linejoin:round;}.d2-rs-theme .blend{mix-blend-mode:multiply;opacity:0.5;}",
    );
    css.push_str(
        ".d2-rs-theme .dots-overlay,.d2-rs-theme .lines-overlay,.d2-rs-theme .grain-overlay,.d2-rs-theme .paper-overlay{mix-blend-mode:multiply;}",
    );
    css.push_str(
        ".d2-rs-theme .appendix-icon{filter:drop-shadow(0px 0px 32px rgba(31,36,58,0.1));}",
    );
    let write_rules = |palette: PaletteView<'_>, output: &mut String| {
        for code in THEME_CODES {
            let Some(color) = palette
                .overrides
                .get(*code)
                .map(String::as_str)
                .or_else(|| resolve_color(palette.base, code))
            else {
                continue;
            };
            if !safe_css_color(color) {
                continue;
            }
            let _ = write!(
                output,
                ".d2-rs-theme .d2-rs-fill-{code},.d2-rs-theme .fill-{code}{{fill:{color} !important;}}"
            );
            let _ = write!(
                output,
                ".d2-rs-theme .d2-rs-stroke-{code},.d2-rs-theme .stroke-{code}{{stroke:{color} !important;}}"
            );
            let _ = write!(
                output,
                ".d2-rs-theme .d2-rs-text-{code}{{fill:{color} !important;color:{color} !important;}}"
            );
        }
    };
    write_rules(light, &mut css);
    if let Some(dark) = dark {
        let mut dark_css = String::new();
        write_rules(dark, &mut dark_css);
        if !dark_css.is_empty() {
            let _ = write!(
                css,
                "@media screen and (prefers-color-scheme:dark){{{dark_css}}}"
            );
        }
    }
    css
}

/// Returns the deterministic bundled-font stylesheet used by the SVG target.
///
/// D2 v0.9.0 embeds the Source Sans Pro and Source Code Pro faces used by the
/// diagram, subset to its text corpus. This boundary mirrors that behavior
/// with the pinned TrueType subsetter and keeps WOFF2 source faces as a
/// fallback for malformed SVG text or unsupported font data. The bytes are
/// cached so normal rendering does not repeatedly base64-encode the assets.
#[cfg(test)]
fn font_face_css(needs: FontNeeds, sketch: bool) -> String {
    font_face_css_with_primary(needs, sketch, false, BundledFamily::Mono)
}

fn font_face_css_with_primary(
    needs: FontNeeds,
    sketch: bool,
    mono_primary: bool,
    mono_family: BundledFamily,
) -> String {
    let mut ttf: [&[u8]; 8] = [
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Regular.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Semibold.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Bold.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Italic.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Regular.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Semibold.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Bold.ttf"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Italic.ttf"
        ))
        .as_slice(),
    ];
    let mut woff2: [&[u8]; 8] = [
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Regular.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Semibold.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Bold.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceSansPro-Italic.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Regular.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Semibold.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Bold.woff2"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../d2-rs-text/assets/SourceCodePro-Italic.woff2"
        ))
        .as_slice(),
    ];
    // Keep CSS face aliases stable, but embed the diagram-selected family.
    // Go embedFonts uses Diagram.FontFamily for each ordinary/Markdown face.
    if mono_primary {
        for index in 0..4 {
            ttf[index] = ttf[index + 4];
            woff2[index] = woff2[index + 4];
        }
    }
    // Primary remapping above must not change the independent mono selection.
    match mono_family {
        BundledFamily::Mono => {}
        BundledFamily::Sans => {
            let sans_ttf: [&[u8]; 4] = [
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Regular.ttf"),
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Semibold.ttf"),
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Bold.ttf"),
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Italic.ttf"),
            ];
            let sans_woff: [&[u8]; 4] = [
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Regular.woff2"),
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Semibold.woff2"),
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Bold.woff2"),
                include_bytes!("../../d2-rs-text/assets/SourceSansPro-Italic.woff2"),
            ];
            ttf[4..].copy_from_slice(&sans_ttf);
            woff2[4..].copy_from_slice(&sans_woff);
        }
        BundledFamily::HandDrawn => {
            let regular: &[u8] = include_bytes!("../../d2-rs-text/assets/FuzzyBubbles-Regular.ttf");
            let bold: &[u8] = include_bytes!("../../d2-rs-text/assets/FuzzyBubbles-Bold.ttf");
            ttf[4..].copy_from_slice(&[regular, bold, bold, regular]);
        }
    }
    let fallback_needed = [
        needs.sans_regular,
        needs.sans_semibold,
        needs.sans_bold,
        needs.sans_italic,
        needs.mono_regular,
        needs.mono_semibold,
        needs.mono_bold,
        needs.mono_italic,
    ];
    let needed: [bool; 8] = std::array::from_fn(|index| {
        if needs.subset_corpus_valid {
            !needs.ordinary_corpora[index].is_empty()
        } else {
            fallback_needed[index]
        }
    });
    let markdown_needed: [bool; 8] = std::array::from_fn(|index| {
        if needs.subset_corpus_valid {
            !needs.markdown_corpora[index].is_empty()
        } else {
            needed[index]
        }
    });
    let families = [
        "Source Sans Pro",
        "Source Sans Pro",
        "Source Sans Pro",
        "Source Sans Pro",
        "Source Code Pro",
        "Source Code Pro",
        "Source Code Pro",
        "Source Code Pro",
    ];
    let weights = [400, 600, 700, 400, 400, 600, 700, 400];
    let styles = [
        "normal", "normal", "normal", "italic", "normal", "normal", "normal", "italic",
    ];
    let markdown_families = [
        "d2-rs-font-regular",
        "d2-rs-font-semibold",
        "d2-rs-font-bold",
        "d2-rs-font-italic",
        "d2-rs-font-mono-regular",
        "d2-rs-font-mono-semibold",
        "d2-rs-font-mono-bold",
        "d2-rs-font-mono-italic",
    ];
    let ordinary_subsets: [Option<Vec<u8>>; 8] = std::array::from_fn(|index| {
        if !needed[index] || !needs.subset_corpus_valid {
            return None;
        }
        d2_rs_text::font_subset::subset_ttf(ttf[index], &needs.ordinary_corpora[index])
    });
    let markdown_subsets: [Option<Vec<u8>>; 8] = std::array::from_fn(|index| {
        if !markdown_needed[index] || !needs.subset_corpus_valid {
            return None;
        }
        d2_rs_text::font_subset::subset_ttf(ttf[index], &needs.markdown_corpora[index])
    });
    let mut css = String::new();
    // Keep the generic selectors in the stylesheet for ordinary label
    // consumers and compatibility with older SVG readers.  The more-specific
    // `.md-text` selectors below choose one family per concrete Markdown face,
    // as in D2's embedded SVG stylesheet.
    css.push_str(".d2-rs-theme .text{font-family:\"Source Sans Pro\",sans-serif;}");
    css.push_str(".d2-rs-theme .text-semibold{font-family:\"Source Sans Pro\",sans-serif;}");
    css.push_str(".d2-rs-theme .text-bold{font-family:\"Source Sans Pro\",sans-serif;}");
    css.push_str(".d2-rs-theme .text-italic{font-family:\"Source Sans Pro\",sans-serif;}");
    css.push_str(".d2-rs-theme .text-mono{font-family:\"Source Code Pro\",monospace;}");
    css.push_str(".d2-rs-theme .text-mono-semibold{font-family:\"Source Code Pro\",monospace;}");
    css.push_str(".d2-rs-theme .text-mono-bold{font-family:\"Source Code Pro\",monospace;}");
    css.push_str(".d2-rs-theme .text-mono-italic{font-family:\"Source Code Pro\",monospace;}");
    // Markdown classes use one-family-per-face, as in D2's embedded SVG
    // stylesheet.  Keeping the generic families for ordinary labels matters:
    // those labels select weights through presentation attributes, while the
    // Markdown painter selects a concrete face through its class and applies
    // any outer emphasis axis synthetically.
    css.push_str(".d2-rs-theme .md-text.text{font-family:\"d2-rs-font-regular\";}");
    css.push_str(".d2-rs-theme .md-text.text-semibold{font-family:\"d2-rs-font-semibold\";}");
    css.push_str(".d2-rs-theme .md-text.text-bold{font-family:\"d2-rs-font-bold\";}");
    css.push_str(".d2-rs-theme .md-text.text-italic{font-family:\"d2-rs-font-italic\";}");
    css.push_str(".d2-rs-theme .md-text.text-mono{font-family:\"d2-rs-font-mono-regular\";}");
    css.push_str(
        ".d2-rs-theme .md-text.text-mono-semibold{font-family:\"d2-rs-font-mono-semibold\";}",
    );
    css.push_str(".d2-rs-theme .md-text.text-mono-bold{font-family:\"d2-rs-font-mono-bold\";}");
    css.push_str(".d2-rs-theme .md-text.text-mono-italic{font-family:\"d2-rs-font-mono-italic\";}");
    css.push_str(r#".d2-rs-theme .md-primary-mono text.md-text.text{font-family:"d2-rs-font-mono-regular";}.d2-rs-theme .md-primary-mono text.md-text.text-bold{font-family:"d2-rs-font-mono-bold";}.d2-rs-theme .md-primary-mono text.md-text.text-semibold{font-family:"d2-rs-font-mono-semibold";}.d2-rs-theme .md-primary-mono text.md-text.text-italic{font-family:"d2-rs-font-mono-italic";}"#);
    if sketch {
        css.push_str(
            r#".d2-rs-theme.d2-rs-sketch text{font-family:"d2-rs-sketch-font-regular",sans-serif;}
.d2-rs-theme.d2-rs-sketch text.md-text.text{font-family:"d2-rs-sketch-font-regular",sans-serif;}
.d2-rs-theme.d2-rs-sketch text.md-text.text-italic{font-family:"d2-rs-sketch-font-regular",sans-serif;}
.d2-rs-theme.d2-rs-sketch text.md-text.text-bold{font-family:"d2-rs-sketch-font-bold",sans-serif;}
.d2-rs-theme.d2-rs-sketch text.md-text.text-semibold{font-family:"d2-rs-sketch-font-bold",sans-serif;}
.d2-rs-theme.d2-rs-sketch text[font-weight="bold"]{font-family:"d2-rs-sketch-font-bold",sans-serif;}
.d2-rs-theme.d2-rs-sketch text[font-family^="Source Code Pro"]{font-family:"Source Code Pro",monospace;}
.d2-rs-theme.d2-rs-sketch text.text-mono,.d2-rs-theme.d2-rs-sketch text.text-mono-semibold,.d2-rs-theme.d2-rs-sketch text.text-mono-bold,.d2-rs-theme.d2-rs-sketch text.text-mono-italic{font-family:"Source Code Pro",monospace;}
.d2-rs-theme.d2-rs-sketch text.md-text.text-mono{font-family:"d2-rs-font-mono-regular";}
.d2-rs-theme.d2-rs-sketch text.md-text.text-mono-semibold{font-family:"d2-rs-font-mono-semibold";}
.d2-rs-theme.d2-rs-sketch text.md-text.text-mono-bold{font-family:"d2-rs-font-mono-bold";}
.d2-rs-theme.d2-rs-sketch text.md-text.text-mono-italic{font-family:"d2-rs-font-mono-italic";}
.d2-rs-theme.d2-rs-sketch .md-primary-mono text.md-text.text{font-family:"d2-rs-font-mono-regular";}
.d2-rs-theme.d2-rs-sketch .md-primary-mono text.md-text.text-bold{font-family:"d2-rs-font-mono-bold";}
.d2-rs-theme.d2-rs-sketch .md-primary-mono text.md-text.text-semibold{font-family:"d2-rs-font-mono-semibold";}
.d2-rs-theme.d2-rs-sketch .md-primary-mono text.md-text.text-italic{font-family:"d2-rs-font-mono-italic";}"#,
        );
    }
    for index in 0..8 {
        if needed[index] {
            let (mime, format, data) = ordinary_subsets[index]
                .as_deref()
                .map(|data| ("font/ttf", "truetype", data))
                .unwrap_or_else(|| {
                    if index >= 4 && mono_family == BundledFamily::HandDrawn {
                        ("font/ttf", "truetype", ttf[index])
                    } else {
                        ("font/woff2", "woff2", woff2[index])
                    }
                });
            css.push_str(&font_face_rule_format(
                families[index],
                weights[index],
                styles[index],
                mime,
                format,
                data,
            ));
        }
        if markdown_needed[index] {
            let (mime, format, data) = markdown_subsets[index]
                .as_deref()
                .map(|data| ("font/ttf", "truetype", data))
                .unwrap_or_else(|| {
                    if index >= 4 && mono_family == BundledFamily::HandDrawn {
                        ("font/ttf", "truetype", ttf[index])
                    } else {
                        ("font/woff2", "woff2", woff2[index])
                    }
                });
            css.push_str(&font_face_rule_format(
                markdown_families[index],
                400,
                "normal",
                mime,
                format,
                data,
            ));
        }
    }
    if sketch {
        css.push_str(&font_face_rule_format(
            "d2-rs-sketch-font-regular",
            400,
            "normal",
            "font/ttf",
            "truetype",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../d2-rs-text/assets/FuzzyBubbles-Regular.ttf"
            )),
        ));
        css.push_str(&font_face_rule_format(
            "d2-rs-sketch-font-bold",
            700,
            "normal",
            "font/ttf",
            "truetype",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../d2-rs-text/assets/FuzzyBubbles-Bold.ttf"
            )),
        ));
    }
    css
}

fn font_face_rule_format(
    family: &str,
    weight: u16,
    style: &str,
    mime: &str,
    format: &str,
    data: &[u8],
) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    format!(
        "@font-face{{font-family:\"{family}\";font-style:{style};font-weight:{weight};src:url(data:{mime};base64,{encoded}) format(\"{format}\");}}"
    )
}

fn active_palette(theme_id: Option<i64>) -> Palette {
    theme_id
        .and_then(theme::palette)
        .or_else(|| theme::palette(0))
        .expect("pinned default theme palette")
}

fn node_level(graph: &Graph, node_id: NodeId) -> usize {
    let mut level = 1;
    let mut current = graph.node(node_id).and_then(|node| node.parent);
    while let Some(parent) = current {
        level += 1;
        current = graph.node(parent).and_then(|node| node.parent);
    }
    level
}

/// D2's SVG target performs a stable sort over all exported objects before it
/// writes them. Enumerate nodes by level/source order before accumulating
/// complete object paints; the final Source sort then interleaves sequence
/// shapes, lifelines, ordinary connections, messages and notes by ZIndex.
struct ObjectPaint {
    z_index: i64,
    shape_level: Option<i64>,
    svg: String,
}

fn capture_object_paint(
    svg: &mut String,
    from: usize,
    z_index: i64,
    shape_level: Option<i64>,
    objects: &mut Vec<ObjectPaint>,
) {
    let fragment = svg.split_off(from);
    if !fragment.is_empty() {
        objects.push(ObjectPaint {
            z_index,
            shape_level,
            svg: fragment,
        });
    }
}

fn paint_objects_in_source_order(svg: &mut String, objects: &mut [ObjectPaint]) {
    // Original d2svg.sortObjects: ZIndex, then shape level, then shapes before
    // connections, preserving export order for ties.
    objects.sort_by(|a, b| {
        a.z_index
            .cmp(&b.z_index)
            .then_with(|| match (a.shape_level, b.shape_level) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
    });
    for object in objects {
        svg.push_str(&object.svg);
    }
}

fn sequence_top_child(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let scope = outer_sequence_scope(graph, id);
    if scope.is_none() && !graph.root_sequence_diagram {
        return None;
    }
    let mut top = id;
    while let Some(parent) = graph.node(top).and_then(|n| n.parent) {
        if Some(parent) == scope {
            break;
        }
        top = parent;
    }
    Some(top)
}

fn sequence_shape_z_index(graph: &Graph, id: NodeId) -> i32 {
    if is_nested_sequence_grid_descendant(graph, id) {
        return 0;
    }
    if sequence_top_child(graph, id).is_some_and(|top| is_sequence_group_like(graph, top)) {
        return 3;
    }
    if (graph.root_sequence_diagram || outer_sequence_scope(graph, id).is_some())
        && sequence_reinjected_root(graph, id)
        && graph.node(id).and_then(|node| node.parent) != outer_sequence_scope(graph, id)
    {
        // The placeholder is classified while its contents are extracted.
        // Reinjection restores attributes, but keeps this draw order.
        let outside = |node| node == id || !is_descendant_or_self(graph, node, id);
        let contained_object = graph.sequence_object_scopes.iter().any(|(node, scopes)| {
            outside(*node)
                && scopes
                    .iter()
                    .any(|scope| is_descendant_or_self(graph, *scope, id))
        });
        let contained_edge = graph.sequence_group_edges.iter().any(|(owner, edges)| {
            is_descendant_or_self(graph, *owner, id)
                && edges.iter().any(|edge| {
                    graph
                        .edges
                        .get(edge.0 as usize)
                        .is_some_and(|edge| outside(edge.source) && outside(edge.target))
                })
        });
        return if !graph.sequence_has_edge_reference(id) && !contained_object && !contained_edge {
            5
        } else {
            2
        };
    }
    if is_sequence_span_like(graph, id) {
        return 2;
    }
    if is_sequence_note_like(graph, id) {
        return 5;
    }
    0
}

fn sequence_connection_z_index(graph: &Graph, edge: &d2_rs_core::Edge) -> i32 {
    let scope = outer_sequence_scope(graph, edge.source);
    if scope != outer_sequence_scope(graph, edge.target)
        || (scope.is_none() && !graph.root_sequence_diagram)
    {
        return 0;
    }
    // Extracted grid contents retain their ordinary connection role. Stop at
    // the owning sequence so a nested sequence inside a grid keeps its role.
    for id in [edge.source, edge.target] {
        let mut parent = graph.node(id).and_then(|n| n.parent);
        while parent != scope {
            let Some(id) = parent else {
                break;
            };
            if graph
                .node(id)
                .is_some_and(|n| n.grid_rows.is_some() || n.grid_columns.is_some())
            {
                return 0;
            }
            parent = graph.node(id).and_then(|n| n.parent);
        }
    }
    4
}

fn render_node_ids(graph: &Graph) -> Vec<NodeId> {
    let mut ids = graph.nodes().map(|(id, _)| id).collect::<Vec<_>>();
    ids.sort_by_key(|id| node_level(graph, *id));
    ids
}

fn node_has_children(graph: &Graph, node_id: NodeId) -> bool {
    graph.nodes().any(|(_, node)| node.parent == Some(node_id))
}

fn node_has_incident_edge(graph: &Graph, node_id: NodeId) -> bool {
    graph
        .edges()
        .any(|(_, edge)| edge.source == node_id || edge.target == node_id)
}

/// Returns the nearest sequence-diagram scope containing `node_id`.
///
/// This mirrors d2graph.Object.OuterSequenceDiagram: a sequence root is not
/// its own outer scope, while every descendant resolves to the nearest root.
fn outer_sequence_scope(graph: &Graph, node_id: NodeId) -> Option<NodeId> {
    let mut parent = graph.node(node_id).and_then(|node| node.parent);
    while let Some(candidate) = parent {
        if graph.is_sequence_diagram(candidate) {
            return Some(candidate);
        }
        parent = graph.node(candidate).and_then(|node| node.parent);
    }
    None
}

/// D2's note fill tests declaration edge references and contained objects/
/// edges. Endpoint incidence alone cannot distinguish glob-created actors.
fn has_sequence_note_fill(graph: &Graph, node_id: NodeId) -> bool {
    // GetFill calls Object.IsSequenceDiagramNote after extracted grid nodes
    // are restored to the final graph. A grid preserves their shape, but is
    // not a boundary for this sequence default-fill predicate.
    (outer_sequence_scope(graph, node_id).is_some() || graph.root_sequence_diagram)
        && !node_has_children(graph, node_id)
        && !graph.sequence_has_edge_reference(node_id)
        && !graph.sequence_contains_object(node_id)
        && !graph.sequence_group_edges.iter().any(|(owner, edges)| {
            !edges.is_empty() && is_descendant_or_self(graph, *owner, node_id)
        })
}

fn is_sequence_note_like(graph: &Graph, node_id: NodeId) -> bool {
    // sequence_diagram.go queues actor.ChildrenArray for page conversion;
    // direct actors keep their authored shape even when IsSequenceDiagramNote
    // gives them the white default fill at export time.
    has_sequence_note_fill(graph, node_id)
        && graph.node(node_id).and_then(|node| node.parent) != outer_sequence_scope(graph, node_id)
        && !is_nested_sequence_grid_descendant(graph, node_id)
}

/// Grid scopes are extracted and laid out separately from the sequence pass.
/// Their nodes retain the shapes chosen by that grid pass even when the grid is
/// nested below an actor. A nearer sequence scope starts its own role
/// partition, so an enclosing grid is not a boundary for that inner sequence.
fn is_nested_sequence_grid_descendant(graph: &Graph, node_id: NodeId) -> bool {
    let sequence = outer_sequence_scope(graph, node_id);
    let mut parent = graph.node(node_id).and_then(|node| node.parent);
    while let Some(candidate) = parent {
        if Some(candidate) == sequence {
            break;
        }
        if graph
            .node(candidate)
            .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
        {
            return true;
        }
        parent = graph.node(candidate).and_then(|node| node.parent);
    }
    false
}

/// Computes the default fill selected by d2graph.Object.GetFill before a
/// source style or theme override is applied. Keeping the result as a theme
/// code lets the renderer's existing palette/override machinery resolve the
/// actual color without changing caller-visible style semantics.
fn default_node_fill(graph: &Graph, node_id: NodeId) -> &'static str {
    let Some(node) = graph.node(node_id) else {
        return "N7";
    };
    let shape = target_shape_for_node(graph, node_id).unwrap_or(node.shape);
    let level = node_level(graph, node_id);

    if !is_sequence_span_like(graph, node_id)
        && matches!(shape, ShapeKind::SqlTable | ShapeKind::Class)
    {
        // D2 stores the header color as fill; ShapeTheme swaps the pair
        // when painting the tabular body.
        return "N1";
    }
    if has_sequence_note_fill(graph, node_id) {
        return "N7";
    }
    if is_sequence_group_like(graph, node_id) {
        return "N5";
    }
    if node
        .parent
        .is_some_and(|parent| graph.is_sequence_diagram(parent))
        || (graph.root_sequence_diagram && node.parent.is_none())
    {
        return "B5";
    }

    let sequence_relative_level = outer_sequence_scope(graph, node_id)
        .map(|sequence| level.saturating_sub(node_level(graph, sequence)))
        // The source graph root is an actual sequence object at level zero.
        // Graph represents it as a flag rather than a NodeId, so retain its
        // relative span levels without requiring an arena ancestor.
        .or_else(|| graph.root_sequence_diagram.then_some(level));
    if let Some(relative_level) = sequence_relative_level {
        match relative_level {
            1 => return "B3",
            2 => return "B4",
            3 => return "B5",
            4 => return "N6",
            _ => return "N7",
        }
    }
    if graph.is_sequence_diagram(node_id) {
        return "N7";
    }

    match shape {
        ShapeKind::Rectangle
        | ShapeKind::Square
        | ShapeKind::RealSquare
        | ShapeKind::Circle
        | ShapeKind::Oval => match level {
            1 if !node_has_children(graph, node_id) => "B6",
            1 => "B4",
            2 => "B5",
            3 => "B6",
            _ => "N7",
        },
        ShapeKind::Cylinder | ShapeKind::StoredData | ShapeKind::Package => {
            if level == 1 {
                "AA4"
            } else {
                "AA5"
            }
        }
        ShapeKind::Step | ShapeKind::Page | ShapeKind::Document => {
            if level == 1 {
                "AB4"
            } else {
                "AB5"
            }
        }
        ShapeKind::Person | ShapeKind::C4Person => "B3",
        ShapeKind::Diamond => "N4",
        ShapeKind::Cloud | ShapeKind::Callout => "N7",
        ShapeKind::Queue | ShapeKind::Parallelogram | ShapeKind::Hexagon => "N5",
        _ => "N7",
    }
}

/// Computes d2graph.Object.GetStroke's default source color. The dash
/// condition intentionally uses the raw source style, matching the upstream
/// call before target serialization.
fn default_node_stroke(style: &SourceNodeStyle, shape: ShapeKind) -> &'static str {
    if matches!(shape, ShapeKind::Text | ShapeKind::Code) {
        return "N1";
    }
    if matches!(shape, ShapeKind::Class | ShapeKind::SqlTable) {
        return "N7";
    }
    if style
        .stroke_dash
        .as_deref()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .is_some_and(|value| value != 0.0)
    {
        "B2"
    } else {
        "B1"
    }
}

fn required_patterns(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    rules: theme::SpecialRules,
) -> BTreeSet<String> {
    let mut patterns = styles
        .values()
        .filter_map(|style| style.fill_pattern.as_deref())
        .filter(|pattern| patternassets::id(pattern).is_some())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if let Some(pattern) = graph
        .root_style
        .as_ref()
        .and_then(|s| s.fill_pattern.as_deref())
        && patternassets::id(pattern).is_some()
    {
        patterns.insert(pattern.into());
    }
    let has_container = graph
        .nodes()
        .any(|(node_id, _)| node_has_children(graph, node_id));
    if rules.container_dots && has_container {
        patterns.insert("dots".into());
    }
    if rules.all_paper {
        patterns.insert("paper".into());
    }
    if let Some(source) = graph
        .legend
        .as_ref()
        .and_then(|legend| legend.source_graph.as_deref())
    {
        patterns.extend(required_patterns(&source.graph, &source.styles, rules));
    }
    patterns
}

/// d2scenebuild gives each plain label line a TextRun.Ink box (label
/// top-left, LabelWidth by one line advance) and d2raster resolves the
/// text's objectBoundingBox gradient against it, not against glyph ink.
/// Re-express a single-line raster label's gradient in user space over that
/// box; multi-line labels keep the shared definition.
fn raster_label_gradient_fill(
    svg: &mut String,
    fill: &str,
    font_color: Option<&str>,
    label: &str,
    label_box: Rect,
    raster: bool,
    id: &str,
) -> String {
    let Some(raw) = font_color.filter(|value| shapes::gradient_definition(value).is_some()) else {
        return fill.to_owned();
    };
    if !raster || label.contains('\n') {
        return fill.to_owned();
    }
    let Some(definition) = shapes::gradient_definition(raw) else {
        return fill.to_owned();
    };
    let shared = format!("id=\"{}\"", shapes::gradient_id(raw));
    let local = format!(
        "id=\"{id}\" gradientUnits=\"userSpaceOnUse\" gradientTransform=\"matrix({} 0 0 {} {} {})\"{}",
        label_box.size.width,
        label_box.size.height,
        label_box.origin.x,
        label_box.origin.y,
        if definition.starts_with("<radialGradient") {
            " cx=\"0.5\" cy=\"0.5\" r=\"0.5\""
        } else {
            ""
        }
    );
    // Under userSpaceOnUse a percentage resolves against the viewport, so
    // restate the vector attributes as the fractions the matrix scales.
    let mut definition = definition.replacen(&shared, &local, 1);
    for attribute in ["x1", "y1", "x2", "y2"] {
        let prefix = format!(" {attribute}=\"");
        let Some(start) = definition.find(&prefix).map(|at| at + prefix.len()) else {
            continue;
        };
        let Some(end) = definition[start..].find('"').map(|at| start + at) else {
            continue;
        };
        if let Some(percent) = definition[start..end]
            .strip_suffix('%')
            .and_then(|value| value.parse::<f64>().ok())
        {
            definition.replace_range(start..end, &(percent / 100.0).to_string());
        }
    }
    let _ = writeln!(svg, "<defs>{definition}</defs>");
    format!("url(#{id})")
}

fn required_gradients(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> BTreeMap<String, String> {
    // d2svg emits a gradient definition for every gradient-valued fill,
    // stroke and font color; d2scenebuild paints all three.
    let fills = styles
        .values()
        .flat_map(|style| {
            [
                style.fill.as_deref(),
                style.stroke.as_deref(),
                style.font_color.as_deref(),
            ]
            .into_iter()
            .flatten()
        })
        .chain(graph.root_style.iter().flat_map(|style| {
            [style.fill.as_deref(), style.stroke.as_deref()]
                .into_iter()
                .flatten()
        }))
        .chain(
            graph
                .legend
                .iter()
                .flat_map(|legend| legend.shapes.iter())
                .filter_map(|shape| shape.fill.as_deref()),
        );
    let mut gradients: BTreeMap<String, String> = fills
        .filter_map(|fill| {
            gradient_definition(fill).map(|definition| (gradient_id(fill), definition))
        })
        .collect();
    for (edge_id, _) in graph.edges() {
        let style = graph.edge_style(edge_id);
        for paint in [
            style.fill.as_deref(),
            style.stroke.as_deref(),
            style.font_color.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(definition) = gradient_definition(paint) {
                gradients.insert(gradient_id(paint), definition);
            }
        }
    }
    gradients
}

// d2exporter.applyTheme + toShape's final C4 text-color pass. Explicit
// source styles win; ordinary hierarchy fills cannot replace these defaults.
fn apply_c4_node_style(
    output: &mut SourceNodeStyle,
    target_shape: ShapeKind,
    has_children: bool,
    level: usize,
) {
    let person = matches!(target_shape, ShapeKind::Person | ShapeKind::C4Person);
    let (fill, stroke) = if person {
        ("B2", "B1")
    } else if has_children {
        ("transparent", "AA2")
    } else if level == 1 {
        ("B6", "B5")
    } else {
        ("B4", "B3")
    };
    if output.fill.is_none() {
        output.fill = Some(fill.into());
    }
    if output.stroke.is_none() {
        output.stroke = Some(stroke.into());
    }
    if has_children && output.stroke_dash.is_none() {
        output.stroke_dash = Some("5".into());
    }
    if output.font_color.is_none() {
        output.font_color = Some(if has_children { "N1" } else { "N7" }.into());
    }
}

/// Applies the non-color part of D2's `d2exporter.applyTheme` to a target
/// shape. Source styles remain authoritative, just as they are after the
/// upstream theme pass. Keeping this as a target-stage copy also means the
/// layout graph remains independent of rendering-only theme choices.
fn themed_node_style(
    graph: &Graph,
    node_id: NodeId,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
) -> SourceNodeStyle {
    let mut output = style.clone();
    output.opacity = output.opacity.map(render_opacity_value);
    let rules = palette.base.special_rules;
    let Some(node) = graph.node(node_id) else {
        return output;
    };
    let target_shape = target_shape_for_node(graph, node_id).unwrap_or(node.shape);
    let has_children = node_has_children(graph, node_id);
    let level = node_level(graph, node_id);

    if rules.no_corner_radius && output.border_radius.is_none() {
        output.border_radius = Some("0".into());
    }
    if rules.outer_container_double_border
        && node.parent.is_none()
        && has_children
        && output.double_border.is_none()
    {
        output.double_border = Some("true".into());
    }
    if rules.container_dots && has_children && output.fill_pattern.is_none() {
        output.fill_pattern = Some("dots".into());
    } else if rules.all_paper && output.fill_pattern.is_none() {
        output.fill_pattern = Some("paper".into());
    }
    // D2 v0.9.0 applies the theme's mono family after source styles on node
    // targets, so the theme wins even over an explicit `style.font: default`.
    // The graph stage has already used that explicit family for measurement.
    if rules.mono {
        output.font = Some("mono".into());
    }

    if rules.c4 {
        apply_c4_node_style(&mut output, target_shape, has_children, level);
    }

    // Apply target defaults after theme rules so special palettes (notably
    // C4) can still supply their own fill/stroke values. Explicit source
    // styles and theme-selected values always win over these defaults.
    if output.fill.is_none() {
        output.fill = Some(default_node_fill(graph, node_id).into());
    }
    if output.stroke.is_none() {
        output.stroke = Some(default_node_stroke(&output, target_shape).into());
    }
    if output.stroke_width.is_none() {
        output.stroke_width = Some(
            if graph.is_sequence_diagram(node_id) || is_sequence_group_like(graph, node_id) {
                "0"
            } else {
                "2"
            }
            .into(),
        );
    }
    output
}

fn draw_shape_with_pattern_overlay(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    shape_id: &str,
    native_scene: bool,
    source_target: bool,
) {
    if source_target
        && !native_scene
        && matches!(
            shape,
            ShapeKind::Rectangle | ShapeKind::Square | ShapeKind::RealSquare
        )
        && !style_flag(Some(style), |s| s.three_dee.as_ref())
        && (style_flag(Some(style), |s| s.multiple.as_ref())
            || style_flag(Some(style), |s| s.double_border.as_ref()))
    {
        // d2svg.drawShape paints each rectangle's pattern before its inner
        // border, including the filled inner border of the multiple copy.
        let mut plain = style.clone();
        plain.multiple = None;
        plain.double_border = None;
        plain.shadow = None;
        let shadow = style_flag(Some(style), |s| s.shadow.as_ref());
        if shadow {
            svg.push_str("<g filter=\"url(#d2-rs-shadow-filter)\">");
        }
        let double = style_flag(Some(style), |s| s.double_border.as_ref());
        let multiple = style_flag(Some(style), |s| s.multiple.as_ref());
        for back in [true, false] {
            if back && !multiple {
                continue;
            }
            let mut bounds = rect;
            if back {
                bounds.origin.x += 10.0;
                bounds.origin.y -= 10.0;
            }
            draw_shape_with_pattern_overlay(
                svg, shape, bounds, &plain, palette, shape_id, false, false,
            );
            if double && bounds.size.width > 10.0 && bounds.size.height > 10.0 {
                let mut inner = plain.clone();
                inner.fill_pattern = None;
                if !back {
                    inner.fill = Some("none".into());
                }
                bounds.origin.x += 5.0;
                bounds.origin.y += 5.0;
                bounds.size.width -= 10.0;
                bounds.size.height -= 10.0;
                draw_shape_with_id(svg, shape, bounds, &inner, palette, shape_id);
            }
        }
        if shadow {
            svg.push_str("</g>");
        }
        return;
    }
    if native_scene
        && style_flag(Some(style), |style| style.three_dee.as_ref())
        && matches!(
            shape,
            ShapeKind::Rectangle | ShapeKind::Square | ShapeKind::RealSquare | ShapeKind::Hexagon
        )
    {
        draw_native_three_dee_shape(svg, shape, rect, style, palette, shape_id);
        return;
    }
    let Some(_pattern) = style
        .fill_pattern
        .as_deref()
        .filter(|pattern| patternassets::id(pattern).is_some())
    else {
        draw_shape_with_id(svg, shape, rect, style, palette, shape_id);
        return;
    };

    let mut base = style.clone();
    base.fill_pattern = None;
    let mut overlay = style.clone();
    overlay.fill = None;
    overlay.stroke = Some("none".into());
    overlay.shadow = None;
    // d2scenebuild effectPatternNode patterns only the front copy of a
    // `multiple` shape.
    overlay.multiple = None;
    if native_scene
        && shapes::draw_native_double_border_pattern(
            svg, shape, rect, &base, &overlay, palette, shape_id,
        )
    {
        return;
    }
    if draw_interleaved_multi_path_pattern(svg, shape, rect, &base, &overlay, palette) {
        return;
    }
    draw_shape_with_id(svg, shape, rect, &base, palette, shape_id);
    draw_shape_with_id(svg, shape, rect, &overlay, palette, "");
}

// Source exporter parses the original graph scalar before SVG/scene output.
// Canonical decimal syntax keeps Go hex/underscore values out of SVG attributes
// while retaining their exact f64 value for private scene transport.
fn render_opacity_value(raw: String) -> String {
    match d2_rs_core::parse_d2_float(&raw) {
        Some(value) if value.is_finite() => value.to_string(),
        _ => raw,
    }
}

fn themed_edge_style(style: EdgeStyle, palette: PaletteView<'_>) -> EdgeStyle {
    let authored_stroke_is_missing = style.stroke.is_none();
    let mut output = style;
    output.opacity = output.opacity.map(render_opacity_value);
    // d2graph.Edge.GetStroke receives the parsed numeric dash value. A
    // non-zero dash selects B2, while malformed or zero values retain B1.
    if output.stroke.is_none()
        && output
            .stroke_dash
            .as_deref()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .is_some_and(|value| value != 0.0)
    {
        output.stroke = Some("B2".into());
    }
    // D2 v0.9.0's d2exporter.toConnection applies the C4 connection defaults
    // after Edge.GetStroke: missing styles receive dash 5 and AA4 stroke.
    // This deliberately overrides GetStroke's ordinary B1/B2 fallback while
    // leaving authored stroke and dash values intact.
    if palette.base.special_rules.c4 {
        if output.stroke_dash.is_none() {
            output.stroke_dash = Some("5".into());
        }
        if authored_stroke_is_missing {
            output.stroke = Some("AA4".into());
        }
    }
    if palette.base.special_rules.mono && output.font.is_none() {
        output.font = Some("mono".into());
    }
    output
}

fn theme_label(
    text: &str,
    style: Option<&SourceNodeStyle>,
    shape: ShapeKind,
    palette: PaletteView<'_>,
) -> String {
    let transform = style.and_then(|style| style.text_transform.as_deref());
    let language = style.and_then(|style| style.language.as_deref());
    let caps_lock = palette.base.special_rules.caps_lock
        && shape != ShapeKind::Code
        && language != Some("latex")
        && transform != Some("none");
    if !caps_lock {
        return text.to_owned();
    }
    match transform {
        Some("lowercase") => text.to_uppercase().to_lowercase(),
        Some("capitalize") => {
            let mut output = String::with_capacity(text.len());
            let mut word_start = true;
            let uppercase = text.to_uppercase();
            for character in uppercase.chars() {
                if character.is_alphanumeric() {
                    if word_start {
                        output.extend(character.to_uppercase());
                    } else {
                        output.extend(character.to_lowercase());
                    }
                    word_start = false;
                } else {
                    output.push(character);
                    word_start = true;
                }
            }
            output
        }
        _ => text.to_uppercase(),
    }
}

fn theme_edge_label(text: &str, style: &EdgeStyle, palette: PaletteView<'_>) -> String {
    let caps_lock =
        palette.base.special_rules.caps_lock && style.text_transform.as_deref() != Some("none");
    if !caps_lock {
        return text.to_owned();
    }
    match style.text_transform.as_deref() {
        Some("lowercase") => text.to_uppercase().to_lowercase(),
        Some("capitalize") => {
            let uppercase = text.to_uppercase();
            let mut output = String::with_capacity(text.len());
            let mut word_start = true;
            for character in uppercase.chars() {
                if character.is_alphanumeric() {
                    if word_start {
                        output.extend(character.to_uppercase());
                    } else {
                        output.extend(character.to_lowercase());
                    }
                    word_start = false;
                } else {
                    output.push(character);
                    word_start = true;
                }
            }
            output
        }
        _ => text.to_uppercase(),
    }
}

// Original exporter sets a sequence group's LabelFill to the target fill.
// d2svg paints it before ordinary text outside the geometry blend owner,
// while the complete shape still shares its authored opacity owner.
fn draw_sequence_label_background(svg: &mut String, rect: Rect, fill: &str, class: Option<&str>) {
    if fill.is_empty() {
        return;
    }
    let fill = if gradient_definition(fill).is_some() {
        format!("url(#{})", gradient_id(fill))
    } else {
        fill.to_owned()
    };
    let class = class
        .map(|class| format!(" class=\"{}\"", escape(class)))
        .unwrap_or_default();
    let _ = writeln!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{class}/>",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width),
        number(rect.size.height),
        escape(&fill)
    );
}

fn themed_color(raw: Option<&str>, fallback: &str, palette: PaletteView<'_>) -> String {
    let Some(raw) = raw else {
        return fallback.to_owned();
    };
    palette.explicit(raw, fallback)
}

fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    // D2's svg.FormatFloat uses six decimal places before trimming zeros.
    // Keeping that precision avoids changing route and marker geometry when
    // measurements or transforms produce sub-millipixel coordinates.
    let mut text = format!("{value:.6}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".into() } else { text }
}

fn target_route_coordinate(value: f64) -> f64 {
    // d2exporter: Point.TruncateDecimals, then Point.TruncateFloat32. This
    // conversion belongs at the raw-layout to target boundary and must run
    // only once: truncating a float32 value again can lose another 0.001px.
    let truncated = (value * 1000.0) as i64 as f64 / 1000.0;
    f64::from(truncated as f32)
}

fn target_route_point(point: Point) -> Point {
    Point {
        x: target_route_coordinate(point.x),
        y: target_route_coordinate(point.y),
    }
}

fn rendered_route_point(graph: &Graph, point: Point) -> Point {
    if graph.target_routes_exported {
        point
    } else {
        target_route_point(point)
    }
}

/// Computes the renderer-side endpoint adjustment used by D2 for arrowheads.
///
/// Layout routes terminate on the shape border.  SVG arrow markers are drawn
/// with their reference point at the path endpoint, so D2 backs the visible
/// stroke away from the border by half the two stroke widths plus one edge
/// stroke width when an arrow is present.  Keeping this adjustment in the
/// renderer leaves the shared layout route (and its trace diagnostics) intact.
fn arrowhead_adjustment(
    start: Point,
    end: Point,
    enabled: bool,
    edge_stroke: f64,
    shape_stroke: f64,
) -> Point {
    let distance = (edge_stroke + shape_stroke) / 2.0 + if enabled { edge_stroke } else { 0.0 };
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length = (dx * dx + dy * dy).sqrt();
    if !length.is_finite() || length <= f64::EPSILON {
        return Point { x: 0.0, y: 0.0 };
    }
    let reciprocal_length = 1.0 / length;
    Point {
        x: (dx * reciprocal_length) * -distance,
        y: (dy * reciprocal_length) * -distance,
    }
}

fn stroke_width(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(2.0)
}

/// Converts a routed edge into D2's SVG path representation. Curved routes
/// use the compact cubic form emitted by the layout adapter; orthogonal routes
/// use line segments and smooth cubics at each bend. The latter is the same
/// corner-rounding construction as v0.9.0's `connectionPath`: the radius is
/// clamped to half of the outgoing segment and short segments are coalesced
/// into one cubic when needed.
fn route_path_data_with_radius(
    route: &[Point],
    source_adjustment: Point,
    target_adjustment: Point,
    border_radius: f64,
    is_curve: bool,
) -> String {
    if route.is_empty() {
        return String::new();
    }
    if route.len() == 1 {
        let point = Point {
            x: route[0].x + source_adjustment.x,
            y: route[0].y + source_adjustment.y,
        };
        return format!("M {} {}", number(point.x), number(point.y));
    }

    if is_curve && route.len() >= 4 {
        let mut path = format!(
            "M {} {}",
            number(route[0].x + source_adjustment.x),
            number(route[0].y + source_adjustment.y)
        );
        let mut index = 1;
        while index + 2 < route.len() {
            let endpoint = route[index + 2];
            let is_last = index + 2 == route.len() - 1;
            path.push_str(&format!(
                " C {} {} {} {} {} {}",
                go_route_number(route[index].x),
                go_route_number(route[index].y),
                go_route_number(route[index + 1].x),
                go_route_number(route[index + 1].y),
                number(endpoint.x + if is_last { target_adjustment.x } else { 0.0 }),
                number(endpoint.y + if is_last { target_adjustment.y } else { 0.0 }),
            ));
            index += 3;
        }
        return path;
    }

    let start = Point {
        x: route[0].x + source_adjustment.x,
        y: route[0].y + source_adjustment.y,
    };
    let end = Point {
        x: route[route.len() - 1].x + target_adjustment.x,
        y: route[route.len() - 1].y + target_adjustment.y,
    };
    let radius = if border_radius.is_finite() {
        border_radius.max(0.0)
    } else {
        0.0
    };
    let mut path = format!("M {} {}", number(start.x), number(start.y));
    let mut index = 1;
    while index < route.len() - 1 {
        let previous_source = route[index - 1];
        let corner = route[index];
        let next = route[index + 1];
        let previous_vector = Point {
            x: corner.x - previous_source.x,
            y: corner.y - previous_source.y,
        };
        let current_vector = Point {
            x: next.x - corner.x,
            y: next.y - corner.y,
        };
        let previous_length =
            (previous_vector.x * previous_vector.x + previous_vector.y * previous_vector.y).sqrt();
        let current_length =
            (current_vector.x * current_vector.x + current_vector.y * current_vector.y).sqrt();
        if previous_length <= f64::EPSILON || current_length <= f64::EPSILON {
            path.push_str(&format!(" L {} {}", number(corner.x), number(corner.y)));
            index += 1;
            continue;
        }
        let distance = current_length;
        let units = radius.min(distance / 2.0);
        let previous_translation = Point {
            x: previous_vector.x / previous_length * units,
            y: previous_vector.y / previous_length * units,
        };
        let current_translation = Point {
            x: current_vector.x / current_length * units,
            y: current_vector.y / current_length * units,
        };
        let line_end = Point {
            x: corner.x - previous_translation.x,
            y: corner.y - previous_translation.y,
        };
        path.push_str(&format!(" L {} {}", number(line_end.x), number(line_end.y)));

        // The upstream builder coalesces a short outgoing segment with the
        // next corner. This keeps the path stable for dense routes where two
        // bends are closer together than the requested radius.
        if units < radius && index < route.len() - 2 {
            let next_target = route[index + 2];
            let next_vector = Point {
                x: next_target.x - next.x,
                y: next_target.y - next.y,
            };
            let next_length =
                (next_vector.x * next_vector.x + next_vector.y * next_vector.y).sqrt();
            if next_length > f64::EPSILON {
                index += 1;
                let next_translation = Point {
                    x: next_vector.x / next_length * units,
                    y: next_vector.y / next_length * units,
                };
                let curve_end = Point {
                    x: next.x + next_translation.x,
                    y: next.y + next_translation.y,
                };
                path.push_str(&format!(
                    " C {} {} {} {} {} {}",
                    number(corner.x + previous_translation.x),
                    number(corner.y + previous_translation.y),
                    number(next.x - next_translation.x),
                    number(next.y - next_translation.y),
                    number(curve_end.x),
                    number(curve_end.y),
                ));
                index += 1;
                continue;
            }
        }
        let curve_end = Point {
            x: corner.x + current_translation.x,
            y: corner.y + current_translation.y,
        };
        path.push_str(&format!(
            " S {} {} {} {}",
            number(corner.x),
            number(corner.y),
            number(curve_end.x),
            number(curve_end.y),
        ));
        index += 1;
    }
    path.push_str(&format!(" L {} {}", number(end.x), number(end.y)));
    path
}

/// Replace an animated connection's exact raster source with the two halves
/// of d2scenebuild splitAnimatedConnectionPath (connection.go:213–308): chord
/// lengths per segment; the crossing L is cut at its parameter, a C command is
/// bisected at t = 0.5, and an S command stays whole in the first half. The
/// SVG path tokens supply the C/S provenance the typed commands lack. Returns
/// the two path ids, or None when no consistent split exists.
fn split_animated_raster_source(
    sources: Option<&mut Vec<RasterPathSource>>,
    raster_path_id: &str,
    path_data: &str,
) -> Option<(String, String)> {
    let sources = sources?;
    let id = raster_path_id
        .strip_prefix(" id=\"")?
        .strip_suffix('"')?
        .to_owned();
    let index = sources.iter().position(|source| source.id == id)?;
    let kinds = path_data
        .split_whitespace()
        .filter(|token| matches!(*token, "M" | "L" | "C" | "S"))
        .collect::<Vec<_>>();
    let commands = &sources[index].commands;
    if kinds.len() != commands.len() || kinds.first() != Some(&"M") {
        return None;
    }
    let end = |command: &RasterPathCommand| match command {
        RasterPathCommand::MoveTo(point) | RasterPathCommand::LineTo(point) => Some(*point),
        RasterPathCommand::CubicTo(_, _, point) => Some(*point),
        RasterPathCommand::Close => None,
    };
    let mut lengths = Vec::with_capacity(commands.len());
    let mut previous = end(&commands[0])?;
    let mut total = 0.0;
    for command in &commands[1..] {
        let point = end(command)?;
        let length = (point.x - previous.x).hypot(point.y - previous.y);
        lengths.push(length);
        total += length;
        previous = point;
    }
    if total <= 0.0 {
        return None;
    }
    let half = total * 0.5;
    let mut first = vec![commands[0].clone()];
    let mut second = Vec::new();
    let mut cumulative = 0.0;
    let mut past = false;
    let mut start = end(&commands[0])?;
    for (offset, command) in commands[1..].iter().enumerate() {
        let length = lengths[offset];
        let point = end(command)?;
        cumulative += length;
        if past {
            second.push(command.clone());
        } else if cumulative < half {
            first.push(command.clone());
        } else {
            match (kinds[offset + 1], command) {
                ("L", RasterPathCommand::LineTo(_)) => {
                    if length == 0.0 {
                        return None;
                    }
                    let t = (half - cumulative + length) / length;
                    let split = Point {
                        x: start.x + (point.x - start.x) * t,
                        y: start.y + (point.y - start.y) * t,
                    };
                    first.push(RasterPathCommand::LineTo(split));
                    second.push(RasterPathCommand::MoveTo(split));
                    second.push(RasterPathCommand::LineTo(point));
                }
                ("C", RasterPathCommand::CubicTo(c1, c2, _)) => {
                    let mid = |a: Point, b: Point| Point {
                        x: (a.x + b.x) / 2.0,
                        y: (a.y + b.y) / 2.0,
                    };
                    let (ab, bc, cd) = (mid(start, *c1), mid(*c1, *c2), mid(*c2, point));
                    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
                    let center = mid(abc, bcd);
                    first.push(RasterPathCommand::CubicTo(ab, abc, center));
                    second.push(RasterPathCommand::MoveTo(center));
                    second.push(RasterPathCommand::CubicTo(bcd, cd, point));
                }
                ("S", RasterPathCommand::CubicTo(..)) => {
                    first.push(command.clone());
                    second.push(RasterPathCommand::MoveTo(point));
                }
                _ => return None,
            }
            past = true;
        }
        start = point;
    }
    if !past || second.is_empty() {
        return None;
    }
    let transform = sources[index].transform;
    sources.remove(index);
    let ids = (format!("{id}-split-reverse"), format!("{id}-split-forward"));
    sources.push(RasterPathSource {
        id: ids.0.clone(),
        commands: first,
        transform,
    });
    sources.push(RasterPathSource {
        id: ids.1.clone(),
        commands: second,
        transform,
    });
    Some(ids)
}

fn route_path_source_commands(
    route: &[Point],
    source_adjustment: Point,
    target_adjustment: Point,
    border_radius: f64,
    is_curve: bool,
) -> Option<Vec<RasterPathCommand>> {
    if route.len() < 2 {
        return None;
    }
    if is_curve && route.len() >= 4 {
        let mut commands = vec![RasterPathCommand::MoveTo(Point {
            x: route[0].x + source_adjustment.x,
            y: route[0].y + source_adjustment.y,
        })];
        let mut index = 1;
        while index + 2 < route.len() {
            let endpoint = route[index + 2];
            let is_last = index + 2 == route.len() - 1;
            commands.push(RasterPathCommand::CubicTo(
                route[index],
                route[index + 1],
                Point {
                    x: endpoint.x + if is_last { target_adjustment.x } else { 0.0 },
                    y: endpoint.y + if is_last { target_adjustment.y } else { 0.0 },
                },
            ));
            index += 3;
        }
        return commands
            .iter()
            .any(|command| matches!(command, RasterPathCommand::CubicTo(..)))
            .then_some(commands);
    }

    let start = Point {
        x: route[0].x + source_adjustment.x,
        y: route[0].y + source_adjustment.y,
    };
    let end = Point {
        x: route[route.len() - 1].x + target_adjustment.x,
        y: route[route.len() - 1].y + target_adjustment.y,
    };
    let radius = if border_radius.is_finite() {
        border_radius.max(0.0)
    } else {
        0.0
    };
    let mut commands = vec![RasterPathCommand::MoveTo(start)];
    let mut index = 1;
    while index < route.len() - 1 {
        let previous_source = route[index - 1];
        let corner = route[index];
        let next = route[index + 1];
        let previous_vector = Point {
            x: corner.x - previous_source.x,
            y: corner.y - previous_source.y,
        };
        let current_vector = Point {
            x: next.x - corner.x,
            y: next.y - corner.y,
        };
        let previous_length =
            (previous_vector.x * previous_vector.x + previous_vector.y * previous_vector.y).sqrt();
        let current_length =
            (current_vector.x * current_vector.x + current_vector.y * current_vector.y).sqrt();
        if previous_length <= f64::EPSILON || current_length <= f64::EPSILON {
            commands.push(RasterPathCommand::LineTo(corner));
            index += 1;
            continue;
        }
        let units = radius.min(current_length / 2.0);
        let previous_translation = Point {
            x: previous_vector.x / previous_length * units,
            y: previous_vector.y / previous_length * units,
        };
        let current_translation = Point {
            x: current_vector.x / current_length * units,
            y: current_vector.y / current_length * units,
        };
        let line_end = Point {
            x: corner.x - previous_translation.x,
            y: corner.y - previous_translation.y,
        };
        commands.push(RasterPathCommand::LineTo(line_end));

        if units < radius && index < route.len() - 2 {
            let next_target = route[index + 2];
            let next_vector = Point {
                x: next_target.x - next.x,
                y: next_target.y - next.y,
            };
            let next_length =
                (next_vector.x * next_vector.x + next_vector.y * next_vector.y).sqrt();
            if next_length > f64::EPSILON {
                index += 1;
                let next_translation = Point {
                    x: next_vector.x / next_length * units,
                    y: next_vector.y / next_length * units,
                };
                let curve_end = Point {
                    x: next.x + next_translation.x,
                    y: next.y + next_translation.y,
                };
                commands.push(RasterPathCommand::CubicTo(
                    Point {
                        x: corner.x + previous_translation.x,
                        y: corner.y + previous_translation.y,
                    },
                    Point {
                        x: next.x - next_translation.x,
                        y: next.y - next_translation.y,
                    },
                    curve_end,
                ));
                index += 1;
                continue;
            }
        }
        let curve_end = Point {
            x: corner.x + current_translation.x,
            y: corner.y + current_translation.y,
        };
        // The SVG `S` follows a line command here, so SVG's reflected first
        // control point is the current point (`line_end`) itself.
        commands.push(RasterPathCommand::CubicTo(line_end, corner, curve_end));
        index += 1;
    }
    commands.push(RasterPathCommand::LineTo(end));
    // Original connectionPath returns the scene's MoveTo/LineTo commands
    // even when there is no cubic. Dropping them narrows fractional arrow
    // adjustments and straight diagonals at the SVG/usvg boundary.
    Some(commands)
}

/// D2's curve control points cross a float32 scene boundary before SVG
/// serialization. Preserve those bits at the text boundary without changing
/// the shared f64 layout geometry or ordinary orthogonal path formatting.
fn go_route_number(value: f64) -> String {
    let value = value as f32 as f64;
    let mut output = format!("{value:.6}");
    while output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    output
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// Original lib/svg.EscapeText delegates to encoding/xml.EscapeText.
// Character references preserve controls through XML newline normalization.
fn escape_xml_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\"' => output.push_str("&#34;"),
            '\'' => output.push_str("&#39;"),
            '\t' => output.push_str("&#x9;"),
            '\n' => output.push_str("&#xA;"),
            '\r' => output.push_str("&#xD;"),
            '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}' => {
                output.push(c)
            }
            _ => output.push('\u{fffd}'),
        }
    }
    output
}

// Original d2svg.RenderText splits LF only and retains every empty line.
fn render_text_content(text: &str, x: f64, height: f64) -> String {
    if !text.contains('\n') {
        return escape_xml_text(text);
    }
    let lines = text.split('\n').collect::<Vec<_>>();
    let mut output = String::new();
    for (index, line) in lines.iter().enumerate() {
        let dy = if index == 0 {
            0.0
        } else {
            height / lines.len() as f64
        };
        let line = if line.is_empty() { " " } else { line };
        let _ = write!(
            output,
            "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
            number(x),
            number(dy),
            escape_xml_text(line)
        );
    }
    output
}

/// D2's SVG target uses URL-safe base64 (with padding) for object classes so
/// source IDs can be recovered without putting arbitrary punctuation in a
/// CSS selector.
fn svg_object_class(id: &str) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(escape_xml_text(id).as_bytes())
}

fn svg_object_classes(id: &str, classes: Option<&Vec<String>>) -> String {
    let mut out = svg_object_class(id);
    for class in classes.into_iter().flatten() {
        out.push(' ');
        out.push_str(class);
    }
    escape(&out)
}

fn edge_svg_id(graph: &Graph, edge_id: EdgeId) -> Option<String> {
    d2_rs_core::edge_abs_id(graph, edge_id)
}

fn asset_mime(key: &str, bytes: &[u8]) -> &'static str {
    let lower = key.to_ascii_lowercase();
    // Prefer the payload signature when it is available.  Raster exports may
    // normalize an animated GIF to a PNG while retaining the source key so
    // SVG asset references remain stable; extension-first detection would
    // then advertise a mismatched MIME type and make usvg reject the data.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if lower.ends_with(".svg") || bytes.windows(4).any(|window| window == b"<svg") {
        "image/svg+xml"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/png"
    }
}

/// Bundle rendered image references without regenerating SVG IDs or geometry.
pub fn bundle_svg_images(svg: &[u8], assets: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    bundle_svg_images_with_content_types(svg, assets, &BTreeMap::new())
}

/// Preserve a remote server's Content-Type at the imgbundler boundary.
pub fn bundle_svg_images_with_content_types(
    svg: &[u8],
    assets: &BTreeMap<String, Vec<u8>>,
    content_types: &BTreeMap<String, String>,
) -> Vec<u8> {
    if assets.is_empty() {
        return svg.to_vec();
    }
    let replacements = assets
        .iter()
        .map(|(key, bytes)| {
            let original = format!("href=\"{}\"", escape(key));
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let mut mime = content_types
                .get(key)
                .filter(|value| !value.is_empty())
                .map_or_else(|| image_mime::sniff(key, bytes), Clone::clone)
                .replacen("text/xml", "image/svg+xml", 1);
            if mime == "application/octet-stream"
                && bytes.windows(4).any(|window| window == b"<svg")
            {
                mime = "image/svg+xml".to_owned();
            }
            let replacement = format!("href=\"data:{};base64,{encoded}\"", escape(&mime));
            (original, replacement)
        })
        .collect::<Vec<_>>();
    let text = String::from_utf8_lossy(svg);
    let mut output = String::with_capacity(text.len());
    let mut remaining = text.as_ref();
    while let Some(start) = remaining.find("<image ") {
        output.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('>') else {
            break;
        };
        let mut tag = remaining[..=end].to_owned();
        for (original, replacement) in &replacements {
            if tag.contains(original) {
                tag = tag.replace(original, replacement);
            }
        }
        output.push_str(&tag);
        remaining = &remaining[end + 1..];
    }
    output.push_str(remaining);
    output.into_bytes()
}

fn resolve_asset_href(key: &str, asset_data: &BTreeMap<String, Vec<u8>>) -> String {
    let Some(bytes) = asset_data.get(key) else {
        return key.to_owned();
    };
    let mime = asset_mime(key, bytes);
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

fn canonical_marker_name(shape: Option<&str>) -> Option<&'static str> {
    match shape
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        // D2's default arrowhead is a filled triangle.  Keep an explicit
        // triangle distinct from the `arrow` (four-point) shape.
        None | Some("") | Some("triangle") | Some("filled-triangle") => Some("triangle"),
        Some("arrow") => Some("arrow"),
        Some("unfilled-triangle") => Some("unfilled-triangle"),
        Some("diamond") => Some("diamond"),
        Some("filled-diamond") => Some("filled-diamond"),
        Some("circle") => Some("circle"),
        Some("filled-circle") => Some("filled-circle"),
        Some("box") => Some("box"),
        Some("filled-box") => Some("filled-box"),
        Some("cross") => Some("cross"),
        Some("line") => Some("line"),
        Some("cf-one") => Some("cf-one"),
        Some("cf-many") => Some("cf-many"),
        Some("cf-one-required") => Some("cf-one-required"),
        Some("cf-many-required") => Some("cf-many-required"),
        Some("none") => None,
        Some(_) => Some("triangle"),
    }
}

fn marker_id(shape: Option<&str>, is_target: bool) -> Option<&'static str> {
    let name = canonical_marker_name(shape)?;
    // Target IDs retain their readable historical names (and the CLI's
    // compatibility tests depend on this).  Source markers use the mirrored
    // geometry and therefore need a separate definition.
    if is_target {
        Some(name)
    } else {
        match name {
            "arrow" => Some("arrow-start"),
            "triangle" => Some("triangle-start"),
            "unfilled-triangle" => Some("unfilled-triangle-start"),
            "diamond" => Some("diamond-start"),
            "filled-diamond" => Some("filled-diamond-start"),
            "circle" => Some("circle-start"),
            "filled-circle" => Some("filled-circle-start"),
            "box" => Some("box-start"),
            "filled-box" => Some("filled-box-start"),
            "cross" => Some("cross-start"),
            "line" => Some("line-start"),
            "cf-one" => Some("cf-one-start"),
            "cf-many" => Some("cf-many-start"),
            "cf-one-required" => Some("cf-one-required-start"),
            "cf-many-required" => Some("cf-many-required-start"),
            _ => None,
        }
    }
}

fn style_flag(
    style: Option<&SourceNodeStyle>,
    value: impl Fn(&SourceNodeStyle) -> Option<&String>,
) -> bool {
    style
        .and_then(value)
        .and_then(|value| d2_rs_core::parse_d2_bool(value))
        .unwrap_or(false)
}

// Pinned d2scenebuild/legend.go constants; measurement/placement are kept in
// legend_layout, independently from ordinary graph dimensions.
const LEGEND_PADDING: f64 = 20.0;
const LEGEND_ITEM_SPACING: f64 = 15.0;
const LEGEND_ICON_SIZE: f64 = 24.0;
const LEGEND_FONT_SIZE: f64 = 14.0;
const LEGEND_CORNER_PADDING: f64 = 10.0;

/// D2 Connection.GetLabelTopLeft uses the stored route control-point polyline,
/// the selected position and stroke width, then rounds the label top-left.
fn edge_label_bounds(
    route: &[Point],
    label: &d2_rs_core::EdgeLabel,
    stroke_width: f64,
    already_exported: bool,
) -> Option<Rect> {
    if route.is_empty() || !label.size.width.is_finite() || !label.size.height.is_finite() {
        return None;
    }
    let position = if label.position == LabelPosition::Unset {
        LabelPosition::InsideMiddleCenter
    } else {
        label.position
    };
    let target_route = route
        .iter()
        .copied()
        .map(|p| {
            if already_exported {
                p
            } else {
                target_route_point(p)
            }
        })
        .collect::<Vec<_>>();
    let origin = d2_rs_core::route_label::top_left(
        &target_route,
        position,
        stroke_width,
        if already_exported {
            label.percentage
        } else {
            f64::from(label.percentage as f32)
        },
        label.size,
    )?;
    Some(Rect {
        origin,
        size: label.size,
    })
}

/// D2 `Connection.GetArrowheadLabelPosition`, including its endpoint segment
/// normal, route-relative unlocked-top placement, minimum marker dimensions,
/// and deliberate integer truncation of measured label dimensions.
fn arrowhead_label_bounds(
    graph: &Graph,
    edge_id: EdgeId,
    route: &[Point],
    is_target: bool,
) -> Option<Rect> {
    let labels = graph.edge_arrowhead_labels(edge_id);
    let label = if is_target {
        labels.target.as_ref()?
    } else {
        labels.source.as_ref()?
    };
    if label.text.is_empty() || route.len() < 2 {
        return None;
    }

    let size_for_position = Size {
        width: label.size.width as i64 as f64,
        height: label.size.height as i64 as f64,
    };
    let target_route = route
        .iter()
        .copied()
        .map(|p| rendered_route_point(graph, p))
        .collect::<Vec<_>>();
    let segment_index = if is_target { target_route.len() - 2 } else { 0 };
    let start = target_route[segment_index];
    let end = target_route[segment_index + 1];
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let segment_length = (dx * dx + dy * dy).sqrt();
    if !segment_length.is_finite() || segment_length <= f64::EPSILON {
        return None;
    }
    // GetUnitNormalVector(end, start), the normal used by D2's unlocked-top
    // label position for the endpoint segment.
    let normal_x = dy / segment_length;
    let normal_y = -dx / segment_length;
    let shift = normal_x.abs() * (size_for_position.height / 2.0 + 5.0)
        + normal_y.abs() * (size_for_position.width / 2.0 + 5.0);
    let mut total_length = 0.0;
    for segment in target_route.windows(2) {
        let dx = segment[1].x - segment[0].x;
        let dy = segment[1].y - segment[0].y;
        total_length += if dx == 0.0 {
            dy.abs()
        } else if dy == 0.0 {
            dx.abs()
        } else {
            (dx * dx + dy * dy).sqrt()
        };
    }
    let percentage = if total_length > 0.0 {
        if is_target {
            1.0 - shift / total_length
        } else {
            shift / total_length
        }
    } else if is_target {
        1.0
    } else {
        0.0
    };
    let stroke = stroke_width(graph.edge_style(edge_id).stroke_width.as_deref());
    let mut origin = d2_rs_core::route_label::top_left(
        &target_route,
        LabelPosition::UnlockedTop,
        stroke,
        percentage,
        size_for_position,
    )?;

    let arrows = graph.edge_arrows(edge_id);
    let arrowheads = graph.edge_arrowheads(edge_id);
    let selected_arrowhead = if is_target
        && arrows.target
        && canonical_marker_name(arrowheads.target.as_deref()).is_some()
    {
        arrowhead_dimensions(arrowheads.target.as_deref(), stroke).1
    } else if arrows.source && canonical_marker_name(arrowheads.source.as_deref()).is_some() {
        arrowhead_dimensions(arrowheads.source.as_deref(), stroke).1
    } else {
        0.0
    };
    let offset = (selected_arrowhead / 2.0 + 2.0) - stroke / 2.0 - 5.0;
    if offset > 0.0 {
        origin.x += normal_x * offset;
        origin.y += normal_y * offset;
    }
    Some(Rect {
        origin,
        size: label.size,
    })
}

/// D2 `Arrowhead.Dimensions` returns the dimensions used to offset arrowhead
/// labels as well as to construct the visible marker.
fn arrowhead_dimensions(shape: Option<&str>, stroke: f64) -> (f64, f64) {
    let (base_width, base_height, width_multiplier, height_multiplier) = match shape
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("triangle") | Some("filled-triangle") => (4.0, 4.0, 3.0, 4.0),
        Some("arrow") => (4.0, 4.0, 4.0, 4.0),
        Some("unfilled-triangle") => (7.0, 7.0, 3.0, 4.0),
        Some("line") => (0.0, 0.0, 5.0, 8.0),
        Some("filled-diamond") => (11.0, 7.0, 5.5, 3.5),
        Some("diamond") => (11.0, 9.0, 5.5, 4.5),
        Some("cross") => (7.0, 7.0, 5.0, 5.0),
        Some("filled-circle" | "circle") => (8.0, 8.0, 5.0, 5.0),
        Some("filled-box" | "box") => (6.0, 6.0, 5.0, 5.0),
        Some("cf-one" | "cf-many" | "cf-one-required" | "cf-many-required") => (9.0, 9.0, 4.5, 4.5),
        Some("none") | Some(_) => return (0.0, 0.0),
    };
    let stroke = stroke.max(2.0);
    (
        base_width + stroke * width_multiplier,
        base_height + stroke * height_multiplier,
    )
}

const EDGE_ICON_SIZE: f64 = 32.0;
const EDGE_ICON_LABEL_GAP: f64 = 8.0;

/// Returns the target-space rectangle occupied by an edge icon. D2 keeps an
/// icon immediately to the left of a visible label; with no label it samples
/// the requested route position and centers the 32px asset there.
fn edge_icon_rect(
    graph: &Graph,
    result: &LayoutResult,
    edge_id: EdgeId,
    route: &[Point],
    dx: f64,
    dy: f64,
) -> Option<Rect> {
    graph.edge_icon(edge_id)?;
    if let Some(label) =
        effective_edge_label(graph, result, edge_id).filter(|label| !label.text.is_empty())
        && let Some(label_rect) = edge_label_bounds(
            route,
            &label,
            stroke_width(graph.edge_style(edge_id).stroke_width.as_deref()),
            graph.target_routes_exported,
        )
    {
        return Some(Rect {
            origin: Point {
                x: label_rect.origin.x + dx - EDGE_ICON_LABEL_GAP - EDGE_ICON_SIZE,
                y: label_rect.origin.y + dy + (label_rect.size.height - EDGE_ICON_SIZE) / 2.0,
            },
            size: Size {
                width: EDGE_ICON_SIZE,
                height: EDGE_ICON_SIZE,
            },
        });
    }
    // Connection.GetIconPosition uses the exported route and the same
    // positional/normal/chopPrecision rules as route labels, not a raw midpoint.
    let target_route = route
        .iter()
        .copied()
        .map(|p| rendered_route_point(graph, p))
        .collect::<Vec<_>>();
    let origin = d2_rs_core::route_label::top_left(
        &target_route,
        graph
            .edge_icon_position(edge_id)
            .unwrap_or(LabelPosition::InsideMiddleCenter),
        stroke_width(graph.edge_style(edge_id).stroke_width.as_deref()),
        -1.0,
        Size {
            width: EDGE_ICON_SIZE,
            height: EDGE_ICON_SIZE,
        },
    )?;
    Some(Rect {
        origin: Point {
            x: origin.x + dx,
            y: origin.y + dy,
        },
        size: Size {
            width: EDGE_ICON_SIZE,
            height: EDGE_ICON_SIZE,
        },
    })
}

/// Merge the layout engine's measured/percentage data with an authored label
/// position. Dagre normalizes an unset position to its middle placement, but
/// an explicit `label-position` remains a renderer instruction and must win
/// at the target boundary.
fn effective_edge_label(
    graph: &Graph,
    result: &LayoutResult,
    edge_id: EdgeId,
) -> Option<d2_rs_core::EdgeLabel> {
    let mut label = result
        .edge_labels
        .get(&edge_id)
        .cloned()
        .or_else(|| graph.edge_label(edge_id).cloned())?;
    if let Some(authored) = graph.edge_label(edge_id)
        && authored.position != LabelPosition::Unset
    {
        label.position = authored.position;
    }
    Some(label)
}

fn bounds(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> (f64, f64, f64, f64) {
    bounds_with_markdown_family(graph, result, labels, styles, MarkdownFamily::Sans)
}

fn bounds_with_markdown_family(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    family: MarkdownFamily,
) -> (f64, f64, f64, f64) {
    bounds_with_markdown_family_and_legend(graph, result, labels, styles, family, true)
}

/// Original target Diagram.BoundingBox, excluding legend and canvas/root padding.
/// Animation unions these target-space bounds before applying its own padding.
pub fn target_diagram_bounds(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> [f64; 4] {
    if graph.nodes.is_empty() {
        return [0.0; 4];
    }
    let family = if options
        .theme_id
        .and_then(theme::palette)
        .is_some_and(|p| p.special_rules.mono)
    {
        MarkdownFamily::Mono
    } else if sketch {
        MarkdownFamily::HandDrawn
    } else {
        MarkdownFamily::Sans
    };
    let (left, top, right, bottom) =
        bounds_with_markdown_family_and_legend(graph, result, labels, styles, family, false);
    [left, top, right, bottom]
}

// Source Diagram.BoundingBox excludes the separately painted legend card.
// Both canvas and static appendix consume the same target geometry, but only
// the canvas grows to include the legend.
fn bounds_with_markdown_family_and_legend(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    family: MarkdownFamily,
    include_legend: bool,
) -> (f64, f64, f64, f64) {
    let mut min_x: f64 = 0.0;
    let mut min_y: f64 = 0.0;
    let mut max_x: f64 = 0.0;
    let mut max_y: f64 = 0.0;
    let mut initialized = false;
    for (node_id, rect) in &result.boxes {
        let Some(node) = graph.node(*node_id) else {
            continue;
        };
        // D2 computes Diagram.BoundingBox from integer target shapes, not
        // fractional layout boxes. Use the same truncated rectangle that the
        // renderer emits before adding stroke and decoration extents; otherwise
        // half-pixel origins can add an extra pixel to each canvas edge.
        let target_rect = rounded_render_rect(*rect);
        let style = styles.get(node_id);
        // SVG strokes are centered on the target geometry. D2's scene bounds
        // retain half the default 2px stroke on each side, which contributes
        // one pixel to the padded canvas even when no route extends there.
        let stroke_width = style
            .and_then(|style| style.stroke_width.as_deref())
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|width| width.is_finite() && *width >= 0.0)
            .unwrap_or_else(|| {
                if graph.is_sequence_diagram(*node_id) || is_sequence_group_like(graph, *node_id) {
                    0.0
                } else {
                    2.0
                }
            });
        let stroke_half = stroke_width / 2.0;
        let left = target_rect.origin.x - stroke_half;
        let mut top = target_rect.origin.y - stroke_half;
        let mut right = target_rect.right() + stroke_half;
        let mut bottom = target_rect.bottom() + stroke_half;
        if node.shape == ShapeKind::C4Person {
            // d2target.BoundingBox extends above the nominal C4 person box
            // when its width-derived head reaches above the height-derived center.
            let head_radius = (target_rect.size.width * 0.22).trunc();
            let head_center_y = (target_rect.size.height * 0.18).trunc();
            top = top.min(target_rect.origin.y + head_center_y - head_radius - stroke_width);
        }
        // The pinned target reserves the full stroke width beyond the rear
        // face of 3D and multiple silhouettes, even though ordinary shape
        // bounds use ceil(strokeWidth / 2) on each side.
        if style_flag(style, |style| style.three_dee.as_ref()) {
            let offset_y = if node.shape == ShapeKind::Hexagon {
                7.0
            } else {
                15.0
            };
            top = top.min(target_rect.origin.y - offset_y - stroke_width);
            right = right.max(target_rect.right() + 15.0 + stroke_width);
        }
        if style_flag(style, |style| style.multiple.as_ref()) {
            top = top.min(target_rect.origin.y - 10.0 - stroke_width);
            right = right.max(target_rect.right() + 10.0 + stroke_width);
        }
        // d2target.Diagram.BoundingBox reserves the translated shadow's
        // right and bottom extents in addition to the normal shape stroke.
        // The SVG filter offsets shadows by (+3,+5), so leaving this out
        // crops that portion and makes the canvas smaller than v0.9.0.
        if style_flag(style, |style| style.shadow.as_ref()) {
            let shadow_stroke_extent = (stroke_width / 2.0).ceil();
            right = right.max(target_rect.right() + shadow_stroke_extent + D2_TARGET_SHADOW_SIZE_X);
            bottom =
                bottom.max(target_rect.bottom() + shadow_stroke_extent + D2_TARGET_SHADOW_SIZE_Y);
        }
        // d2target.BoundingBox reserves only the side on which an outside
        // icon sits, including image shapes whose icon is the image itself.
        let icon_bounds = style
            .and_then(|style| style.icon.as_deref())
            .filter(|icon| !icon.is_empty())
            .and_then(|_| {
                outside_icon_envelope(
                    target_rect,
                    node.shape,
                    result
                        .node_labels
                        .get(node_id)
                        .and_then(|state| state.icon_position)
                        .or(node.icon_position)
                        .unwrap_or(LabelPosition::InsideMiddleCenter),
                )
            });
        let external_bounds = graph
            .node(*node_id)
            .filter(|_| {
                result
                    .node_labels
                    .get(node_id)
                    .map(|state| state.position.is_outside())
                    .unwrap_or_else(|| {
                        graph
                            .node(*node_id)
                            .is_some_and(|node| node.label_position.is_outside())
                    })
            })
            .and_then(|node| node.external_label)
            .map(|mut label| {
                // d2exporter copies the post-layout LabelDimensions. The
                // compiler's external decoration may still hold pre-scale
                // dimensions, so use the live target size for bounds as well.
                label.size = result
                    .node_labels
                    .get(node_id)
                    .and_then(|state| state.size)
                    .or(node.label_size)
                    .unwrap_or(label.size);
                label
            })
            .map(|label| {
                let position = result
                    .node_labels
                    .get(node_id)
                    .map(|state| state.position)
                    .unwrap_or(node.label_position);
                let label = target_modifier_label_bounds(
                    Rect {
                        origin: label.at_position(position).top_left(target_rect, 5.0),
                        size: label.size,
                    },
                    node.shape,
                    position,
                    style_flag(style, |style| style.three_dee.as_ref()),
                );
                // d2target.Diagram.BoundingBox expands by int(labelTL) and
                // the int label dimensions: truncation toward zero, as below.
                Rect {
                    origin: Point {
                        x: label.origin.x as i64 as f64,
                        y: label.origin.y as i64 as f64,
                    },
                    size: Size {
                        width: label.size.width as i64 as f64,
                        height: label.size.height as i64 as f64,
                    },
                }
            });
        // d2target.Diagram.BoundingBox reserves the top and right using
        // strokeWidth +16 regardless of badge count or traced centers. It
        // does not grow the left/bottom to contain badges on compact shapes.
        let badge_bounds = if style
            .and_then(|style| style.link.as_deref())
            .is_some_and(|value| !value.is_empty())
            || style
                .and_then(|style| style.tooltip.as_deref())
                .is_some_and(|value| !value.is_empty())
        {
            if let Some(style) = style.filter(|style| {
                style
                    .tooltip_position
                    .as_deref()
                    .is_some_and(|position| !position.is_empty())
            }) {
                // Source's positioned branch excludes the tail and link
                // badge; absent tooltip text supplies a zero bounds tuple.
                let rect = positioned_tooltip_rect(target_rect, style, family).unwrap_or_default();
                Some(Rect {
                    origin: Point {
                        x: rect.origin.x.floor(),
                        y: rect.origin.y.floor(),
                    },
                    size: Size {
                        width: rect.origin.x.ceil() + rect.size.width - rect.origin.x.floor(),
                        height: rect.origin.y.ceil() + rect.size.height - rect.origin.y.floor(),
                    },
                })
            } else {
                let extension = stroke_width + 16.0;
                Some(Rect {
                    origin: Point {
                        x: target_rect.right(),
                        y: target_rect.origin.y - extension,
                    },
                    size: Size {
                        width: extension,
                        height: extension,
                    },
                })
            }
        } else {
            None
        };
        let (left, top, right, bottom) = [icon_bounds, external_bounds, badge_bounds]
            .into_iter()
            .flatten()
            .fold(
                (left, top, right, bottom),
                |(left, top, right, bottom), decoration| {
                    (
                        left.min(decoration.origin.x),
                        top.min(decoration.origin.y),
                        right.max(decoration.right()),
                        bottom.max(decoration.bottom()),
                    )
                },
            );
        // d2target.Diagram.BoundingBox includes ordinary node labels in the
        // scene bounds.  This matters for outside-top container labels: their
        // box sits 40px above a typical 28px node label baseline, so omitting
        // it shortens the exported canvas while leaving node geometry intact.
        let label_bounds = if !matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable)
            && !is_sequence_span_like(graph, *node_id)
            && labels.get(node_id).is_some_and(|label| !label.is_empty())
        {
            let label_size = result
                .node_labels
                .get(node_id)
                .and_then(|state| state.size)
                .or(node.label_size);
            label_size.map(|size| {
                let position = result
                    .node_labels
                    .get(node_id)
                    .map(|state| state.position)
                    .unwrap_or(node.label_position);
                let rendered_position = if styles.get(node_id).is_some_and(|style| style.markdown)
                    && position == LabelPosition::Unset
                {
                    LabelPosition::InsideMiddleCenter
                } else {
                    position
                };
                target_modifier_label_bounds(
                    node_label_box(
                        target_rect,
                        node.shape,
                        rendered_position,
                        size,
                        cloud_content_aspect(node, size),
                    ),
                    node.shape,
                    rendered_position,
                    style_flag(style, |style| style.three_dee.as_ref()),
                )
            })
        } else {
            None
        };
        let (left, top, right, bottom) = label_bounds.map_or((left, top, right, bottom), |label| {
            // d2target.Diagram.BoundingBox casts both the label top-left and
            // its measured dimensions to ints before expanding the canvas.
            // Go truncates negative fractional coordinates toward zero, so
            // flooring this rectangle later would add an extra pixel above
            // the diagram (for example, -5.5 becomes -5 in the target).
            let label_left = label.origin.x as i64 as f64;
            let label_top = label.origin.y as i64 as f64;
            let label_width = label.size.width as i64 as f64;
            let label_height = label.size.height as i64 as f64;
            (
                left.min(label_left),
                top.min(label_top),
                right.max(label_left + label_width),
                bottom.max(label_top + label_height),
            )
        });
        min_x = if initialized { min_x.min(left) } else { left };
        min_y = if initialized { min_y.min(top) } else { top };
        max_x = if initialized { max_x.max(right) } else { right };
        max_y = if initialized {
            max_y.max(bottom)
        } else {
            bottom
        };
        initialized = true;
    }
    for (edge_id, route) in &result.routes {
        let stroke_half =
            (stroke_width(graph.edge_style(*edge_id).stroke_width.as_deref()) / 2.0).ceil();
        for point in route {
            let point = rendered_route_point(graph, *point);
            min_x = if initialized {
                min_x.min(point.x.floor() - stroke_half)
            } else {
                point.x.floor() - stroke_half
            };
            min_y = if initialized {
                min_y.min(point.y.floor() - stroke_half)
            } else {
                point.y.floor() - stroke_half
            };
            max_x = if initialized {
                max_x.max(point.x.ceil() + stroke_half)
            } else {
                point.x.ceil() + stroke_half
            };
            max_y = if initialized {
                max_y.max(point.y.ceil() + stroke_half)
            } else {
                point.y.ceil() + stroke_half
            };
            initialized = true;
        }
    }
    // Connection labels are target-stage decorations. Include their final
    // placement (or the graph's pre-layout metadata for manually supplied
    // results) in the canvas bounds just as D2's scene builder includes its
    // label mask. This is especially visible for link-only edges whose URL
    // can be wider than either endpoint node.
    for (edge_id, _) in graph.edges() {
        let Some(label) = effective_edge_label(graph, result, edge_id) else {
            continue;
        };
        let Some(route) = result.routes.get(&edge_id) else {
            continue;
        };
        let Some(label_rect) = edge_label_bounds(
            route,
            &label,
            stroke_width(graph.edge_style(edge_id).stroke_width.as_deref()),
            graph.target_routes_exported,
        ) else {
            continue;
        };
        min_x = if initialized {
            min_x.min(label_rect.origin.x)
        } else {
            label_rect.origin.x
        };
        min_y = if initialized {
            min_y.min(label_rect.origin.y)
        } else {
            label_rect.origin.y
        };
        max_x = if initialized {
            max_x.max(label_rect.right())
        } else {
            label_rect.right()
        };
        max_y = if initialized {
            max_y.max(label_rect.bottom())
        } else {
            label_rect.bottom()
        };
        initialized = true;
    }
    for (edge_id, _) in graph.edges() {
        let Some(route) = result.routes.get(&edge_id) else {
            continue;
        };
        let labels = graph.edge_arrowhead_labels(edge_id);
        for (is_target, label) in [
            (false, labels.source.as_ref()),
            (true, labels.target.as_ref()),
        ] {
            let Some(label) = label.filter(|label| !label.text.is_empty()) else {
                continue;
            };
            let Some(label_rect) = arrowhead_label_bounds(graph, edge_id, route, is_target) else {
                continue;
            };
            // d2target.Diagram.BoundingBox converts the arrowhead label's
            // top-left and dimensions to integers before expanding the
            // canvas. Go's int conversion truncates toward zero.
            let left = label_rect.origin.x as i64 as f64;
            let top = label_rect.origin.y as i64 as f64;
            let right = left + label.size.width as i64 as f64;
            let bottom = top + label.size.height as i64 as f64;
            min_x = if initialized { min_x.min(left) } else { left };
            min_y = if initialized { min_y.min(top) } else { top };
            max_x = if initialized { max_x.max(right) } else { right };
            max_y = if initialized {
                max_y.max(bottom)
            } else {
                bottom
            };
            initialized = true;
        }
    }
    for lifeline in sequence_lifelines(graph, result) {
        let lifeline = SequenceLifeline {
            start: rendered_route_point(graph, lifeline.start),
            end: rendered_route_point(graph, lifeline.end),
            ..lifeline
        };

        // d2sequence adds real target connections with 2px strokes. Their
        // bounds are scoped to their own sequence's messages and notes;
        // unrelated outer edges cannot lengthen an actor's lifeline.
        for point in [lifeline.start, lifeline.end] {
            min_x = min_x.min(point.x.floor() - 1.0);
            min_y = min_y.min(point.y.floor() - 1.0);
            max_x = max_x.max(point.x.ceil() + 1.0);
            max_y = max_y.max(point.y.ceil() + 1.0);
        }
    }
    // The source target bounds always exclude the legend. Its layout expands
    // the padded scene viewbox later, before root stroke expansion.
    let _ = include_legend;
    // d2target publishes integer bounds: route points are floored/ceiled and
    // shape strokes use a ceiled half-width. Quantize only at this target
    // boundary; layout coordinates and label placement retain full precision.
    let bounds = (min_x.floor(), min_y.floor(), max_x.ceil(), max_y.ceil());
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_TRACE_RENDER_BOUNDS").is_some() {
        eprintln!(
            "D2_RS_RENDER_BOUNDS min=({:.17e},{:.17e}) max=({:.17e},{:.17e}) final={bounds:?}",
            min_x, min_y, max_x, max_y
        );
        for (node_id, rect) in &result.boxes {
            let Some(node) = graph.node(*node_id) else {
                continue;
            };
            let icon_position = result
                .node_labels
                .get(node_id)
                .and_then(|state| state.icon_position)
                .or(node.icon_position)
                .unwrap_or(LabelPosition::InsideMiddleCenter);
            let style = styles.get(node_id);
            eprintln!(
                "D2_RS_RENDER_NODE_BOUNDS node={} external_id={:?} shape={:?} rect={rect:?} icon={:?} has_icon={} icon_position={icon_position:?} icon_bounds={:?} labels={:?}",
                node_id.0,
                node.external_id,
                node.shape,
                style.and_then(|style| style.icon.as_deref()),
                node.has_icon,
                style
                    .and_then(|style| style.icon.as_deref())
                    .and_then(|_| outside_icon_envelope(
                        rounded_render_rect(*rect),
                        node.shape,
                        icon_position
                    )),
                result.node_labels.get(node_id),
            );
        }
        for (edge_id, route) in &result.routes {
            for is_target in [false, true] {
                if let Some(rect) = arrowhead_label_bounds(graph, *edge_id, route, is_target) {
                    eprintln!(
                        "D2_RS_RENDER_ARROWHEAD_LABEL edge={} target={} rect=({:.17e},{:.17e},{:.17e},{:.17e})",
                        edge_id.0,
                        is_target,
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height
                    );
                }
            }
        }
    }
    bounds
}

fn draw_legend(
    svg: &mut String,
    legend: &Legend,
    bounds: (f64, f64, f64, f64),
    dx: f64,
    dy: f64,
    palette: PaletteView<'_>,
    precision: bool,
    sketch: bool,
    streaks: &mut BTreeSet<&'static str>,
    raster_sources: Option<&mut Vec<RasterPathSource>>,
) {
    draw_legend_with_target(
        svg,
        legend,
        bounds,
        dx,
        dy,
        palette,
        precision,
        sketch,
        streaks,
        raster_sources,
        None,
    );
}

fn draw_legend_with_target(
    svg: &mut String,
    legend: &Legend,
    bounds: (f64, f64, f64, f64),
    dx: f64,
    dy: f64,
    palette: PaletteView<'_>,
    precision: bool,
    sketch: bool,
    streaks: &mut BTreeSet<&'static str>,
    mut raster_sources: Option<&mut Vec<RasterPathSource>>,
    target: Option<&target::PaintContext<'_>>,
) {
    let Some(layout) = legend_layout::measure(legend, bounds, palette.base.special_rules.mono)
    else {
        return;
    };
    let legend_width = layout.width;
    let legend_height = layout.height;
    let x = layout.x + dx;
    let y = layout.y + dy;
    let card_fill = "#ffffff";
    let card_stroke = "#DEE1EB";
    if target.is_some() {
        // d2svg.RenderLegend filters the shadow rectangle itself using CSS.
        // A group filter changes the filter region and edge rasterization.
        let _ = write!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" stroke=\"#DEE1EB\" fill=\"#F7F7FA\" style=\"stroke-width: 1px; filter: drop-shadow(0px 2px 3px rgba(0, 0, 0, 0.1))\"/><g class=\"d2-rs-legend\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" stroke=\"#DEE1EB\" fill=\"#ffffff\" style=\"stroke-width: 1px\"/>",
            number(x),
            number(y),
            number(legend_width),
            number(legend_height),
            number(x),
            number(y),
            number(legend_width),
            number(legend_height)
        );
    } else {
        let carrier = if precision {
            " id=\"d2-rs-native-legend-shadow-card\" data-d2-rs-legend-shadow=\"scene\""
        } else {
            ""
        };
        let _ = writeln!(
            svg,
            "<defs><filter id=\"d2-rs-legend-shadow\" x=\"-100%\" y=\"-100%\" width=\"300%\" height=\"300%\" color-interpolation-filters=\"sRGB\"><feDropShadow dx=\"0\" dy=\"2\" stdDeviation=\"3\" flood-color=\"#000000\" flood-opacity=\"0.10196078431372549\"/></filter></defs><g{carrier} filter=\"url(#d2-rs-legend-shadow)\"><rect id=\"d2-rs-native-legend-shadow-rect\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" fill=\"#F7F7FA\" stroke=\"#DEE1EB\" stroke-width=\"1\"/></g><g class=\"d2-rs-legend\"><rect id=\"d2-rs-native-legend-panel\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
            number(x),
            number(y),
            number(legend_width),
            number(legend_height),
            number(x),
            number(y),
            number(legend_width),
            number(legend_height),
            escape(card_fill),
            escape(card_stroke)
        );
    }
    let title = if legend.label.is_empty() {
        "Legend"
    } else {
        &legend.label
    };
    legend_layout::draw_text_with_target(
        svg,
        title,
        x + LEGEND_PADDING,
        y + LEGEND_PADDING + LEGEND_FONT_SIZE,
        16,
        true,
        palette.base.special_rules.mono,
        "title",
        precision,
        target.is_some(),
    );
    let mut current_y = y + LEGEND_PADDING * 2.0 + LEGEND_FONT_SIZE;
    let icon_x = x + LEGEND_PADDING;
    let text_x = icon_x + LEGEND_ICON_SIZE + LEGEND_PADDING;
    let mut shape_count = 0;
    for (index, shape) in legend
        .shapes
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.label.is_empty())
    {
        if let Some(target) = target {
            let _ = write!(
                svg,
                "<g transform=\"translate({} {}) scale(0.2)\">{}</g>",
                number(icon_x),
                number(current_y),
                target.legend_shape(index)
            );
        } else {
            legend_shape::draw(
                svg, legend, shape, index, icon_x, current_y, palette, precision, sketch, streaks,
            );
        }
        let dimensions = layout.shapes[index];
        let row_height = dimensions.height.max(LEGEND_ICON_SIZE);
        let baseline = current_y + (row_height / 2.0).trunc() + (dimensions.height * 0.3).trunc();
        legend_layout::draw_text_with_target(
            svg,
            &shape.label,
            text_x,
            baseline,
            14,
            false,
            palette.base.special_rules.mono,
            &format!("shape-{index}"),
            precision,
            target.is_some(),
        );
        current_y += row_height + LEGEND_ITEM_SPACING;
        shape_count += 1;
    }
    if shape_count > 0 && !legend.connections.is_empty() {
        current_y += (LEGEND_ITEM_SPACING / 2.0).trunc();
        let _ = writeln!(
            svg,
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-dasharray=\"2,2\" />",
            number(icon_x),
            number(current_y),
            number(x + legend_width - LEGEND_PADDING),
            number(current_y),
            escape(&card_stroke),
        );
        current_y += LEGEND_ITEM_SPACING;
    }
    for (index, connection) in legend
        .connections
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.label.is_empty())
    {
        if let Some(target) = target {
            let _ = write!(
                svg,
                "<g transform=\"translate({} {}) scale(0.5)\">{}</g>",
                number(icon_x),
                number(current_y + LEGEND_ICON_SIZE / 2.0),
                target.legend_connection(index)
            );
        } else {
            legend_connection::draw(
                svg,
                connection,
                index,
                icon_x,
                current_y + LEGEND_ICON_SIZE / 2.0,
                palette,
                precision,
                raster_sources.as_deref_mut(),
            );
        }
        let dimensions = layout.connections[index];
        let row_height = dimensions.height.max(LEGEND_ICON_SIZE);
        let baseline = current_y + (row_height / 2.0).trunc() + (dimensions.height * 0.2).trunc();
        legend_layout::draw_text_with_target(
            svg,
            &connection.label,
            text_x,
            baseline,
            14,
            false,
            palette.base.special_rules.mono,
            &format!("connection-{index}"),
            precision,
            target.is_some(),
        );
        current_y += row_height + LEGEND_ITEM_SPACING;
    }
    svg.push_str("</g>\n");
}

fn draw_sequence_lifelines(
    graph: &Graph,
    result: &LayoutResult,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    svg: &mut String,
    dx: f64,
    dy: f64,
    palette: PaletteView<'_>,
    diagram_mask_id: Option<&str>,
    scope: Option<NodeId>,
    native_sketch: bool,
    svg_sketch: bool,
    mut raster_sources: Option<&mut Vec<RasterPathSource>>,
) {
    let default_stroke = palette.color("B2", palette.base.b2);
    for lifeline in sequence_lifelines_in_scope(graph, result, scope) {
        // Synthetic lifelines are ordinary exported connections in Go. Their
        // raw layout points cross the same decimal/float32 boundary once.
        let lifeline = SequenceLifeline {
            start: rendered_route_point(graph, lifeline.start),
            end: rendered_route_point(graph, lifeline.end),
            ..lifeline
        };

        let style = styles.get(&lifeline.actor);
        let source_shape_stroke_width = style
            .and_then(|style| style.stroke_width.as_deref())
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(2)
            .max(0) as f64;
        // d2svg.getArrowheadAdjustments trims every endpoint by half the
        // connection and endpoint-shape strokes, even for a no-arrow edge.
        // A synthetic lifeline end has no target shape, so its target trim is
        // half of the 2px connection stroke; its actor start also includes
        // half of the actor outline.
        let start_y = lifeline.start.y + (2.0 + source_shape_stroke_width) / 2.0;
        let end_y = lifeline.end.y - 1.0;
        let stroke = themed_color(
            style.and_then(|style| style.stroke.as_deref()),
            &default_stroke,
            palette,
        );
        let stroke_class = palette
            .semantic_code(style.and_then(|style| style.stroke.as_deref()), "B2")
            .map(|code| format!(" class=\"d2-rs-stroke-{code}\""))
            .unwrap_or_default();
        let dash = style
            .and_then(|style| style.stroke_dash.as_deref())
            .unwrap_or("6");
        let dash = stroke_dasharray("2", Some(dash)).unwrap_or_else(|| dash.to_owned());
        let exact_dash_id = stroke_dash_values(
            "2",
            style
                .and_then(|style| style.stroke_dash.as_deref())
                .or(Some("6")),
        )
        .map(|(dash_size, gap_size)| {
            format!(
                "d2rs-exact-dash-{:016x}-{:016x}-{:016x}-{:08x}",
                dash_size.to_bits(),
                gap_size.to_bits(),
                0.0_f64.to_bits(),
                lifeline.actor.0,
            )
        });
        let native_path_id = exact_dash_id.clone();
        let exact_dash_id = exact_dash_id
            .map(|id| format!(" id=\"{id}\""))
            .unwrap_or_default();
        let mask = diagram_mask_id
            .map(|id| format!(" mask=\"url(#{id})\""))
            .unwrap_or_default();
        if native_sketch {
            let path = sketch_connection::route(&[
                RasterPathCommand::MoveTo(Point {
                    x: lifeline.start.x + dx,
                    y: start_y + dy,
                }),
                RasterPathCommand::LineTo(Point {
                    x: lifeline.end.x + dx,
                    y: end_y + dy,
                }),
            ])
            .expect("lifeline has one painted segment");
            if let Some(sources) = raster_sources.as_deref_mut()
                && let Some(id) = &native_path_id
            {
                sources.push(RasterPathSource {
                    id: id.clone(),
                    commands: path.clone(),
                    transform: None,
                });
            }
            let _ = writeln!(
                svg,
                "<path{} d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" stroke-dasharray=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{}{} />",
                exact_dash_id,
                raster_path_data(&path),
                escape(&stroke),
                escape(&dash),
                stroke_class,
                mask
            );
            continue;
        }
        if svg_sketch {
            let data = format!(
                "M{} {} L{} {}",
                number(lifeline.start.x + dx),
                number(start_y + dy),
                number(lifeline.end.x + dx),
                number(end_y + dy)
            );
            let class = if stroke_class.is_empty() {
                " class=\"connection\"".to_owned()
            } else {
                stroke_class.replacen("class=\"", "class=\"connection ", 1)
            };
            let _ = writeln!(
                svg,
                "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" stroke-dasharray=\"{}\"{}{} />",
                sketch_svg::route(&data),
                escape(&stroke),
                escape(&dash),
                class,
                mask
            );
            continue;
        }
        let _ = writeln!(
            svg,
            "<line{} x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"2\" stroke-dasharray=\"{}\" stroke-linecap=\"round\"{}{} />",
            exact_dash_id,
            number(lifeline.start.x + dx),
            number(start_y + dy),
            number(lifeline.end.x + dx),
            number(end_y + dy),
            escape(&stroke),
            escape(&dash),
            stroke_class,
            mask,
        );
    }
}

struct SequenceLifeline {
    actor: NodeId,
    start: Point,
    end: Point,
}

/// Reconstruct d2sequence.addLifelineEdges at each extracted scope boundary.
/// These synthetic connections are not stored in the shared Graph edge arena.
fn sequence_lifelines(graph: &Graph, result: &LayoutResult) -> Vec<SequenceLifeline> {
    let mut scopes = Vec::new();
    if graph.root_sequence_diagram {
        scopes.push(None);
    }
    scopes.extend(
        graph
            .nodes()
            .filter_map(|(id, _)| graph.is_sequence_diagram(id).then_some(Some(id))),
    );
    scopes
        .into_iter()
        .flat_map(|scope| sequence_lifelines_in_scope(graph, result, scope))
        .collect()
}

fn sequence_lifelines_in_scope(
    graph: &Graph,
    result: &LayoutResult,
    scope: Option<NodeId>,
) -> Vec<SequenceLifeline> {
    if !result.sequence_lifelines.is_empty() {
        return result
            .sequence_lifelines
            .iter()
            .filter_map(|line| {
                if graph.node(line.actor)?.parent != scope {
                    return None;
                }
                Some(SequenceLifeline {
                    actor: line.actor,
                    start: *line.route.first()?,
                    end: *line.route.last()?,
                })
            })
            .collect();
    }
    let actors = sequence_actor_ids(graph, scope);
    if actors.is_empty() {
        return Vec::new();
    }
    let mut lifelines = Vec::new();
    let mut end_y = actors
        .iter()
        .filter_map(|actor| result.boxes.get(actor).map(|rect| rect.bottom()))
        .fold(f64::NEG_INFINITY, f64::max);
    // Use the final edge in the same filtered sequence-message set that the
    // layout adapter consumes, including a self-message's lower elbow.
    let mut messages = graph
        .edges()
        .filter_map(|(edge_id, edge)| {
            if graph.is_sequence_diagram(edge.source)
                || graph.is_sequence_diagram(edge.target)
                || outer_sequence_scope(graph, edge.source) != scope
                || outer_sequence_scope(graph, edge.target) != scope
            {
                return None;
            }
            let source_actor = sequence_message_actor(graph, edge.source, scope)?;
            let target_actor = sequence_message_actor(graph, edge.target, scope)?;
            (!sequence_edge_is_in_extracted_grid(graph, edge, source_actor, target_actor))
                .then_some((edge_id, edge))
        })
        .collect::<Vec<_>>();
    d2_rs_core::go_sort::sort_by(&mut messages, |left, right| {
        graph
            .sequence_edge_lines
            .get(&left.0)
            .copied()
            .unwrap_or(i32::MAX as i64)
            < graph
                .sequence_edge_lines
                .get(&right.0)
                .copied()
                .unwrap_or(i32::MAX as i64)
    });
    let last_message = messages.last().copied();
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_RS_DEBUG_SEQUENCE_LIFELINES").is_some() {
        let scope_name = scope
            .and_then(|id| graph.node(id))
            .map(|node| node.external_id.as_str())
            .unwrap_or("<root>");
        let selected = last_message.map(|(edge_id, edge)| {
            let source = graph
                .node(edge.source)
                .map(|node| node.external_id.as_str())
                .unwrap_or("?");
            let target = graph
                .node(edge.target)
                .map(|node| node.external_id.as_str())
                .unwrap_or("?");
            let max_y = result
                .routes
                .get(&edge_id)
                .into_iter()
                .flatten()
                .map(|point| point.y)
                .fold(f64::NEG_INFINITY, f64::max);
            format!("{}: {source} -> {target}, max_y={max_y}", edge_id.0)
        });
        eprintln!(
            "D2_RS_SEQUENCE_LIFELINE scope={scope_name:?} actors={} selected={selected:?} actor_end_y={end_y}",
            actors.len(),
        );
        for (edge_id, edge) in graph.edges().filter(|(_, edge)| {
            graph
                .node(edge.source)
                .is_some_and(|node| node.external_id.starts_with("l.here.grid"))
                || graph
                    .node(edge.target)
                    .is_some_and(|node| node.external_id.starts_with("l.here.grid"))
        }) {
            let source = graph
                .node(edge.source)
                .map(|node| node.external_id.as_str())
                .unwrap_or("?");
            let target = graph
                .node(edge.target)
                .map(|node| node.external_id.as_str())
                .unwrap_or("?");
            eprintln!(
                "D2_RS_SEQUENCE_EDGE edge={} {source} -> {target} source_scope={:?} target_scope={:?}",
                edge_id.0,
                outer_sequence_scope(graph, edge.source)
                    .and_then(|id| graph.node(id))
                    .map(|node| node.external_id.as_str()),
                outer_sequence_scope(graph, edge.target)
                    .and_then(|id| graph.node(id))
                    .map(|node| node.external_id.as_str()),
            );
        }
    }
    if let Some((edge_id, _)) = last_message
        && let Some(route) = result.routes.get(&edge_id)
    {
        for point in route {
            end_y = end_y.max(point.y);
        }
    }
    for (id, _) in graph.nodes() {
        if outer_sequence_scope(graph, id) == scope
            && is_sequence_note_like(graph, id)
            && let Some(rect) = result.boxes.get(&id)
        {
            end_y = end_y.max(rect.bottom());
        }
    }
    if !end_y.is_finite() {
        return lifelines;
    }
    end_y += 70.0; // MIN_MESSAGE_DISTANCE + VERTICAL_PAD
    for actor in actors {
        let Some(rect) = result.boxes.get(&actor) else {
            continue;
        };
        let Some(node) = graph.node(actor) else {
            continue;
        };
        let label_state = result.node_labels.get(&actor);
        let position = label_state
            .map(|state| state.position)
            .unwrap_or(node.label_position);
        let size = label_state.and_then(|state| state.size).or(node.label_size);
        let label_pad = if position == LabelPosition::OutsideBottomCenter {
            size.map_or(0.0, |size| size.height + 5.0)
        } else {
            0.0
        };
        lifelines.push(SequenceLifeline {
            actor,
            start: Point {
                x: rect.center().x,
                y: rect.bottom() + label_pad,
            },
            end: Point {
                x: rect.center().x,
                y: end_y,
            },
        });
    }
    lifelines
}

/// Resolves a message endpoint to the actor directly owned by its sequence
/// scope. Descendants inside an actor still belong to that actor, while edge
/// group vessels resolve through their member name to the corresponding
/// actor, matching d2sequence's directActor behavior.
fn sequence_message_actor(graph: &Graph, node_id: NodeId, scope: Option<NodeId>) -> Option<NodeId> {
    if scope == Some(node_id) {
        return None;
    }
    let mut current = node_id;
    loop {
        let parent = graph.node(current)?.parent;
        if parent == scope || (scope.is_none() && parent.is_none()) {
            if !is_sequence_group_like(graph, current) {
                return (!graph.is_sequence_diagram(current)).then_some(current);
            }

            let group = current;
            let mut member = Some(node_id);
            while let Some(candidate) = member {
                if candidate == group {
                    break;
                }
                let terminal = graph
                    .node(candidate)
                    .and_then(|node| node.external_id.rsplit('.').next());
                if let Some(terminal) = terminal
                    && let Some(actor) =
                        sequence_actor_ids(graph, scope).into_iter().find(|actor| {
                            graph.node(*actor).is_some_and(|node| {
                                node.external_id.rsplit('.').next() == Some(terminal)
                            })
                        })
                {
                    return Some(actor);
                }
                member = graph.node(candidate).and_then(|node| node.parent);
            }
            return None;
        }
        current = parent?;
    }
}

/// Sequence actor grids are extracted and laid out independently. Their
/// internal edges are not messages in the containing sequence diagram and do
/// not determine its lifeline endpoint.
fn sequence_edge_is_in_extracted_grid(
    graph: &Graph,
    edge: &d2_rs_core::Edge,
    source_actor: NodeId,
    target_actor: NodeId,
) -> bool {
    if source_actor != target_actor || edge.source == source_actor || edge.target == source_actor {
        return false;
    }
    let Some(actor) = graph.node(source_actor) else {
        return false;
    };
    (actor.grid_rows.is_some() || actor.grid_columns.is_some())
        && graph
            .nodes()
            .any(|(_, node)| node.parent == Some(source_actor))
}

fn sequence_actor_ids(graph: &Graph, scope: Option<NodeId>) -> Vec<NodeId> {
    if scope.is_none() && !graph.root_sequence_diagram {
        return Vec::new();
    }
    if scope.is_some_and(|id| !graph.is_sequence_diagram(id)) {
        return Vec::new();
    }
    graph
        .nodes()
        .filter_map(|(id, node)| {
            (node.parent == scope
                && !graph.is_sequence_diagram(id)
                && !is_sequence_group_like(graph, id))
            .then_some(id)
        })
        .collect()
}

fn is_sequence_group_like(graph: &Graph, id: NodeId) -> bool {
    if !graph.root_sequence_diagram && outer_sequence_scope(graph, id).is_none() {
        return false;
    }
    if node_has_incident_edge(graph, id) {
        return false;
    }
    let scope = outer_sequence_scope(graph, id);
    let contains_reference_edge = |container| {
        graph.sequence_group_edges.iter().any(|(owner, edges)| {
            !edges.is_empty() && is_descendant_or_self(graph, *owner, container)
        })
    };
    if !contains_reference_edge(id) && !graph.sequence_contains_object(id) {
        return false;
    }
    // Source checks child.ContainsAnyEdge using reference scopes, not the
    // ancestry of message endpoints. Lowered actor proxies are reattached to
    // root actors by sequence layout and are not final children of the group.
    graph
        .nodes()
        .filter(|(_, node)| node.parent == Some(id))
        .all(|(child, node)| {
            let actor_proxy = !is_nested_sequence_grid_descendant(graph, id)
                && node.external_id.rsplit('.').next().is_some_and(|name| {
                    graph.nodes().any(|(actor, candidate)| {
                        actor != id
                            && candidate.parent == scope
                            && !graph.is_sequence_diagram(actor)
                            && candidate.external_id.rsplit('.').next() == Some(name)
                    })
                });
            actor_proxy || contains_reference_edge(child)
        })
}

/// D2 turns non-note descendants of sequence actors into unlabeled square
/// spans before exporting the target. The source compiler retains those
/// implicit path objects (for edge resolution and layout), so apply the same
/// target-only classification while rendering them.
fn is_sequence_span_like(graph: &Graph, id: NodeId) -> bool {
    let scope = outer_sequence_scope(graph, id);
    if scope.is_none() && !graph.root_sequence_diagram {
        return false;
    }
    let mut top_child = id;
    loop {
        let parent = graph.node(top_child).and_then(|node| node.parent);
        if parent == scope {
            break;
        }
        let Some(parent) = parent else {
            break;
        };
        top_child = parent;
    }
    // Direct actors and direct group vessels keep their authored labels.
    // Source queues every descendant of a top-level group into sd.groups.
    // Only actor descendants enter the note/span partition; nested groups
    // keep their authored shape and label instead of becoming actor spans.
    if top_child == id
        || is_sequence_group_like(graph, top_child)
        || is_nested_sequence_grid_descendant(graph, id)
        || sequence_reinjected_root(graph, id)
    {
        return false;
    }
    !is_sequence_note_like(graph, id)
}

fn sequence_reinjected_root(graph: &Graph, id: NodeId) -> bool {
    node_has_children(graph, id)
        && graph.node(id).is_some_and(|node| {
            node.grid_rows.is_some() || node.grid_columns.is_some() || graph.is_sequence_diagram(id)
        })
}

fn target_shape_for_node(graph: &Graph, id: NodeId) -> Option<ShapeKind> {
    let node = graph.node(id)?;
    Some(if is_sequence_note_like(graph, id) {
        ShapeKind::Page
    } else if is_sequence_span_like(graph, id) {
        ShapeKind::Square
    } else {
        node.shape
    })
}

fn is_descendant_or_self(graph: &Graph, id: NodeId, ancestor: NodeId) -> bool {
    let mut current = Some(id);
    while let Some(candidate) = current {
        if candidate == ancestor {
            return true;
        }
        current = graph.node(candidate).and_then(|node| node.parent);
    }
    false
}

fn translated(rect: Rect, dx: f64, dy: f64) -> Rect {
    Rect {
        origin: Point {
            x: rect.origin.x + dx,
            y: rect.origin.y + dy,
        },
        size: rect.size,
    }
}

fn rounded_render_rect(rect: Rect) -> Rect {
    Rect {
        origin: Point {
            x: rect.origin.x.trunc(),
            y: rect.origin.y.trunc(),
        },
        size: Size {
            width: rect.size.width.trunc(),
            height: rect.size.height.trunc(),
        },
    }
}

/// Return the box that the v0.9.0 SVG renderer uses for an ordinary node
/// label.  D2 positions labels from their measured label rectangle, rather
/// than aligning the text directly to the node's outer rectangle.  Keeping
/// this calculation beside the SVG text path also makes the same geometry
/// available to the Markdown viewport and the accessibility fallback.
fn cloud_content_aspect(node: &Node, label_size: Size) -> Option<f64> {
    if node.shape != ShapeKind::Cloud || label_size.width <= 0.0 || label_size.height <= 0.0 {
        return None;
    }
    // d2graph.SizeToContent computes Cloud.ContentAspectRatio from the
    // content box before shape fitting. Ordinary labels receive the 5px
    // inner label padding only when neither dimension is authored.
    let padding = if !node.fixed_width && !node.fixed_height {
        5.0
    } else {
        0.0
    };
    Some((label_size.width + padding) / (label_size.height + padding))
}

fn node_inner_box(rect: Rect, shape: ShapeKind, cloud_aspect: Option<f64>) -> Rect {
    let mut inner = rect;
    match shape {
        ShapeKind::Circle => {
            let radius = rect.size.width / 2.0;
            let offset = (radius - radius * std::f64::consts::SQRT_2 / 2.0).ceil();
            inner.origin.x += offset;
            inner.origin.y += offset;
            inner.size.width = (rect.size.width - 2.0 * offset).max(0.0);
            inner.size.height = (rect.size.height - 2.0 * offset).max(0.0);
        }
        ShapeKind::Oval => {
            let rx = rect.size.width / 2.0;
            let ry = rect.size.height / 2.0;
            let theta = (ry / rx.max(f64::MIN_POSITIVE)).atan();
            let sin = theta.sin();
            let cos = theta.cos();
            let radius = rx * ry / ((rx * sin).powi(2) + (ry * cos).powi(2)).sqrt();
            let offset_x = (rx - cos * radius).ceil();
            let offset_y = (ry - sin * radius).ceil();
            inner.origin.x += offset_x;
            inner.origin.y += offset_y;
            inner.size.width = (rect.size.width - 2.0 * offset_x).max(0.0);
            inner.size.height = (rect.size.height - 2.0 * offset_y).max(0.0);
        }
        ShapeKind::Diamond => {
            inner.origin.x += rect.size.width / 4.0;
            inner.origin.y += rect.size.height / 4.0;
            inner.size.width = rect.size.width / 2.0;
            inner.size.height = rect.size.height / 2.0;
        }
        ShapeKind::Hexagon => {
            inner.origin.x += rect.size.width / 6.0;
            inner.origin.y += rect.size.height / 6.0;
            inner.size.width = rect.size.width / 1.5;
            inner.size.height = rect.size.height / 1.5;
        }
        ShapeKind::Person => {
            let shoulder = rect.size.width * (20.2 / 68.3);
            inner.origin.x += shoulder;
            inner.size.width = (rect.size.width - 2.0 * shoulder).max(0.0);
        }
        ShapeKind::C4Person => {
            // These are the constants used by d2shape's C4 person.  The
            // head is not part of the content box, so labels start at the
            // body rather than at the silhouette's top edge.
            let head_radius = rect.size.width * 0.22;
            let body_top = head_radius + head_radius * 0.8;
            let horizontal = rect.size.width * 0.05;
            let vertical = rect.size.height * 0.03;
            inner.origin.x += horizontal;
            inner.origin.y += body_top + vertical;
            inner.size.width = (rect.size.width - 2.0 * horizontal).max(0.0);
            inner.size.height = (rect.size.height - body_top - 2.0 * vertical).max(0.0);
        }
        ShapeKind::Cloud => {
            let aspect = cloud_aspect
                .unwrap_or_else(|| rect.size.width / rect.size.height.max(f64::MIN_POSITIVE));
            let (x, y, width, height) = if aspect > (1.0 + 0.819 / 0.548) / 2.0 {
                (0.085, 0.409, 0.819, 0.548)
            } else if aspect < (1.0 + 0.549 / 0.820) / 2.0 {
                (0.228, 0.179, 0.549, 0.820)
            } else {
                (0.167, 0.335, 0.663, 0.663)
            };
            inner.origin.x += (rect.size.width * x).ceil();
            inner.origin.y += (rect.size.height * y).ceil();
            inner.size.width = rect.size.width * width;
            inner.size.height = rect.size.height * height;
        }
        ShapeKind::Cylinder => {
            let arc = if rect.size.height < 48.0 {
                rect.size.height / 2.0
            } else {
                24.0
            };
            inner.origin.y += 2.0 * arc;
            inner.size.height = (rect.size.height - 3.0 * arc).max(0.0);
        }
        ShapeKind::Queue => {
            let arc = if rect.size.width < 48.0 {
                rect.size.width / 2.0
            } else {
                24.0
            };
            inner.origin.x += arc;
            inner.size.width = (rect.size.width - 3.0 * arc).max(0.0);
        }
        ShapeKind::Parallelogram => {
            inner.origin.x += 26.0;
            inner.size.width = (rect.size.width - 52.0).max(0.0);
        }
        ShapeKind::Document => {
            inner.size.height = rect.size.height * 14.0 / 18.925;
        }
        ShapeKind::Page => {
            if rect.size.height < 3.0 * 20.348 {
                inner.size.width = (rect.size.width - 20.8164).max(0.0);
            }
        }
        ShapeKind::Package => {
            let top_height = (rect.size.height * 0.2).min(55.0);
            inner.origin.y += top_height;
            inner.size.height = (rect.size.height - top_height).max(0.0);
        }
        ShapeKind::Step => {
            inner.origin.x += 35.0;
            inner.size.width = (rect.size.width - 70.0).max(0.0);
        }
        ShapeKind::StoredData => {
            inner.origin.x += 15.0;
            inner.size.width = (rect.size.width - 30.0).max(0.0);
        }
        ShapeKind::Callout => {
            let tip = if rect.size.height < 90.0 {
                rect.size.height / 2.0
            } else {
                45.0
            };
            inner.size.height = (rect.size.height - tip).max(0.0);
        }
        // The remaining rectangular shapes use the full shape box in the
        // pinned release.  Their decorative paths do not reserve a separate
        // content inset for ordinary labels.
        _ => {}
    }
    inner
}

fn node_label_box(
    rect: Rect,
    shape: ShapeKind,
    position: LabelPosition,
    label_size: Size,
    cloud_aspect: Option<f64>,
) -> Rect {
    let bounds_box = if matches!(
        position,
        LabelPosition::OutsideTopLeft
            | LabelPosition::OutsideTopCenter
            | LabelPosition::OutsideTopRight
            | LabelPosition::OutsideLeftTop
            | LabelPosition::OutsideLeftMiddle
            | LabelPosition::OutsideLeftBottom
            | LabelPosition::OutsideRightTop
            | LabelPosition::OutsideRightMiddle
            | LabelPosition::OutsideRightBottom
            | LabelPosition::OutsideBottomLeft
            | LabelPosition::OutsideBottomCenter
            | LabelPosition::OutsideBottomRight
            | LabelPosition::BorderTopLeft
            | LabelPosition::BorderTopCenter
            | LabelPosition::BorderTopRight
            | LabelPosition::BorderLeftTop
            | LabelPosition::BorderLeftMiddle
            | LabelPosition::BorderLeftBottom
            | LabelPosition::BorderRightTop
            | LabelPosition::BorderRightMiddle
            | LabelPosition::BorderRightBottom
            | LabelPosition::BorderBottomLeft
            | LabelPosition::BorderBottomCenter
            | LabelPosition::BorderBottomRight
    ) {
        rect
    } else {
        node_inner_box(rect, shape, cloud_aspect)
    };
    let width = label_size.width;
    let height = label_size.height;
    let center = bounds_box.center();
    let origin = match position {
        LabelPosition::InsideTopLeft => Point {
            x: bounds_box.origin.x + 5.0,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::InsideTopCenter => Point {
            x: center.x - width / 2.0,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::InsideTopRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::InsideMiddleLeft => Point {
            x: bounds_box.origin.x + 5.0,
            y: center.y - height / 2.0,
        },
        // label.Position.GetPointOnBox has no case for an unset position and
        // returns the box's top-left (d2scenebuild shapeLabelPlacement).
        LabelPosition::Unset => bounds_box.origin,
        LabelPosition::InsideMiddleCenter => Point {
            x: center.x - width / 2.0,
            y: center.y - height / 2.0,
        },
        LabelPosition::InsideMiddleRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: center.y - height / 2.0,
        },
        LabelPosition::InsideBottomLeft => Point {
            x: bounds_box.origin.x + 5.0,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::InsideBottomCenter => Point {
            x: center.x - width / 2.0,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::InsideBottomRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::OutsideTopLeft => Point {
            x: bounds_box.origin.x - 5.0,
            y: bounds_box.origin.y - 5.0 - height,
        },
        LabelPosition::OutsideTopCenter => Point {
            x: center.x - width / 2.0,
            y: bounds_box.origin.y - 5.0 - height,
        },
        LabelPosition::OutsideTopRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: bounds_box.origin.y - 5.0 - height,
        },
        LabelPosition::OutsideLeftTop => Point {
            x: bounds_box.origin.x - 5.0 - width,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::OutsideLeftMiddle => Point {
            x: bounds_box.origin.x - 5.0 - width,
            y: center.y - height / 2.0,
        },
        LabelPosition::OutsideLeftBottom => Point {
            x: bounds_box.origin.x - 5.0 - width,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::OutsideRightTop => Point {
            x: bounds_box.right() + 5.0,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::OutsideRightMiddle => Point {
            x: bounds_box.right() + 5.0,
            y: center.y - height / 2.0,
        },
        LabelPosition::OutsideRightBottom => Point {
            x: bounds_box.right() + 5.0,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::OutsideBottomLeft => Point {
            x: bounds_box.origin.x + 5.0,
            y: bounds_box.bottom() + 5.0,
        },
        LabelPosition::OutsideBottomCenter => Point {
            x: center.x - width / 2.0,
            y: bounds_box.bottom() + 5.0,
        },
        LabelPosition::OutsideBottomRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: bounds_box.bottom() + 5.0,
        },
        // Border positions are centered on the corresponding border, as in
        // label.Position.GetPointOnBox in the pinned Go release.
        LabelPosition::BorderTopLeft => Point {
            x: bounds_box.origin.x + 5.0,
            y: bounds_box.origin.y - height / 2.0,
        },
        LabelPosition::BorderTopCenter => Point {
            x: center.x - width / 2.0,
            y: bounds_box.origin.y - height / 2.0,
        },
        LabelPosition::BorderTopRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: bounds_box.origin.y - height / 2.0,
        },
        LabelPosition::BorderLeftTop => Point {
            x: bounds_box.origin.x - width / 2.0,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::BorderLeftMiddle => Point {
            x: bounds_box.origin.x - width / 2.0,
            y: center.y - height / 2.0,
        },
        LabelPosition::BorderLeftBottom => Point {
            x: bounds_box.origin.x - width / 2.0,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::BorderRightTop => Point {
            x: bounds_box.right() - width / 2.0,
            y: bounds_box.origin.y + 5.0,
        },
        LabelPosition::BorderRightMiddle => Point {
            x: bounds_box.right() - width / 2.0,
            y: center.y - height / 2.0,
        },
        LabelPosition::BorderRightBottom => Point {
            x: bounds_box.right() - width / 2.0,
            y: bounds_box.bottom() - height - 5.0,
        },
        LabelPosition::BorderBottomLeft => Point {
            x: bounds_box.origin.x + 5.0,
            y: bounds_box.bottom() - height / 2.0,
        },
        LabelPosition::BorderBottomCenter => Point {
            x: center.x - width / 2.0,
            y: bounds_box.bottom() - height / 2.0,
        },
        LabelPosition::BorderBottomRight => Point {
            x: bounds_box.right() - width - 5.0,
            y: bounds_box.bottom() - height / 2.0,
        },
        LabelPosition::UnlockedTop
        | LabelPosition::UnlockedMiddle
        | LabelPosition::UnlockedBottom => Point {
            x: center.x - width / 2.0,
            y: center.y - height / 2.0,
        },
    };
    Rect {
        origin,
        size: label_size,
    }
}

fn modifier_adjusted_label_rect(
    mut rect: Rect,
    shape: ShapeKind,
    position: LabelPosition,
    three_dee: bool,
    multiple: bool,
) -> Rect {
    let is_border = matches!(
        position,
        LabelPosition::BorderTopLeft
            | LabelPosition::BorderTopCenter
            | LabelPosition::BorderTopRight
            | LabelPosition::BorderLeftTop
            | LabelPosition::BorderLeftMiddle
            | LabelPosition::BorderLeftBottom
            | LabelPosition::BorderRightTop
            | LabelPosition::BorderRightMiddle
            | LabelPosition::BorderRightBottom
            | LabelPosition::BorderBottomLeft
            | LabelPosition::BorderBottomCenter
            | LabelPosition::BorderBottomRight
    );
    if !(position.is_outside() || is_border) {
        return rect;
    }
    // d2svg uses integer target offsets; TALA's floating-point 7.5
    // hexagon protrusion is a separate layout contract.
    let (offset_x, offset_y) = if three_dee {
        (
            15.0,
            if shape == ShapeKind::Hexagon {
                7.0
            } else {
                15.0
            },
        )
    } else if multiple {
        (10.0, 10.0)
    } else {
        (0.0, 0.0)
    };
    rect.origin.y -= offset_y;
    rect.size.width += offset_x;
    rect.size.height += offset_y;
    rect
}

// d2target.Diagram.BoundingBox moves only outside-right/top labels after
// positioning them on the original box. SVG painting instead positions labels
// around the expanded silhouette, including border and centered side labels.
fn target_modifier_label_bounds(
    mut label: Rect,
    shape: ShapeKind,
    position: LabelPosition,
    three_dee: bool,
) -> Rect {
    if three_dee {
        let offset = if shape == ShapeKind::Hexagon {
            7.0
        } else {
            15.0
        };
        match position {
            LabelPosition::OutsideRightTop
            | LabelPosition::OutsideRightMiddle
            | LabelPosition::OutsideRightBottom => label.origin.x += offset,
            LabelPosition::OutsideTopLeft
            | LabelPosition::OutsideTopCenter
            | LabelPosition::OutsideTopRight => label.origin.y -= offset,
            _ => {}
        }
    }
    label
}

// d2scenebuild.borderLabelMaskBox uses only the strip of shape outline
// beneath a border label, rather than the entire label rectangle.
fn border_label_mask_rect(
    mut shape_box: Rect,
    shape: ShapeKind,
    position: LabelPosition,
    label_size: Size,
    stroke: f64,
    three_dee: bool,
    multiple: bool,
) -> Option<Rect> {
    if matches!(shape, ShapeKind::Class | ShapeKind::SqlTable) {
        return None;
    }
    let (offset_x, offset_y) = if three_dee {
        (
            15.0,
            if shape == ShapeKind::Hexagon {
                7.0
            } else {
                15.0
            },
        )
    } else if multiple {
        (10.0, 10.0)
    } else {
        (0.0, 0.0)
    };
    shape_box.origin.y -= offset_y;
    shape_box.size.width += offset_x;
    shape_box.size.height += offset_y;
    let top_left = node_label_box(shape_box, shape, position, label_size, None).origin;
    let (x, y, width, height) = match position {
        LabelPosition::BorderTopLeft
        | LabelPosition::BorderTopCenter
        | LabelPosition::BorderTopRight => (
            top_left.x - 2.0,
            shape_box.origin.y - stroke / 2.0,
            label_size.width + 4.0,
            stroke,
        ),
        LabelPosition::BorderBottomLeft
        | LabelPosition::BorderBottomCenter
        | LabelPosition::BorderBottomRight => (
            top_left.x - 2.0,
            shape_box.bottom() - stroke / 2.0,
            label_size.width + 4.0,
            stroke,
        ),
        LabelPosition::BorderLeftTop
        | LabelPosition::BorderLeftMiddle
        | LabelPosition::BorderLeftBottom => (
            shape_box.origin.x - stroke / 2.0,
            top_left.y - 2.0,
            stroke,
            label_size.height + 4.0,
        ),
        LabelPosition::BorderRightTop
        | LabelPosition::BorderRightMiddle
        | LabelPosition::BorderRightBottom => (
            shape_box.right() - stroke / 2.0,
            top_left.y - 2.0,
            stroke,
            label_size.height + 4.0,
        ),
        _ => return None,
    };
    (width > 0.0 && height > 0.0).then_some(Rect {
        origin: Point { x, y },
        size: Size { width, height },
    })
}

// Original d2svgimport/style.go treats a MathJax root ex as eight pixels.
// This is the vector asset size, before ceil used for target label dimensions.
fn latex_shape_label_box(
    rect: Rect,
    shape: ShapeKind,
    position: LabelPosition,
    label_size: Size,
    cloud_aspect: Option<f64>,
) -> Rect {
    let position = if position == LabelPosition::Unset {
        LabelPosition::InsideMiddleCenter
    } else {
        position
    };
    let bounds = if position.is_outside() {
        rect
    } else {
        node_inner_box(rect, shape, cloud_aspect)
    };
    node_label_box(bounds, ShapeKind::Rectangle, position, label_size, None)
}

fn latex_intrinsic_viewport(svg: &str) -> Option<Size> {
    let document = roxmltree::Document::parse(svg).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return None;
    }
    let length = |name| {
        let raw = root.attribute(name)?.trim();
        let (value, scale) = if let Some(value) = raw.strip_suffix("ex") {
            (value, 8.0)
        } else {
            (raw.strip_suffix("px").unwrap_or(raw), 1.0)
        };
        let value = value.parse::<f64>().ok()? * scale;
        (value.is_finite() && value > 0.0).then_some(value)
    };
    Some(Size {
        width: length("width")?,
        height: length("height")?,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_node_text_box(
    svg: &mut String,
    text: &str,
    label_box: Rect,
    font_size: u32,
    fill: &str,
    language: Option<&str>,
    style: TextStyle,
    class: Option<&str>,
    native_geometry_id: Option<&str>,
) {
    // Original d2scenebuild.buildLatexLabelNode places an intrinsic SVG image
    // at the measured target top-left. Font size does not scale this asset.
    if language == Some("latex")
        && let Ok(math_svg) = d2_rs_math::render(text)
    {
        let math_svg = if native_geometry_id.is_some() {
            d2_rs_math::import_scene_svg(&math_svg)
                .expect("raster math import was checked before emission")
        } else {
            math_svg
        };
        // Original SVG import resolves em/ex against its fixed 16px root
        // font. usvg otherwise inherits its 12px fallback or an owner font.
        let math_svg = math_svg.replacen("<svg ", "<svg font-size=\"16\" ", 1);
        if let Some(id) = native_geometry_id
            && let Some(size) = latex_intrinsic_viewport(&math_svg)
        {
            let document =
                roxmltree::Document::parse(&math_svg).expect("parsed intrinsic math SVG");
            let root = document.root_element();
            let width = root.attribute("width").unwrap();
            let height = root.attribute("height").unwrap();
            let asset = math_svg
                .replacen(
                    &format!(" width=\"{width}\""),
                    &format!(" width=\"{}\"", size.width),
                    1,
                )
                .replacen(
                    &format!(" height=\"{height}\""),
                    &format!(" height=\"{}\"", size.height),
                    1,
                )
                .replacen(
                    "<svg ",
                    &format!(
                        "<svg data-d2-rs-intrinsic-math=\"ordinary\" color=\"{}\" ",
                        escape(fill)
                    ),
                    1,
                );
            // The original scene retains math as a clipped vector Image.
            // Reuse imported-image command/transform precision rather than
            // usvg's inline quadratic-to-cubic conversion.
            let encoded = base64::engine::general_purpose::STANDARD.encode(asset);
            let _ = writeln!(
                svg,
                "<image id=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"data:image/svg+xml;base64,{}\"/>",
                escape(id),
                label_box.origin.x,
                label_box.origin.y,
                size.width,
                size.height,
                encoded
            );
            return;
        }
        let class_attribute = class
            .map(|value| format!(" class=\"{}\"", escape(value)))
            .unwrap_or_default();
        let _ = writeln!(
            svg,
            "<g{} data-d2-language=\"latex\" color=\"{}\" transform=\"translate({} {})\">{}</g>",
            class_attribute,
            escape(fill),
            label_box.origin.x,
            label_box.origin.y,
            math_svg
        );
        return;
    }
    let lines = text.split('\n').collect::<Vec<_>>();
    // d2svg.RenderText divides the measured target LabelHeight by all lines.
    let line_height = label_box.size.height / lines.len() as f64;
    let class = format!(" class=\"{}\"", escape(&text_class_with_font(class, style)));
    let language = language
        .map(|value| format!(" data-d2-language=\"{}\"", escape(value)))
        .unwrap_or_default();
    let font_weight = if style.bold {
        // Source's synthetic bold is a round stroke, preserving the concrete face.
        // The public SVG keeps its CSS axis; native transport below carries paint.
        " font-weight=\"bold\""
    } else {
        ""
    };
    let font_style = if style.italic && !style.bold {
        " font-style=\"italic\""
    } else {
        ""
    };
    let text_decoration = if style.underline {
        " text-decoration=\"underline\""
    } else {
        ""
    };
    let font_family = if style.mono {
        "Source Code Pro, monospace"
    } else {
        "Source Sans Pro, sans-serif"
    };
    let x = label_box.origin.x + label_box.size.width / 2.0;
    let y = label_box.origin.y + f64::from(font_size);
    if let Some(native_geometry_id) = native_geometry_id {
        // Source node/edge labels retain independent TextRuns even for one
        // line. Carry original origins so export preserves their paint frame.
        // Go shapes the raw line, including leading/trailing/repeated spaces;
        // standalone SVG text otherwise applies whitespace normalization.
        for (index, line) in lines.iter().enumerate() {
            let line_y = y + index as f64 * line_height;
            let line = if line.is_empty() { " " } else { line };
            let id = format!("{native_geometry_id}-line-{index}");
            let _ = writeln!(
                svg,
                "<text xml:space=\"preserve\" data-d2-rs-text-raw=\"true\" id=\"{}\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\" x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\"{}{}{}{}{}>{}</text>",
                escape(&id),
                x.to_bits(),
                line_y.to_bits(),
                x,
                line_y,
                font_family,
                font_size,
                escape(fill),
                font_weight,
                font_style,
                text_decoration,
                language,
                class,
                escape_xml_text(line)
            );
        }
        return;
    }
    let _ = write!(
        svg,
        "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\"{}{}{}{}{}>",
        number(x),
        number(y),
        font_family,
        font_size,
        escape(fill),
        font_weight,
        font_style,
        text_decoration,
        language,
        class
    );
    if lines.len() == 1 {
        let _ = write!(svg, "{}", escape_xml_text(text));
    } else {
        for (index, line) in lines.iter().enumerate() {
            let dy = if index == 0 { 0.0 } else { line_height };
            let line = if line.is_empty() { " " } else { line };
            let _ = write!(
                svg,
                "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                number(x),
                number(dy),
                escape_xml_text(line)
            );
        }
    }
    svg.push_str("</text>\n");
}

#[derive(Clone, Copy, Default)]
struct TextStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    mono: bool,
}

fn text_font_class(style: TextStyle) -> &'static str {
    match (style.mono, style.bold, style.italic) {
        (true, true, _) => "text-mono-bold",
        (true, false, true) => "text-mono-italic",
        (true, false, false) => "text-mono",
        (false, true, _) => "text-bold",
        (false, false, true) => "text-italic",
        (false, false, false) => "text",
    }
}

fn text_class_with_font(class: Option<&str>, style: TextStyle) -> String {
    match class.filter(|value| !value.is_empty()) {
        Some(class) => format!("{class} {}", text_font_class(style)),
        None => text_font_class(style).to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    position: LabelPosition,
    fill: &str,
    language: Option<&str>,
    style: TextStyle,
    class: Option<&str>,
) {
    if language == Some("latex")
        && let Ok(size) = d2_rs_math::measure(text)
    {
        let label_box = latex_shape_label_box(rect, ShapeKind::Rectangle, position, size, None);
        draw_node_text_box(
            svg, text, label_box, font_size, fill, language, style, class, None,
        );
        return;
    }
    let lines = text.split('\n').collect::<Vec<_>>();
    let line_height = f64::from(font_size) * 1.2;
    let total_line_height = lines.len().saturating_sub(1) as f64 * line_height;
    let padding = 5.0;
    let (x, anchor, start_y) = match position {
        LabelPosition::InsideTopLeft
        | LabelPosition::BorderTopLeft
        | LabelPosition::InsideMiddleLeft
        | LabelPosition::BorderLeftTop
        | LabelPosition::BorderLeftMiddle
        | LabelPosition::InsideBottomLeft
        | LabelPosition::BorderBottomLeft => (
            rect.origin.x + padding,
            "start",
            if matches!(
                position,
                LabelPosition::InsideTopLeft
                    | LabelPosition::BorderTopLeft
                    | LabelPosition::BorderLeftTop
            ) {
                rect.origin.y + padding + f64::from(font_size)
            } else if matches!(
                position,
                LabelPosition::InsideBottomLeft | LabelPosition::BorderBottomLeft
            ) {
                rect.bottom() - padding - total_line_height
            } else {
                rect.origin.y + rect.size.height / 2.0 - total_line_height / 2.0
            },
        ),
        LabelPosition::InsideTopRight
        | LabelPosition::BorderTopRight
        | LabelPosition::InsideMiddleRight
        | LabelPosition::BorderRightTop
        | LabelPosition::BorderRightMiddle
        | LabelPosition::InsideBottomRight
        | LabelPosition::BorderBottomRight => (
            rect.right() - padding,
            "end",
            if matches!(
                position,
                LabelPosition::InsideTopRight
                    | LabelPosition::BorderTopRight
                    | LabelPosition::BorderRightTop
            ) {
                rect.origin.y + padding + f64::from(font_size)
            } else if matches!(
                position,
                LabelPosition::InsideBottomRight | LabelPosition::BorderBottomRight
            ) {
                rect.bottom() - padding - total_line_height
            } else {
                rect.origin.y + rect.size.height / 2.0 - total_line_height / 2.0
            },
        ),
        LabelPosition::InsideTopCenter | LabelPosition::BorderTopCenter => (
            rect.origin.x + rect.size.width / 2.0,
            "middle",
            rect.origin.y + padding + f64::from(font_size),
        ),
        LabelPosition::InsideBottomCenter | LabelPosition::BorderBottomCenter => (
            rect.origin.x + rect.size.width / 2.0,
            "middle",
            rect.bottom() - padding - total_line_height,
        ),
        _ => (
            rect.origin.x + rect.size.width / 2.0,
            "middle",
            rect.origin.y + rect.size.height / 2.0 - total_line_height / 2.0
                + f64::from(font_size) * 0.34375,
        ),
    };
    let language = language
        .map(|value| format!(" data-d2-language=\"{}\"", escape(value)))
        .unwrap_or_default();
    let font_weight = if style.bold {
        // D2 v0.9.0's `.text-bold` class selects the dedicated bold face.
        // Keep the family resolver on weight 700; numeric 600 selects the
        // distinct semibold face and changes both advances and ink coverage.
        " font-weight=\"bold\""
    } else {
        ""
    };
    let font_style = if style.italic && !style.bold {
        " font-style=\"italic\""
    } else {
        ""
    };
    let text_decoration = if style.underline {
        " text-decoration=\"underline\""
    } else {
        ""
    };
    let font_family = if style.mono {
        "Source Code Pro, monospace"
    } else {
        "Source Sans Pro, sans-serif"
    };
    let class = format!(" class=\"{}\"", escape(&text_class_with_font(class, style)));
    let _ = write!(
        svg,
        "<text x=\"{}\" y=\"{}\" text-anchor=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\"{}{}{}{}{}>",
        number(x),
        number(start_y),
        anchor,
        font_family,
        font_size,
        escape(fill),
        font_weight,
        font_style,
        text_decoration,
        language,
        class,
    );
    for (index, line) in lines.iter().enumerate() {
        if index == 0 {
            let _ = write!(svg, "{}", escape(line));
        } else {
            let _ = write!(
                svg,
                "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                number(x),
                number(line_height),
                escape(line)
            );
        }
    }
    svg.push_str("</text>\n");
}

#[derive(Clone, Debug, PartialEq)]
struct NativeMarkdownRun {
    direct_code_text: bool,
    link_title: Option<String>,
    code_link_title: Option<String>,
    code_link: Option<String>,
    decoration_accent: bool,
    padding_only: bool,
    text: String,
    bold: bool,
    semibold: bool,
    italic: bool,
    /// Goldmark keeps the innermost concrete face and synthesizes the other
    /// axis. These flags preserve that order for the native painter.
    bold_after_italic: bool,
    italic_after_bold: bool,
    code: bool,
    primary_mono: bool,
    code_bold: bool,
    code_italic: bool,
    code_group: usize,
    decoration_group: usize,
    strikethrough: bool,
    link: Option<String>,
    line_break: bool,
}

#[derive(Clone, Default)]
struct NativeMarkdownStyle {
    parent_code: bool,
    link_title: Option<String>,
    code_link_title: Option<String>,
    code_link: Option<String>,
    decoration_accent: bool,
    bold: bool,
    semibold: bool,
    italic: bool,
    bold_after_italic: bool,
    italic_after_bold: bool,
    code: bool,
    code_bold: bool,
    code_italic: bool,
    code_group: usize,
    decoration_group: usize,
    strikethrough: bool,
    link: Option<String>,
}

#[derive(Clone, Debug)]
enum NativeMarkdownBlock {
    Blockquote(Vec<Vec<NativeMarkdownRun>>),
    ThematicBreak,
    FencedCode(Vec<String>),
}

fn native_markdown_font(
    slot: &OnceLock<Option<TrueTypeFont>>,
    load: fn() -> Result<TrueTypeFont, d2_rs_text::measure::FontError>,
) -> Option<&TrueTypeFont> {
    slot.get_or_init(|| load().ok()).as_ref()
}

fn native_markdown_run_width(run: &NativeMarkdownRun, font_size: f64) -> f64 {
    static REGULAR: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static BOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static SEMIBOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static ITALIC: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static MONO: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    let font = if run.code {
        native_markdown_font(&MONO, bundled_source_code_pro_regular)
    } else if run.semibold {
        native_markdown_font(&SEMIBOLD, bundled_source_sans_pro_semibold)
    } else if run.bold {
        native_markdown_font(&BOLD, bundled_source_sans_pro_bold)
    } else if run.italic {
        native_markdown_font(&ITALIC, bundled_source_sans_pro_italic)
    } else {
        native_markdown_font(&REGULAR, bundled_source_sans_pro_regular)
    };
    font.map_or(0.0, |font| font.measure_advance(&run.text, font_size))
}

fn native_markdown_unescape(value: &str) -> Option<String> {
    String::from_utf8(unescape_html(value.as_bytes())).ok()
}

fn native_markdown_html_attribute(tag: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=\"");
    let start = tag.find(&prefix)? + prefix.len();
    let end = tag[start..].find('"')? + start;
    native_markdown_unescape(&tag[start..end])
}

fn native_markdown_html_inline(source: &str) -> Option<Vec<NativeMarkdownRun>> {
    native_markdown_html_inline_context(source, false)
}

fn native_markdown_html_inline_context(
    source: &str,
    header: bool,
) -> Option<Vec<NativeMarkdownRun>> {
    let normalized = normalize_html_names(source);
    let source = normalized.as_str();
    let body = source.strip_prefix("<p>")?.strip_suffix("</p>\n")?;
    let mut runs = Vec::new();
    let mut style = NativeMarkdownStyle::default();
    if header {
        style.bold = true;
        style.semibold = true;
    }
    let mut stack = Vec::new();
    let mut next_code_group = 0;
    let mut next_decoration_group = 0;
    let mut collapse_space_after_image = false;
    let mut cursor = 0;
    while cursor < body.len() {
        if body.as_bytes()[cursor] == b'<' {
            let end = body[cursor..].find('>')? + cursor;
            let tag = &body[cursor..=end];
            if tag.starts_with("</") {
                if tag == "</code>"
                    && style.code
                    && !runs.iter().any(|run: &NativeMarkdownRun| {
                        run.code_group == style.code_group && run.code && !run.line_break
                    })
                {
                    // collectInlineTokens retains a code group's padding even
                    // without text children. This zero-text owner is consumed
                    // by wrapping, but never emitted as a text primitive.
                    runs.push(NativeMarkdownRun {
                        direct_code_text: style.parent_code,
                        link_title: style.link_title.clone(),
                        code_link_title: style.code_link_title.clone(),
                        code_link: style.code_link.clone(),
                        decoration_accent: style.decoration_accent,
                        padding_only: true,
                        text: String::new(),
                        bold: style.bold,
                        semibold: style.semibold,
                        italic: style.italic,
                        bold_after_italic: style.bold_after_italic,
                        italic_after_bold: style.italic_after_bold,
                        code: true,
                        primary_mono: false,
                        code_bold: style.code_bold,
                        code_italic: style.code_italic,
                        code_group: style.code_group,
                        decoration_group: style.decoration_group,
                        strikethrough: style.strikethrough,
                        link: style.link.clone(),
                        line_break: false,
                    });
                }
                stack.pop().map(|previous| style = previous)?;
            } else if tag == "<br />" || tag == "<br/>" {
                runs.push(NativeMarkdownRun {
                    direct_code_text: style.parent_code,
                    link_title: style.link_title.clone(),
                    code_link_title: style.code_link_title.clone(),
                    code_link: style.code_link.clone(),
                    decoration_accent: style.decoration_accent,
                    padding_only: false,
                    text: String::new(),
                    bold: style.bold,
                    semibold: style.semibold,
                    italic: style.italic,
                    bold_after_italic: style.bold_after_italic,
                    italic_after_bold: style.italic_after_bold,
                    code: style.code,
                    primary_mono: false,
                    code_bold: style.code_bold,
                    code_italic: style.code_italic,
                    code_group: style.code_group,
                    decoration_group: style.decoration_group,
                    strikethrough: style.strikethrough,
                    link: style.link.clone(),
                    line_break: true,
                });
            } else if tag.starts_with("<img") {
                // D2's native renderer omits images when no asset provider
                // supplied a fetched bitmap; alt text is not a text run.
                collapse_space_after_image = true;
            } else {
                let name_end = tag[1..]
                    .find(|character: char| character.is_ascii_whitespace() || character == '>')?
                    + 1;
                let name = &tag[1..name_end];
                let previous = style.clone();
                match name {
                    "strong" | "b" => {
                        style.semibold = false;
                        if style.italic {
                            style.bold_after_italic = true;
                        }
                        if style.code {
                            style.code_bold = true;
                            style.code_italic = false;
                        }
                        style.bold = true;
                    }
                    "em" | "i" => {
                        style.semibold = false;
                        if style.bold {
                            style.italic_after_bold = true;
                        }
                        if style.code {
                            style.code_italic = true;
                            style.code_bold = false;
                        }
                        style.italic = true;
                    }
                    "del" | "s" | "strike" => {
                        if !style.strikethrough {
                            next_decoration_group += 1;
                            style.decoration_group = next_decoration_group;
                            style.decoration_accent = style.link.is_some();
                        }
                        style.strikethrough = true;
                    }
                    "code" => {
                        next_code_group += 1;
                        style.code = true;
                        style.code_bold = false;
                        style.code_italic = false;
                        style.code_group = next_code_group;
                        style.code_link = style.link.clone();
                        style.code_link_title = style.link_title.clone();
                    }
                    "a" => {
                        let href = native_markdown_html_attribute(tag, "href").unwrap_or_default();
                        style.link_title = native_markdown_html_attribute(tag, "title");
                        style.link = Some(native_markdown_safe_link(&href)?);
                    }
                    _ => return None,
                }
                style.parent_code = name == "code";
                stack.push(previous);
            }
            cursor = end + 1;
            continue;
        }
        let end = body[cursor..]
            .find('<')
            .map_or(body.len(), |relative| cursor + relative);
        let mut text = native_markdown_unescape(&body[cursor..end])?;
        if collapse_space_after_image {
            let previous_has_space = runs.last().is_some_and(|run: &NativeMarkdownRun| {
                run.text.chars().last().is_some_and(char::is_whitespace)
            });
            if previous_has_space {
                text = text.strip_prefix(' ').unwrap_or(&text).to_owned();
            }
            collapse_space_after_image = false;
        }
        if !text.is_empty() {
            runs.push(NativeMarkdownRun {
                direct_code_text: style.parent_code,
                link_title: style.link_title.clone(),
                code_link_title: style.code_link_title.clone(),
                code_link: style.code_link.clone(),
                decoration_accent: style.decoration_accent,
                padding_only: false,
                text,
                bold: style.bold,
                semibold: style.semibold,
                italic: style.italic,
                bold_after_italic: style.bold_after_italic,
                italic_after_bold: style.italic_after_bold,
                code: style.code,
                primary_mono: false,
                code_bold: style.code_bold,
                code_italic: style.code_italic,
                code_group: style.code_group,
                decoration_group: style.decoration_group,
                strikethrough: style.strikethrough,
                link: style.link.clone(),
                line_break: false,
            });
        }
        cursor = end;
    }
    markdown_wrap::normalize_whitespace(&mut runs);
    stack
        .is_empty()
        .then_some(runs)
        .filter(|runs| !runs.is_empty())
}

fn parse_native_markdown_inline(line: &str) -> Option<Vec<NativeMarkdownRun>> {
    if let Some(runs) = native_markdown_html_inline(&render_html(line.as_bytes())) {
        return Some(runs);
    }
    let bytes = line.as_bytes();
    let mut runs = Vec::new();
    let mut cursor = 0;
    let mut plain_start = 0;
    let flush_plain = |runs: &mut Vec<NativeMarkdownRun>, start: usize, end: usize| {
        if end > start {
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: line[start..end].to_owned(),
                bold: false,
                semibold: false,
                italic: false,
                bold_after_italic: false,
                italic_after_bold: false,
                code: false,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: false,
                link: None,
                line_break: false,
            });
        }
    };
    while cursor < bytes.len() {
        if bytes[cursor..].starts_with(b"![") {
            let label_end_relative = line[cursor + 2..].find(']')?;
            let label_end = cursor + 2 + label_end_relative;
            if line
                .get(label_end + 1..)
                .is_some_and(|rest| rest.starts_with('('))
            {
                let destination_end_relative = line[label_end + 2..].find(')')?;
                flush_plain(&mut runs, plain_start, cursor);
                cursor = label_end + 2 + destination_end_relative + 1;
                plain_start = cursor;
                continue;
            }
        }
        if bytes[cursor..].starts_with(b"**") {
            let relative_end = line[cursor + 2..].find("**")?;
            let end = cursor + 2 + relative_end;
            flush_plain(&mut runs, plain_start, cursor);
            if end == cursor + 2 {
                return None;
            }
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: line[cursor + 2..end].to_owned(),
                bold: true,
                semibold: false,
                italic: false,
                bold_after_italic: false,
                italic_after_bold: false,
                code: false,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: false,
                link: None,
                line_break: false,
            });
            cursor = end + 2;
            plain_start = cursor;
            continue;
        }
        if bytes[cursor] == b'`' {
            let relative_end = line[cursor + 1..].find('`')?;
            let end = cursor + 1 + relative_end;
            flush_plain(&mut runs, plain_start, cursor);
            if end == cursor + 1 {
                return None;
            }
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: line[cursor + 1..end].to_owned(),
                bold: false,
                semibold: false,
                italic: false,
                bold_after_italic: false,
                italic_after_bold: false,
                code: true,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: false,
                link: None,
                line_break: false,
            });
            cursor = end + 1;
            plain_start = cursor;
            continue;
        }
        if bytes[cursor..].starts_with(b"~~") {
            let relative_end = line[cursor + 2..].find("~~")?;
            let end = cursor + 2 + relative_end;
            flush_plain(&mut runs, plain_start, cursor);
            if end == cursor + 2 {
                return None;
            }
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: line[cursor + 2..end].to_owned(),
                bold: false,
                semibold: false,
                italic: false,
                bold_after_italic: false,
                italic_after_bold: false,
                code: false,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: true,
                link: None,
                line_break: false,
            });
            cursor = end + 2;
            plain_start = cursor;
            continue;
        }
        if bytes[cursor] == b'<' {
            let relative_end = line[cursor + 1..].find('>')?;
            let end = cursor + 1 + relative_end;
            let destination = &line[cursor + 1..end];
            let link = native_markdown_autolink(destination)?;
            flush_plain(&mut runs, plain_start, cursor);
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: destination.to_owned(),
                bold: false,
                semibold: false,
                italic: false,
                bold_after_italic: false,
                italic_after_bold: false,
                code: false,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: false,
                link: Some(link),
                line_break: false,
            });
            cursor = end + 1;
            plain_start = cursor;
            continue;
        }
        if bytes[cursor] == b'[' {
            let label_end_relative = line[cursor + 1..].find(']')?;
            let label_end = cursor + 1 + label_end_relative;
            let destination_start = label_end + 1;
            if destination_start >= line.len() || !line[destination_start..].starts_with("(") {
                return None;
            }
            if destination_start + 1 > line.len() {
                return None;
            }
            let destination_end_relative = line[destination_start + 1..].find(')')?;
            let destination_end = destination_start + 1 + destination_end_relative;
            let label = &line[cursor + 1..label_end];
            let destination = &line[destination_start + 1..destination_end];
            if label.is_empty()
                || destination.is_empty()
                || destination.contains(char::is_whitespace)
            {
                return None;
            }
            let link = native_markdown_safe_link(destination);
            flush_plain(&mut runs, plain_start, cursor);
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: label.to_owned(),
                bold: false,
                semibold: false,
                italic: false,
                bold_after_italic: false,
                italic_after_bold: false,
                code: false,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: false,
                link,
                line_break: false,
            });
            cursor = destination_end + 1;
            plain_start = cursor;
            continue;
        }
        if bytes[cursor] == b'*' || bytes[cursor] == b'_' {
            let marker = bytes[cursor] as char;
            let relative_end = line[cursor + 1..].find(marker)?;
            let end = cursor + 1 + relative_end;
            flush_plain(&mut runs, plain_start, cursor);
            if end == cursor + 1 {
                return None;
            }
            runs.push(NativeMarkdownRun {
                direct_code_text: false,
                link_title: None,
                code_link_title: None,
                code_link: None,
                decoration_accent: false,
                padding_only: false,
                text: line[cursor + 1..end].to_owned(),
                bold: false,
                semibold: false,
                italic: true,
                bold_after_italic: false,
                italic_after_bold: false,
                code: false,
                primary_mono: false,
                code_bold: false,
                code_italic: false,
                code_group: 0,
                decoration_group: 0,
                strikethrough: false,
                link: None,
                line_break: false,
            });
            cursor = end + 1;
            plain_start = cursor;
            continue;
        }
        cursor += 1;
    }
    flush_plain(&mut runs, plain_start, bytes.len());
    (!runs.is_empty()).then_some(runs)
}

fn split_native_markdown_lines(runs: &[NativeMarkdownRun]) -> Vec<Vec<NativeMarkdownRun>> {
    let mut lines = vec![Vec::new()];
    for run in runs {
        if run.line_break {
            // Code's DOM break resets whitespace but collectInlineTokens
            // omits it from painting the inline code group.
            if !run.code {
                lines.push(Vec::new());
            }
        } else {
            lines
                .last_mut()
                .expect("line is always present")
                .push(run.clone());
        }
    }
    lines
}

fn native_markdown_safe_link(link: &str) -> Option<String> {
    let normalized = link
        .chars()
        .filter(|character| !character.is_control() && !character.is_whitespace())
        .collect::<String>();
    let lower = normalized.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
    {
        None
    } else {
        Some(link.to_owned())
    }
}

fn native_markdown_autolink(destination: &str) -> Option<String> {
    if destination.is_empty() || destination.chars().any(char::is_whitespace) {
        return None;
    }
    let is_uri = destination.starts_with("http://")
        || destination.starts_with("https://")
        || destination.starts_with("mailto:");
    let is_email = !destination.contains([':', '/'])
        && destination
            .split_once('@')
            .is_some_and(|(local, host)| !local.is_empty() && !host.is_empty());
    let href = if is_uri {
        destination.to_owned()
    } else if is_email {
        format!("mailto:{destination}")
    } else {
        return None;
    };
    native_markdown_safe_link(&href)
}

fn native_markdown_table_row(line: &str) -> Option<Vec<Vec<NativeMarkdownRun>>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let body = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed.strip_prefix('|').unwrap_or(trimmed));
    let mut cells = Vec::new();
    for cell in body.split('|') {
        let cell = cell.trim();
        if cell.is_empty() {
            return None;
        }
        cells.push(parse_native_markdown_inline(cell)?);
    }
    (!cells.is_empty()).then_some(cells)
}

fn native_markdown_table_separator(line: &str, columns: usize) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return false;
    }
    let body = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed.strip_prefix('|').unwrap_or(trimmed));
    let cells = body.split('|').map(str::trim).collect::<Vec<_>>();
    cells.len() == columns
        && cells.iter().all(|cell| {
            let cell = cell.strip_prefix(':').unwrap_or(cell);
            let cell = cell.strip_suffix(':').unwrap_or(cell);
            !cell.is_empty() && cell.bytes().all(|byte| byte == b'-')
        })
}

fn parse_native_markdown_blocks(text: &str) -> Option<Vec<NativeMarkdownBlock>> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix(">"))
            .is_some()
        {
            let mut rows = Vec::new();
            while index < lines.len() {
                let candidate = lines[index].trim_start();
                let Some(content) = candidate
                    .strip_prefix("> ")
                    .or_else(|| candidate.strip_prefix(">"))
                else {
                    break;
                };
                let runs = parse_native_markdown_inline(content.trim())?;
                rows.push(runs);
                index += 1;
            }
            if rows.is_empty() {
                return None;
            }
            blocks.push(NativeMarkdownBlock::Blockquote(rows));
            continue;
        }
        if is_native_markdown_thematic_break(trimmed) {
            blocks.push(NativeMarkdownBlock::ThematicBreak);
            index += 1;
            continue;
        }
        if let Some(_info) = trimmed.strip_prefix("```") {
            let mut code = Vec::new();
            index += 1;
            let mut closed = false;
            while index < lines.len() {
                let candidate = lines[index];
                if candidate.trim_start().starts_with("```") {
                    closed = true;
                    index += 1;
                    break;
                }
                code.push(
                    candidate
                        .strip_prefix("    ")
                        .unwrap_or(candidate)
                        .to_owned(),
                );
                index += 1;
            }
            if !closed {
                return None;
            }
            blocks.push(NativeMarkdownBlock::FencedCode(code));
            continue;
        }
        return None;
    }
    (!blocks.is_empty()).then_some(blocks)
}

fn is_native_markdown_thematic_break(line: &str) -> bool {
    let Some(first) = line.chars().next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_')
        && line
            .chars()
            .all(|character| character == first || character.is_ascii_whitespace())
        && line.chars().filter(|character| *character == first).count() >= 3
}

fn draw_native_markdown_blocks(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    _fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
) -> bool {
    let Some(blocks) = parse_native_markdown_blocks(text) else {
        return false;
    };
    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let regular = measurer.font;
    let local_class = class.unwrap_or("");
    let base_size = f64::from(font_size);
    let code_size = base_size * 0.85;
    let code_line_height = code_size * 1.45;
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
    );
    let mut y = 0.0;
    for (block_index, block) in blocks.iter().enumerate() {
        if block_index > 0 {
            y += 24.0;
        }
        match block {
            NativeMarkdownBlock::Blockquote(rows) => {
                for (row_index, runs) in rows.iter().enumerate() {
                    let baseline = y + 18.0 + row_index as f64 * 24.0;
                    let mut cursor_x = 20.0;
                    for run in runs {
                        native_markdown_text(
                            svg,
                            cursor_x,
                            baseline,
                            run,
                            base_size,
                            "#676C7E",
                            local_class,
                            preserve_text_geometry_precision,
                        );
                        cursor_x += regular.measure_advance(&run.text, base_size);
                    }
                }
                let height = 24.0 * rows.len() as f64;
                let _ = write!(
                    svg,
                    "<rect x=\"0.000\" y=\"{}\" width=\"4.000\" height=\"{}\" rx=\"0.000\" class=\"fill-B1\" fill=\"#0D32B2\"/>",
                    native_markdown_number(y),
                    native_markdown_number(height)
                );
                y += height;
            }
            NativeMarkdownBlock::ThematicBreak => {
                let _ = write!(
                    svg,
                    "<rect x=\"0.000\" y=\"{}\" width=\"{}\" height=\"4.000\" rx=\"0.000\" class=\"fill-B1\" fill=\"#0D32B2\"/>",
                    native_markdown_number(y),
                    native_markdown_number(rect.size.width.max(0.0))
                );
                y += 4.0;
            }
            NativeMarkdownBlock::FencedCode(lines) => {
                let code_height = if lines.is_empty() {
                    32.0
                } else {
                    lines.len() as f64 * code_line_height + 32.0
                };
                let _ = write!(
                    svg,
                    "<rect x=\"0.000\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6.000\" class=\"fill-N6\" fill=\"#EEF1F8\"/>",
                    native_markdown_number(y),
                    native_markdown_number(rect.size.width.max(0.0)),
                    native_markdown_number(code_height)
                );
                for (line_index, line) in lines.iter().enumerate() {
                    if line.is_empty() {
                        continue;
                    }
                    let run = NativeMarkdownRun {
                        direct_code_text: false,
                        link_title: None,
                        code_link_title: None,
                        code_link: None,
                        decoration_accent: false,
                        padding_only: false,
                        text: line.clone(),
                        bold: false,
                        semibold: false,
                        italic: false,
                        bold_after_italic: false,
                        italic_after_bold: false,
                        code: true,
                        primary_mono: false,
                        code_bold: false,
                        code_italic: false,
                        code_group: 0,
                        decoration_group: 0,
                        strikethrough: false,
                        link: None,
                        line_break: false,
                    };
                    native_markdown_text(
                        svg,
                        16.0,
                        y + 30.36 + line_index as f64 * code_line_height,
                        &run,
                        code_size,
                        "#0A0F25",
                        local_class,
                        preserve_text_geometry_precision,
                    );
                }
                y += code_height;
            }
        }
    }
    svg.push_str("</g></svg>");
    true
}

fn draw_native_markdown_table(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
) -> bool {
    markdown_table::paint(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    )
}

fn draw_native_markdown_list(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
) -> bool {
    draw_native_markdown_list_at(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
        0.0,
    )
}

fn draw_native_markdown_list_at(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
    flow_y: f64,
) -> bool {
    let mut items = Vec::new();
    let mut ordered = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        let (marker, content) = if let Some(content) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            if ordered == Some(true) {
                return false;
            }
            ordered = Some(false);
            (None, content)
        } else {
            let Some(dot) = trimmed.find(". ") else {
                return false;
            };
            if dot == 0 || !trimmed[..dot].bytes().all(|byte| byte.is_ascii_digit()) {
                return false;
            }
            if ordered == Some(false) {
                return false;
            }
            ordered = Some(true);
            (Some(format!("{}.", &trimmed[..dot])), &trimmed[dot + 2..])
        };
        let Some(runs) = parse_native_markdown_inline(content) else {
            return false;
        };
        items.push((marker, runs));
    }
    if items.is_empty() {
        return false;
    }
    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let regular = measurer.font;
    let local_class = class.unwrap_or("");
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
    );
    let base_size = f64::from(font_size);
    let (ascent, descent) = regular.css_box_metrics(base_size);
    let parent_baseline = (base_size * 1.5 - ascent - descent).max(0.0) / 2.0 + ascent;
    for (index, (marker, runs)) in items.iter().enumerate() {
        let baseline =
            (flow_y + index as f64 * (base_size * 1.5 + base_size * 0.25)) + parent_baseline;
        if let Some(marker) = marker {
            let marker_width = regular.measure_with_dot(marker, base_size).width;
            native_markdown_text(
                svg,
                base_size * 2.0 - marker_width - regular.measure_advance(" ", base_size),
                baseline,
                &NativeMarkdownRun {
                    direct_code_text: false,
                    link_title: None,
                    code_link_title: None,
                    code_link: None,
                    decoration_accent: false,
                    padding_only: false,
                    text: marker.clone(),
                    bold: false,
                    semibold: false,
                    italic: false,
                    bold_after_italic: false,
                    italic_after_bold: false,
                    code: false,
                    primary_mono: false,
                    code_bold: false,
                    code_italic: false,
                    code_group: 0,
                    decoration_group: 0,
                    strikethrough: false,
                    link: None,
                    line_break: false,
                },
                base_size,
                fill,
                local_class,
                preserve_text_geometry_precision,
            );
        } else {
            let side = native_markdown_snap(0.3 * base_size).max(3.0);
            let center = base_size * 2.0 - base_size / 2.0 - 6.5;
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" class=\"fill-N1\" fill=\"{}\"/>",
                native_markdown_number(native_markdown_snap(center - side / 2.0)),
                native_markdown_text_number(
                    baseline - (1.5 * side).floor(),
                    preserve_text_geometry_precision
                ),
                native_markdown_number(side),
                native_markdown_number(side),
                native_markdown_number(side / 2.0),
                escape(fill)
            );
        }
        // Source paintInlineNodes uses the same CSS line painter for list
        // contents and paragraphs. Share code-box metrics and paint all inline
        // backgrounds before glyphs, rather than one code chip per text run.
        let parsed = split_native_markdown_lines(runs);
        let mut child = String::new();
        if !draw_native_markdown_paragraph_lines(
            &mut child,
            &parsed,
            Rect {
                origin: Point::default(),
                size: rect.size,
            },
            font_size,
            fill,
            class,
            preserve_text_geometry_precision,
            family,
            Point {
                x: base_size * 2.0,
                y: flow_y + index as f64 * base_size * 1.75,
            },
        ) {
            return false;
        }
        let inside = &child[child.find('>').unwrap() + 1..child.len() - "</svg>".len()];
        // The list already owns the Markdown style group. Keep one Source
        // group for the complete block, rather than introducing a second text
        // ownership/reference frame around each line.
        let content = &inside[inside.find('>').unwrap() + 1..inside.len() - "</g>".len()];
        svg.push_str(content);
    }
    svg.push_str("</g></svg>");
    true
}

fn native_markdown_number(value: f64) -> String {
    // D2 v0.9.0's Markdown SVG serializer uses mdFloat, which rounds to three
    // decimal places and canonicalizes values below half a thousandth to
    // positive zero. Keep this local to Markdown primitives; the outer SVG
    // target uses its separate six-decimal coordinate formatter.
    let value = if value.abs() < 0.0005 { 0.0 } else { value };
    format!("{value:.3}")
}

fn native_markdown_text_number(value: f64, preserve_precision: bool) -> String {
    if preserve_precision && value.is_finite() {
        value.to_string()
    } else {
        native_markdown_number(value)
    }
}

fn native_markdown_snap(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn native_markdown_link_open(svg: &mut String, link: &str, title: Option<&str>) {
    let _ = write!(
        svg,
        "<a href=\"{}\" xlink:href=\"{}\">",
        escape(link),
        escape(link)
    );
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        let _ = write!(svg, "<title>{}</title>", escape(title));
    }
}

fn native_markdown_text(
    svg: &mut String,
    x: f64,
    baseline: f64,
    run: &NativeMarkdownRun,
    font_size: f64,
    fill: &str,
    class: &str,
    preserve_text_geometry_precision: bool,
) {
    if run.padding_only {
        return;
    }
    // Goldmark's painter keeps the innermost concrete face and represents the
    // outer emphasis axis as a synthetic CSS property.  For em > strong the
    // concrete face is bold with synthetic italic; for strong > em it is
    // italic with synthetic bold.  Selecting a single generic face merely
    // because both flags are set changes the embedded face and raster edge.
    // D2's Markdown SVG painter selects concrete faces through role classes.
    // It emits a font-weight attribute only when the outer emphasis axis is
    // synthetic; adding the concrete role's weight here changes rasterized
    // glyph selection on platforms whose SVG resolver handles presentation
    // attributes before the generated class rules.
    let (font_class, font_family, weight): (&str, &str, Option<&str>) = if run.code {
        (
            if run.code_bold {
                "text-mono-bold"
            } else if run.code_italic {
                "text-mono-italic"
            } else {
                "text-mono"
            },
            "Source Code Pro, monospace",
            None,
        )
    } else if run.primary_mono {
        (
            if run.bold_after_italic {
                "text-mono-bold"
            } else if run.italic_after_bold {
                "text-mono-italic"
            } else if run.semibold {
                "text-mono-semibold"
            } else if run.bold {
                "text-mono-bold"
            } else if run.italic {
                "text-mono-italic"
            } else {
                "text-mono"
            },
            "Source Code Pro, monospace",
            None,
        )
    } else if run.bold_after_italic {
        ("text-bold", "Source Sans Pro, sans-serif", None)
    } else if run.italic_after_bold {
        ("text-italic", "Source Sans Pro, sans-serif", None)
    } else if run.semibold {
        ("text-semibold", "Source Sans Pro, sans-serif", None)
    } else if run.bold {
        ("text-bold", "Source Sans Pro, sans-serif", None)
    } else if run.italic {
        ("text-italic", "Source Sans Pro, sans-serif", None)
    } else {
        ("text", "Source Sans Pro, sans-serif", None)
    };
    let color = run.link.as_deref().map_or(fill, |_| "#0D32B2");
    // Original markdownRolePaints maps link accent to theme B2, including
    // the fill used for inherited synthetic-bold scene strokes. Keep the
    // default palette color as the SVG fallback and resolve active themes
    // through the existing palette classes.
    let inherited_class = if run.link.is_some() {
        if preserve_text_geometry_precision
            && ((!run.code && run.italic_after_bold) || (run.code && run.bold && !run.code_bold))
        {
            "d2-rs-text-B2 d2-rs-stroke-B2"
        } else {
            "d2-rs-text-B2"
        }
    } else {
        class
    };
    let classes = if inherited_class.is_empty() {
        format!("md-text {font_class}")
    } else {
        format!("md-text {font_class} {inherited_class}")
    };
    if let Some(link) = run.link.as_deref().filter(|link| !link.is_empty()) {
        native_markdown_link_open(svg, link, run.link_title.as_deref());
    }
    let synthetic_bold = if (!run.code && run.italic_after_bold)
        || (run.code && run.bold && !run.code_bold)
    {
        if preserve_text_geometry_precision {
            format!(
                " stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-miterlimit=\"4\"",
                escape(color),
                native_markdown_text_number((font_size / 32.0).max(0.35), true)
            )
        } else {
            " font-weight=\"bold\"".to_owned()
        }
    } else {
        String::new()
    };
    let synthetic_italic =
        if (!run.code && run.bold_after_italic) || (run.code && run.italic && !run.code_italic) {
            " font-style=\"italic\""
        } else {
            ""
        };
    let _ = write!(
        svg,
        "<text x=\"{}\" y=\"{}\" class=\"{}\" fill=\"{}\" font-family=\"{}\" font-size=\"{}\"{}{}{} xml:space=\"preserve\">{}</text>",
        native_markdown_text_number(x, preserve_text_geometry_precision),
        native_markdown_text_number(baseline, preserve_text_geometry_precision),
        classes,
        escape(color),
        font_family,
        native_markdown_text_number(font_size, preserve_text_geometry_precision),
        weight.map_or_else(String::new, |value| format!(" font-weight=\"{value}\"")),
        synthetic_bold,
        synthetic_italic,
        escape(&run.text)
    );
    if run.strikethrough {
        let width = native_markdown_run_width(run, font_size);
        if width.is_finite() && width > 0.0 {
            // Goldmark's SVG painter represents a deletion as a one-pixel
            // foreground rule centered on the text's x-height. Keeping this
            // as a primitive (rather than CSS text-decoration) matches the
            // v0.9.0 scene structure and remains stable in raster exports.
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"1.000\" rx=\"0.000\" class=\"fill-N1\" fill=\"{}\"/>",
                native_markdown_text_number(x, preserve_text_geometry_precision),
                native_markdown_text_number(
                    baseline - font_size * 0.375,
                    preserve_text_geometry_precision
                ),
                native_markdown_text_number(width, preserve_text_geometry_precision),
                escape(color)
            );
        }
    }
    if run.link.as_deref().is_some_and(|link| !link.is_empty()) {
        svg.push_str("</a>");
    }
}

/// Paint a plain paragraph (including inline emphasis and soft/hard breaks)
/// with the same native SVG primitives used by D2's Markdown renderer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BundledFamily {
    Sans,
    Mono,
    HandDrawn,
}

#[derive(Clone, Copy, Default)]
enum MarkdownFamily {
    #[default]
    Sans,
    Mono,
    HandDrawn,
    Pair {
        primary: BundledFamily,
        mono: BundledFamily,
        mono_label: bool,
    },
}
impl MarkdownFamily {
    fn primary_kind(self) -> BundledFamily {
        match self {
            Self::Sans => BundledFamily::Sans,
            Self::Mono => BundledFamily::Mono,
            Self::HandDrawn => BundledFamily::HandDrawn,
            Self::Pair { primary, .. } => primary,
        }
    }
    fn mono_kind(self) -> BundledFamily {
        match self {
            Self::Pair { mono, .. } => mono,
            _ => BundledFamily::Mono,
        }
    }
    fn uses_mono_css(self) -> bool {
        matches!(
            self,
            Self::Mono
                | Self::Pair {
                    mono_label: true,
                    ..
                }
        )
    }
    fn for_mono_text(self) -> Self {
        Self::Pair {
            primary: self.mono_kind(),
            mono: self.mono_kind(),
            mono_label: true,
        }
    }
}

fn draw_native_markdown_paragraph(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
) -> bool {
    let mut logical_lines = Vec::<String>::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            return false;
        }
        let trimmed_end = line.trim_end();
        let hard_break = line.ends_with("  ");
        let content = if hard_break {
            trimmed_end.trim_end_matches(' ')
        } else {
            trimmed_end
        };
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(content.trim_start());
        if hard_break {
            logical_lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        logical_lines.push(current);
    }
    if logical_lines.is_empty()
        || logical_lines.iter().any(|line| {
            let trimmed = line.trim_start();
            is_native_markdown_thematic_break(trimmed)
                || trimmed.starts_with('>')
                || trimmed.starts_with("```")
                || trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || trimmed.starts_with("+ ")
                || (trimmed.find(". ").is_some_and(|dot| {
                    dot > 0 && trimmed[..dot].bytes().all(|b| b.is_ascii_digit())
                }))
                || (trimmed.starts_with('#')
                    && trimmed
                        .as_bytes()
                        .get(1)
                        .is_some_and(|byte| byte.is_ascii_whitespace()))
        })
    {
        return false;
    }
    let Some(parsed) = logical_lines
        .iter()
        .map(|line| parse_native_markdown_inline(line))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let parsed = parsed
        .iter()
        .flat_map(|runs| split_native_markdown_lines(runs))
        .collect::<Vec<_>>();
    draw_native_markdown_paragraph_lines(
        svg,
        &parsed,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
        Point::default(),
    )
}

fn draw_native_markdown_paragraph_lines(
    svg: &mut String,
    parsed: &[Vec<NativeMarkdownRun>],
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
    inline_origin: Point,
) -> bool {
    draw_native_markdown_inline_context(
        svg,
        parsed,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
        inline_origin,
        false,
        "",
        None,
    )
}

fn draw_native_markdown_inline_context(
    svg: &mut String,
    parsed: &[Vec<NativeMarkdownRun>],
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
    inline_origin: Point,
    table_cell: bool,
    align: &str,
    painted_height: Option<&mut f64>,
) -> bool {
    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let (regular, bold, italic, mono) = (
        measurer.font,
        measurer.bold_font,
        measurer.italic_font,
        measurer.mono_font,
    );
    let local_class = class.unwrap_or("");
    let width = rect.size.width.max(1.0);
    let height = rect.size.height.max(1.0);
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">",
        number(rect.origin.x),
        number(rect.origin.y),
        number(width),
        number(height),
        number(width),
        number(height),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        }
    );
    let base_size = f64::from(font_size);
    let default_line_height = base_size * 1.5;
    let measurer = native_markdown_measurer(family).unwrap();
    let measure = |run: &NativeMarkdownRun, text: &str| {
        let font = if run.code && run.code_bold {
            measurer.mono_bold_font
        } else if run.code && run.code_italic {
            measurer.mono_italic_font
        } else if run.code {
            mono
        } else if table_cell && run.italic_after_bold {
            italic
        } else if run.semibold {
            measurer.semibold_font
        } else if run.bold {
            bold
        } else if run.italic {
            italic
        } else {
            regular
        };
        let text = text.replace('\u{ad}', "");
        font.measure_markdown_css_advance(&text, base_size) * if run.code { 0.85 } else { 1.0 }
    };
    let lines = if parsed.iter().flatten().next().is_none() {
        Vec::new()
    } else {
        markdown_wrap::wrap(parsed, width - inline_origin.x, base_size, measure)
    };
    let mut painted = 0.0;
    for tokens in &lines {
        let all_code = tokens
            .iter()
            .filter(|t| !t.padding && !t.run.text.is_empty())
            .all(|t| t.run.code)
            && tokens.iter().any(|t| !t.padding && !t.run.text.is_empty());
        let line_height = if table_cell {
            if all_code {
                mono.css_normal_line_height(base_size * 0.85)
                    .max((base_size * 0.85 * 1.4).round())
            } else {
                regular.css_normal_line_height(base_size).max(
                    tokens
                        .iter()
                        .filter(|t| !t.padding && !t.run.code)
                        .map(|t| {
                            let font = if t.run.semibold {
                                measurer.semibold_font
                            } else if t.run.bold {
                                bold
                            } else if t.run.italic {
                                italic
                            } else {
                                regular
                            };
                            font.markdown_fallback_line_height(&t.run.text, base_size)
                        })
                        .fold(0.0, f64::max),
                )
            }
        } else {
            default_line_height
        };
        let (ascent, descent) = regular.css_box_metrics(base_size);
        let parent_baseline = (line_height - ascent - descent).max(0.0) / 2.0 + ascent;
        let code_size = base_size * 0.85;
        let (code_ascent, code_descent) = mono.css_box_metrics(code_size);
        let vertical_padding = code_size * 0.2;
        let code_height = code_ascent + code_descent + 2.0 * vertical_padding;
        let content_height = (code_height - 2.0 * vertical_padding).max(0.0);
        let code_top = (line_height - code_height) / 2.0 + vertical_padding;
        let code_baseline =
            code_top + (content_height - code_ascent - code_descent).max(0.0) / 2.0 + code_ascent;
        let code_shift = parent_baseline - code_baseline;
        let code_shift = if table_cell { 0.0 } else { code_shift };
        let line_y = inline_origin.y + painted;
        painted += line_height;
        let baseline = line_y + parent_baseline;
        let extra =
            (width - inline_origin.x - tokens.iter().map(|t| t.width).sum::<f64>()).max(0.0);
        let line_x = inline_origin.x
            + match align {
                "center" => extra / 2.0,
                "right" => extra,
                _ => 0.0,
            };
        let mut cursor_x = line_x;
        let mut code_spans =
            std::collections::BTreeMap::<usize, (f64, f64, Option<String>, Option<String>)>::new();
        let mut decoration_spans =
            std::collections::BTreeMap::<usize, (f64, f64, String, &str)>::new();
        for token in tokens {
            if token.run.code_group > 0 {
                let span = code_spans.entry(token.run.code_group).or_insert((
                    cursor_x,
                    cursor_x,
                    token.run.link.clone(),
                    token.run.link_title.clone(),
                ));
                span.1 = cursor_x + token.width;
            }
            if token.run.decoration_group > 0 {
                let span = decoration_spans
                    .entry(token.run.decoration_group)
                    .or_insert((
                        cursor_x,
                        cursor_x,
                        if token.run.decoration_accent {
                            "#0D32B2"
                        } else {
                            fill
                        }
                        .to_owned(),
                        if token.run.decoration_accent {
                            "fill-B2"
                        } else {
                            local_class
                        },
                    ));
                span.1 = cursor_x + token.width;
            }
            cursor_x += token.width;
        }
        // Original paintInlineNodes composites complete code backgrounds before
        // glyphs, and paints each decoration group's continuous rule afterward.
        for (_, (start, end, link, title)) in code_spans {
            if let Some(link) = link.as_deref().filter(|link| !link.is_empty()) {
                native_markdown_link_open(svg, link, title.as_deref());
            }
            let primitive_start = svg.len();
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6.000\" class=\"fill-N6\" fill=\"#EEF1F8\"/>",
                native_markdown_text_number(start, preserve_text_geometry_precision),
                native_markdown_text_number(
                    line_y + (line_height - code_height) / 2.0 + code_shift,
                    preserve_text_geometry_precision
                ),
                native_markdown_text_number(end - start, preserve_text_geometry_precision),
                native_markdown_text_number(code_height, preserve_text_geometry_precision)
            );
            if link.as_deref().is_some_and(|link| !link.is_empty()) {
                markdown_scene::tag_primitive(
                    svg,
                    primitive_start,
                    start,
                    line_y + (line_height - code_height) / 2.0 + code_shift,
                    end - start,
                    code_height,
                    0.0,
                    false,
                );
                svg.push_str("</a>");
            }
        }
        cursor_x = line_x;
        for token in tokens {
            if !token.padding {
                let mut run = token.run.clone();
                run.strikethrough = false;
                run.primary_mono = family.uses_mono_css();
                let begin = svg.len();
                native_markdown_text(
                    svg,
                    cursor_x,
                    if table_cell && run.code {
                        line_y + code_baseline
                    } else {
                        baseline
                    },
                    &run,
                    if run.code { code_size } else { base_size },
                    fill,
                    local_class,
                    preserve_text_geometry_precision,
                );
                if run.link.as_deref().is_some_and(|link| !link.is_empty()) {
                    let font = if run.code && run.code_bold {
                        measurer.mono_bold_font
                    } else if run.code && run.code_italic {
                        measurer.mono_italic_font
                    } else if run.code {
                        mono
                    } else if run.italic_after_bold {
                        italic
                    } else if run.semibold {
                        measurer.semibold_font
                    } else if run.bold {
                        bold
                    } else if run.italic {
                        italic
                    } else {
                        regular
                    };
                    let height = font.measure_markdown_raw(&run.text, base_size).height
                        * if run.direct_code_text { 0.85 } else { 1.0 };
                    markdown_scene::tag_primitive(
                        svg,
                        begin,
                        cursor_x,
                        if table_cell && run.code {
                            line_y + code_baseline
                        } else {
                            baseline
                        },
                        token.width,
                        height,
                        if run.code { code_size } else { base_size },
                        (!run.code && run.bold_after_italic)
                            || (run.code && run.italic && !run.code_italic),
                    );
                }
                if token.discretionary_hyphen {
                    if let Some(position) = svg[begin..].find("<text ") {
                        svg.insert_str(
                            begin + position + 6,
                            &format!(
                                "textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" ",
                                native_markdown_text_number(
                                    token.width,
                                    preserve_text_geometry_precision
                                )
                            ),
                        );
                    }
                }
            }
            cursor_x += token.width;
        }
        for (_, (start, end, color, color_class)) in decoration_spans {
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"1.000\" rx=\"0.000\" class=\"{}\" fill=\"{}\"/>",
                native_markdown_text_number(start, preserve_text_geometry_precision),
                native_markdown_text_number(
                    native_markdown_snap(line_y + line_height / 2.0),
                    preserve_text_geometry_precision
                ),
                native_markdown_text_number(end - start, preserve_text_geometry_precision),
                escape(color_class),
                escape(&color)
            );
        }
    }
    if let Some(height) = painted_height {
        *height = painted;
    }
    svg.push_str("</g></svg>");
    true
}
/// Paint the small, common Markdown subset used by D2's native SVG path.
/// Complex Markdown stays on the deterministic HTML fallback until its block
/// painter has been ported; this gate prevents partially painted tables,
/// images, and raw HTML from changing their established behavior.
fn draw_native_markdown_subset(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
) -> bool {
    draw_native_markdown_subset_with_precision(svg, text, rect, font_size, fill, class, false)
}

fn draw_native_markdown_subset_with_precision(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
) -> bool {
    draw_native_markdown_subset_in_family(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        MarkdownFamily::Sans,
    )
}

fn draw_native_markdown_subset_in_family(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
) -> bool {
    if markdown_flow::draw(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return true;
    }
    if draw_native_markdown_flow(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return true;
    }
    if draw_native_markdown_blocks(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return true;
    }
    if draw_native_markdown_table(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return true;
    }
    if draw_native_markdown_list(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return true;
    }
    if !text.lines().next().is_some_and(|line| {
        let trimmed = line.trim_start();
        let heading_level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
        (1..=6).contains(&heading_level) && trimmed.as_bytes().get(heading_level) == Some(&b' ')
    }) && draw_native_markdown_paragraph(
        svg,
        text,
        rect,
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return true;
    }
    if text.contains("\n\n")
        && draw_native_markdown_paragraph_document(
            svg,
            text,
            rect,
            font_size,
            fill,
            class,
            preserve_text_geometry_precision,
            family,
        )
    {
        return true;
    }
    let mut lines = text.lines().peekable();
    let Some(first) = lines.next() else {
        return false;
    };
    let first_trimmed = first.trim_start();
    let heading_level = first_trimmed
        .bytes()
        .take_while(|byte| *byte == b'#')
        .count();
    if !(1..=6).contains(&heading_level)
        || first_trimmed.as_bytes().get(heading_level) != Some(&b' ')
    {
        return false;
    }
    let Some(mut heading_runs) =
        parse_native_markdown_inline(first_trimmed[heading_level + 1..].trim())
    else {
        return false;
    };
    for run in &mut heading_runs {
        if !run.bold {
            run.semibold = true;
        }
    }
    let mut list_items = Vec::new();
    while let Some(line) = lines.peek().copied() {
        if line.trim().is_empty() {
            lines.next();
            continue;
        }
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ")) {
            break;
        }
        lines.next();
        let Some(runs) = parse_native_markdown_inline(&trimmed[2..]) else {
            return false;
        };
        list_items.push(runs);
    }
    let mut paragraph = String::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(line.trim());
    }
    let paragraph_runs = if paragraph.is_empty() {
        Vec::new()
    } else {
        let Some(runs) = parse_native_markdown_inline(&paragraph) else {
            return false;
        };
        runs
    };

    if list_items.is_empty() && paragraph_runs.is_empty() {
        return draw_native_markdown_heading(
            svg,
            &heading_runs,
            heading_level,
            rect,
            font_size,
            fill,
            class,
            preserve_text_geometry_precision,
            family,
            None,
        );
    }

    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let regular = measurer.font;
    let bold = measurer.bold_font;
    let italic = measurer.italic_font;
    let semibold = measurer.semibold_font;
    let mono = measurer.mono_font;
    let base_size = f64::from(font_size);
    let heading_size = base_size
        * match heading_level {
            1 => 2.0,
            2 => 1.5,
            3 => 1.25,
            4 => 1.0,
            5 => 0.875,
            _ => 0.85,
        };
    let local_class = class.unwrap_or("");
    let emit_runs =
        |out: &mut String, runs: &[NativeMarkdownRun], start_x: f64, baseline: f64, size: f64| {
            let mut cursor_x = start_x;
            for run in runs {
                let font = if run.code {
                    mono
                } else if run.semibold {
                    semibold
                } else if run.bold {
                    bold
                } else if run.italic {
                    italic
                } else {
                    regular
                };
                let run_size = if run.code { size * 0.85 } else { size };
                native_markdown_text(
                    out,
                    cursor_x,
                    baseline,
                    run,
                    run_size,
                    fill,
                    local_class,
                    preserve_text_geometry_precision,
                );
                // D2's Markdown painter measures an inline-code run at the
                // inherited CSS size and then applies the `0.85em` code
                // scale.  Measuring directly at the fractional painted size
                // rounds each glyph differently in x/image's 26.6 metrics,
                // shifting subsequent runs by a fraction of a pixel.
                cursor_x += if run.code {
                    font.measure_advance(&run.text, size) * 0.85
                } else {
                    font.measure_advance(&run.text, run_size)
                };
            }
        };

    let mut local_svg = String::new();
    let _ = write!(
        local_svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
    );
    // The pinned GitHub Markdown CSS uses a 1.25em heading line box, .3em
    // bottom padding, and a one-pixel rule for h1/h2.
    let heading_baseline = if heading_level <= 2 {
        heading_size - 1.0
    } else {
        heading_size * 0.9583333333
    };
    emit_runs(
        &mut local_svg,
        &heading_runs,
        0.0,
        heading_baseline,
        heading_size,
    );
    if heading_level <= 2 {
        let heading_height = heading_size * 1.25 + heading_size * 0.3 + 1.0;
        let y = native_markdown_snap(heading_height - 1.0);
        let _ = write!(
            local_svg,
            "<rect x=\"0.000\" y=\"{}\" width=\"{}\" height=\"1.000\" rx=\"0.000\" class=\"fill-B2\" fill=\"#0D32B2\"/>",
            native_markdown_number(y),
            native_markdown_number(rect.size.width.max(1.0))
        );
    }
    let heading_height = if heading_level <= 2 {
        heading_size * 1.25 + heading_size * 0.3 + 1.0
    } else {
        heading_size * 1.25
    };
    let mut list_y = heading_height + 16.0;
    for runs in &list_items {
        let baseline = list_y + base_size * 1.125;
        let _ = write!(
            local_svg,
            "<rect x=\"15.000\" y=\"{}\" width=\"5.000\" height=\"5.000\" rx=\"2.500\" class=\"fill-N1\" fill=\"{}\"/>",
            native_markdown_number(baseline - 7.0),
            escape(fill)
        );
        emit_runs(&mut local_svg, runs, base_size * 2.0, baseline, base_size);
        list_y += base_size * 1.5 + base_size * 0.25;
    }
    if !paragraph_runs.is_empty() {
        // The final list item has no trailing 0.25em inter-item margin; its
        // 16px block margin starts immediately after the last line box.
        let paragraph_y = if list_items.is_empty() {
            heading_height + 16.0
        } else {
            list_y + 12.0
        };
        let baseline = paragraph_y + base_size * 1.125;
        let mut cursor_x = 0.0;
        for run in &paragraph_runs {
            let font = if run.code {
                mono
            } else if run.bold {
                bold
            } else if run.italic {
                italic
            } else {
                regular
            };
            let run_size = if run.code {
                base_size * 0.85
            } else {
                base_size
            };
            if run.code {
                let content_width = font.measure_advance(&run.text, base_size) * 0.85;
                let padding = base_size * 0.4 * 0.85;
                let width = content_width + padding * 2.0;
                let _ = write!(
                    local_svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6.000\" class=\"fill-N6\" fill=\"#EEF1F8\"/>",
                    native_markdown_number(cursor_x),
                    native_markdown_number(paragraph_y + 2.28),
                    native_markdown_number(width),
                    native_markdown_number(base_size * 1.4025)
                );
                native_markdown_text(
                    &mut local_svg,
                    cursor_x + padding,
                    baseline,
                    run,
                    run_size,
                    fill,
                    local_class,
                    preserve_text_geometry_precision,
                );
                cursor_x += width;
            } else {
                native_markdown_text(
                    &mut local_svg,
                    cursor_x,
                    baseline,
                    run,
                    run_size,
                    fill,
                    local_class,
                    preserve_text_geometry_precision,
                );
                cursor_x += font.measure_advance(&run.text, run_size);
            }
        }
    }
    local_svg.push_str("</g></svg>");
    svg.push_str(&local_svg);
    true
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn draw_native_markdown_heading(
    svg: &mut String,
    runs: &[NativeMarkdownRun],
    level: usize,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    private: bool,
    family: MarkdownFamily,
    painted_height: Option<&mut f64>,
) -> bool {
    draw_native_markdown_heading_at(
        svg,
        runs,
        level,
        rect,
        font_size,
        fill,
        class,
        private,
        family,
        painted_height,
        0.0,
    )
}

fn draw_native_markdown_heading_at(
    svg: &mut String,
    runs: &[NativeMarkdownRun],
    level: usize,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    private: bool,
    family: MarkdownFamily,
    painted_height: Option<&mut f64>,
    origin_y: f64,
) -> bool {
    draw_native_markdown_heading_at_origin(
        svg,
        runs,
        level,
        rect,
        font_size,
        fill,
        class,
        private,
        family,
        painted_height,
        Point {
            x: 0.0,
            y: origin_y,
        },
    )
}

fn draw_native_markdown_heading_at_origin(
    svg: &mut String,
    runs: &[NativeMarkdownRun],
    level: usize,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    private: bool,
    family: MarkdownFamily,
    painted_height: Option<&mut f64>,
    origin: Point,
) -> bool {
    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let (semibold, bold, italic, mono) = (
        measurer.semibold_font,
        measurer.bold_font,
        measurer.italic_font,
        measurer.mono_font,
    );
    let base_size = f64::from(font_size);
    let scale = match level {
        1 => 2.0,
        2 => 1.5,
        3 => 1.25,
        4 => 1.0,
        5 => 0.875,
        _ => 0.85,
    };
    let size = base_size * scale;
    let line_height = size * 1.25;
    let (ascent, descent) = semibold.css_box_metrics(size);
    let parent_baseline = (line_height - ascent - descent).max(0.0) / 2.0 + ascent;
    let correction = if level == 5 || (level == 2 && family.primary_kind() == BundledFamily::Sans) {
        1.0
    } else {
        0.0
    };
    let (code_ascent, code_descent) = mono.css_box_metrics(size);
    let code_baseline = (line_height - code_ascent - code_descent).max(0.0) / 2.0 + code_ascent;
    let code_shift = parent_baseline - code_baseline;
    let width = rect.size.width.max(1.0);
    let height = rect.size.height.max(1.0);
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">",
        number(rect.origin.x),
        number(rect.origin.y),
        number(width),
        number(height),
        number(width),
        number(height),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        }
    );
    let font_for = |run: &NativeMarkdownRun| {
        if run.code {
            mono
        } else if run.bold {
            bold
        } else if run.italic {
            italic
        } else {
            semibold
        }
    };
    // markdown_svg.go paints headings through paintInlineNodes, so they wrap
    // with the same atomized wrapInlineTokens as paragraphs: per-word atoms
    // and separately measured spaces, not one kerned run.
    let lines: Vec<Vec<NativeMarkdownRun>> = if runs.is_empty() {
        vec![Vec::new()]
    } else {
        markdown_wrap::wrap_with_code_padding(&[runs.to_vec()], width, size, 0.2 * size, |run, text| {
            font_for(run).measure_advance(&text.replace('\u{ad}', ""), base_size) * scale
        })
        .into_iter()
        .map(|tokens| {
            tokens
                .into_iter()
                .filter(|token| !token.padding)
                .map(|token| token.run)
                .collect()
        })
        .collect()
    };
    let line_count = lines.len();
    for (line_index, runs) in lines.iter().enumerate() {
        let top = origin.y + line_index as f64 * line_height;
        // Go paintInlineNodes paints all code backgrounds before text. Heading
        // code keeps the heading size, uses .2em horizontal padding and the
        // complete heading line box, shifted to the parent baseline.
        let mut cursor = origin.x;
        for run in runs {
            let advance = font_for(run).measure_advance(&run.text, base_size) * scale;
            if run.code {
                let start = cursor;
                cursor += 0.2 * size;
                cursor += advance;
                cursor += 0.2 * size;
                let _ = write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6.000\" class=\"fill-N6\" fill=\"#EEF1F8\"/>",
                    native_markdown_text_number(start, private),
                    native_markdown_text_number(top + code_shift, private),
                    native_markdown_text_number(cursor - start, private),
                    native_markdown_text_number(line_height, private)
                );
            } else {
                cursor += advance
            }
        }
        let heading_class = if level == 6 {
            "fill-N2"
        } else {
            class.unwrap_or("")
        };
        let heading_fill = if level == 6 { "#6F7E8B" } else { fill };
        cursor = origin.x;
        for original in runs {
            let mut run = original.clone();
            run.primary_mono = family.uses_mono_css();
            run.semibold = !run.bold && !run.italic && !run.code;
            if run.code {
                cursor += 0.2 * size
            }
            let font = font_for(&run);
            let baseline = if run.code {
                code_baseline + code_shift
            } else {
                let (a, d) = font.css_box_metrics(size);
                (line_height - a - d).max(0.0) / 2.0 + a
            };
            let primitive_start = svg.len();
            native_markdown_text(
                svg,
                cursor,
                top + baseline
                    - if run.code && level == 2 {
                        0.0
                    } else {
                        correction
                    },
                &run,
                size,
                heading_fill,
                heading_class,
                private,
            );
            if run.link.as_deref().is_some_and(|link| !link.is_empty()) {
                markdown_scene::tag_primitive(
                    svg,
                    primitive_start,
                    cursor,
                    top + baseline
                        - if run.code && level == 2 {
                            0.0
                        } else {
                            correction
                        },
                    font.measure_advance(&run.text, base_size) * scale,
                    font.measure_markdown_raw(&run.text, base_size).height * scale,
                    size,
                    run.bold_after_italic,
                );
            }
            cursor += font.measure_advance(&run.text, base_size) * scale;
            if run.code {
                cursor += 0.2 * size
            }
        }
    }
    if level <= 2 {
        let painted_height = line_height * line_count as f64 + 0.3 * size + 1.0;
        let y1 = native_markdown_snap(origin.y + painted_height - 1.0);
        let y2 = native_markdown_snap(origin.y + painted_height);
        let _ = write!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"0.000\" class=\"fill-B2\" fill=\"#0D32B2\"/>",
            native_markdown_number(native_markdown_snap(origin.x)),
            native_markdown_number(y1),
            native_markdown_number(
                native_markdown_snap(origin.x + width) - native_markdown_snap(origin.x)
            ),
            native_markdown_number(y2 - y1)
        );
    }
    if let Some(height) = painted_height {
        *height = line_height * line_count as f64 + if level <= 2 { 0.3 * size + 1.0 } else { 0.0 };
    }
    svg.push_str("</g></svg>");
    true
}

fn prepare_native_markdown_text(svg: &mut String, from: usize, underline: bool, private: bool) {
    let original = svg[from..].to_owned();
    let mut decorated = String::with_capacity(original.len());
    let mut cursor = 0;
    while let Some(relative) = original[cursor..].find("<text ") {
        let start = cursor + relative;
        decorated.push_str(&original[cursor..start]);
        let Some(end) = original[start..].find('>').map(|end| start + end) else {
            break;
        };
        let tag = &original[start..end];
        let mut decorated_tag = tag.to_owned();
        if underline && let Some(value_start) = tag.find(" text-decoration=\"") {
            let value_start = value_start + " text-decoration=\"".len();
            if let Some(value_end) = tag[value_start..].find('"') {
                let value_end = value_start + value_end;
                let value = &tag[value_start..value_end];
                if !value.split_whitespace().any(|value| value == "underline") {
                    decorated_tag
                        .replace_range(value_start..value_end, &format!("underline {value}"));
                }
            }
        } else if underline {
            decorated_tag.push_str(" text-decoration=\"underline\"");
        }
        decorated.push_str(&decorated_tag);
        if private {
            let coordinate = |name: &str| -> Option<f64> {
                let prefix = format!(" {name}=\"");
                let value = tag.split_once(&prefix)?.1.split('"').next()?;
                value.parse().ok()
            };
            if let Some((x, y)) = coordinate("x").zip(coordinate("y")) {
                if !tag.contains(" id=\"") {
                    let _ = write!(
                        decorated,
                        " id=\"d2-rs-native-md-text-{}-{}\"",
                        from + start,
                        next_native_markdown_id()
                    );
                }
                let _ = write!(
                    decorated,
                    " data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\"",
                    x.to_bits(),
                    y.to_bits()
                );
            }
        }
        decorated.push('>');
        cursor = end + 1;
    }
    decorated.push_str(&original[cursor..]);
    svg.truncate(from);
    svg.push_str(&decorated);
    if private {
        for (element, prefix, marker) in [
            (
                "svg",
                "d2-rs-native-md-viewport",
                " data-d2-rs-markdown-reference=\"ordinary\"",
            ),
            ("rect", "d2-rs-native-md-rect", ""),
        ] {
            let mut cursor = from;
            while let Some(relative) = svg[cursor..].find(&format!("<{element} ")) {
                let start = cursor + relative;
                let Some(end) = svg[start..].find('>').map(|end| start + end) else {
                    break;
                };
                if !svg[start..end].contains(" id=\"") {
                    // Offsets are local to one object's buffer; the counter
                    // keeps these lookup identities unique across objects.
                    let attribute =
                        format!(" id=\"{prefix}-{start}-{}\"{marker}", next_native_markdown_id());
                    svg.insert_str(start + element.len() + 1, &attribute);
                    cursor = end + attribute.len() + 1;
                } else {
                    cursor = end + 1;
                }
                if element == "svg" {
                    break;
                }
            }
        }
    }
}

fn next_native_markdown_id() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// LayoutMarkdown paints its natural width independently of the exported
// label's clip dimensions. Keep the low-level forced-width painters available
// for callers that explicitly request wrapping; target rendering uses this
// layout/viewport boundary instead (original d2svg/markdown.go render).
fn draw_native_markdown_layout_viewport(
    svg: &mut String,
    text: &str,
    viewport: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    preserve_text_geometry_precision: bool,
    family: MarkdownFamily,
) -> bool {
    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let natural =
        measurer.markdown_dimensions(text, font_size, false, false, family.uses_mono_css());
    let mut fragment = String::new();
    if !draw_native_markdown_subset_in_family(
        &mut fragment,
        text,
        Rect {
            origin: viewport.origin,
            size: natural,
        },
        font_size,
        fill,
        class,
        preserve_text_geometry_precision,
        family,
    ) {
        return false;
    }
    let Some(opening_end) = fragment.find('>') else {
        return false;
    };
    if !fragment.starts_with("<svg ") {
        return false;
    }
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\">",
        number(viewport.origin.x),
        number(viewport.origin.y),
        number(viewport.size.width),
        number(viewport.size.height),
        number(viewport.size.width),
        number(viewport.size.height),
    );
    svg.push_str(&fragment[opening_end + 1..]);
    true
}

fn draw_markdown_label(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    position: LabelPosition,
    fill: &str,
    background: Option<&str>,
    background_class: Option<&str>,
    language: Option<&str>,
    style: TextStyle,
    class: Option<&str>,
    raster_text_geometry_precision: bool,
    primary_family: MarkdownFamily,
) {
    {
        // Both original SVG and scene builders take emphasis from Markdown
        // primitives; target-level bold/italic are not inherited by this label.
        let from = svg.len();
        let family = if style.mono {
            primary_family.for_mono_text()
        } else {
            primary_family
        };
        let rendered = draw_native_markdown_layout_viewport(
            svg,
            text,
            rect,
            font_size,
            fill,
            class,
            raster_text_geometry_precision,
            family,
        );
        if rendered {
            // Original d2scenebuild.buildMarkdownLabel and d2svg markdown
            // paint the target fill inside the clipped Markdown viewport.
            // Painting glyphs on a transparent layer and compositing later
            // changes the integer color rounding even over the same fill.
            if let Some(background) = background.filter(|value| {
                !value.is_empty()
                    && !value.eq_ignore_ascii_case("none")
                    && !value.eq_ignore_ascii_case("transparent")
            }) {
                if let Some(end) = svg[from..].find('>') {
                    let background = if shapes::gradient_definition(background).is_some() {
                        format!("url(#{})", shapes::gradient_id(background))
                    } else {
                        background.to_owned()
                    };
                    let rect = format!(
                        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
                        number(rect.size.width),
                        number(rect.size.height),
                        escape(&background),
                        background_class
                            .map(|class| format!(" class=\"{}\"", escape(class)))
                            .unwrap_or_default()
                    );
                    svg.insert_str(from + end + 1, &rect);
                }
            }
            if style.underline || raster_text_geometry_precision {
                prepare_native_markdown_text(
                    svg,
                    from,
                    style.underline,
                    raster_text_geometry_precision,
                );
            }
            return;
        }
    }
    let html = render_html(text.as_bytes());
    let text_align = match position {
        LabelPosition::InsideTopLeft
        | LabelPosition::InsideMiddleLeft
        | LabelPosition::InsideBottomLeft
        | LabelPosition::BorderTopLeft
        | LabelPosition::BorderLeftTop
        | LabelPosition::BorderLeftMiddle
        | LabelPosition::BorderBottomLeft => "left",
        LabelPosition::InsideTopRight
        | LabelPosition::InsideMiddleRight
        | LabelPosition::InsideBottomRight
        | LabelPosition::BorderTopRight
        | LabelPosition::BorderRightTop
        | LabelPosition::BorderRightMiddle
        | LabelPosition::BorderBottomRight => "right",
        _ => "center",
    };
    let language_attribute = language
        .map(|value| format!(" data-d2-language=\"{}\"", escape(value)))
        .unwrap_or_default();
    let font_weight = if style.bold { "font-weight:bold;" } else { "" };
    let font_style = if style.italic {
        "font-style:italic;"
    } else {
        ""
    };
    let text_decoration = if style.underline {
        "text-decoration:underline;"
    } else {
        ""
    };
    let font_family = if style.mono {
        "Source Code Pro,monospace"
    } else {
        "Source Sans Pro,sans-serif"
    };
    let class_attribute = class
        .map(|value| format!(" class=\"{}\"", escape(value)))
        .unwrap_or_default();
    let _ = writeln!(
        svg,
        "<foreignObject x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"><div xmlns=\"http://www.w3.org/1999/xhtml\"{} style=\"box-sizing:border-box;width:100%;height:100%;display:flex;align-items:center;justify-content:center;text-align:{};color:{};font-family:{};font-size:{}px;line-height:1.2;padding:5px;{}{}{}\"{}>{}</div></foreignObject>",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width.max(1.0)),
        number(rect.size.height.max(1.0)),
        class_attribute,
        text_align,
        escape(fill),
        font_family,
        font_size,
        font_weight,
        font_style,
        text_decoration,
        language_attribute,
        html
    );

    // Keep an invisible plain-text fallback for SVG consumers without
    // foreignObject support and for accessible text extraction.
    let mut fallback = String::new();
    draw_text(
        &mut fallback,
        &plain_text(text.as_bytes()),
        rect,
        font_size,
        position,
        fill,
        language,
        style,
        class,
    );
    if let Some(start) = fallback.find("<text ") {
        fallback.insert_str(start + "<text ".len(), "style=\"display:none\" ");
    }
    svg.push_str(&fallback);
}

#[allow(clippy::too_many_arguments)]
fn draw_label(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    underline: bool,
    bold: bool,
    italic: bool,
    mono: bool,
    native_geometry_id: Option<&str>,
) {
    // d2svg uses the rounded target label box for both text and masking.
    let x = rect.origin.x + rect.size.width / 2.0;
    let y = rect.origin.y + f64::from(font_size);
    let anchor = "middle";
    let content = if native_geometry_id.is_some() {
        String::new()
    } else {
        render_text_content(text, x, rect.size.height)
    };
    let class = format!(
        " class=\"{}\"",
        escape(&text_class_with_font(
            class,
            TextStyle {
                bold,
                italic,
                underline,
                mono,
            },
        ))
    );
    let text_decoration = if underline {
        " text-decoration=\"underline\""
    } else {
        ""
    };
    // Go connection labels choose the bold face before the italic face.
    let font_style = if italic && !bold {
        " font-style=\"italic\""
    } else {
        ""
    };
    let font_weight = if bold { " font-weight=\"bold\"" } else { "" };
    let font_family = if mono {
        "Source Code Pro, monospace"
    } else {
        "Source Sans Pro, sans-serif"
    };
    if let Some(native_geometry_id) = native_geometry_id {
        // Go centeredTextRuns retains one independent primitive per line.
        // Native raster coordinates must come from the scene's label box,
        // before SVG number formatting or usvg's f32 layout accumulates dy.
        let lines = text.split('\n').collect::<Vec<_>>();
        let line_advance = rect.size.height / lines.len() as f64;
        for (index, line) in lines.iter().enumerate() {
            let line_y = y + index as f64 * line_advance;
            let line = if line.is_empty() { " " } else { line };
            let id = format!("{native_geometry_id}-line-{index}");
            let _ = writeln!(
                svg,
                "<text xml:space=\"preserve\" data-d2-rs-text-raw=\"true\" id=\"{}\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\" x=\"{}\" y=\"{}\" text-anchor=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\"{}{}{}{}>{}</text>",
                escape(&id),
                x.to_bits(),
                line_y.to_bits(),
                x,
                line_y,
                anchor,
                font_family,
                font_size,
                escape(fill),
                class,
                text_decoration,
                font_weight,
                font_style,
                escape_xml_text(line),
            );
        }
        return;
    }
    let _ = writeln!(
        svg,
        "<text x=\"{}\" y=\"{}\" text-anchor=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\"{}{}{}{}>{}</text>",
        number(x),
        number(y),
        anchor,
        font_family,
        font_size,
        escape(fill),
        class,
        text_decoration,
        font_weight,
        font_style,
        content
    );
}

fn route_point_at(route: &[Point], percentage: f64) -> Point {
    let Some(first) = route.first().copied() else {
        return Point::default();
    };
    if route.len() == 1 {
        return first;
    }
    let total = route
        .windows(2)
        .map(|pair| {
            let dx = pair[1].x - pair[0].x;
            let dy = pair[1].y - pair[0].y;
            dx.hypot(dy)
        })
        .sum::<f64>();
    if total <= f64::EPSILON {
        return first;
    }
    let mut remaining = total * percentage.clamp(0.0, 1.0);
    for pair in route.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let length = dx.hypot(dy);
        if remaining <= length {
            let ratio = if length <= f64::EPSILON {
                0.0
            } else {
                remaining / length
            };
            return Point {
                x: dx.mul_add(ratio, start.x),
                y: dy.mul_add(ratio, start.y),
            };
        }
        remaining -= length;
    }
    route.last().copied().unwrap_or(first)
}

/// D2 target.GetIconSize truncates the minimum dimension before halving it.
fn icon_size(rect: Rect, position: LabelPosition) -> f64 {
    let minimum = rect.size.width.min(rect.size.height).trunc();
    let half = (minimum * 0.5).ceil();
    let mut size = if position == LabelPosition::InsideMiddleCenter {
        half
    } else {
        minimum.min(32.0_f64.max(half))
    }
    .min(64.0);
    if !position.is_outside() {
        size = size
            .min((rect.size.width.trunc() - 10.0).max(0.0))
            .min((rect.size.height.trunc() - 10.0).max(0.0));
    }
    size
}

fn icon_rect(rect: Rect, shape: ShapeKind, position: LabelPosition) -> Rect {
    let icon_box = if position.is_outside() {
        rect
    } else {
        node_inner_box(rect, shape, None)
    };
    let size = icon_size(icon_box, position);
    // icon_box is already the selected inner/outer shape box. Rectangular
    // placement avoids applying a shaped inset a second time.
    node_label_box(
        icon_box,
        ShapeKind::Rectangle,
        position,
        Size {
            width: size,
            height: size,
        },
        None,
    )
}

fn outside_icon_envelope(rect: Rect, _shape: ShapeKind, position: LabelPosition) -> Option<Rect> {
    if !position.is_outside() {
        return None;
    }
    // Original Diagram.BoundingBox passes lowercase target shape names to
    // shape.NewShape, whose cases use title-cased internal names. Its fallback
    // is a square with the complete outer box, including for oval/diamond.
    let extent = 5.0 + icon_size(rect, position);
    let mut envelope = rect;
    match position {
        LabelPosition::OutsideTopLeft
        | LabelPosition::OutsideTopCenter
        | LabelPosition::OutsideTopRight => {
            envelope.origin.y -= extent;
            envelope.size.height += extent;
        }
        LabelPosition::OutsideBottomLeft
        | LabelPosition::OutsideBottomCenter
        | LabelPosition::OutsideBottomRight => envelope.size.height += extent,
        LabelPosition::OutsideLeftTop
        | LabelPosition::OutsideLeftMiddle
        | LabelPosition::OutsideLeftBottom => {
            envelope.origin.x -= extent;
            envelope.size.width += extent;
        }
        LabelPosition::OutsideRightTop
        | LabelPosition::OutsideRightMiddle
        | LabelPosition::OutsideRightBottom => envelope.size.width += extent,
        _ => unreachable!("outside positions exhaustively handled"),
    }
    Some(envelope)
}

// Source d2scenebuild/assets.go uses true arcs for ordinary icons and
// smooth cubics (with first control at the line endpoint) for image shapes.
fn image_radius_clip(
    svg: &mut String,
    rect: Rect,
    radius: f64,
    image_shape: bool,
    key: &str,
) -> String {
    let radius = radius
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0)
        .max(0.0);
    if radius == 0.0 {
        return String::new();
    }
    let (left, top, right, bottom) = (rect.origin.x, rect.origin.y, rect.right(), rect.bottom());
    let n = number;
    let path = if image_shape {
        format!(
            "M {} {} L {} {} C {} {} {} {} {} {} L {} {} L {} {} C {} {} {} {} {} {} L {} {} C {} {} {} {} {} {} L {} {} C {} {} {} {} {} {} L {} {} Z",
            n(left),
            n(top + radius),
            n(left),
            n(top + radius),
            n(left),
            n(top + radius),
            n(left),
            n(top),
            n(left + radius),
            n(top),
            n(right - radius),
            n(top),
            n(right - radius),
            n(top),
            n(right - radius),
            n(top),
            n(right),
            n(top),
            n(right),
            n(top + radius),
            n(right),
            n(bottom - radius),
            n(right),
            n(bottom - radius),
            n(right),
            n(bottom),
            n(right - radius),
            n(bottom),
            n(left + radius),
            n(bottom),
            n(left + radius),
            n(bottom),
            n(left),
            n(bottom),
            n(left),
            n(bottom - radius),
            n(left),
            n(top + radius)
        )
    } else {
        format!(
            "M {} {} L {} {} A {} {} 0 0 1 {} {} L {} {} A {} {} 0 0 1 {} {} L {} {} A {} {} 0 0 1 {} {} L {} {} A {} {} 0 0 1 {} {} Z",
            n(left + radius),
            n(top),
            n(right - radius),
            n(top),
            n(radius),
            n(radius),
            n(right),
            n(top + radius),
            n(right),
            n(bottom - radius),
            n(radius),
            n(radius),
            n(right - radius),
            n(bottom),
            n(left + radius),
            n(bottom),
            n(radius),
            n(radius),
            n(left),
            n(bottom - radius),
            n(left),
            n(top + radius),
            n(radius),
            n(radius),
            n(left + radius),
            n(top)
        )
    };
    let id = format!("d2-rs-image-clip-{}", svg_object_class(key));
    let _ = writeln!(
        svg,
        "<defs><clipPath id=\"{}\" data-d2-rs-image-radius=\"scene\"><path d=\"{}\"/></clipPath></defs>",
        escape(&id),
        path
    );
    format!(" clip-path=\"url(#{})\"", escape(&id))
}

fn draw_icon(
    svg: &mut String,
    rect: Rect,
    shape: ShapeKind,
    icon: &str,
    position: LabelPosition,
    radius: f64,
    key: &str,
) {
    let icon_box = icon_rect(rect, shape, position);
    // Dedicated structured painters intentionally ignore icon radius.
    let radius = if matches!(shape, ShapeKind::Class | ShapeKind::SqlTable) {
        0.0
    } else {
        radius
    };
    let clip = image_radius_clip(svg, icon_box, radius, false, key);
    let _ = writeln!(
        svg,
        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\" xlink:href=\"{}\" preserveAspectRatio=\"xMidYMid meet\"{}/>",
        number(icon_box.origin.x),
        number(icon_box.origin.y),
        number(icon_box.size.width),
        number(icon_box.size.height),
        escape(icon),
        escape(icon),
        clip,
    );
}

fn draw_link_tooltip_badges(
    svg: &mut String,
    rect: Rect,
    shape: ShapeKind,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    family: MarkdownFamily,
    raster_text_geometry_precision: bool,
) {
    let has_tooltip = style
        .tooltip
        .as_deref()
        .is_some_and(|value| !value.is_empty());
    let has_link = style.link.as_deref().is_some_and(|value| !value.is_empty());
    if !has_tooltip && !has_link {
        return;
    }
    // d2svg.addAppendixItems traces the selected corner onto the shape,
    // then offsets the second icon according to its shape family.
    let mut corner = Point {
        x: rect.right(),
        y: rect.origin.y,
    };
    let mut center = rect.center();
    let left_on_shape = has_tooltip
        && has_link
        && matches!(
            shape,
            ShapeKind::Circle
                | ShapeKind::Oval
                | ShapeKind::Diamond
                | ShapeKind::Person
                | ShapeKind::Cloud
                | ShapeKind::Cylinder
        );
    match shape {
        ShapeKind::Step | ShapeKind::Hexagon | ShapeKind::Queue | ShapeKind::Page => {
            center.y = rect.origin.y
        }
        ShapeKind::Package => center.x = rect.right(),
        _ => {}
    }
    if left_on_shape {
        corner.x -= 32.0;
    }
    let previous = Point {
        x: corner.x + (corner.x - center.x),
        y: corner.y + (corner.y - center.y),
    };
    let mut tooltip_point =
        d2_rs_core::shape_geometry::trace_shape_border(shape, rect, corner, previous);
    let mut link_point = tooltip_point;
    if has_tooltip && has_link {
        if left_on_shape {
            tooltip_point.x += 32.0;
        } else {
            link_point.x -= 32.0;
        }
    }
    let draw_badge = |svg: &mut String, point: Point, tooltip: bool, title: Option<&str>| {
        let _ = write!(
            svg,
            "<g transform=\"translate({} {})\" class=\"appendix-icon\" style=\"filter:drop-shadow(0px 0px 32px rgba(31,36,58,0.1));\">",
            number(point.x.ceil() - 16.0),
            number(point.y.ceil() - 16.0),
        );
        if let Some(title) = title {
            let _ = write!(
                svg,
                "<title>{}</title>",
                escape(&plain_text(title.as_bytes()))
            );
        }
        svg.push_str("<svg width=\"32\" height=\"32\" viewBox=\"0 0 32 32\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\">");
        svg.push_str("<path d=\"M16 31.1109C24.3456 31.1109 31.1111 24.3454 31.1111 15.9998C31.1111 7.65415 24.3456 0.888672 16 0.888672C7.65436 0.888885 0.888885 7.65415 0.888885 15.9998C0.888885 24.3454 7.65436 31.1109 16 31.1109Z\" fill=\"white\" stroke=\"#DEE1EB\"/>");
        if tooltip {
            svg.push_str("<path d=\"M16 26C21.5228 26 26 21.5228 26 16C26 10.4772 21.5228 6 16 6C10.4772 6 6 10.4772 6 16C6 21.5228 10.4772 26 16 26Z\" stroke=\"#2E3346\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/><path d=\"M16 19.998V15.998\" stroke=\"#2E3346\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/><path d=\"M16 12H16.0098\" stroke=\"#2E3346\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>");
        } else {
            svg.push_str("<path d=\"M14.3909 16.7965C14.7364 17.2584 15.1772 17.6406 15.6834 17.9171C16.1896 18.1938 16.7494 18.3582 17.3248 18.3993C17.9001 18.4405 18.4777 18.3575 19.0181 18.1559C19.5586 17.9543 20.0492 17.6389 20.4571 17.2309L22.8708 14.8173C23.6036 14.0586 24.0089 13.0425 23.9998 11.9877C23.9906 10.933 23.5676 9.92404 22.8217 9.17821C22.0759 8.43237 21.067 8.00931 20.0123 8.00015C18.9575 7.99098 17.9413 8.39644 17.1827 9.1292L15.7988 10.505\" stroke=\"#2E3346\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/><path d=\"M17.609 15.1874C17.2635 14.7255 16.8227 14.3433 16.3165 14.0667C15.8103 13.7902 15.2505 13.6257 14.6752 13.5845C14.0998 13.5433 13.5223 13.6263 12.9819 13.8279C12.4414 14.0295 11.9506 14.345 11.5428 14.753L9.1292 17.1666C8.39644 17.9252 7.99098 18.9414 8.00015 19.9962C8.00931 21.0509 8.43237 22.0598 9.17821 22.8056C9.92405 23.5515 10.933 23.9745 11.9877 23.9837C13.0425 23.9928 14.0586 23.5875 14.8173 22.8547L16.193 21.4788\" stroke=\"#2E3346\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>");
        }
        svg.push_str("</svg></g>\n");
    };
    if has_tooltip {
        if style
            .tooltip_position
            .as_deref()
            .is_some_and(|position| !position.is_empty())
        {
            draw_positioned_tooltip(
                svg,
                rect,
                style,
                palette,
                family,
                raster_text_geometry_precision,
            );
        } else {
            draw_badge(svg, tooltip_point, true, style.tooltip.as_deref());
        }
    }
    if has_link {
        draw_badge(svg, link_point, false, None);
    }
}

fn sketch_shape_supported(shape: ShapeKind) -> bool {
    matches!(
        shape,
        ShapeKind::Rectangle
            | ShapeKind::Square
            | ShapeKind::Page
            | ShapeKind::Package
            | ShapeKind::Step
            | ShapeKind::Callout
            | ShapeKind::StoredData
            | ShapeKind::Queue
            | ShapeKind::Cylinder
            | ShapeKind::Document
            | ShapeKind::Class
            | ShapeKind::SqlTable
            | ShapeKind::Oval
            | ShapeKind::Circle
    )
}

fn draw_sketch_overlay(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
) {
    let sketch_options = d2_rs_sketch::SketchOptions {
        seed: 1,
        roughness: 1.0,
        bowing: 2.0,
        ..Default::default()
    };
    let path = if matches!(shape, ShapeKind::Oval | ShapeKind::Circle) {
        d2_rs_sketch::rough_ellipse_path(
            Point {
                x: rect.origin.x + rect.size.width / 2.0,
                y: rect.origin.y + rect.size.height / 2.0,
            },
            rect.size.width,
            rect.size.height,
            sketch_options,
        )
    } else {
        d2_rs_sketch::rough_rect_path(rect, sketch_options)
    };
    let stroke = style
        .stroke
        .as_deref()
        .map(|raw| themed_color(Some(raw), palette.base.b1, palette))
        .unwrap_or_else(|| palette.color("B1", palette.base.b1));
    let stroke_width = style.stroke_width.as_deref().unwrap_or("2");
    let opacity = style
        .opacity
        .as_deref()
        .map(|value| format!(" opacity=\"{}\"", escape(value)))
        .unwrap_or_default();
    let _ = writeln!(
        svg,
        "<path class=\"d2-rs-sketch\" d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{} />",
        escape(&path),
        escape(&stroke),
        escape(stroke_width),
        opacity,
    );
}

fn marker_dimensions(name: &str, stroke_width: f64) -> (f64, f64) {
    let width = stroke_width.max(2.0);
    match name {
        "arrow" => (4.0 + width * 4.0, 4.0 + width * 4.0),
        "triangle" => (4.0 + width * 3.0, 4.0 + width * 4.0),
        "unfilled-triangle" => (7.0 + width * 3.0, 7.0 + width * 4.0),
        "diamond" => (11.0 + width * 5.5, 9.0 + width * 4.5),
        "filled-diamond" => (11.0 + width * 5.5, 7.0 + width * 3.5),
        "circle" | "filled-circle" => (8.0 + width * 5.0, 8.0 + width * 5.0),
        "box" | "filled-box" => (6.0 + width * 5.0, 6.0 + width * 5.0),
        "cross" => (7.0 + width * 5.0, 7.0 + width * 5.0),
        "line" => (width * 5.0, width * 8.0),
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => {
            (9.0 + width * 4.5, 9.0 + width * 4.5)
        }
        _ => (4.0 + width * 3.0, 4.0 + width * 4.0),
    }
}

/// Returns the v0.9.0 marker-cache hash.
///
/// `d2renderers/d2svg.hash` uses Go's `fnv.New32a` over the comma-separated
/// arrowhead tuple followed by the literal `lalalas` secret.  Keeping the
/// exact input spelling matters: the value is part of every generated marker
/// ID and therefore visible to SVG consumers and snapshot tests.
fn marker_hash(name: &str, is_target: bool, stroke_width: f64, stroke: &str) -> u32 {
    let key = format!(
        "{name},{},{},{}lalalas",
        if is_target { "true" } else { "false" },
        stroke_width.round() as i64,
        stroke
    );
    let mut hash = 2_166_136_261u32;
    for byte in key.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
    }
    hash
}

#[derive(Clone, Copy)]
struct MarkerTheme<'a> {
    palette: PaletteView<'a>,
    inline: bool,
}

impl MarkerTheme<'_> {
    /// Match ThemableElement.Render's raw color classes and paint order.
    /// Symbolic colors remain in CSS when a dark theme is configured; literal
    /// colors retain their presentation attributes in both theme modes.
    fn paint_attributes(self, base_class: &str, stroke: Option<&str>, fill: &str) -> String {
        let mut attributes = String::new();
        let mut class = base_class.to_owned();
        for (property, raw) in [("stroke", stroke.unwrap_or("")), ("fill", fill)] {
            if THEME_CODES.contains(&raw) {
                let _ = write!(class, " {property}-{raw}");
                if self.inline {
                    let color = escape(&self.palette.color(raw, raw));
                    let _ = write!(attributes, " {property}=\"{color}\"");
                }
            } else if !raw.is_empty() {
                let _ = write!(attributes, " {property}=\"{}\"", escape(raw));
            }
        }
        if !class.is_empty() {
            let _ = write!(attributes, " class=\"{}\"", escape(&class));
        }
        attributes
    }
}

/// Builds one marker using D2's v0.9.0 dimensions and source/target geometry.
fn dynamic_marker_definition(
    id: &str,
    name: &str,
    is_target: bool,
    stroke_width: f64,
    stroke: &str,
    marker_theme: MarkerTheme<'_>,
) -> String {
    let (base_width, height) = marker_dimensions(name, stroke_width);
    let raw_width = stroke_width.max(0.0);
    let mut width = base_width;
    let ref_x = match name {
        "diamond" => {
            // v0.9.0 computes refX from the unscaled diamond body, then
            // expands the marker viewport by 10% for the stroke envelope.
            // Keeping those operations ordered matters for custom stroke
            // widths (the target refX is 30.6 for a 4px diamond).
            let ref_x = if is_target {
                base_width - 0.6 * raw_width
            } else {
                base_width / 8.0 + 0.6 * raw_width
            };
            width *= 1.1;
            ref_x
        }
        _ if is_target => width - 1.5 * raw_width,
        _ => 1.5 * raw_width,
    };
    let ref_y = height / 2.0;
    let filled_paint = marker_theme.paint_attributes("connection", None, stroke);
    let hollow_paint = marker_theme.paint_attributes("connection", Some(stroke), "N7");
    // Go's standalone hollow circle deliberately has no connection class.
    let circle_paint = marker_theme.paint_attributes("", Some(stroke), "N7");
    let line_paint = marker_theme.paint_attributes("connection", Some(stroke), "none");
    let stroke_attr = number(raw_width);
    let body = match name {
        "arrow" => {
            let points = if is_target {
                format!(
                    "0,0 {},{} 0,{} {},{}",
                    number(base_width),
                    number(ref_y),
                    number(height),
                    number(base_width / 4.0),
                    number(ref_y)
                )
            } else {
                format!(
                    "0,{} {},0 {},{} {},{}",
                    number(ref_y),
                    number(base_width),
                    number(base_width * 3.0 / 4.0),
                    number(ref_y),
                    number(base_width),
                    number(height)
                )
            };
            format!("<polygon points=\"{points}\"{filled_paint} stroke-width=\"{stroke_attr}\"/>")
        }
        "triangle" => {
            let points = if is_target {
                format!(
                    "0,0 {},{} 0,{}",
                    number(base_width),
                    number(ref_y),
                    number(height)
                )
            } else {
                format!(
                    "{},0 0,{} {},{}",
                    number(base_width),
                    number(ref_y),
                    number(base_width),
                    number(height)
                )
            };
            format!("<polygon points=\"{points}\"{filled_paint} stroke-width=\"{stroke_attr}\"/>")
        }
        "unfilled-triangle" => {
            let inset = raw_width / 2.0;
            let points = if is_target {
                format!(
                    "{},{} {},{} {},{}",
                    number(inset),
                    number(inset),
                    number(base_width - inset),
                    number(ref_y),
                    number(inset),
                    number(height - inset)
                )
            } else {
                format!(
                    "{},{} {},{} {},{}",
                    number(base_width - inset),
                    number(inset),
                    number(inset),
                    number(ref_y),
                    number(base_width - inset),
                    number(height - inset)
                )
            };
            format!("<polygon points=\"{points}\"{hollow_paint} stroke-width=\"{stroke_attr}\"/>")
        }
        "diamond" => {
            let points = if is_target {
                format!(
                    "0,{} {},{} {},{} {},{}",
                    number(height / 2.0),
                    number(base_width / 2.0),
                    number(height / 8.0),
                    number(base_width),
                    number(height / 2.0),
                    number(base_width / 2.0),
                    number(height * 0.9)
                )
            } else {
                format!(
                    "{},{} {},{} {},{} {},{}",
                    number(base_width / 8.0),
                    number(height / 2.0),
                    number(base_width * 0.6),
                    number(height / 8.0),
                    number(base_width * 1.1),
                    number(height / 2.0),
                    number(base_width * 0.6),
                    number(height * 7.0 / 8.0)
                )
            };
            format!("<polygon points=\"{points}\"{hollow_paint} stroke-width=\"{stroke_attr}\"/>")
        }
        "filled-diamond" => format!(
            "<polygon points=\"0,{} {},0 {},{} {},{}\"{filled_paint} stroke-width=\"{stroke_attr}\"/>",
            number(height / 2.0),
            number(base_width / 2.0),
            number(base_width),
            number(height / 2.0),
            number(base_width / 2.0),
            number(height)
        ),
        "circle" => {
            let radius = base_width / 2.0;
            let cx = if is_target {
                radius + raw_width / 2.0
            } else {
                radius - raw_width / 2.0
            };
            format!(
                "<circle r=\"{}\" cx=\"{}\" cy=\"{}\"{circle_paint} stroke-width=\"{stroke_attr}\"/>",
                number(radius - raw_width),
                number(cx),
                number(radius)
            )
        }
        "filled-circle" => {
            let radius = base_width / 2.0;
            let cx = if is_target {
                radius + raw_width / 2.0
            } else {
                radius - raw_width / 2.0
            };
            format!(
                "<circle r=\"{}\" cx=\"{}\" cy=\"{}\"{filled_paint} stroke-width=\"{stroke_attr}\"/>",
                number(radius - raw_width / 2.0),
                number(cx),
                number(radius)
            )
        }
        "box" => format!(
            "<polygon points=\"{},{} {},{} {},{} {},{}\"{hollow_paint} style=\"stroke-linejoin:miter;\" stroke-width=\"{stroke_attr}\"/>",
            number(raw_width / 2.0),
            number(raw_width / 2.0),
            number(raw_width / 2.0),
            number(height - raw_width / 2.0),
            number(width - raw_width / 2.0),
            number(height - raw_width / 2.0),
            number(width - raw_width / 2.0),
            number(raw_width / 2.0)
        ),
        "filled-box" => format!(
            "<polygon points=\"0,0 0,{} {},{} {},0\"{filled_paint} stroke-width=\"{stroke_attr}\"/>",
            number(height),
            number(width),
            number(height),
            number(width)
        ),
        "line" => {
            let points = if is_target {
                format!(
                    "{},{} {},{} {},{}",
                    number(raw_width / 2.0),
                    number(raw_width / 2.0),
                    number(width - raw_width / 2.0),
                    number(ref_y),
                    number(raw_width / 2.0),
                    number(height - raw_width / 2.0)
                )
            } else {
                format!(
                    "{},{} {},{} {},{}",
                    number(width - raw_width / 2.0),
                    number(raw_width / 2.0),
                    number(raw_width / 2.0),
                    number(ref_y),
                    number(width - raw_width / 2.0),
                    number(height - raw_width / 2.0)
                )
            };
            format!("<polyline points=\"{points}\"{line_paint} stroke-width=\"{stroke_attr}\"/>")
        }
        "cross" => {
            let inset = raw_width / 8.0;
            let points = [
                format!("0,{}", number(height / 2.0 + inset)),
                format!(
                    "{},{}",
                    number(width / 2.0 - inset),
                    number(height / 2.0 + inset)
                ),
                format!("{},{}", number(width / 2.0 - inset), number(height)),
                // Retain v0.9.0's comma separator after this coordinate pair.
                format!("{},{},", number(width / 2.0 + inset), number(height)),
                format!(
                    "{},{}",
                    number(width / 2.0 + inset),
                    number(height / 2.0 + inset)
                ),
                format!("{},{}", number(width), number(height / 2.0 + inset)),
                format!("{},{}", number(width), number(height / 2.0 - inset)),
                format!(
                    "{},{}",
                    number(width / 2.0 + inset),
                    number(height / 2.0 - inset)
                ),
                format!("{},{}", number(width / 2.0 + inset), number(0.0)),
                format!("{},{}", number(width / 2.0 - inset), number(0.0)),
                format!(
                    "{},{}",
                    number(width / 2.0 - inset),
                    number(height / 2.0 - inset)
                ),
                format!("0,{}", number(height / 2.0 - inset)),
            ]
            .join(" ");
            let path = if is_target {
                format!(
                    "M{},{} {},{}",
                    number(width / 2.0),
                    number(height / 2.0),
                    number(width),
                    number(height / 2.0)
                )
            } else {
                format!(
                    "M{},{} 0,{}",
                    number(width / 2.0),
                    number(height / 2.0),
                    number(height / 2.0)
                )
            };
            let cos_sin = std::f64::consts::FRAC_1_SQRT_2;
            let tx = width / 2.0 - (width - height) * cos_sin / 2.0;
            let ty = height / 2.0 - (width + height) * cos_sin / 2.0;
            format!(
                "<g{hollow_paint} stroke-width=\"{stroke_attr}\"><polygon points=\"{points}\" transform=\"translate({}, {}) rotate(45)\"/><path d=\"{path}\" /></g>",
                number(tx),
                number(ty)
            )
        }
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => {
            // Match d2svg.arrowheadMarker's crow's-foot modifier and spine.
            // The source endpoint mirrors the complete marker group, including
            // its optional circle/required bar, before SVG auto-orientation.
            let offset = 3.0 + raw_width * 1.8;
            let required = matches!(name, "cf-one-required" | "cf-many-required");
            let many = matches!(name, "cf-many" | "cf-many-required");
            let modifier = if required {
                format!(
                    "<path d=\"M{},0 {},{}\"{hollow_paint} stroke-width=\"{stroke_attr}\"/>",
                    number(offset),
                    number(offset),
                    number(height)
                )
            } else {
                format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"{hollow_paint} stroke-width=\"{stroke_attr}\"/>",
                    number(offset / 2.0 + 2.0),
                    number(height / 2.0),
                    number(offset / 2.0)
                )
            };
            let branches = if many {
                format!(
                    "M{},{} {},{} M{},{} {},0 M{},{} {},{}",
                    number(width - 3.0),
                    number(height / 2.0),
                    number(width + offset),
                    number(height / 2.0),
                    number(offset + 3.0),
                    number(height / 2.0),
                    number(width + offset),
                    number(offset + 3.0),
                    number(height / 2.0),
                    number(width + offset),
                    number(height)
                )
            } else {
                format!(
                    "M{},{} {},{} M{},0 {},{}",
                    number(width - 3.0),
                    number(height / 2.0),
                    number(width + offset),
                    number(height / 2.0),
                    number(offset * 2.0),
                    number(offset * 2.0),
                    number(height)
                )
            };
            let transform = if is_target {
                String::new()
            } else {
                format!(
                    " transform=\"scale(-1) translate(-{}, -{})\"",
                    number(width),
                    number(height)
                )
            };
            format!(
                "<g{transform}{hollow_paint} stroke-width=\"{stroke_attr}\">{modifier}<path d=\"{branches}\"/></g>"
            )
        }
        _ => String::new(),
    };
    format!(
        "<marker id=\"{id}\" markerWidth=\"{}\" markerHeight=\"{}\" refX=\"{}\" refY=\"{}\" viewBox=\"0 0 {} {}\" orient=\"auto\" markerUnits=\"userSpaceOnUse\">{body}</marker>",
        number(width),
        number(height),
        number(ref_x),
        number(ref_y),
        number(width),
        number(height)
    )
}

/// Builds D2's native-scene triangle arrowhead for the PNG scanline path.
///
/// The ordinary SVG renderer leaves marker placement to SVG's `orient="auto"`
/// implementation. D2 v0.9.0's native raster scene instead composes a Go
/// `d2scene.Matrix` from the route tangent and endpoint. Keeping that matrix's
/// operation order here avoids the extra f32 rounding introduced when usvg
/// expands an SVG marker.
fn raster_triangle_arrow(
    id: String,
    route: &[Point],
    tangent_route: &[Point],
    target: bool,
    endpoint_adjustment: Point,
    dx: f64,
    dy: f64,
    stroke_width: f64,
    stroke: &str,
) -> Option<RasterInlineArrow> {
    if route.len() < 2 || tangent_route.len() < 2 {
        return None;
    }
    let (width, height) = marker_dimensions("triangle", stroke_width);
    let ref_x = if target {
        width - 1.5 * stroke_width
    } else {
        1.5 * stroke_width
    };
    let endpoint_index = if target { route.len() - 1 } else { 0 };
    let tangent_start = if target { route.len() - 2 } else { 0 };
    let tangent_end = if target { route.len() - 1 } else { 1 };
    let tangent_x = tangent_route[tangent_end].x - tangent_route[tangent_start].x;
    let tangent_y = tangent_route[tangent_end].y - tangent_route[tangent_start].y;
    let endpoint = Point {
        x: route[endpoint_index].x + endpoint_adjustment.x + dx,
        y: route[endpoint_index].y + endpoint_adjustment.y + dy,
    };
    let angle = tangent_y.atan2(tangent_x);
    let (sin, cos) = go_sincos_route_angle(angle);

    // Match d2scene.Translate(endpoint).Mul(Rotate(angle)).Mul(Translate(-ref)).
    let local_translate_x = -ref_x;
    let local_translate_y = -height / 2.0;
    let matrix_x = cos * local_translate_x + -sin * local_translate_y + endpoint.x;
    let matrix_y = sin * local_translate_x + cos * local_translate_y + endpoint.y;
    let source_points = if target {
        [
            Point { x: 0.0, y: 0.0 },
            Point {
                x: width,
                y: height / 2.0,
            },
            Point { x: 0.0, y: height },
        ]
    } else {
        [
            Point { x: width, y: 0.0 },
            Point {
                x: 0.0,
                y: height / 2.0,
            },
            Point {
                x: width,
                y: height,
            },
        ]
    };
    let mapped = source_points.map(|point| Point {
        x: cos * point.x + -sin * point.y + matrix_x,
        y: sin * point.x + cos * point.y + matrix_y,
    });
    let path_data = format!(
        "M {} {} L {} {} L {} {} Z",
        number(mapped[0].x),
        number(mapped[0].y),
        number(mapped[1].x),
        number(mapped[1].y),
        number(mapped[2].x),
        number(mapped[2].y),
    );
    let commands = vec![
        RasterPathCommand::MoveTo(mapped[0]),
        RasterPathCommand::LineTo(mapped[1]),
        RasterPathCommand::LineTo(mapped[2]),
        RasterPathCommand::Close,
    ];
    Some(RasterInlineArrow {
        paths: vec![RasterInlineArrowPath {
            id,
            path_data,
            commands,
            source_transform: None,
            fill: stroke.to_owned(),
            stroke: None,
            stroke_width: None,
            stroke_style: None,
        }],
    })
}

#[allow(clippy::too_many_arguments)]
fn raster_crowfoot_arrow(
    id: String,
    shape: &str,
    route: &[Point],
    tangent_route: &[Point],
    target: bool,
    endpoint_adjustment: Point,
    dx: f64,
    dy: f64,
    stroke_width: f64,
    stroke: &str,
) -> Option<RasterInlineArrow> {
    if route.len() < 2 || tangent_route.len() < 2 {
        return None;
    }
    let (width, height) = marker_dimensions(shape, stroke_width);
    let ref_x = if target {
        width - 1.5 * stroke_width
    } else {
        1.5 * stroke_width
    };
    let endpoint_index = if target { route.len() - 1 } else { 0 };
    let tangent_start = if target { route.len() - 2 } else { 0 };
    let tangent_end = if target { route.len() - 1 } else { 1 };
    let tangent_x = tangent_route[tangent_end].x - tangent_route[tangent_start].x;
    let tangent_y = tangent_route[tangent_end].y - tangent_route[tangent_start].y;
    let endpoint = Point {
        x: route[endpoint_index].x + endpoint_adjustment.x,
        y: route[endpoint_index].y + endpoint_adjustment.y,
    };
    let (sin, cos) = go_sincos_route_angle(tangent_y.atan2(tangent_x));
    let parent_x = cos * -ref_x + -sin * (-height / 2.0) + endpoint.x;
    let parent_y = sin * -ref_x + cos * (-height / 2.0) + endpoint.y;
    // Compose the same Translate(endpoint).Mul(Rotate).Mul(Translate(-ref))
    // matrix as d2scene, then include the source-only marker reflection.
    let source_transform = if target {
        [cos, -sin, sin, cos, parent_x + dx, parent_y + dy]
    } else {
        [
            -cos,
            sin,
            -sin,
            -cos,
            cos * width + -sin * height + parent_x + dx,
            sin * width + cos * height + parent_y + dy,
        ]
    };
    let map_point = |point: Point| Point {
        x: point.x * source_transform[0] + point.y * source_transform[1] + source_transform[4],
        y: point.x * source_transform[2] + point.y * source_transform[3] + source_transform[5],
    };
    let map_command = |command: RasterPathCommand| match command {
        RasterPathCommand::MoveTo(point) => RasterPathCommand::MoveTo(map_point(point)),
        RasterPathCommand::LineTo(point) => RasterPathCommand::LineTo(map_point(point)),
        RasterPathCommand::CubicTo(a, b, point) => {
            RasterPathCommand::CubicTo(map_point(a), map_point(b), map_point(point))
        }
        RasterPathCommand::Close => RasterPathCommand::Close,
    };
    let offset = 3.0 + stroke_width * 1.8;
    let required = matches!(shape, "cf-one-required" | "cf-many-required");
    let many = matches!(shape, "cf-many" | "cf-many-required");
    let modifier_commands = if required {
        vec![
            RasterPathCommand::MoveTo(Point { x: offset, y: 0.0 }),
            RasterPathCommand::LineTo(Point {
                x: offset,
                y: height,
            }),
        ]
    } else {
        let radius = offset / 2.0;
        let center = Point {
            x: offset / 2.0 + 2.0,
            y: height / 2.0,
        };
        let k = 0.552_284_749_830_793_6;
        let start = Point {
            x: center.x + radius,
            y: center.y,
        };
        vec![
            RasterPathCommand::MoveTo(start),
            RasterPathCommand::CubicTo(
                Point {
                    x: center.x + radius,
                    y: center.y + k * radius,
                },
                Point {
                    x: center.x + k * radius,
                    y: center.y + radius,
                },
                Point {
                    x: center.x,
                    y: center.y + radius,
                },
            ),
            RasterPathCommand::CubicTo(
                Point {
                    x: center.x - k * radius,
                    y: center.y + radius,
                },
                Point {
                    x: center.x - radius,
                    y: center.y + k * radius,
                },
                Point {
                    x: center.x - radius,
                    y: center.y,
                },
            ),
            RasterPathCommand::CubicTo(
                Point {
                    x: center.x - radius,
                    y: center.y - k * radius,
                },
                Point {
                    x: center.x - k * radius,
                    y: center.y - radius,
                },
                Point {
                    x: center.x,
                    y: center.y - radius,
                },
            ),
            RasterPathCommand::CubicTo(
                Point {
                    x: center.x + k * radius,
                    y: center.y - radius,
                },
                Point {
                    x: center.x + radius,
                    y: center.y - k * radius,
                },
                start,
            ),
            RasterPathCommand::Close,
        ]
    };
    let mut branch_commands = Vec::with_capacity(6);
    branch_commands.extend([
        RasterPathCommand::MoveTo(Point {
            x: width - 3.0,
            y: height / 2.0,
        }),
        RasterPathCommand::LineTo(Point {
            x: width + offset,
            y: height / 2.0,
        }),
    ]);
    if many {
        branch_commands.extend([
            RasterPathCommand::MoveTo(Point {
                x: offset + 3.0,
                y: height / 2.0,
            }),
            RasterPathCommand::LineTo(Point {
                x: width + offset,
                y: 0.0,
            }),
            RasterPathCommand::MoveTo(Point {
                x: offset + 3.0,
                y: height / 2.0,
            }),
            RasterPathCommand::LineTo(Point {
                x: width + offset,
                y: height,
            }),
        ]);
    } else {
        branch_commands.extend([
            RasterPathCommand::MoveTo(Point {
                x: offset * 2.0,
                y: 0.0,
            }),
            RasterPathCommand::LineTo(Point {
                x: offset * 2.0,
                y: height,
            }),
        ]);
    }
    let make_path = |suffix: &str, commands: Vec<RasterPathCommand>| {
        let path_data_commands = commands
            .iter()
            .copied()
            .map(map_command)
            .collect::<Vec<_>>();
        RasterInlineArrowPath {
            id: format!("{id}-{suffix}"),
            path_data: raster_path_data(&path_data_commands),
            commands,
            source_transform: Some(source_transform),
            fill: "#FFFFFF".to_owned(),
            stroke: Some(stroke.to_owned()),
            stroke_width: Some(stroke_width),
            stroke_style: Some("stroke-linecap:butt;stroke-linejoin:miter"),
        }
    };
    Some(RasterInlineArrow {
        paths: vec![
            make_path("modifier", modifier_commands),
            make_path("marks", branch_commands),
        ],
    })
}

/// Original d2scene.Matrix.Mul, in [A,C,B,D,E,F] order. Pinned
/// Go1.27 ARM64 rounds the second product then fuses the first product/add.
/// Other targets retain separate arithmetic until independently qualified.
fn raster_scene_matrix_mul(left: [f64; 6], right: [f64; 6]) -> [f64; 6] {
    #[cfg(target_arch = "aarch64")]
    let product_sum = |a: f64, b: f64, c: f64, d: f64| a.mul_add(b, c * d);
    #[cfg(not(target_arch = "aarch64"))]
    let product_sum = |a: f64, b: f64, c: f64, d: f64| a * b + c * d;
    [
        product_sum(left[0], right[0], left[1], right[2]),
        product_sum(left[0], right[1], left[1], right[3]),
        product_sum(left[2], right[0], left[3], right[2]),
        product_sum(left[2], right[1], left[3], right[3]),
        product_sum(left[0], right[4], left[1], right[5]) + left[4],
        product_sum(left[2], right[4], left[3], right[5]) + left[5],
    ]
}

/// Source connection.go:544–557 and d2raster/geometry.go:242: retain
/// ellipse control points and the complete native endpoint matrix before
/// curve flattening. SVG marker expansion introduces an extra f32 boundary.
#[allow(clippy::too_many_arguments)]
fn raster_circle_arrow(
    id: String,
    name: &str,
    route: &[Point],
    tangent_route: &[Point],
    target: bool,
    endpoint_adjustment: Point,
    dx: f64,
    dy: f64,
    stroke_width: f64,
    stroke: &str,
    background: &str,
) -> Option<RasterInlineArrow> {
    if route.len() < 2 || tangent_route.len() < 2 {
        return None;
    }
    let (width, height) = marker_dimensions(name, stroke_width);
    let ref_x = if target {
        width - 1.5 * stroke_width
    } else {
        1.5 * stroke_width
    };
    let end = if target { route.len() - 1 } else { 0 };
    let start_tangent = if target { tangent_route.len() - 2 } else { 0 };
    let tangent_x = tangent_route[start_tangent + 1].x - tangent_route[start_tangent].x;
    let tangent_y = tangent_route[start_tangent + 1].y - tangent_route[start_tangent].y;
    let (sin, cos) = go_sincos_route_angle(tangent_y.atan2(tangent_x));
    let endpoint = Point {
        x: route[end].x + endpoint_adjustment.x,
        y: route[end].y + endpoint_adjustment.y,
    };
    let transform = raster_scene_matrix_mul(
        raster_scene_matrix_mul(
            [1.0, 0.0, 0.0, 1.0, endpoint.x, endpoint.y],
            [cos, -sin, sin, cos, 0.0, 0.0],
        ),
        [1.0, 0.0, 0.0, 1.0, -ref_x, -height / 2.0],
    );
    let transform = [
        transform[0],
        transform[1],
        transform[2],
        transform[3],
        transform[4] + dx,
        transform[5] + dy,
    ];
    let radius = width / 2.0;
    let cx = if target {
        radius + stroke_width / 2.0
    } else {
        radius - stroke_width / 2.0
    };
    let cy = radius;
    let hollow = name == "circle";
    let r = if hollow {
        radius - stroke_width
    } else {
        radius - stroke_width / 2.0
    };
    let k = 0.5522847498307936;
    let point = |x, y| Point { x, y };
    let commands = vec![
        RasterPathCommand::MoveTo(point(cx + r, cy)),
        RasterPathCommand::CubicTo(
            point(cx + r, cy + k * r),
            point(cx + k * r, cy + r),
            point(cx, cy + r),
        ),
        RasterPathCommand::CubicTo(
            point(cx - k * r, cy + r),
            point(cx - r, cy + k * r),
            point(cx - r, cy),
        ),
        RasterPathCommand::CubicTo(
            point(cx - r, cy - k * r),
            point(cx - k * r, cy - r),
            point(cx, cy - r),
        ),
        RasterPathCommand::CubicTo(
            point(cx + k * r, cy - r),
            point(cx + r, cy - k * r),
            point(cx + r, cy),
        ),
        RasterPathCommand::Close,
    ];
    let map = |p: Point| {
        point(
            transform[0] * p.x + transform[1] * p.y + transform[4],
            transform[2] * p.x + transform[3] * p.y + transform[5],
        )
    };
    let mapped = commands
        .iter()
        .map(|command| match *command {
            RasterPathCommand::MoveTo(p) => RasterPathCommand::MoveTo(map(p)),
            RasterPathCommand::CubicTo(a, b, p) => {
                RasterPathCommand::CubicTo(map(a), map(b), map(p))
            }
            RasterPathCommand::Close => RasterPathCommand::Close,
            RasterPathCommand::LineTo(p) => RasterPathCommand::LineTo(map(p)),
        })
        .collect::<Vec<_>>();
    Some(RasterInlineArrow {
        paths: vec![RasterInlineArrowPath {
            id,
            path_data: raster_path_data(&mapped),
            commands,
            source_transform: Some(transform),
            fill: if hollow {
                background.to_owned()
            } else {
                stroke.to_owned()
            },
            stroke: hollow.then(|| stroke.to_owned()),
            stroke_width: hollow.then_some(stroke_width),
            stroke_style: hollow.then_some("stroke-linecap:butt;stroke-linejoin:round"),
        }],
    })
}

/// Source connection.go:558–595: outline the cross in local coordinates,
/// rotate its child, then compose the endpoint marker transform before paint.
#[allow(clippy::too_many_arguments)]
fn raster_cross_arrow(
    id: String,
    route: &[Point],
    tangent_route: &[Point],
    target: bool,
    endpoint_adjustment: Point,
    dx: f64,
    dy: f64,
    stroke_width: f64,
    stroke: &str,
    background: &str,
) -> Option<RasterInlineArrow> {
    if route.len() < 2 || tangent_route.len() < 2 {
        return None;
    }
    let (width, height) = marker_dimensions("cross", stroke_width);
    let ref_x = if target {
        width - 1.5 * stroke_width
    } else {
        1.5 * stroke_width
    };
    let end = if target { route.len() - 1 } else { 0 };
    let start_tangent = if target { tangent_route.len() - 2 } else { 0 };
    let end_tangent = start_tangent + 1;
    let tangent_x = tangent_route[end_tangent].x - tangent_route[start_tangent].x;
    let tangent_y = tangent_route[end_tangent].y - tangent_route[start_tangent].y;
    let (sin, cos) = go_sincos_route_angle(tangent_y.atan2(tangent_x));
    let endpoint = Point {
        x: route[end].x + endpoint_adjustment.x,
        y: route[end].y + endpoint_adjustment.y,
    };
    let parent = [
        cos,
        -sin,
        sin,
        cos,
        cos * -ref_x + -sin * (-height / 2.0) + endpoint.x + dx,
        sin * -ref_x + cos * (-height / 2.0) + endpoint.y + dy,
    ];
    let (rs, rc) = go_sincos_route_angle(std::f64::consts::FRAC_PI_4);
    let tx = -(rc * (width / 2.0) + -rs * (height / 2.0)) + width / 2.0;
    let ty = -(rs * (width / 2.0) + rc * (height / 2.0)) + height / 2.0;
    let cross_transform = [
        parent[0] * rc + parent[1] * rs,
        parent[0] * -rs + parent[1] * rc,
        parent[2] * rc + parent[3] * rs,
        parent[2] * -rs + parent[3] * rc,
        parent[0] * tx + parent[1] * ty + parent[4],
        parent[2] * tx + parent[3] * ty + parent[5],
    ];
    let inset = stroke_width / 8.0;
    let vertices = [
        (0.0, height / 2.0 + inset),
        (width / 2.0 - inset, height / 2.0 + inset),
        (width / 2.0 - inset, height),
        (width / 2.0 + inset, height),
        (width / 2.0 + inset, height / 2.0 + inset),
        (width, height / 2.0 + inset),
        (width, height / 2.0 - inset),
        (width / 2.0 + inset, height / 2.0 - inset),
        (width / 2.0 + inset, 0.0),
        (width / 2.0 - inset, 0.0),
        (width / 2.0 - inset, height / 2.0 - inset),
        (0.0, height / 2.0 - inset),
    ];
    let mut cross = Vec::with_capacity(13);
    for (i, (x, y)) in vertices.into_iter().enumerate() {
        cross.push(if i == 0 {
            RasterPathCommand::MoveTo(Point { x, y })
        } else {
            RasterPathCommand::LineTo(Point { x, y })
        });
    }
    cross.push(RasterPathCommand::Close);
    let stem = vec![
        RasterPathCommand::MoveTo(Point {
            x: width / 2.0,
            y: height / 2.0,
        }),
        RasterPathCommand::LineTo(Point {
            x: if target { width } else { 0.0 },
            y: height / 2.0,
        }),
    ];
    let make_path = |suffix: &str, commands: Vec<RasterPathCommand>, transform: [f64; 6]| {
        let mapped = commands
            .iter()
            .copied()
            .map(|command| {
                let map = |p: Point| Point {
                    x: transform[0] * p.x + transform[1] * p.y + transform[4],
                    y: transform[2] * p.x + transform[3] * p.y + transform[5],
                };
                match command {
                    RasterPathCommand::MoveTo(p) => RasterPathCommand::MoveTo(map(p)),
                    RasterPathCommand::LineTo(p) => RasterPathCommand::LineTo(map(p)),
                    RasterPathCommand::Close => RasterPathCommand::Close,
                    RasterPathCommand::CubicTo(a, b, p) => {
                        RasterPathCommand::CubicTo(map(a), map(b), map(p))
                    }
                }
            })
            .collect::<Vec<_>>();
        RasterInlineArrowPath {
            id: format!("{id}-{suffix}"),
            path_data: raster_path_data(&mapped),
            commands,
            source_transform: Some(transform),
            fill: background.to_owned(),
            stroke: Some(stroke.to_owned()),
            stroke_width: Some(stroke_width),
            stroke_style: Some("stroke-linecap:butt;stroke-linejoin:miter;stroke-miterlimit:4"),
        }
    };
    Some(RasterInlineArrow {
        paths: vec![
            make_path("cross", cross, cross_transform),
            make_path("stem", stem, parent),
        ],
    })
}

#[allow(clippy::too_many_arguments)]
/// Original d2scenebuild/connection.go arrowhead polygon contracts. Keep
/// local f64 vertices and the endpoint matrix until native raster submission.
#[allow(clippy::too_many_arguments)]
fn raster_polygon_arrow(
    id: String,
    name: &str,
    route: &[Point],
    tangent_route: &[Point],
    target: bool,
    endpoint_adjustment: Point,
    dx: f64,
    dy: f64,
    stroke_width: f64,
    stroke: &str,
    background: &str,
) -> Option<RasterInlineArrow> {
    if route.len() < 2 || tangent_route.len() < 2 {
        return None;
    }
    let (width, height) = marker_dimensions(name, stroke_width);
    let point = |x, y| Point { x, y };
    let inset = stroke_width / 2.0;
    let vertices = match name {
        "arrow" if target => vec![
            point(0.0, 0.0),
            point(width, height / 2.0),
            point(0.0, height),
            point(width / 4.0, height / 2.0),
        ],
        "arrow" => vec![
            point(0.0, height / 2.0),
            point(width, 0.0),
            point(width * 0.75, height / 2.0),
            point(width, height),
        ],
        "unfilled-triangle" | "line" if target => vec![
            point(inset, inset),
            point(width - inset, height / 2.0),
            point(inset, height - inset),
        ],
        "unfilled-triangle" | "line" => vec![
            point(width - inset, inset),
            point(inset, height / 2.0),
            point(width - inset, height - inset),
        ],
        "filled-diamond" => vec![
            point(0.0, height / 2.0),
            point(width / 2.0, 0.0),
            point(width, height / 2.0),
            point(width / 2.0, height),
        ],
        "diamond" if target => vec![
            point(0.0, height / 2.0),
            point(width / 2.0, height / 8.0),
            point(width, height / 2.0),
            point(width / 2.0, height * 0.9),
        ],
        "diamond" => vec![
            point(width / 8.0, height / 2.0),
            point(width * 0.6, height / 8.0),
            point(width * 1.1, height / 2.0),
            point(width * 0.6, height * 7.0 / 8.0),
        ],
        "box" => vec![
            point(inset, inset),
            point(inset, height - inset),
            point(width - inset, height - inset),
            point(width - inset, inset),
        ],
        "filled-box" => vec![
            point(0.0, 0.0),
            point(0.0, height),
            point(width, height),
            point(width, 0.0),
        ],
        _ => return None,
    };
    let ref_x = if name == "diamond" {
        if target {
            width - 0.6 * stroke_width
        } else {
            width / 8.0 + 0.6 * stroke_width
        }
    } else if target {
        width - 1.5 * stroke_width
    } else {
        1.5 * stroke_width
    };
    let end = if target { route.len() - 1 } else { 0 };
    let tangent_index = if target { tangent_route.len() - 2 } else { 0 };
    let tangent_x = tangent_route[tangent_index + 1].x - tangent_route[tangent_index].x;
    let tangent_y = tangent_route[tangent_index + 1].y - tangent_route[tangent_index].y;
    let (sin, cos) = go_sincos_route_angle(tangent_y.atan2(tangent_x));
    let endpoint = point(
        route[end].x + endpoint_adjustment.x,
        route[end].y + endpoint_adjustment.y,
    );
    let matrix = raster_scene_matrix_mul(
        raster_scene_matrix_mul(
            [1.0, 0.0, 0.0, 1.0, endpoint.x, endpoint.y],
            [cos, -sin, sin, cos, 0.0, 0.0],
        ),
        [1.0, 0.0, 0.0, 1.0, -ref_x, -height / 2.0],
    );
    let matrix = [
        matrix[0],
        matrix[1],
        matrix[2],
        matrix[3],
        matrix[4] + dx,
        matrix[5] + dy,
    ];
    let mut commands = vec![RasterPathCommand::MoveTo(vertices[0])];
    commands.extend(vertices[1..].iter().copied().map(RasterPathCommand::LineTo));
    if name != "line" {
        commands.push(RasterPathCommand::Close);
    }
    let mapped = commands
        .iter()
        .map(|command| match *command {
            RasterPathCommand::MoveTo(p) => RasterPathCommand::MoveTo(point(
                matrix[0] * p.x + matrix[1] * p.y + matrix[4],
                matrix[2] * p.x + matrix[3] * p.y + matrix[5],
            )),
            RasterPathCommand::LineTo(p) => RasterPathCommand::LineTo(point(
                matrix[0] * p.x + matrix[1] * p.y + matrix[4],
                matrix[2] * p.x + matrix[3] * p.y + matrix[5],
            )),
            RasterPathCommand::Close => RasterPathCommand::Close,
            RasterPathCommand::CubicTo(..) => unreachable!(),
        })
        .collect::<Vec<_>>();
    let hollow = matches!(name, "unfilled-triangle" | "diamond" | "box" | "line");
    let round = matches!(name, "unfilled-triangle" | "diamond");
    Some(RasterInlineArrow {
        paths: vec![RasterInlineArrowPath {
            id,
            path_data: raster_path_data(&mapped),
            commands,
            source_transform: Some(matrix),
            fill: if name == "line" {
                "none".to_owned()
            } else if hollow {
                background.to_owned()
            } else {
                stroke.to_owned()
            },
            stroke: hollow.then(|| stroke.to_owned()),
            stroke_width: hollow.then_some(stroke_width),
            stroke_style: if !hollow {
                None
            } else if round {
                Some("stroke-linecap:butt;stroke-linejoin:round")
            } else {
                Some("stroke-linecap:butt;stroke-linejoin:miter;stroke-miterlimit:4")
            },
        }],
    })
}

fn raster_inline_arrowhead(
    shape: Option<&str>,
    id: String,
    route: &[Point],
    tangent_route: &[Point],
    target: bool,
    endpoint_adjustment: Point,
    dx: f64,
    dy: f64,
    stroke_width: f64,
    stroke: &str,
    background: &str,
) -> Option<RasterInlineArrow> {
    match canonical_marker_name(shape)? {
        name @ ("arrow" | "unfilled-triangle" | "line" | "diamond" | "filled-diamond" | "box"
        | "filled-box") => raster_polygon_arrow(
            id,
            name,
            route,
            tangent_route,
            target,
            endpoint_adjustment,
            dx,
            dy,
            stroke_width,
            stroke,
            background,
        ),
        name @ ("circle" | "filled-circle") => raster_circle_arrow(
            id,
            name,
            route,
            tangent_route,
            target,
            endpoint_adjustment,
            dx,
            dy,
            stroke_width,
            stroke,
            background,
        ),
        "cross" => raster_cross_arrow(
            id,
            route,
            tangent_route,
            target,
            endpoint_adjustment,
            dx,
            dy,
            stroke_width,
            stroke,
            background,
        ),
        "triangle" => raster_triangle_arrow(
            id,
            route,
            tangent_route,
            target,
            endpoint_adjustment,
            dx,
            dy,
            stroke_width,
            stroke,
        ),
        name @ ("cf-one" | "cf-many" | "cf-one-required" | "cf-many-required") => {
            raster_crowfoot_arrow(
                id,
                name,
                route,
                tangent_route,
                target,
                endpoint_adjustment,
                dx,
                dy,
                stroke_width,
                stroke,
            )
        }
        _ => None,
    }
}

fn raster_path_data(commands: &[RasterPathCommand]) -> String {
    let mut output = String::new();
    for command in commands {
        let command_text = match command {
            RasterPathCommand::MoveTo(point) => {
                format!("M {} {}", number(point.x), number(point.y))
            }
            RasterPathCommand::LineTo(point) => {
                format!("L {} {}", number(point.x), number(point.y))
            }
            RasterPathCommand::CubicTo(a, b, point) => format!(
                "C {} {} {} {} {} {}",
                number(a.x),
                number(a.y),
                number(b.x),
                number(b.y),
                number(point.x),
                number(point.y)
            ),
            RasterPathCommand::Close => "Z".to_owned(),
        };
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(&command_text);
    }
    output
}

/// D2 v0.9.0's Go `math.Sincos` polynomial for route angles in [-π, π].
///
/// `f64::sin_cos` uses the host libm implementation, while Go evaluates its
/// split-π/4 reduction and Horner polynomials. The last bits affect arrowhead
/// vertices that are subsequently rounded at the raster crop boundary.
fn go_sincos_route_angle(angle: f64) -> (f64, f64) {
    const PI: f64 = std::f64::consts::PI;
    const PI4A: f64 = 7.853_981_256_484_985_351_56e-1;
    const PI4B: f64 = 3.774_894_707_930_798_176_68e-8;
    const PI4C: f64 = 2.695_151_429_079_059_526_45e-15;
    const SIN: [f64; 6] = [
        1.589_623_015_765_465_7e-10,
        -2.505_074_776_285_780_7e-8,
        2.755_731_362_138_572_5e-6,
        -1.984_126_982_958_954e-4,
        8.333_333_333_322_119e-3,
        -1.666_666_666_666_663_1e-1,
    ];
    const COS: [f64; 6] = [
        -1.135_853_652_138_768_2e-11,
        2.087_570_084_197_473_2e-9,
        -2.755_731_417_929_674e-7,
        2.480_158_728_885_170_5e-5,
        -1.388_888_888_887_305_6e-3,
        4.166_666_666_666_659_e-2,
    ];
    let (x, mut sin_negative) = if angle < 0.0 {
        (-angle, true)
    } else {
        (angle, false)
    };
    let mut cos_negative = false;
    let mut j = (x * (4.0 / PI)).trunc() as u64;
    let mut y = j as f64;
    if j & 1 == 1 {
        j += 1;
        y += 1.0;
    }
    j &= 7;
    let z = ((x - y * PI4A) - y * PI4B) - y * PI4C;
    if j > 3 {
        j -= 4;
        sin_negative = !sin_negative;
        cos_negative = !cos_negative;
    }
    if j > 1 {
        cos_negative = !cos_negative;
    }
    let zz = z * z;
    let cos_poly =
        (((((COS[0] * zz + COS[1]) * zz + COS[2]) * zz + COS[3]) * zz + COS[4]) * zz) + COS[5];
    let sin_poly =
        (((((SIN[0] * zz + SIN[1]) * zz + SIN[2]) * zz + SIN[3]) * zz + SIN[4]) * zz) + SIN[5];
    let mut cos = 1.0 - 0.5 * zz + zz * zz * cos_poly;
    let mut sin = z + z * zz * sin_poly;
    if j == 1 || j == 2 {
        std::mem::swap(&mut sin, &mut cos);
    }
    if cos_negative {
        cos = -cos;
    }
    if sin_negative {
        sin = -sin;
    }
    (sin, cos)
}

#[allow(clippy::too_many_arguments)]
fn marker_reference(
    shape: Option<&str>,
    is_target: bool,
    stroke_width: f64,
    stroke: &str,
    hash_stroke: &str,
    default_stroke: &str,
    diagram_hash: Option<&str>,
    marker_theme: MarkerTheme<'_>,
    dynamic: &mut BTreeMap<String, String>,
) -> Option<String> {
    let name = canonical_marker_name(shape)?;
    if diagram_hash.is_none()
        && (stroke_width - 2.0).abs() <= f64::EPSILON
        && hash_stroke == "B1"
        && stroke == default_stroke
        && marker_theme.inline
        && marker_theme.palette.base == active_palette(None)
        && marker_theme.palette.overrides.is_empty()
    {
        return marker_id(shape, is_target).map(str::to_owned);
    }
    let hash = marker_hash(name, is_target, stroke_width, hash_stroke);
    let id = diagram_hash
        .map(|diagram_hash| format!("mk-{diagram_hash}-{hash}"))
        .unwrap_or_else(|| {
            format!(
                "d2-rs-marker-{name}-{}-{hash}",
                if is_target { "end" } else { "start" },
            )
        });
    dynamic.entry(id.clone()).or_insert_with(|| {
        dynamic_marker_definition(
            &id,
            name,
            is_target,
            stroke_width,
            hash_stroke,
            marker_theme,
        )
    });
    Some(id)
}

/// SVG marker definitions matching the pinned D2 v0.9.0 renderer at the
/// default connection stroke width (2px). Marker geometry is kept in one
/// place so source and target arrowheads cannot drift apart.
/// Renders a laid-out source graph as a deterministic semantic SVG.
pub fn render_svg(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Vec<u8> {
    render_svg_with_options(graph, result, labels, styles, RenderOptions::default())
}

/// Renders a graph as SVG with explicit canvas options.
///
/// `pad` is clamped to a finite, non-negative value. The option is applied
/// only while constructing the output viewBox; node boxes and routes remain
/// exactly those returned by the layout engine.
pub fn render_svg_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
) -> Vec<u8> {
    render_svg_with_sketch(graph, result, labels, styles, options, false)
}

const NATIVE_MARKDOWN_HTML_ELEMENTS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "del",
    "em",
    "head",
    "html",
    "body",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "img",
    "li",
    "ol",
    "p",
    "pre",
    "s",
    "strong",
    "strike",
    "table",
    "td",
    "th",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "ul",
];

fn first_unsupported_markdown_html(markdown: &str) -> Option<String> {
    let html = normalize_html_names(&render_html(markdown.as_bytes()));
    let mut cursor = 0;
    while let Some(relative_start) = html[cursor..].find('<') {
        let start = cursor + relative_start + 1;
        let rest = &html[start..];
        if rest.starts_with('/') {
            cursor = start + 1;
            continue;
        }
        let Some(end) = rest.find('>') else {
            break;
        };
        let tag = &rest[..end];
        let name_end = tag
            .find(|character: char| character.is_ascii_whitespace() || character == '/')
            .unwrap_or(tag.len());
        let name = &tag[..name_end];
        if !name.is_empty()
            && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && !NATIVE_MARKDOWN_HTML_ELEMENTS.contains(&name)
        {
            return Some(name.to_owned());
        }
        cursor = start + end + 1;
    }
    None
}

/// Source preflightShape admits label languages before importing rich assets.
fn validate_shape_labels(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<(), RenderError> {
    validate_shape_labels_inner(graph, styles, None)
}

fn validate_shape_labels_inner(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    native: Option<(&LayoutResult, &BTreeMap<NodeId, String>)>,
) -> Result<(), RenderError> {
    validate_shape_labels_policy(graph, styles, native, false)
}

fn validate_shape_labels_policy(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    native: Option<(&LayoutResult, &BTreeMap<NodeId, String>)>,
    public_svg: bool,
) -> Result<(), RenderError> {
    for (id, node) in graph.nodes() {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let language = style.language.as_deref().unwrap_or("");
        let object = format!(
            "shape {}",
            d2_rs_core::quote_go_bytes(node.external_id.as_bytes())
        );
        let invalid_utf8 = graph
            .node_label_bytes
            .get(&id)
            .is_some_and(|bytes| std::str::from_utf8(bytes).is_err());
        let validate_code = || {
            let Some((result, labels)) = native else {
                return Ok(());
            };
            let font_size = result
                .node_labels
                .get(&id)
                .and_then(|state| state.font_size)
                .or(node.font_size)
                .unwrap_or(16);
            let label = labels.get(&id).map_or(&[][..], |s| s.as_bytes());
            let bytes = graph
                .node_label_bytes
                .get(&id)
                .filter(|_| invalid_utf8)
                .map_or(label, Vec::as_slice);
            code_admission::validate(&object, bytes, language, i64::from(font_size))
        };
        // Visible code-shape validation precedes the language switch. Hidden
        // owners skip code geometry admission but still reject unsupported tags.
        if node.shape == ShapeKind::Code
            && style
                .opacity
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                != Some(0.0)
        {
            validate_code()?;
        }
        if node.shape == ShapeKind::Code
            && invalid_utf8
            && style
                .opacity
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                != Some(0.0)
        {
            return Err(RenderError::InvalidLabelUtf8 { object });
        }
        let includes_code = matches!(language, "markdown" | "latex");
        // d2svg.Render dispatches structured shapes before label language,
        // and supports Markdown/LaTeX on code-shaped owners. The separate
        // d2scenebuild.Build API rejects these combinations.
        if !public_svg
            && !language.is_empty()
            && (matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable)
                || (includes_code && node.shape == ShapeKind::Code))
        {
            return Err(RenderError::UnsupportedLabelLanguage {
                object,
                language: language.to_owned(),
                includes_code,
            });
        }
        if !language.is_empty() && !matches!(language, "markdown" | "latex") {
            validate_code()?;
        }
        if !language.is_empty() && invalid_utf8 {
            return Err(RenderError::InvalidLabelUtf8 { object });
        }
        if language == "latex"
            && let Some((_, labels)) = native
        {
            latex_admission::validate(&object, labels.get(&id).map_or(&[][..], |s| s.as_bytes()))?;
        }
    }
    Ok(())
}

/// Validates source Markdown labels against D2's native SVG element boundary.
pub fn validate_markdown_labels(
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<(), RenderError> {
    validate_markdown_labels_for_shapes(labels, styles, None)
}

fn validate_markdown_labels_for_shapes(
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    public_svg: Option<&Graph>,
) -> Result<(), RenderError> {
    for (node_id, style) in styles {
        let structured = public_svg
            .and_then(|graph| graph.node(*node_id))
            .is_some_and(|node| matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable));
        if style.markdown && !structured {
            if let Some(label) = labels.get(node_id) {
                if let Some(element) = first_unsupported_markdown_html(label) {
                    return Err(RenderError::UnsupportedMarkdownHtml { element });
                }
            }
        }
        if style
            .tooltip_position
            .as_deref()
            .is_some_and(|position| !position.is_empty())
        {
            if let Some(tooltip) = style.tooltip.as_deref() {
                if let Some(element) = first_unsupported_markdown_html(tooltip) {
                    return Err(RenderError::UnsupportedMarkdownHtml { element });
                }
            }
        }
    }
    Ok(())
}

// Source addAppendixItem validates raw target bytes only when appendices
// are requested. Public JSON values can already contain replacement runes,
// so source compilation must retain the original byte spelling separately.
fn validate_appendix_metadata(
    graph: &Graph,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    enabled: bool,
) -> Result<(), RenderError> {
    if !enabled {
        return Ok(());
    }
    for (index, (id, node)) in graph.nodes().enumerate() {
        if styles
            .get(&id)
            .and_then(|style| style.pretty_link_bytes.as_ref())
            .is_some_and(|value| std::str::from_utf8(value.as_bytes()).is_err())
        {
            return Err(RenderError::MarkdownScene(format!(
                "scene: shape[{index}] {} field \"prettyLink\" must be valid UTF-8",
                d2_rs_core::quote_go_bytes(node.external_id.as_bytes()),
            )));
        }
    }
    Ok(())
}

/// Checked SVG source path matching D2's unsupported-Markdown error boundary.
// d2svg.drawConnection needs two route points, even for an invisible edge.
// A nonempty label obtains its point first and dereferences nil on a short
// route; otherwise getArrowheadAdjustments indexes route[1]. No shape/input
// selectors belong in this state check.
fn validate_connection_routes(graph: &Graph, result: &LayoutResult) -> Result<(), RenderError> {
    let mut edges = graph
        .edges()
        .filter(|(id, _)| !result.removed_edges.contains(id))
        .collect::<Vec<_>>();
    edges.sort_by_key(|(_, edge)| sequence_connection_z_index(graph, edge));
    for (id, _) in edges {
        let length = result.routes.get(&id).map_or(0, Vec::len);
        if length >= 2 {
            continue;
        }
        if graph
            .edge_label(id)
            .is_some_and(|label| !label.text.is_empty())
        {
            return Err(RenderError::SourceNilPointer {
                source_site: "d2svg.drawConnection:1136",
            });
        }
        return Err(RenderError::SourceRouteIndex { index: 1, length });
    }
    Ok(())
}

// d2exporter truncates route coordinates before d2scenebuild validates the
// target. Keep this admission separate from the legacy public SVG renderer.
fn validate_scene_route_tangents(graph: &Graph, result: &LayoutResult) -> Result<(), RenderError> {
    let mut edges = graph
        .edges()
        .filter(|(id, _)| !result.removed_edges.contains(id))
        .collect::<Vec<_>>();
    edges.sort_by_key(|(_, e)| sequence_connection_z_index(graph, e));
    for (id, _) in edges {
        let Some(route) = result.routes.get(&id) else {
            continue;
        };
        if route.len() < 2 {
            continue;
        }
        for (target, from, to, endpoint) in [
            (false, 0, 1, "source"),
            (true, route.len() - 2, route.len() - 1, "destination"),
        ] {
            if target_arrow_name(graph, id, target) == "none" {
                continue;
            }
            let a = rendered_route_point(graph, route[from]);
            let b = rendered_route_point(graph, route[to]);
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            if dy.mul_add(dy, dx * dx).sqrt() == 0. {
                return Err(RenderError::SceneRouteTangent {
                    connection: d2_rs_core::edge_abs_id(graph, id).unwrap_or_default(),
                    endpoint,
                });
            }
        }
    }
    Ok(())
}

pub fn render_svg_with_sketch_checked(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, RenderError> {
    validate_shape_labels_policy(graph, styles, None, true)?;
    validate_markdown_labels_for_shapes(labels, styles, Some(graph))?;
    validate_appendix_metadata(graph, styles, options.force_appendix)?;
    validate_connection_routes(graph, result)?;
    Ok(render_svg_with_sketch(
        graph, result, labels, styles, options, sketch,
    ))
}

/// Renders a graph as SVG and optionally applies D2's seeded sketch overlay.
///
/// The sketch switch is kept outside [`RenderOptions`] so existing callers and
/// serialized option shapes remain source-compatible. With `sketch == false`
/// this follows the ordinary renderer byte-for-byte. The seeded overlay is
/// deterministic and affects only the supported rectangular shape family.
pub fn render_svg_with_sketch(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Vec<u8> {
    render_svg_with_sketch_inner(
        graph, result, labels, styles, options, sketch, None, false, None, None,
    )
}

// Preserve native scene opacity before usvg narrows it to f32. Public SVG
// serialization does not call this private transport step.
fn tag_raster_effect_opacities(svg: &str) -> String {
    let Ok(document) = roxmltree::Document::parse(svg) else {
        return svg.to_owned();
    };
    let mut ids = document
        .descendants()
        .filter_map(|node| node.attribute("id"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut insertions = Vec::new();
    let mut ordinal = 0;
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "g")
    {
        let Some(opacity) = node
            .attribute("opacity")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
        else {
            continue;
        };
        if node.attribute("style").is_some_and(|style| {
            style.split(';').any(|declaration| {
                declaration
                    .split(':')
                    .next()
                    .is_some_and(|property| property.trim() == "opacity")
            })
        }) {
            continue;
        }
        let mut attributes = format!(" data-d2-rs-exact-opacity=\"{}\"", opacity.clamp(0.0, 1.0));
        if node.attribute("id").is_none() {
            let id = loop {
                let candidate = format!("d2-rs-native-effect-opacity-{ordinal}");
                ordinal += 1;
                if ids.insert(candidate.clone()) {
                    break candidate;
                }
            };
            attributes.push_str(&format!(" id=\"{id}\""));
        }
        let start = node.range().start;
        // Insert after the element name, before existing attributes.
        insertions.push((start + 2, attributes));
    }
    for node in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "image"
            && node
                .attribute("preserveAspectRatio")
                .is_some_and(|raw| d2_rs_math::svg_viewport::aspect("scene", raw, "image").is_ok())
    }) {
        let href = node
            .attribute("href")
            .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")));
        if !href.is_some_and(|href| {
            href.starts_with("data:image/png;base64,")
                || href.starts_with("data:image/svg+xml;base64,")
        }) {
            continue;
        }
        let mut attributes = String::from(" data-d2-rs-scene-image=\"ordinary\"");
        if node.attribute("id").is_none() {
            let id = loop {
                let candidate = format!("d2-rs-native-scene-image-{ordinal}");
                ordinal += 1;
                if ids.insert(candidate.clone()) {
                    break candidate;
                }
            };
            attributes.push_str(&format!(" id=\"{id}\""));
        }
        insertions.push((node.range().start + "<image".len(), attributes));
    }
    insertions.sort_by_key(|(position, _)| *position);
    let mut tagged = svg.to_owned();
    for (position, attributes) in insertions.into_iter().rev() {
        tagged.insert_str(position, &attributes);
    }
    tagged
}

/// Renders the raster target and returns the original curved-route commands
/// needed by the native rasterizer before they are rounded for SVG text.
#[doc(hidden)]
pub fn render_svg_with_raster_sources_checked(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<(Vec<u8>, Vec<RasterPathSource>), RenderError> {
    render_svg_with_raster_sources_controlled(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        &mut d2_rs_text::code::CodeBudget::default(),
        None,
    )
}

/// Native code-stage shared budget and cooperative context. Other scene work limits remain independent.
pub fn render_svg_with_raster_sources_controlled(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    budget: &mut d2_rs_text::code::CodeBudget,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(Vec<u8>, Vec<RasterPathSource>), RenderError> {
    code_budget::context(cancel)?;
    if let Some(style) = &graph.root_style {
        for (value, effect) in [
            (&style.three_dee, "3d effect"),
            (&style.multiple, "multiple effect"),
            (&style.shadow, "effect"),
        ] {
            if value.as_deref().and_then(d2_rs_core::parse_d2_bool) == Some(true) {
                return Err(RenderError::UnsupportedRootEffect { effect });
            }
        }
    }
    validate_shape_labels_inner(graph, styles, Some((result, labels)))?;
    validate_connection_routes(graph, result)?;
    validate_scene_route_tangents(graph, result)?;
    legend_connection::validate(graph.legend.as_ref())?;
    validate_appendix_metadata(graph, styles, options.force_appendix)?;
    for (id, _) in graph.edges() {
        let style = graph.edge_style(id);
        let language = style.language.as_deref().unwrap_or_default();
        code_budget::context(cancel)?;
        if language.is_empty() || language == "markdown" {
            continue;
        }
        let label = effective_edge_label(graph, result, id);
        let bytes = label
            .as_ref()
            .map_or(&[][..], |label| label.text.as_bytes());
        let object = format!(
            "connection {}",
            d2_rs_core::quote_go_bytes(edge_svg_id(graph, id).unwrap_or_default().as_bytes())
        );
        if language == "latex" {
            latex_admission::validate(&object, bytes)?;
        } else {
            code_admission::validate(
                &object,
                bytes,
                language,
                i64::from(graph.edge_font_size(id)),
            )?;
        }
    }
    // Source assets.go preflights visible nonempty math labels before painting.
    // Opacity-zero shapes are deliberately skipped by the original scene builder.
    for (id, node) in graph.nodes() {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        if style.language.as_deref() != Some("latex")
            || style
                .opacity
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                == Some(0.0)
        {
            continue;
        }
        let Some(label) = labels.get(&id).filter(|s| !s.is_empty()) else {
            continue;
        };
        let object = format!(
            "shape {}",
            d2_rs_core::quote_go_bytes(node.external_id.as_bytes())
        );
        code_budget::context(cancel)?;
        let math = d2_rs_math::render(label).map_err(|e| RenderError::LatexScene {
            object: object.clone(),
            stage: "render",
            error: e.to_string(),
        })?;
        code_budget::context(cancel)?;
        d2_rs_math::import_scene_svg(&math).map_err(|error| RenderError::LatexScene {
            object,
            stage: "SVG",
            error,
        })?;
        code_budget::context(cancel)?;
    }
    // Source assets.go imports visible connection math after shape assets.
    for (id, _) in graph.edges() {
        let style = graph.edge_style(id);
        if style
            .opacity
            .as_deref()
            .and_then(d2_rs_core::parse_d2_float)
            == Some(0.0)
        {
            continue;
        }
        let Some(label) =
            effective_edge_label(graph, result, id).filter(|label| !label.text.is_empty())
        else {
            continue;
        };
        if style.language.as_deref() == Some("latex") {
            let object = format!(
                "connection {}",
                d2_rs_core::quote_go_bytes(edge_svg_id(graph, id).unwrap_or_default().as_bytes())
            );
            code_budget::context(cancel)?;
            let math =
                d2_rs_math::render(&label.text).map_err(|error| RenderError::LatexScene {
                    object: object.clone(),
                    stage: "render",
                    error: error.to_string(),
                })?;
            code_budget::context(cancel)?;
            d2_rs_math::import_scene_svg(&math).map_err(|error| RenderError::LatexScene {
                object,
                stage: "SVG",
                error,
            })?;
            code_budget::context(cancel)?;
        }
    }
    validate_markdown_labels(labels, styles)?;
    let mut raster_sources = Vec::new();
    let code_streams =
        code_budget::collect(graph, result, labels, styles, &options, budget, cancel)?;
    let svg = render_svg_with_sketch_inner(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        Some(&mut raster_sources),
        true,
        Some(&code_streams),
        None,
    );
    code_budget::context(cancel)?;
    markdown_scene::collect_svg(&svg, markdown_scene::LinkBudget::default(), true)
        .map_err(RenderError::MarkdownScene)?;
    let svg = tag_raster_effect_opacities(std::str::from_utf8(&svg).expect("renderer emits UTF-8"))
        .into_bytes();
    Ok((svg, raster_sources))
}

fn draw_root_rectangle(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    identity: &str,
    native_scene: bool,
) {
    let mut fragment = String::new();
    let mut style = style.clone();
    // Go's root ThemableElement omits an empty Fill. Ordinary shape painting
    // otherwise substitutes a theme color for the same empty string. Paint a
    // literal default while constructing this root, then omit its attribute
    // so SVG inheritance/default black retains the source behavior.
    let omit_fill = style.fill.as_deref() == Some("");
    if omit_fill {
        style.fill = Some("black".into());
    }
    // buildRoot clamps both radius axes to the same smallest half-extent.
    // SVG's implicit independent-axis clamping would produce an ellipse.
    if native_scene
        && let Some(radius) = style
            .border_radius
            .as_deref()
            .and_then(|raw| raw.parse::<f64>().ok())
    {
        style.border_radius = Some(
            radius
                .max(0.0)
                .min(rect.size.width.min(rect.size.height) / 2.0)
                .to_string(),
        );
    }
    if let Some(raw) = style
        .stroke
        .as_ref()
        .filter(|s| gradient_definition(s).is_some())
    {
        style.stroke = Some(format!("url(#{})", gradient_id(raw)));
    }
    draw_shape_with_pattern_overlay(
        &mut fragment,
        ShapeKind::Rectangle,
        rect,
        &style,
        palette,
        "",
        native_scene,
        false,
    );
    if let Some(at) = fragment.find("<rect") {
        if omit_fill {
            if let Some(end) = fragment[at..].find('>') {
                let opening = fragment[at..at + end].replace(" fill=\"black\"", "");
                fragment.replace_range(at..at + end, &opening);
            }
        }
        fragment.insert_str(at + 5, &format!(" data-d2-rs-root=\"{identity}\""));
    }
    svg.push_str(&fragment);
}

fn draw_root_canvas(
    svg: &mut String,
    graph: &Graph,
    background: Rect,
    stroke_padding: f64,
    double: bool,
    options: &RenderOptions,
    palette: PaletteView<'_>,
    native_scene: bool,
) {
    let root = graph.root_style.as_ref();
    let mut style = SourceNodeStyle {
        fill: Some(if options.transparent_background {
            "transparent".into()
        } else {
            root.and_then(|s| s.fill.clone())
                .unwrap_or_else(|| "N7".into())
        }),
        stroke: Some(
            root.and_then(|s| s.stroke.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "none".into()),
        ),
        stroke_width: Some(
            root.and_then(|s| s.stroke_width.clone())
                .unwrap_or_else(|| "0".into()),
        ),
        stroke_dash: root.and_then(|s| s.stroke_dash.clone()),
        fill_pattern: root.and_then(|s| s.fill_pattern.clone()),
        border_radius: root.and_then(|s| s.border_radius.clone()),
        ..SourceNodeStyle::default()
    };
    // Root opacity/shadow/multiple/3d are not ordinary shape effects. Scene
    // preflight handles rejected effects; root rectangles keep unit opacity.
    if double {
        let offset = 2.0 * stroke_padding + 5.0;
        let outer = Rect {
            origin: Point {
                x: background.origin.x - offset,
                y: background.origin.y - offset,
            },
            size: Size {
                width: background.size.width + 2.0 * offset,
                height: background.size.height + 2.0 * offset,
            },
        };
        draw_root_rectangle(svg, outer, &style, palette, "outer", native_scene);
        style.fill = Some("transparent".into());
    }
    draw_root_rectangle(svg, background, &style, palette, "background", native_scene);
}

fn render_svg_with_sketch_inner(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    mut raster_sources: Option<&mut Vec<RasterPathSource>>,
    raster_text_geometry_precision: bool,
    code_streams: Option<&code_budget::Streams>,
    target: Option<&target::PaintContext<'_>>,
) -> Vec<u8> {
    // D2 derives every target-level SVG identifier from the compact diagram
    // hash, including ordinary (unsalted) renders. A caller-provided salt is
    // part of the same target byte stream; it only adds isolation to the
    // resulting diagram hash and to local IDs such as gradients.
    let requested_salt = options
        .salt
        .as_deref()
        .filter(|salt| !salt.is_empty())
        .unwrap_or("");
    let diagram_hash = target.map(|t| t.hash().to_owned()).unwrap_or_else(|| {
        format!(
            "d2-{}",
            target_hash_id(
                graph,
                result,
                labels,
                styles,
                &options,
                sketch,
                requested_salt,
            )
        )
    });
    // Theme special rules are exporter transformations. A public target
    // already contains their result; render-only theme selection supplies
    // colors, without rewriting its fonts, case, borders or patterns.
    let target_palette = |mut palette: Palette| {
        if target.is_some() {
            palette.special_rules = Default::default();
        }
        palette
    };
    let palette = PaletteView::new(
        target_palette(active_palette(options.theme_id)),
        &options.theme_overrides,
    );
    let dark_palette = options.dark_theme_id.map(|theme_id| {
        PaletteView::new(
            target_palette(active_palette(Some(theme_id))),
            &options.dark_theme_overrides,
        )
    });
    let marker_theme = MarkerTheme {
        palette,
        inline: dark_palette.is_none(),
    };
    // Both original SVG and scene builders derive the canvas from the
    // diagram bounds. Rough painter perturbations do not add canvas padding.
    let markdown_family = target.map_or_else(
        || {
            if palette.base.special_rules.mono {
                MarkdownFamily::Mono
            } else if sketch {
                MarkdownFamily::HandDrawn
            } else {
                MarkdownFamily::Sans
            }
        },
        |t| t.primary_family(),
    );
    let hand_drawn_font = target.map_or(sketch, |t| {
        matches!(t.primary_family().primary_kind(), BundledFamily::HandDrawn)
    });
    let mono_primary =
        target.is_some_and(|t| matches!(t.primary_family().primary_kind(), BundledFamily::Mono));
    let (min_x, min_y, max_x, max_y) =
        bounds_with_markdown_family(graph, result, labels, styles, markdown_family);
    let required_patterns = required_patterns(graph, styles, palette.base.special_rules);
    let padding = if options.pad.is_finite() {
        options.pad.max(0.0)
    } else {
        PADDING
    };
    let scale = options
        .scale
        .filter(|scale| scale.is_finite())
        .unwrap_or(1.0);
    let dx = 0.0;
    let dy = 0.0;
    // D2 keeps the layout's source-space coordinates in the inner SVG and
    // puts the padding in its viewBox origin. The outer fit-to-screen wrapper
    // remains normalized at 0,0; this preserves the browser-facing canvas
    // dimensions without translating every shape, route, and mask.
    let root_style = graph.root_style.as_ref();
    let root_width = root_style
        .and_then(|s| s.stroke_width.as_deref())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0) as f64;
    let root_padding = (root_width / 2.0).ceil();
    let root_double = root_style
        .and_then(|s| s.double_border.as_deref())
        .and_then(d2_rs_core::parse_d2_bool)
        == Some(true);
    let mut canvas = (
        min_x - padding,
        min_y - padding,
        (max_x - min_x + 2.0 * padding).max(1.0),
        (max_y - min_y + 2.0 * padding).max(1.0),
    );
    if let Some(layout) = graph.legend.as_ref().and_then(|legend| {
        legend_layout::measure(
            legend,
            (min_x, min_y, max_x, max_y),
            palette.base.special_rules.mono,
        )
    }) {
        let mut layout = layout;
        if target.is_some() {
            let count = graph
                .legend
                .as_ref()
                .unwrap()
                .shapes
                .iter()
                .filter(|s| !s.label.is_empty())
                .count()
                + graph
                    .legend
                    .as_ref()
                    .unwrap()
                    .connections
                    .iter()
                    .filter(|c| !c.label.is_empty())
                    .count();
            let tail = if count > 0 && !graph.legend.as_ref().unwrap().connections.is_empty() {
                30.0
            } else {
                24.0
            };
            layout.height += 20.0 - tail;
        }
        canvas = layout.expand_viewbox(canvas, padding);
    }
    let background = Rect {
        origin: Point {
            x: canvas.0 - root_padding,
            y: canvas.1 - root_padding,
        },
        size: Size {
            width: canvas.2 + 2.0 * root_padding,
            height: canvas.3 + 2.0 * root_padding,
        },
    };
    let root_extra = root_padding
        + if root_double {
            2.0 * root_padding + 5.0
        } else {
            0.0
        };
    let view_left = background.origin.x - root_extra;
    let view_top = background.origin.y - root_extra;
    let base_width = background.size.width + 2.0 * root_extra;
    let base_height = background.size.height + 2.0 * root_extra;
    let width = base_width * scale;
    let height = base_height * scale;
    let dimensions = if options.scale.is_some() || raster_sources.is_some() {
        format!(
            " width=\"{}\" height=\"{}\"",
            number(width.ceil()),
            number(height.ceil())
        )
    } else {
        String::new()
    };
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" class=\"d2-rs-theme{}\" preserveAspectRatio=\"{} meet\"{} viewBox=\"0 0 {} {}\">",
        if hand_drawn_font { " d2-rs-sketch" } else { "" },
        if options.center {
            "xMidYMid"
        } else {
            "xMinYMin"
        },
        dimensions,
        number(base_width),
        number(base_height)
    );
    let font_need_fallback = rendered_font_needs(graph, labels, styles);
    svg.push_str("<!--D2RS_FONT_CSS-->\n");
    let _ = writeln!(
        svg,
        "<style>@keyframes dashdraw{{from{{stroke-dashoffset:0}}}}</style>"
    );
    // The native SVG wrapper is a serialization viewport, not a retained
    // scene effect. Its private identity lets the raster pass distinguish the
    // viewport clip introduced by usvg from actual authored clip effects.
    let native_viewport = if raster_sources.is_some() {
        format!(
            " id=\"d2-rs-native-canvas-viewport-{diagram_hash}\" data-d2-rs-canvas-reference=\"ordinary\""
        )
    } else {
        String::new()
    };
    let _ = writeln!(
        svg,
        "<svg class=\"{} d2-svg\"{native_viewport} width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">",
        diagram_hash,
        number(base_width),
        number(base_height),
        number(view_left),
        number(view_top),
        number(base_width),
        number(base_height),
    );
    // v0.9.0 does not emit a static marker catalog. Arrowheads are keyed by
    // the diagram hash and written at their first connection use below.
    // Keep the built-in D2 fill patterns in the SVG target so explicit
    // `style.fill-pattern` values stay self-contained for browsers and the
    // native resvg path. Definitions are copied from the pinned v0.9.0
    // renderer; only the diagram hash is replaced with a stable local ID.
    if !required_patterns.is_empty() {
        let _ = writeln!(
            svg,
            "<defs>{}</defs>",
            if raster_text_geometry_precision {
                patternassets::definitions_for_raster(&required_patterns)
            } else {
                patternassets::definitions_for(&required_patterns)
            }
        );
    }
    let gradients = required_gradients(graph, styles);
    if !gradients.is_empty() {
        let definitions = gradients.values().map(String::as_str).collect::<String>();
        let _ = writeln!(svg, "<defs>{definitions}</defs>");
    }
    let _ = writeln!(
        svg,
        "<defs><filter id=\"d2-rs-shadow-filter\" x=\"-20%\" y=\"-20%\" width=\"160%\" height=\"160%\"><feFlood flood-color=\"#3d4574\" flood-opacity=\"0.4\" result=\"shadow-color\"/><feComposite in=\"shadow-color\" in2=\"SourceAlpha\" operator=\"in\"/><feOffset dx=\"3\" dy=\"5\"/><feBlend in=\"SourceGraphic\" mode=\"normal\"/></filter></defs>"
    );
    // D2's SVG target masks each connection beneath its label (and icon) so
    // a stroke never shows through a transparent text glyph.  Build the
    // masks once at the target boundary; ordinary paths only carry a stable
    // URL reference, keeping the hot route/label loops free of geometry or
    // allocation work that is unrelated to rendering.
    let mut diagram_mask_rects = Vec::<String>::new();
    // The shared connection mask also hides routes beneath border labels on
    // unrelated shapes. This stage precedes connection label/icon holes.
    for node_id in render_node_ids(graph) {
        let Some(node) = graph.node(node_id) else {
            continue;
        };
        if !labels.get(&node_id).is_some_and(|label| !label.is_empty()) {
            continue;
        }
        let Some(rect) = result.boxes.get(&node_id).copied() else {
            continue;
        };
        let position = result
            .node_labels
            .get(&node_id)
            .map(|state| state.position)
            .filter(|position| *position != LabelPosition::Unset)
            .unwrap_or(node.label_position);
        let label_size = result
            .node_labels
            .get(&node_id)
            .and_then(|state| state.size)
            .or(node.label_size)
            .unwrap_or_default();
        let label_size = Size {
            width: label_size.width.round(),
            height: label_size.height.round(),
        };
        let style = styles.get(&node_id);
        let shape = target_shape_for_node(graph, node_id).unwrap_or(node.shape);
        let Some(hole) = border_label_mask_rect(
            rounded_render_rect(translated(rect, dx, dy)),
            shape,
            position,
            label_size,
            stroke_width(style.and_then(|style| style.stroke_width.as_deref())).round(),
            style_flag(style, |style| style.three_dee.as_ref()),
            style_flag(style, |style| style.multiple.as_ref()),
        ) else {
            continue;
        };
        diagram_mask_rects.push(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"black\"/>",
            number(hole.origin.x),
            number(hole.origin.y),
            number(hole.size.width),
            number(hole.size.height),
        ));
    }
    for (edge_id, _) in graph.edges() {
        let Some(route) = result.routes.get(&edge_id) else {
            continue;
        };
        let label =
            effective_edge_label(graph, result, edge_id).filter(|label| !label.text.is_empty());
        let label_rect = label.as_ref().and_then(|label| {
            edge_label_bounds(
                route,
                label,
                stroke_width(graph.edge_style(edge_id).stroke_width.as_deref()),
                graph.target_routes_exported,
            )
        });
        let icon_rect = edge_icon_rect(graph, result, edge_id, route, dx, dy);
        if label_rect.is_none() && icon_rect.is_none() {
            continue;
        }
        // makeLabelMask in d2svg gives ordinary labels a two-pixel horizontal
        // clear border. Its vertical extent is the measured label height,
        // starting at the rounded label top-left; keep that asymmetry because
        // it is visible on vertical routes in the v0.9.0 raster target.
        let (x, y, label_width, label_height) = if let Some(label_rect) = label_rect {
            let x = label_rect.origin.x + dx - 2.0;
            let y = label_rect.origin.y + dy;
            let right = label_rect.right() + dx;
            let (x, width) = if let Some(icon_rect) = icon_rect {
                let icon_x = icon_rect.origin.x - 2.0;
                (x.min(icon_x), (right - x.min(icon_x) + 2.0).max(1.0))
            } else {
                (x, (label_rect.size.width + 4.0).max(1.0))
            };
            (x, y, width, label_rect.size.height.max(1.0))
        } else {
            let icon_rect = icon_rect.expect("icon rectangle checked above");
            (
                icon_rect.origin.x - 2.0,
                icon_rect.origin.y,
                (icon_rect.size.width + 4.0).max(1.0),
                icon_rect.size.height.max(1.0),
            )
        };
        // d2scenebuild.buildConnectionMask uses text placement in preference
        // to icon placement. Off-edge holes are only 75% black so overlapping
        // connection geometry retains 25% luminance beneath a nearby label.
        let position = label
            .as_ref()
            .map(|label| label.position)
            .or_else(|| graph.edge_icon_position(edge_id))
            .filter(|position| *position != LabelPosition::Unset)
            .unwrap_or(LabelPosition::InsideMiddleCenter);
        let fill = if matches!(
            position,
            LabelPosition::InsideMiddleLeft
                | LabelPosition::InsideMiddleCenter
                | LabelPosition::InsideMiddleRight
                | LabelPosition::UnlockedMiddle
        ) {
            "black"
        } else {
            "rgba(0,0,0,0.75)"
        };
        diagram_mask_rects.push(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            number(x),
            number(y),
            number(label_width),
            number(label_height),
            fill,
        ));
    }
    // The Go SVG always emits its shared base mask, while buildConnectionMask
    // returns nil for the raster scene when there are no actual label holes.
    let connection_mask_id = if raster_sources.is_some() && diagram_mask_rects.is_empty() {
        None
    } else {
        Some(diagram_hash.as_str())
    };
    draw_root_canvas(
        &mut svg,
        graph,
        background,
        root_padding,
        root_double,
        &options,
        palette,
        raster_text_geometry_precision,
    );
    // Source scales the outer logical viewport; scene geometry stays in the
    // base viewBox. A second shape transform would apply scale twice.
    let mut object_paints = Vec::new();
    if graph.root_sequence_diagram {
        let from = svg.len();
        draw_sequence_lifelines(
            graph,
            result,
            styles,
            &mut svg,
            dx,
            dy,
            palette,
            connection_mask_id,
            None,
            sketch && raster_text_geometry_precision,
            sketch && !raster_text_geometry_precision,
            raster_sources.as_deref_mut(),
        );
        capture_object_paint(&mut svg, from, 1, None, &mut object_paints);
    }
    let default_marker_stroke = palette.color("B1", palette.base.b1);
    let mut dynamic_markers = BTreeMap::<String, String>::new();
    let mut emitted_dynamic_markers = BTreeSet::<String>::new();
    let mut sketch_streak_categories = BTreeSet::new();
    // d2svg.Render accumulates shape appendix items separately, then paints
    // them after every diagram object and before the legend.
    let mut appendix_items = String::new();
    for node_id in render_node_ids(graph) {
        let Some(node) = graph.node(node_id) else {
            continue;
        };
        let Some(rect) = result.boxes.get(&node_id).copied() else {
            continue;
        };
        let object_from = svg.len();
        let sequence_span = is_sequence_span_like(graph, node_id);
        let target_shape = target_shape_for_node(graph, node_id).unwrap_or(node.shape);
        // d2target.Shape stores node position and dimensions as integers. The
        // layout graph may keep fractional values (for example proportional
        // grid growth), but the v0.9.0 SVG target rounds those box fields at
        // the render boundary. Keep the shared layout result precise while
        // matching the target's emitted geometry.
        let rect = rounded_render_rect(translated(rect, dx, dy));
        // The TALA result may rescale the font (placement/stages.go:72–99).
        // Source patches that live font into the object before SVG export.
        let live_font_size = result
            .node_labels
            .get(&node_id)
            .and_then(|label| label.font_size)
            .or(node.font_size);
        let label_position = if graph.is_sequence_diagram(node_id) {
            // Export copies the live position. Completed sequence roots
            // commit top-center; empty declarations remain ordinary leaves.
            result
                .node_labels
                .get(&node_id)
                .map(|state| state.position)
                .filter(|position| *position != LabelPosition::Unset)
                .unwrap_or_else(|| {
                    if graph
                        .nodes()
                        .any(|(_, child)| child.parent == Some(node_id))
                    {
                        LabelPosition::InsideTopCenter
                    } else {
                        node.label_position
                    }
                })
        } else if is_sequence_group_like(graph, node_id) {
            // Export copies Object.LabelPosition. Actual sequence placement
            // commits top-left; extracted grid containers retain their live
            // core/grid position even when the group predicate is true.
            result
                .node_labels
                .get(&node_id)
                .map(|state| state.position)
                .filter(|position| *position != LabelPosition::Unset)
                .or((node.label_position != LabelPosition::Unset).then_some(node.label_position))
                .unwrap_or(LabelPosition::InsideTopLeft)
        } else {
            result
                .node_labels
                .get(&node_id)
                .map(|state| state.position)
                .filter(|position| *position != LabelPosition::Unset)
                .unwrap_or(node.label_position)
        };
        let base_style = styles.get(&node_id).cloned().unwrap_or_default();
        let mut style = if target.is_some() {
            base_style.clone()
        } else {
            themed_node_style(graph, node_id, &base_style, palette)
        };
        // buildObjectPatch writes TALA's committed base font into Style.
        // Structured painters otherwise treat the live default as a header
        // size and subtract HeaderFontAdd a second time.
        if let Some(font) = result
            .node_labels
            .get(&node_id)
            .filter(|state| state.font_size_committed)
            .and_then(|state| state.font_size)
        {
            style.font_size = Some(font);
        }
        if let Some(icon) = style.icon.as_deref() {
            let resolved = resolve_asset_href(icon, &options.asset_data);
            if resolved != icon {
                style.icon = Some(resolved);
            }
        }
        // d2svg.drawShape and d2scenebuild retain opacity on the complete
        // shape object, including its label and icon. Primitive opacity
        // would apply it separately to overlapping paints and leave text
        // opaque. Remove it from geometry styles once the owner carries it.
        let node_opacity = style.opacity.take();
        let linked = style.link.as_deref().filter(|value| !value.is_empty());
        if let Some(link) = linked {
            let _ = writeln!(
                svg,
                "<a href=\"{}\" xlink:href=\"{}\">",
                escape(link),
                escape(link)
            );
            if let Some(tooltip) = style.tooltip.as_deref().filter(|value| !value.is_empty()) {
                let tooltip = plain_text(tooltip.as_bytes());
                let _ = writeln!(svg, "<title>{}</title>", escape(&tooltip));
            }
        }
        let mut opacity_attribute = node_opacity
            .as_deref()
            .map(|value| format!(" opacity=\"{}\"", escape(value)))
            .unwrap_or_default();
        // d2scenebuild finishShape: an animated shape's CSS filter makes the
        // complete object an isolated group even at its transparent t=0
        // shadows, so d2raster composites it through an offscreen layer.
        if raster_sources.is_some() && parse_hash_bool(style.animated.as_deref()) {
            opacity_attribute.push_str(" style=\"isolation:isolate\"");
        }
        let _ = writeln!(
            svg,
            "<g class=\"{}\"{opacity_attribute}>",
            svg_object_classes(&node.external_id, graph.target_node_classes.get(&node_id))
        );
        // d2exporter marks sequence-diagram groups with a target-level
        // `shape blend` wrapper around geometry only. Icons and labels remain
        // outside this wrapper, under the complete object's opacity owner.
        let sequence_group = target.map_or_else(
            || is_sequence_group_like(graph, node_id),
            |t| t.blend(node_id),
        );
        if sequence_group {
            svg.push_str("<g class=\"shape blend\">\n");
        }
        let native_dash = if raster_sources.is_some() {
            stroke_dash_values(
                style.stroke_width.as_deref().unwrap_or("1"),
                style.stroke_dash.as_deref(),
            )
            .map(|(dash_size, gap_size)| {
                format!(
                    " data-d2-rs-dash-owner-id=\"{diagram_hash}-native-raster-shape-{}\"{}",
                    node_id.0,
                    raster_dash_attributes(dash_size, gap_size),
                )
            })
            .unwrap_or_default()
        } else {
            String::new()
        };
        let _ = writeln!(svg, "<g class=\"shape\"{native_dash}>");
        let mut rough_geometry = false;
        if target_shape == ShapeKind::Class {
            shapes::draw_class_shape_with_sketch(
                &mut svg,
                rect,
                &style,
                labels.get(&node_id).map_or("", String::as_str),
                live_font_size.unwrap_or(24),
                node.label_size.unwrap_or_default(),
                palette,
                raster_text_geometry_precision
                    .then(|| format!("{diagram_hash}-native-class-{}", node_id.0))
                    .as_deref(),
                sketch.then_some(&mut sketch_streak_categories),
            );
            rough_geometry = sketch;
        } else if target_shape == ShapeKind::SqlTable {
            draw_sql_table_shape_with_sketch(
                &mut svg,
                rect,
                &style,
                labels.get(&node_id).map_or("", String::as_str),
                live_font_size.unwrap_or(24),
                node.label_size.unwrap_or_default(),
                palette,
                raster_text_geometry_precision
                    .then(|| format!("{diagram_hash}-native-sql-{}", node_id.0))
                    .as_deref(),
                sketch.then_some(&mut sketch_streak_categories),
            );
            rough_geometry = sketch;
        } else if target_shape != ShapeKind::Code {
            let native_shadow_id = (raster_text_geometry_precision
                && style_flag(Some(&style), |value| value.shadow.as_ref())
                && target_shape != ShapeKind::Text)
                .then(|| format!("d2-rs-native-shape-shadow-{diagram_hash}-{}", node_id.0));
            let mut geometry_style = style.clone();
            if let Some(id) = native_shadow_id.as_deref() {
                // Source appendShapeGeometry filters geometry and pattern
                // paints together, keeping sibling labels/icons outside.
                geometry_style.shadow = None;
                let _ = writeln!(
                    svg,
                    "<g id=\"{}\" data-d2-rs-shape-shadow=\"sharp\" filter=\"url(#d2-rs-shadow-filter)\">",
                    escape(id)
                );
            }
            rough_geometry = sketch
                && sketch_shapes::draw_rectangle(
                    &mut svg,
                    target_shape,
                    rect,
                    &geometry_style,
                    palette,
                    labels
                        .get(&node_id)
                        .filter(|label| !label.is_empty())
                        .and_then(|_| {
                            sketch_shapes::is_border(label_position)
                                .then_some(connection_mask_id)
                                .flatten()
                        }),
                    &mut sketch_streak_categories,
                    raster_text_geometry_precision,
                );
            if sketch && !rough_geometry {
                rough_geometry = sketch_shapes::draw_typed(
                    &mut svg,
                    target_shape,
                    rect,
                    &geometry_style,
                    palette,
                    labels
                        .get(&node_id)
                        .filter(|label| !label.is_empty())
                        .and_then(|_| {
                            sketch_shapes::is_border(label_position)
                                .then_some(connection_mask_id)
                                .flatten()
                        }),
                    &mut sketch_streak_categories,
                    raster_text_geometry_precision,
                );
            }
            if !rough_geometry {
                draw_shape_with_pattern_overlay(
                    &mut svg,
                    target_shape,
                    rect,
                    &geometry_style,
                    palette,
                    &if raster_text_geometry_precision {
                        format!("{diagram_hash}-native-raster-shape-{}", node_id.0)
                    } else {
                        node.external_id.clone()
                    },
                    raster_text_geometry_precision,
                    target.is_some(),
                );
            }
            if native_shadow_id.is_some() {
                svg.push_str("</g>\n");
            }
        }
        svg.push_str("</g>\n");
        if sketch && !rough_geometry && sketch_shape_supported(target_shape) {
            draw_sketch_overlay(&mut svg, target_shape, rect, &style, palette);
        }
        if sequence_group {
            svg.push_str("</g>\n");
        }
        if target_shape != ShapeKind::Image
            && let Some(icon) = style.icon.as_deref().filter(|value| !value.is_empty())
        {
            draw_icon(
                &mut svg,
                rect,
                target_shape,
                icon,
                result
                    .node_labels
                    .get(&node_id)
                    .and_then(|state| state.icon_position)
                    .or(node.icon_position)
                    .unwrap_or(LabelPosition::InsideMiddleCenter),
                style
                    .icon_border_radius
                    .as_deref()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0) as f64,
                &format!("{diagram_hash}-node-icon-{}", node_id.0),
            );
        }
        if !sequence_span
            && !(raster_text_geometry_precision
                && node_opacity.as_deref().and_then(d2_rs_core::parse_d2_float) == Some(0.0))
            && !matches!(target_shape, ShapeKind::Class | ShapeKind::SqlTable)
            && let Some(label) = labels.get(&node_id)
        {
            let label = theme_label(label, Some(&style), target_shape, palette);
            let markdown = style.markdown;
            // MText.GetColor uses N2 for italic metadata even when the
            // ordinary SVG face selector gives bold precedence over italic.
            let default_text_code = if style.italic { "N2" } else { "N1" };
            let default_text_fill = palette.color(
                default_text_code,
                if style.italic {
                    palette.base.neutrals.n2
                } else {
                    palette.base.neutrals.n1
                },
            );
            let fill = style
                .font_color
                .as_deref()
                .map(|value| themed_color(Some(value), &default_text_fill, palette))
                .unwrap_or(default_text_fill);
            // A gradient font color paints the label through its definition.
            let fill = gradient_definition(&fill)
                .map(|_| format!("url(#{})", gradient_id(&fill)))
                .unwrap_or(fill);
            let text_class = palette
                .semantic_code(style.font_color.as_deref(), default_text_code)
                .map(|code| format!("d2-rs-text-{code}"));
            // Original exporter.applyStyles forces an unauthored text fill
            // to transparent after theme selection. Shape painting already
            // suppresses text geometry; Markdown now needs that target fill.
            let markdown_background =
                if target_shape == ShapeKind::Text && base_style.fill.is_none() {
                    "transparent".to_owned()
                } else {
                    themed_color(style.fill.as_deref(), "", palette)
                };
            let markdown_background_class = palette
                .semantic_code(style.fill.as_deref(), "B6")
                .map(|code| format!("d2-rs-fill-{code}"));
            let plain_label_background = if let Some(context) = target {
                context.label_fill(node_id).map(|value| {
                    (
                        themed_color(Some(value), "", palette),
                        palette
                            .semantic_code(Some(value), "N7")
                            .map(|code| format!("d2-rs-fill-{code}")),
                    )
                })
            } else if sequence_group {
                Some((
                    markdown_background.clone(),
                    markdown_background_class.clone(),
                ))
            } else {
                None
            };
            let language = style.language.as_deref();
            let text_style = TextStyle {
                bold: style.bold,
                italic: style.italic,
                underline: style.underline,
                mono: target_shape == ShapeKind::Code
                    || target_shape == ShapeKind::Class
                    || style
                        .font
                        .as_deref()
                        .is_some_and(|font| font.eq_ignore_ascii_case("mono")),
            };
            // Native scenes append buildCodeShape for every nonempty other-
            // language shape label, after ordinary geometry. Public SVG's
            // Chroma label placement is a separate source contract.
            let code_rendered = language
                .is_some_and(|value| !value.is_empty() && !matches!(value, "latex" | "markdown"))
                && if raster_text_geometry_precision {
                    code_shape::draw_cached(
                        &mut svg,
                        &label,
                        language.unwrap_or_default(),
                        rect,
                        live_font_size.unwrap_or(16),
                        &style,
                        palette,
                        options
                            .theme_id
                            .and_then(theme::find)
                            .is_some_and(|theme| theme.dark),
                        true,
                        code_streams.and_then(|streams| streams.nodes.get(&node_id)),
                        &format!("{diagram_hash}-node-{}", node_id.0),
                    )
                } else {
                    let code_box = if label_position.is_outside()
                        || sketch_shapes::is_border(label_position)
                    {
                        modifier_adjusted_label_rect(
                            rect,
                            target_shape,
                            label_position,
                            style_flag(Some(&style), |style| style.three_dee.as_ref()),
                            style_flag(Some(&style), |style| style.multiple.as_ref()),
                        )
                    } else {
                        let label_size = result
                            .node_labels
                            .get(&node_id)
                            .and_then(|state| state.size)
                            .or(node.label_size)
                            .unwrap_or_default();
                        node_inner_box(rect, target_shape, cloud_content_aspect(node, label_size))
                    };
                    code_shape::draw_public(
                        &mut svg,
                        &label,
                        language.unwrap_or_default(),
                        code_box.origin,
                        Some((rect.size, &style, palette)),
                        live_font_size.unwrap_or(16),
                    )
                };
            if code_rendered {
                // Complete code background/token runs were emitted above.
            } else if language == Some("latex")
                && raster_text_geometry_precision
                && node_opacity.as_deref().and_then(d2_rs_core::parse_d2_float) == Some(0.0)
            {
                // Original assets.go and build.go omit invisible math labels.
            } else if language == Some("latex") {
                let label_size = result
                    .node_labels
                    .get(&node_id)
                    .and_then(|state| state.size)
                    .or(node.label_size)
                    .unwrap_or_default();
                // Source LaTeX uses the inner box for border positions and
                // ignores 3D/multiple expansion.
                let label_box = latex_shape_label_box(
                    rect,
                    target_shape,
                    label_position,
                    label_size,
                    cloud_content_aspect(node, label_size),
                );
                let default_stroke = default_node_stroke(&base_style, target_shape);
                let math_fill = themed_color(
                    style.stroke.as_deref(),
                    &palette.color(default_stroke, palette.base.neutrals.n1),
                    palette,
                );
                let math_class = palette
                    .semantic_code(style.stroke.as_deref(), default_stroke)
                    .map(|code| format!("d2-rs-text-{code}"));
                draw_node_text_box(
                    &mut svg,
                    &label,
                    label_box,
                    live_font_size.unwrap_or(16),
                    &math_fill,
                    language,
                    text_style,
                    math_class.as_deref(),
                    raster_text_geometry_precision
                        .then(|| format!("{diagram_hash}-native-raster-math-{}", node_id.0))
                        .as_deref(),
                );
            } else if let Some(external) = node
                .external_label
                .filter(|_| label_position.is_outside())
                .map(|mut external| {
                    // Source renders the exported post-layout dimensions,
                    // including TALA font-size changes; retain placement
                    // provenance without retaining the old text size.
                    external.size = result
                        .node_labels
                        .get(&node_id)
                        .and_then(|state| state.size)
                        .or(node.label_size)
                        .unwrap_or(external.size);
                    external.at_position(label_position)
                })
            {
                let label_anchor_rect = modifier_adjusted_label_rect(
                    rect,
                    target_shape,
                    label_position,
                    style_flag(Some(&style), |style| style.three_dee.as_ref()),
                    style_flag(Some(&style), |style| style.multiple.as_ref()),
                );
                // markdownShapeLabelTopLeft places an outside Markdown label
                // with GetPointOnBox(label.PADDING) like any outside label.
                let label_rect = Rect {
                    origin: external.top_left(label_anchor_rect, 5.0),
                    size: external.size,
                };
                if markdown {
                    draw_markdown_label(
                        &mut svg,
                        &label,
                        label_rect,
                        live_font_size.unwrap_or(16),
                        LabelPosition::InsideMiddleCenter,
                        &fill,
                        Some(&markdown_background),
                        markdown_background_class.as_deref(),
                        language,
                        text_style,
                        text_class.as_deref(),
                        raster_text_geometry_precision,
                        markdown_family,
                    );
                } else {
                    if language.is_none_or(|value| value.is_empty()) {
                        if let Some((fill, class)) = &plain_label_background {
                            draw_sequence_label_background(
                                &mut svg,
                                label_rect,
                                fill,
                                class.as_deref(),
                            );
                        }
                    }
                    let fill = raster_label_gradient_fill(
                        &mut svg,
                        &fill,
                        style.font_color.as_deref(),
                        &label,
                        label_rect,
                        raster_text_geometry_precision,
                        &format!("{diagram_hash}-node-label-{}", node_id.0),
                    );
                    draw_node_text_box(
                        &mut svg,
                        &label,
                        label_rect,
                        live_font_size.unwrap_or(16),
                        &fill,
                        language,
                        text_style,
                        text_class.as_deref(),
                        raster_text_geometry_precision
                            .then(|| {
                                format!("{diagram_hash}-native-raster-node-label-{}", node_id.0)
                            })
                            .as_deref(),
                    );
                }
            } else {
                let label_size = result
                    .node_labels
                    .get(&node_id)
                    .and_then(|state| state.size)
                    .or(node.label_size)
                    .unwrap_or(Size {
                        width: rect.size.width,
                        height: rect.size.height,
                    });
                // d2scenebuild markdownShapeLabelTopLeft reads the exported
                // target LabelPosition and coerces only an unset one to
                // inside-middle-center (a person's markdown label stays
                // outside-bottom).
                let rendered_position = if markdown && label_position == LabelPosition::Unset {
                    LabelPosition::InsideMiddleCenter
                } else {
                    label_position
                };
                let cloud_aspect = if sequence_span {
                    None
                } else {
                    cloud_content_aspect(node, label_size)
                };
                let label_box = node_label_box(
                    modifier_adjusted_label_rect(
                        rect,
                        target_shape,
                        rendered_position,
                        style_flag(Some(&style), |style| style.three_dee.as_ref()),
                        style_flag(Some(&style), |style| style.multiple.as_ref()),
                    ),
                    target_shape,
                    rendered_position,
                    label_size,
                    cloud_aspect,
                );
                if markdown {
                    draw_markdown_label(
                        &mut svg,
                        &label,
                        label_box,
                        live_font_size.unwrap_or(16),
                        LabelPosition::InsideMiddleCenter,
                        &fill,
                        Some(&markdown_background),
                        markdown_background_class.as_deref(),
                        language,
                        text_style,
                        text_class.as_deref(),
                        raster_text_geometry_precision,
                        markdown_family,
                    );
                } else {
                    if language.is_none_or(|value| value.is_empty()) {
                        if let Some((fill, class)) = &plain_label_background {
                            draw_sequence_label_background(
                                &mut svg,
                                label_box,
                                fill,
                                class.as_deref(),
                            );
                        }
                    }
                    let fill = raster_label_gradient_fill(
                        &mut svg,
                        &fill,
                        style.font_color.as_deref(),
                        &label,
                        label_box,
                        raster_text_geometry_precision,
                        &format!("{diagram_hash}-node-label-{}", node_id.0),
                    );
                    draw_node_text_box(
                        &mut svg,
                        &label,
                        label_box,
                        live_font_size.unwrap_or(16),
                        &fill,
                        language,
                        text_style,
                        text_class.as_deref(),
                        raster_text_geometry_precision
                            .then(|| {
                                format!("{diagram_hash}-native-raster-node-label-{}", node_id.0)
                            })
                            .as_deref(),
                    );
                }
            }
        }
        svg.push_str("</g>\n");
        if linked.is_some() {
            svg.push_str("</a>\n");
        }
        draw_link_tooltip_badges(
            &mut appendix_items,
            rect,
            target_shape,
            &style,
            palette,
            markdown_family,
            raster_text_geometry_precision,
        );
        // Nested sequence scopes paint their white panel before their
        // synthetic lifelines. Emit those lines immediately after the panel
        // and before the parent-first loop reaches any actor boxes.
        capture_object_paint(
            &mut svg,
            object_from,
            target.map_or_else(
                || i64::from(sequence_shape_z_index(graph, node_id)),
                |t| t.shape_order(node_id).0,
            ),
            Some(target.map_or_else(
                || node_level(graph, node_id) as i64,
                |t| t.shape_order(node_id).1,
            )),
            &mut object_paints,
        );
        if graph.is_sequence_diagram(node_id) {
            let from = svg.len();
            draw_sequence_lifelines(
                graph,
                result,
                styles,
                &mut svg,
                dx,
                dy,
                palette,
                connection_mask_id,
                Some(node_id),
                sketch && raster_text_geometry_precision,
                sketch && !raster_text_geometry_precision,
                raster_sources.as_deref_mut(),
            );
            capture_object_paint(&mut svg, from, 1, None, &mut object_paints);
        }
    }
    for (edge_id, edge) in graph.edges() {
        let Some(route) = result.routes.get(&edge_id) else {
            continue;
        };
        if route.is_empty() {
            continue;
        }
        let object_from = svg.len();
        let mut edge_style = themed_edge_style(graph.edge_style(edge_id), palette);
        // Source connection.go keeps opacity on the complete owner: icon,
        // masked route/arrow geometry and labels all share one paint layer.
        let owner_opacity = (raster_sources.is_some() || sketch)
            .then(|| edge_style.opacity.take())
            .flatten();
        let owner_opacity_attribute = owner_opacity
            .as_deref()
            .map(|value| format!(" opacity=\"{}\"", escape(value)))
            .unwrap_or_default();
        let edge_group_open = if let Some(edge_identity) = target
            .map(|t| t.edge_id(edge_id).to_owned())
            .or_else(|| edge_svg_id(graph, edge_id))
        {
            let _ = writeln!(
                svg,
                "<g class=\"{}\"{owner_opacity_attribute}>",
                svg_object_classes(&edge_identity, graph.target_edge_classes.get(&edge_id))
            );
            true
        } else if owner_opacity.is_some() {
            let _ = writeln!(svg, "<g{owner_opacity_attribute}>");
            true
        } else {
            false
        };
        let points = route
            .iter()
            .copied()
            .map(|p| rendered_route_point(graph, p))
            .map(|point| Point {
                x: point.x + dx,
                y: point.y + dy,
            })
            .collect::<Vec<_>>();
        let arrows = graph.edge_arrows(edge_id);
        let arrowheads = graph.edge_arrowheads(edge_id);
        let edge_stroke_width = stroke_width(edge_style.stroke_width.as_deref());
        let source_stroke_width = if graph.is_sequence_diagram(edge.source) {
            0.0
        } else {
            styles
                .get(&edge.source)
                .and_then(|style| style.stroke_width.as_deref())
                .map_or(2.0, |value| stroke_width(Some(value)))
        };
        let target_stroke_width = if graph.is_sequence_diagram(edge.target) {
            0.0
        } else {
            styles
                .get(&edge.target)
                .and_then(|style| style.stroke_width.as_deref())
                .map_or(2.0, |value| stroke_width(Some(value)))
        };
        // An explicit `shape: none` keeps the syntactic arrow direction in
        // the graph, but D2 does not reserve marker space or shorten the
        // visible route for it.
        let source_marker_enabled =
            arrows.source && canonical_marker_name(arrowheads.source.as_deref()).is_some();
        let target_marker_enabled =
            arrows.target && canonical_marker_name(arrowheads.target.as_deref()).is_some();
        let source_adjustment = if points.len() >= 2 {
            arrowhead_adjustment(
                points[1],
                points[0],
                source_marker_enabled,
                edge_stroke_width,
                source_stroke_width,
            )
        } else {
            Point { x: 0.0, y: 0.0 }
        };
        let target_adjustment = if points.len() >= 2 {
            arrowhead_adjustment(
                points[points.len() - 2],
                points[points.len() - 1],
                target_marker_enabled,
                edge_stroke_width,
                target_stroke_width,
            )
        } else {
            Point { x: 0.0, y: 0.0 }
        };
        let stroke = themed_color(
            edge_style.stroke.as_deref(),
            &default_marker_stroke,
            palette,
        );
        let hash_stroke = edge_style.stroke.as_deref().unwrap_or("B1");
        let source_marker_id = arrows
            .source
            .then(|| {
                marker_reference(
                    arrowheads.source.as_deref(),
                    false,
                    edge_stroke_width,
                    &stroke,
                    hash_stroke,
                    &default_marker_stroke,
                    Some(&diagram_hash),
                    marker_theme,
                    &mut dynamic_markers,
                )
            })
            .flatten();
        let target_marker_id = arrows
            .target
            .then(|| {
                marker_reference(
                    arrowheads.target.as_deref(),
                    true,
                    edge_stroke_width,
                    &stroke,
                    hash_stroke,
                    &default_marker_stroke,
                    Some(&diagram_hash),
                    marker_theme,
                    &mut dynamic_markers,
                )
            })
            .flatten();
        // v0.9.0 writes a newly encountered marker definition inside the
        // connection group, immediately before its path.  Keeping the
        // definition adjacent to its first use matters for source-shaped SVG
        // inspection and also avoids making marker order depend on a global
        // definition pass.  The set keeps shared marker definitions emitted
        // only once when several connections use the same arrowhead tuple.
        for marker_id in [source_marker_id.as_ref(), target_marker_id.as_ref()]
            .into_iter()
            .flatten()
        {
            if emitted_dynamic_markers.insert(marker_id.clone())
                && let Some(definition) = dynamic_markers.get(marker_id)
            {
                svg.push_str(definition);
            }
        }
        // d2exporter toConnection: NoCornerRadius themes export radius 0
        // unless the edge sets one.
        let border_radius = edge_style
            .border_radius
            .as_deref()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(if target_theme_rules(graph).no_corner_radius {
                0.0
            } else {
                10.0
            });
        let mut path_data = route_path_data_with_radius(
            &points,
            source_adjustment,
            target_adjustment,
            border_radius,
            result.route_is_curve(graph, edge_id),
        );
        let native_sketch = sketch && raster_sources.is_some();
        let svg_sketch = sketch && raster_sources.is_none();
        let animated_sketch = edge_style
            .animated
            .as_deref()
            .is_some_and(|v| d2_rs_core::parse_d2_bool(v) == Some(true));
        let rough_commands = if native_sketch && !animated_sketch {
            route_path_source_commands(
                &points,
                source_adjustment,
                target_adjustment,
                border_radius,
                result.route_is_curve(graph, edge_id),
            )
            .and_then(|source| sketch_connection::route(&source))
        } else {
            None
        };
        if let Some(commands) = &rough_commands {
            path_data = raster_path_data(commands);
        }
        if svg_sketch && !animated_sketch {
            path_data = sketch_svg::route(&path_data);
        }
        let raster_path_id = if raster_sources.is_some() {
            // Identity is independent of source-command availability and is
            // also required for exact stroke operands on straight routes.
            let id = format!("{diagram_hash}-native-raster-edge-{}", edge_id.0);
            if let Some(commands) = rough_commands.clone().or_else(|| {
                route_path_source_commands(
                    &points,
                    source_adjustment,
                    target_adjustment,
                    border_radius,
                    result.route_is_curve(graph, edge_id),
                )
            }) && let Some(sources) = raster_sources.as_deref_mut()
            {
                sources.push(RasterPathSource {
                    id: id.clone(),
                    commands,
                    transform: None,
                });
            }
            format!(" id=\"{}\"", escape(&id))
        } else {
            String::new()
        };
        let inline_source_arrow = if raster_sources.is_some() && arrows.source && route.len() >= 2 {
            (if native_sketch {
                sketch_connection::arrow(
                    arrowheads.source.as_deref(),
                    format!(
                        "{diagram_hash}-native-raster-arrow-rough-source-{}",
                        edge_id.0
                    ),
                    &points,
                    false,
                    source_adjustment,
                    edge_stroke_width,
                    &stroke,
                    &palette.color("N7", "#FFFFFF"),
                )
            } else {
                None
            })
            .or_else(|| {
                raster_inline_arrowhead(
                    arrowheads.source.as_deref(),
                    format!("{diagram_hash}-native-raster-arrow-src-{}", edge_id.0),
                    // Marker endpoint geometry and its tangent both use the D2
                    // target-route representation. The raw engine route can still
                    // contain the pre-quantized thirds that d2target truncates.
                    &points,
                    &points,
                    false,
                    source_adjustment,
                    dx,
                    dy,
                    edge_stroke_width,
                    &stroke,
                    &palette.color("N7", "#FFFFFF"),
                )
            })
        } else {
            None
        };
        let inline_target_arrow = if raster_sources.is_some() && arrows.target && route.len() >= 2 {
            (if native_sketch {
                sketch_connection::arrow(
                    arrowheads.target.as_deref(),
                    format!(
                        "{diagram_hash}-native-raster-arrow-rough-target-{}",
                        edge_id.0
                    ),
                    &points,
                    true,
                    target_adjustment,
                    edge_stroke_width,
                    &stroke,
                    &palette.color("N7", "#FFFFFF"),
                )
            } else {
                None
            })
            .or_else(|| {
                raster_inline_arrowhead(
                    arrowheads.target.as_deref(),
                    format!("{diagram_hash}-native-raster-arrow-dst-{}", edge_id.0),
                    &points,
                    &points,
                    true,
                    target_adjustment,
                    dx,
                    dy,
                    edge_stroke_width,
                    &stroke,
                    &palette.color("N7", "#FFFFFF"),
                )
            })
        } else {
            None
        };
        if let Some(sources) = raster_sources.as_deref_mut() {
            for arrow in [inline_source_arrow.as_ref(), inline_target_arrow.as_ref()]
                .into_iter()
                .flatten()
            {
                for path in &arrow.paths {
                    sources.push(RasterPathSource {
                        id: path.id.clone(),
                        commands: path.commands.clone(),
                        transform: path.source_transform,
                    });
                    #[cfg(feature = "diagnostic-traces")]
                    if std::env::var_os("D2_RS_TRACE_RASTER_PIXEL").is_some() {
                        eprintln!(
                            "d2-rs-render-inline-arrow id={} commands={:?}",
                            path.id, path.commands
                        );
                    }
                }
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if std::env::var_os("D2_RS_TRACE_RASTER_PIXEL").is_some() {
            eprintln!(
                "d2-rs-render-route edge={} curve={} border_radius={:.17e} source_adjustment=({:.17e},{:.17e}) target_adjustment=({:.17e},{:.17e})",
                edge_id.0,
                result.route_is_curve(graph, edge_id),
                border_radius,
                source_adjustment.x,
                source_adjustment.y,
                target_adjustment.x,
                target_adjustment.y,
            );
            for (index, (raw, target)) in route.iter().zip(&points).enumerate() {
                eprintln!(
                    "d2-rs-render-route-point edge={} index={} raw=({:.17e},{:.17e}) target=({:.17e},{:.17e}) svg=({:.17e},{:.17e})",
                    edge_id.0,
                    index,
                    raw.x,
                    raw.y,
                    rendered_route_point(graph, *raw).x,
                    rendered_route_point(graph, *raw).y,
                    target.x,
                    target.y,
                );
            }
            eprintln!("d2-rs-render-route-data edge={} {}", edge_id.0, path_data);
        }
        let start = source_marker_id
            .filter(|_| !svg_sketch && arrows.source && inline_source_arrow.is_none())
            .map(|id| format!(" marker-start=\"url(#{id})\""))
            .unwrap_or_default();
        let end = target_marker_id
            .filter(|_| !svg_sketch && arrows.target && inline_target_arrow.is_none())
            .map(|id| format!(" marker-end=\"url(#{id})\""))
            .unwrap_or_default();
        let stroke_class = palette
            .semantic_code(edge_style.stroke.as_deref(), "B1")
            .map(|code| format!("connection d2-rs-stroke-{code}"))
            .unwrap_or_else(|| "connection".to_owned());
        let stroke_class = if edge_style
            .animated
            .as_deref()
            .is_some_and(|value| matches!(value, "true" | "TRUE" | "True"))
        {
            format!(" class=\"{stroke_class} animated-connection\"")
        } else {
            format!(" class=\"{stroke_class}\"")
        };
        let stroke_width = edge_style.stroke_width.as_deref().unwrap_or("2");
        let opacity = edge_style
            .opacity
            .as_deref()
            .map(|value| format!(" opacity=\"{}\"", escape(value)))
            .unwrap_or_default();
        let effective_dash = edge_style.stroke_dash.as_deref().or_else(|| {
            edge_style
                .animated
                .as_deref()
                .filter(|value| d2_rs_core::parse_d2_bool(value) == Some(true))
                .map(|_| "5")
        });
        let dash = effective_dash
            .map(|value| {
                let value =
                    stroke_dasharray(stroke_width, Some(value)).unwrap_or_else(|| value.to_owned());
                format!(" stroke-dasharray=\"{}\"", escape(&value))
            })
            .unwrap_or_default();
        let animation = edge_style
            .animated
            .as_deref()
            .filter(|value| matches!(*value, "true" | "TRUE" | "True"))
            .and_then(|_| {
                stroke_dash_values(stroke_width, effective_dash).map(|(dash_size, gap_size)| {
                    let direction = if arrows.source && !arrows.target {
                        10.0
                    } else {
                        -10.0
                    };
                    format!(
                        " stroke-dashoffset=\"{}\" animation=\"dashdraw {}s linear infinite\"",
                        number(direction * (dash_size + gap_size)),
                        number(gap_size * 0.5),
                    )
                })
            })
            .unwrap_or_default();
        // The Go scene keeps ordinary dash operands before SVG's six-decimal
        // serialization. Animated paths retain their existing timestamp-aware
        // transport in the exporter instead of receiving a static offset.
        let native_dash = if raster_sources.is_some() && animation.is_empty() {
            stroke_dash_values(stroke_width, effective_dash)
                .map(|(dash_size, gap_size)| raster_dash_attributes(dash_size, gap_size))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let linked = graph.edge_link(edge_id).filter(|value| !value.is_empty());
        let mask = connection_mask_id
            .map(|id| format!(" mask=\"url(#{id})\""))
            .unwrap_or_default();
        if let Some(icon_rect) = edge_icon_rect(graph, result, edge_id, route, dx, dy)
            && let Some(icon) = graph.edge_icon(edge_id)
        {
            let href = resolve_asset_href(icon, &options.asset_data);
            let radius = graph.edge_icon_border_radius(edge_id);
            let clip = image_radius_clip(
                &mut svg,
                icon_rect,
                radius,
                false,
                &format!("{diagram_hash}-edge-icon-{}", edge_id.0),
            );
            let _ = writeln!(
                svg,
                "<image href=\"{}\" xlink:href=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"xMidYMid meet\"{} />",
                escape(&href),
                escape(&href),
                number(icon_rect.origin.x),
                number(icon_rect.origin.y),
                number(icon_rect.size.width),
                number(icon_rect.size.height),
                clip,
            );
        }
        // D2's retained scene applies the connection mask once to the
        // combined route-and-arrow geometry. Keep the normal SVG structure
        // unchanged, but mirror that group boundary for the PNG-only
        // high-precision raster path. Applying the mask separately to each
        // primitive changes antialiasing where arrowheads overlap the route.
        let grouped_geometry_mask = raster_sources.is_some();
        if grouped_geometry_mask {
            let _ = writeln!(svg, "<g{mask}>");
        }
        if svg_sketch && animated_sketch {
            let paths = if source_marker_enabled == target_marker_enabled {
                let (a, b) = sketch_svg::split(&path_data)
                    .expect("D2's animated route uses nonempty M/L/C/S geometry");
                vec![(a, true), (b, false)]
            } else {
                vec![(path_data.clone(), false)]
            };
            let (dash_size, gap_size) =
                stroke_dash_values(stroke_width, effective_dash).unwrap_or((0.0, 0.0));
            let direction = if source_marker_enabled && !target_marker_enabled {
                10.0
            } else {
                -10.0
            };
            for (path, reverse) in paths {
                let _ = writeln!(
                    svg,
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}{}{}{} style=\"stroke-dashoffset:{:.6};animation:dashdraw {:.6}s linear infinite;{}\" />",
                    path,
                    escape(&stroke),
                    escape(stroke_width),
                    opacity,
                    dash,
                    stroke_class,
                    mask,
                    direction * (dash_size + gap_size),
                    gap_size * 0.5,
                    if reverse {
                        "animation-direction:reverse;"
                    } else {
                        ""
                    }
                );
            }
        } else if let Some(((first_data, second_data), raster_split)) = (raster_sources.is_some()
            && !animation.is_empty()
            && source_marker_enabled == target_marker_enabled)
            .then(|| sketch_svg::split(&path_data))
            .flatten()
            .and_then(|halves| {
                split_animated_raster_source(raster_sources.as_deref_mut(), &raster_path_id, &path_data)
                    .map(|ids| (halves, ids))
            })
        {
            // d2scenebuild buildConnection splits an animated connection whose
            // two ends have the same arrow state: the first half animates in
            // reverse (carrying the source marker), the second forward.
            let reverse_animation = animation.replacen("linear infinite\"", "linear infinite reverse\"", 1);
            for (id, data, animation, marker) in [
                (&raster_split.0, &first_data, &reverse_animation, &start),
                (&raster_split.1, &second_data, &animation, &end),
            ] {
                let _ = writeln!(
                    svg,
                    "<path id=\"{}\" d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}{}{}{}{} />",
                    escape(id),
                    data.trim_end(),
                    escape(&stroke),
                    escape(stroke_width),
                    opacity,
                    dash,
                    animation,
                    marker,
                    stroke_class,
                );
            }
        } else {
            let _ = writeln!(
                svg,
                "<path{} d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}{}{}{}{}{}{}{} />",
                raster_path_id,
                path_data,
                escape(&stroke),
                escape(stroke_width),
                opacity,
                dash,
                animation,
                native_dash,
                start,
                end,
                stroke_class,
                if grouped_geometry_mask { "" } else { &mask }
            );
        }
        for arrow in [inline_source_arrow.as_ref(), inline_target_arrow.as_ref()]
            .into_iter()
            .flatten()
        {
            for path in &arrow.paths {
                let arrow_stroke = path
                    .stroke
                    .as_ref()
                    .map(|stroke| {
                        format!(
                            " stroke=\"{}\" stroke-width=\"{}\"",
                            escape(stroke),
                            number(path.stroke_width.unwrap_or(edge_stroke_width)),
                        )
                    })
                    .unwrap_or_default();
                let arrow_style = path
                    .stroke_style
                    .map(|style| format!(" style=\"{style}\""))
                    .unwrap_or_default();
                let _ = writeln!(
                    svg,
                    "<path id=\"{}\" d=\"{}\" fill=\"{}\"{} class=\"connection\"{}{}{} />",
                    escape(&path.id),
                    path.path_data,
                    escape(&path.fill),
                    arrow_stroke,
                    arrow_style,
                    opacity,
                    if grouped_geometry_mask { "" } else { &mask },
                );
            }
        }
        if grouped_geometry_mask {
            svg.push_str("</g>\n");
        }
        if svg_sketch {
            sketch_svg::arrows(
                &mut svg,
                &points,
                if arrows.source {
                    canonical_marker_name(arrowheads.source.as_deref())
                } else {
                    None
                },
                if arrows.target {
                    canonical_marker_name(arrowheads.target.as_deref())
                } else {
                    None
                },
                source_adjustment,
                target_adjustment,
                edge_stroke_width,
                &stroke,
                &palette.color("N7", "#FFFFFF"),
            );
        }
        // TALA may move an unlocked edge label during layout.  Prefer the
        // placement recorded in LayoutResult and retain the graph metadata as
        // a fallback for callers that supply a pre-routed result.
        if let Some(label) =
            effective_edge_label(graph, result, edge_id).filter(|label| !label.text.is_empty())
            && let Some(mut label_rect) = edge_label_bounds(
                route,
                &label,
                edge_stroke_width,
                graph.target_routes_exported,
            )
        {
            label_rect.origin.x += dx;
            label_rect.origin.y += dy;
            let label_fill = if linked.is_some()
                && edge_style
                    .language
                    .as_deref()
                    .is_none_or(|language| language.is_empty())
            {
                // D2 uses the browser's unvisited-link blue for ordinary
                // linked connection labels, regardless of the edge stroke.
                "blue".to_owned()
            } else {
                edge_style
                    .font_color
                    .as_deref()
                    .map(|value| {
                        themed_color(
                            Some(value),
                            &palette.color("N1", palette.base.neutrals.n1),
                            palette,
                        )
                    })
                    // Source chooses the default color from exported italic,
                    // independently of the always-italic measurement face.
                    .unwrap_or_else(|| {
                        if edge_style
                            .italic
                            .as_deref()
                            .and_then(d2_rs_core::parse_d2_bool)
                            .unwrap_or(true)
                        {
                            palette.color("N2", palette.base.neutrals.n2)
                        } else {
                            palette.color("N1", palette.base.neutrals.n1)
                        }
                    })
            };
            let label_class = edge_style
                .font_color
                .as_deref()
                .and_then(|value| palette.semantic_code(Some(value), "N1"))
                .map(|code| format!("d2-rs-text-{code}"));
            // Native scene links are metadata, not an additional paint group.
            let paint_link = linked.filter(|_| {
                !raster_text_geometry_precision
                    || edge_style
                        .language
                        .as_deref()
                        .is_none_or(|language| language.is_empty())
            });
            if let Some(link) = paint_link {
                let _ = writeln!(
                    svg,
                    "<a href=\"{}\" xlink:href=\"{}\"{}>",
                    escape(link),
                    escape(link),
                    if raster_text_geometry_precision
                        && edge_style
                            .language
                            .as_deref()
                            .is_some_and(|language| !language.is_empty())
                    {
                        " style=\"text-decoration:none\""
                    } else {
                        ""
                    }
                );
                if let Some(tooltip) = graph
                    .edge_tooltip(edge_id)
                    .filter(|value| !value.is_empty())
                {
                    let tooltip = plain_text(tooltip.as_bytes());
                    let _ = writeln!(svg, "<title>{}</title>", escape(&tooltip));
                }
            }
            // Original d2svg.go:1326 and d2scenebuild/labels.go:196 retain
            // an authored connection fill behind its ordinary label. This
            // sibling belongs to the label/connection opacity owner, outside
            // the masked route-and-arrow geometry.
            if let Some(fill) = edge_style.fill.as_deref().filter(|fill| {
                !fill.is_empty()
                    && edge_style
                        .language
                        .as_deref()
                        .is_none_or(|language| language.is_empty())
            }) {
                let fill_color = if gradient_definition(fill).is_some() {
                    format!("url(#{})", gradient_id(fill))
                } else {
                    themed_color(Some(fill), "", palette)
                };
                let fill_class = palette
                    .semantic_code(Some(fill), "N7")
                    .map(|code| format!(" class=\"d2-rs-fill-{code}\""))
                    .unwrap_or_default();
                let _ = writeln!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"{}\"{} />",
                    number(label_rect.origin.x - 4.0),
                    number(label_rect.origin.y - 3.0),
                    number(label_rect.size.width + 8.0),
                    number(label_rect.size.height + 6.0),
                    escape(&fill_color),
                    fill_class
                );
            }
            let native_geometry_id = raster_text_geometry_precision
                .then(|| format!("{diagram_hash}-native-raster-edge-label-{}", edge_id.0));
            let code_rendered = edge_style.language.as_deref().is_some_and(|language| {
                !language.is_empty() && !matches!(language, "latex" | "markdown")
            }) && if raster_text_geometry_precision {
                code_shape::draw_runs_cached(
                    &mut svg,
                    &theme_edge_label(&label.text, &edge_style, palette),
                    edge_style.language.as_deref().unwrap_or_default(),
                    label_rect.origin,
                    graph.edge_font_size(edge_id),
                    options
                        .theme_id
                        .and_then(theme::find)
                        .is_some_and(|theme| theme.dark),
                    true,
                    code_streams.and_then(|streams| streams.edges.get(&edge_id)),
                    native_geometry_id
                        .as_deref()
                        .expect("native code edge identity"),
                )
            } else {
                code_shape::draw_public(
                    &mut svg,
                    &theme_edge_label(&label.text, &edge_style, palette),
                    edge_style.language.as_deref().unwrap_or_default(),
                    label_rect.origin,
                    None,
                    graph.edge_font_size(edge_id),
                )
            };
            if edge_style.language.as_deref() == Some("latex") {
                {
                    // Source buildConnectionLabels resolves math even under an opacity-zero owner.
                    draw_node_text_box(
                        &mut svg,
                        &label.text,
                        label_rect,
                        graph.edge_font_size(edge_id),
                        &edge_style
                            .font_color
                            .as_deref()
                            .map(|value| themed_color(Some(value), "", palette))
                            .unwrap_or_else(|| {
                                palette.color(
                                    if edge_style
                                        .italic
                                        .as_deref()
                                        .and_then(d2_rs_core::parse_d2_bool)
                                        .unwrap_or(true)
                                    {
                                        "N2"
                                    } else {
                                        "N1"
                                    },
                                    if edge_style
                                        .italic
                                        .as_deref()
                                        .and_then(d2_rs_core::parse_d2_bool)
                                        .unwrap_or(true)
                                    {
                                        palette.base.neutrals.n2
                                    } else {
                                        palette.base.neutrals.n1
                                    },
                                )
                            }),
                        Some("latex"),
                        TextStyle::default(),
                        None,
                        native_geometry_id.as_deref(),
                    );
                }
            } else if edge_style.language.as_deref() == Some("markdown") {
                let background = edge_style
                    .fill
                    .as_deref()
                    .map(|value| themed_color(Some(value), "", palette));
                draw_markdown_label(
                    &mut svg,
                    &theme_edge_label(&label.text, &edge_style, palette),
                    label_rect,
                    graph.edge_font_size(edge_id),
                    LabelPosition::InsideMiddleCenter,
                    &label_fill,
                    background.as_deref(),
                    None,
                    Some("markdown"),
                    TextStyle {
                        mono: edge_style
                            .font
                            .as_deref()
                            .map_or(palette.base.special_rules.mono, |font| {
                                font.eq_ignore_ascii_case("mono")
                            }),
                        underline: edge_style
                            .underline
                            .as_deref()
                            .and_then(d2_rs_core::parse_d2_bool)
                            == Some(true),
                        ..TextStyle::default()
                    },
                    label_class.as_deref(),
                    raster_text_geometry_precision,
                    markdown_family,
                );
            } else if !code_rendered {
                draw_label(
                    &mut svg,
                    &theme_edge_label(&label.text, &edge_style, palette),
                    label_rect,
                    graph.edge_font_size(edge_id),
                    &label_fill,
                    label_class.as_deref(),
                    edge_style
                        .underline
                        .as_deref()
                        .and_then(|value| d2_rs_core::parse_d2_bool(value))
                        .unwrap_or(false)
                        || linked.is_some(),
                    graph.edge_label_bold(edge_id),
                    edge_style
                        .italic
                        .as_deref()
                        .and_then(d2_rs_core::parse_d2_bool)
                        .unwrap_or(true),
                    edge_style
                        .font
                        .as_deref()
                        .map_or(palette.base.special_rules.mono, |font| {
                            font.eq_ignore_ascii_case("mono")
                        }),
                    native_geometry_id.as_deref(),
                );
            }
            if paint_link.is_some() {
                svg.push_str("</a>\n");
            }
        }
        let arrowhead_labels = graph.edge_arrowhead_labels(edge_id);
        for (is_target, label) in [
            (false, arrowhead_labels.source.as_ref()),
            (true, arrowhead_labels.target.as_ref()),
        ] {
            let Some(label) = label.filter(|label| !label.text.is_empty()) else {
                continue;
            };
            let Some(mut label_rect) = arrowhead_label_bounds(graph, edge_id, route, is_target)
            else {
                continue;
            };
            label_rect.origin.x += dx;
            label_rect.origin.y += dy;
            let default_fill = palette.color("N1", palette.base.neutrals.n1);
            let fill = label
                .color
                .as_deref()
                .map(|color| themed_color(Some(color), &default_fill, palette))
                .unwrap_or(default_fill);
            let label_class = label
                .color
                .as_deref()
                .and_then(|color| palette.semantic_code(Some(color), "N1"))
                .map(|code| format!("d2-rs-text-{code}"));
            let native_geometry_id = raster_text_geometry_precision.then(|| {
                format!(
                    "{diagram_hash}-native-raster-edge-{}-{}-label",
                    edge_id.0,
                    if is_target { "dst" } else { "src" },
                )
            });
            // d2scenebuild arrowhead labels use the diagram's primary italic
            // face ("default" family), which a mono theme makes Source Code Pro.
            draw_label(
                &mut svg,
                &label.text,
                label_rect,
                graph.edge_font_size(edge_id),
                &fill,
                label_class.as_deref(),
                false,
                false,
                true,
                palette.base.special_rules.mono,
                native_geometry_id.as_deref(),
            );
        }
        if edge_group_open {
            svg.push_str("</g>\n");
        }
        capture_object_paint(
            &mut svg,
            object_from,
            target.map_or_else(
                || i64::from(sequence_connection_z_index(graph, edge)),
                |t| t.edge_order(edge_id),
            ),
            None,
            &mut object_paints,
        );
    }
    paint_objects_in_source_order(&mut svg, &mut object_paints);
    svg.push_str(&appendix_items);
    if let Some(legend) = graph.legend.as_ref() {
        draw_legend_with_target(
            &mut svg,
            legend,
            (min_x, min_y, max_x, max_y),
            dx,
            dy,
            palette,
            raster_text_geometry_precision,
            sketch,
            &mut sketch_streak_categories,
            raster_sources.as_deref_mut(),
            target,
        );
    }
    if dynamic_markers
        .keys()
        .any(|marker_id| !emitted_dynamic_markers.contains(marker_id))
    {
        svg.push_str("<defs>");
        for (marker_id, definition) in &dynamic_markers {
            if !emitted_dynamic_markers.contains(marker_id) {
                svg.push_str(definition);
            }
        }
        svg.push_str("</defs>\n");
    }
    svg.push_str("<mask id=\"");
    svg.push_str(&diagram_hash);
    let _ = writeln!(
        svg,
        "\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"white\"/>{}</mask>",
        number(view_left),
        number(view_top),
        number(base_width),
        number(base_height),
        number(view_left),
        number(view_top),
        number(base_width),
        number(base_height),
        diagram_mask_rects.join(""),
    );
    svg.push_str("</svg>\n");
    svg.push_str("</svg>\n");
    if raster_sources.is_some() {
        tag_raster_dashed_primitive_ids(&mut svg);
    }
    if !requested_salt.is_empty() {
        salt_svg_ids(&mut svg, &diagram_hash);
    }
    let mut font_needs = font_needs_from_svg(&svg, font_need_fallback);
    if let Some(target) = target {
        target.restrict_legend_font_corpora(&mut font_needs);
    }
    // Go's scoped regular face also paints appendix text when ordinary text
    // or Markdown caused embedFonts to emit it. Its subset has no hint/kerning
    // instructions; a full fallback face is used only without that trigger.
    let appendix_primary = (!raster_text_geometry_precision
        && (!font_needs.ordinary_corpora[0].is_empty()
            || svg.contains("class=\"md ")
            || svg.contains("class=\"md\"")))
    .then_some(markdown_family.primary_kind());
    let mut css = theme_css_with_primary(
        palette,
        dark_palette,
        font_needs,
        hand_drawn_font,
        mono_primary,
        markdown_family.mono_kind(),
    );
    if target.is_some() && graph.legend.is_some() {
        // Match Go's scoped font-family without a generic sans/mono fallback;
        // missing legacy-corpus glyphs use the browser's default SVG fallback.
        css.push_str(".d2-rs-legend text{font-family:\"Source Sans Pro\"}.d2-rs-legend text.text-mono{font-family:\"Source Code Pro\"}");
    }
    if hand_drawn_font && raster_sources.is_none() {
        // Source SVG picks the concrete Fuzzy Bubbles regular face for its
        // italic family, without requesting browser-synthesized slant.
        css.push_str(".d2-rs-theme.d2-rs-sketch text.text-italic{font-style:normal;}");
    }
    if !raster_text_geometry_precision {
        let visibility = |dark: bool| {
            if dark {
                ".d2-rs-theme .light-code{display:none}.d2-rs-theme .dark-code{display:block}"
            } else {
                ".d2-rs-theme .light-code{display:block}.d2-rs-theme .dark-code{display:none}"
            }
        };
        // Public token spans use source concrete mono faces. Ordinary text
        // selects these weights through attributes; source token classes need
        // the equivalent face selection, with italic winning over bold.
        css.push_str(".d2-rs-theme .light-code .text-mono-bold,.d2-rs-theme .dark-code .text-mono-bold{font-weight:700}.d2-rs-theme .light-code .text-mono-italic,.d2-rs-theme .dark-code .text-mono-italic{font-weight:400;font-style:italic}");
        css.push_str(visibility(
            options
                .theme_id
                .and_then(theme::find)
                .is_some_and(|theme| theme.dark),
        ));
        if let Some(dark_id) = options.dark_theme_id {
            let _ = write!(
                css,
                "@media screen and (prefers-color-scheme:dark){{{}}}",
                visibility(theme::find(dark_id).is_some_and(|theme| theme.dark))
            );
        }
    }
    let css_slot = svg
        .find("<!--D2RS_FONT_CSS-->")
        .expect("font stylesheet placeholder is emitted once");
    svg.replace_range(
        css_slot..css_slot + "<!--D2RS_FONT_CSS-->".len(),
        &format!("<style>{css}</style>"),
    );
    let svg = svg.into_bytes();
    if options.force_appendix {
        append_static_appendix_with_theme_and_bounds(
            &svg,
            graph,
            result,
            styles,
            options.theme_id,
            Some(bounds_with_markdown_family_and_legend(
                graph,
                result,
                labels,
                styles,
                markdown_family,
                false,
            )),
            raster_text_geometry_precision,
            appendix_primary,
        )
    } else {
        svg
    }
}

/// Appends a compact static-export appendix to the nested SVG target.
///
/// D2's browser SVG keeps links and tooltips interactive. PNG/PDF callers can
/// request the v0.9.0 appendix instead; keeping this as a post-processing
/// step means it never participates in layout, hashing, or route generation.
pub fn append_static_appendix(
    svg: &[u8],
    graph: &Graph,
    result: &LayoutResult,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Vec<u8> {
    append_static_appendix_with_theme(svg, graph, result, styles, None)
}

/// Appends the static-export appendix using the selected theme's text family.
pub fn append_static_appendix_with_theme(
    svg: &[u8],
    graph: &Graph,
    result: &LayoutResult,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    theme_id: Option<i64>,
) -> Vec<u8> {
    append_static_appendix_with_theme_and_bounds(
        svg, graph, result, styles, theme_id, None, false, None,
    )
}

fn append_static_appendix_with_theme_and_bounds(
    svg: &[u8],
    graph: &Graph,
    result: &LayoutResult,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    theme_id: Option<i64>,
    diagram_bounds: Option<(f64, f64, f64, f64)>,
    precision: bool,
    svg_primary: Option<BundledFamily>,
) -> Vec<u8> {
    let theme_mono = precision
        && theme_id
            .and_then(theme::palette)
            .is_some_and(|palette| palette.special_rules.mono);
    let mut entries = Vec::new();
    let mut node_badges = BTreeMap::new();
    let mut next_badge = 1;
    // Appendix rows and badge numbers follow Diagram.Shapes definition order.
    // SVG icon replacement separately follows the painter's stable render order.
    for (node_id, _) in graph.nodes() {
        if !result.boxes.contains_key(&node_id) {
            continue;
        }
        let Some(style) = styles.get(&node_id) else {
            continue;
        };
        let mut badges = Vec::new();
        if let Some(tooltip) = style.tooltip.as_deref().filter(|value| !value.is_empty()) {
            entries.push(tooltip.to_owned());
            if style.tooltip_position.as_deref().is_none_or(str::is_empty) {
                badges.push(Some(next_badge));
            }
            next_badge += 1;
        }
        if let Some(link) = style.link.as_deref().filter(|value| !value.is_empty()) {
            let display = style.pretty_link.as_deref().unwrap_or(link);
            if !display.is_empty() {
                entries.push(display.to_owned());
            }
            badges.push(Some(next_badge));
            next_badge += 1;
        }
        node_badges.insert(node_id, badges);
    }
    let badge_numbers = render_node_ids(graph)
        .into_iter()
        .flat_map(|node_id| node_badges.remove(&node_id).unwrap_or_default())
        .collect::<Vec<_>>();
    if precision {
        // Original scene metadata.go retains connection tooltip appendix rows;
        // connection links and shape PrettyLink use distinct export contracts.
        for (edge_id, _) in graph.edges() {
            if graph
                .edge_tooltip(edge_id)
                .is_some_and(|value| !value.is_empty())
            {
                entries.push(graph.edge_tooltip(edge_id).unwrap_or_default().to_owned());
            }
            // Original exporter.toConnection copies Link but never sets
            // PrettyLink. The scene appendix reads only PrettyLink and tooltip;
            // an authored edge URL remains interactive label/link metadata.
        }
        // Original buildMarkdownLabel appends one title for every visible link
        // primitive after shape/connection metadata has been compiled.
        if let Ok(metadata) =
            markdown_scene::collect_svg(svg, markdown_scene::LinkBudget::default(), true)
        {
            entries.extend(metadata.appendix);
        }
    }
    if entries.is_empty() {
        // Original SVG appendix.Append returns its input before replacing
        // icons when generateAppendix has no rows.
        if !precision {
            return svg.to_vec();
        }
        // The Go raster scene only paints numbered badges when it has at
        // least one appendix row. A lone internal link back to the root has
        // no PrettyLink text, so its interactive SVG glyph is omitted from
        // PNG/PDF output as well.
        return replace_static_appendix_badges_with_theme(svg, &badge_numbers, false, theme_mono);
    }

    let svg = replace_static_appendix_badges_with_theme(svg, &badge_numbers, true, theme_mono);

    let mut viewboxes = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = std::str::from_utf8(svg[cursor..].as_ref())
        .ok()
        .and_then(|text| text.find("viewBox=\""))
    {
        let start = cursor + relative + "viewBox=\"".len();
        let Some(end_relative) = svg[start..].iter().position(|byte| *byte == b'\"') else {
            break;
        };
        let end = start + end_relative;
        let raw = std::str::from_utf8(&svg[start..end]).unwrap_or_default();
        let values = raw
            .split_whitespace()
            .filter_map(|value| value.parse::<f64>().ok())
            .collect::<Vec<_>>();
        if values.len() == 4 {
            viewboxes.push((start, end, values));
        }
        cursor = end + 1;
        if viewboxes.len() == 2 {
            break;
        }
    }
    if viewboxes.len() != 2 {
        return svg.to_vec();
    }
    let inner = &viewboxes[1].2;
    // The Go appendix asks d2graph.GetTextDimensions for each 16px regular
    // line.  Character-count estimates make long URLs needlessly widen the
    // document, so use the same bundled Source Sans Pro advance table as the
    // normal native text target.  The 32px minimum is the icon diameter in
    // d2svg/appendix.
    static APPENDIX_FONT: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    let appendix_font = APPENDIX_FONT
        .get_or_init(|| bundled_source_sans_pro_regular().ok())
        .as_ref();
    // GetTextDimensions receives no font family in both Source appendix paths.
    // Measure regular Source Sans Pro even when the painter uses a mono theme.
    let dimensions = entries
        .iter()
        .map(|entry| {
            appendix_font.map_or_else(
                || {
                    (
                        entry
                            .split('\n')
                            .map(|line| line.chars().count())
                            .max()
                            .unwrap_or(0) as f64
                            * 8.0,
                        entry.split('\n').count() as f64 * 21.0,
                    )
                },
                |font| {
                    let measured = native_markdown_measurer(MarkdownFamily::Sans).map_or_else(
                        || font.measure(entry, 16.0),
                        |measurer| font.measure_with_fallback(entry, 16.0, measurer.mono_font),
                    );
                    (measured.width.ceil(), measured.height.ceil())
                },
            )
        })
        .collect::<Vec<_>>();
    let max_text_width = dimensions
        .iter()
        .map(|(width, _)| *width)
        .fold(0.0, f64::max);
    let appendix_text_width = max_text_width + 48.0;
    let appendix_height = 50.0
        + dimensions
            .iter()
            .map(|(_, height)| height.max(32.0) + 20.0)
            .sum::<f64>()
        + 20.0;
    let old_inner_width = inner[2];
    // Match appendix.go's coordinate arithmetic: width starts at the widest
    // line plus the icon/text gap, then compensates for the negative left
    // viewbox origin and both 40px side margins.
    let appendix_required_width = appendix_text_width - inner[0] + 80.0;
    let new_inner_width = old_inner_width.max(appendix_required_width);
    let new_inner_height = inner[3] + appendix_height;
    let scale = if viewboxes[0].2[3].is_finite() && inner[3] > 0.0 {
        viewboxes[0].2[3] / inner[3]
    } else {
        1.0
    };
    let new_outer_width = viewboxes[0].2[2].max(new_inner_width * scale);
    let new_outer_height = viewboxes[0].2[3] + appendix_height * scale;

    let mut replacements = vec![
        (
            viewboxes[0].0,
            viewboxes[0].1,
            format!(
                "0 0 {} {}",
                number(new_outer_width),
                number(new_outer_height)
            ),
        ),
        (
            viewboxes[1].0,
            viewboxes[1].1,
            format!(
                "{} {} {} {}",
                number(inner[0]),
                number(inner[1]),
                number(new_inner_width),
                number(new_inner_height)
            ),
        ),
    ];
    replacements.sort_by_key(|(start, _, _)| *start);
    let mut output = Vec::with_capacity(svg.len() + entries.len() * 100);
    let mut last = 0;
    for (start, end, replacement) in replacements {
        output.extend_from_slice(&svg[last..start]);
        output.extend_from_slice(replacement.as_bytes());
        last = end;
    }
    output.extend_from_slice(&svg[last..]);

    // The source updates the nested SVG and background rectangle dimensions
    // along with both viewboxes.  Without this, a browser or rasterizer can
    // clip the newly appended footer even though the viewBox is larger.
    let old_width = number(old_inner_width);
    let old_height = number(inner[3]);
    let new_width = number(new_inner_width);
    let new_height = number(new_inner_height);
    replace_first_matching_attr(&mut output, b"width=\"", &old_width, &new_width, 8);
    replace_first_matching_attr(&mut output, b"height=\"", &old_height, &new_height, 8);
    // Source appendix exports reset requested scale to one. Update the root
    // logical dimensions too, even when its requested size differs from the
    // old base viewBox and did not match the replacements above.
    for (name, value) in [("width", &new_width), ("height", &new_height)] {
        let root_end = output.iter().position(|byte| *byte == b'>').unwrap_or(0);
        let root = std::str::from_utf8(&output[..root_end]).unwrap_or_default();
        let prefix = format!("{name}=\"");
        let previous = root.find(&prefix).and_then(|start| {
            let tail = &root[start + prefix.len()..];
            tail.find('"').map(|end| tail[..end].to_owned())
        });
        if let Some(previous) = previous {
            replace_first_matching_attr(&mut output, prefix.as_bytes(), &previous, value, 1);
        }
    }
    let insertion = output
        .windows(b"</svg>\n</svg>\n".len())
        .position(|window| window == b"</svg>\n</svg>\n")
        .unwrap_or(output.len());
    if insertion == output.len() {
        return output;
    }

    // Original scene measureAppendix receives Diagram.BoundingBox, including
    // connection routes, labels, stroke extents and node decorations. Object
    // rectangles alone can shorten or misplace the separator and appendix.
    let (diagram_left, _, diagram_right, diagram_bottom) = diagram_bounds.unwrap_or_else(|| {
        bounds_with_markdown_family_and_legend(
            graph,
            result,
            &BTreeMap::new(),
            styles,
            if theme_mono {
                MarkdownFamily::Mono
            } else {
                MarkdownFamily::Sans
            },
            false,
        )
    });
    let separator_y = diagram_bottom + 50.0;
    let appendix_y = diagram_bottom + 100.0;
    let badge_class = if theme_mono {
        "text-mono-bold"
    } else {
        "text-bold"
    };
    let mut group = format!(
        "<line x1=\"{}\" x2=\"{}\" y1=\"{}\" y2=\"{}\" class=\"d2-rs-stroke-B2\"/>\\n<g class=\"appendix\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"100%\">\\n",
        number(diagram_left - 40.0),
        number((diagram_right.max(appendix_text_width)) + 40.0),
        number(separator_y),
        number(separator_y),
        number(diagram_left),
        number(diagram_bottom),
        // Decoration was included in diagram_right above; do not add it twice.
        number((diagram_right - diagram_left).max(0.0)),
    );
    let mut row_offset = 0.0;
    for (index, entry) in entries.iter().enumerate() {
        let y = appendix_y + row_offset;
        let measured_height = dimensions[index].1;
        row_offset += measured_height.max(32.0) + 20.0;
        let text = if entry.contains('\n') {
            let lines = entry.split('\n').collect::<Vec<_>>();
            let line_height = measured_height / lines.len() as f64;
            lines
                .iter()
                .enumerate()
                .map(|(line_index, line)| {
                    format!(
                        "<tspan x=\"48\" dy=\"{}\">{}</tspan>",
                        number(if line_index == 0 { 0.0 } else { line_height }),
                        if line.is_empty() {
                            " ".to_owned()
                        } else {
                            escape(line)
                        }
                    )
                })
                .collect::<String>()
        } else {
            escape(entry)
        };
        // Original SVG appendix text inherits the diagram's N1 fill.
        // Its family is unaffected by a render-only theme's exporter rules.
        let entry_class = if theme_mono {
            "text-mono d2-rs-text-N1"
        } else {
            "text d2-rs-text-N1"
        };
        if precision {
            // Source scene retains one TextRun per appendix line. Transport
            // its complete f64 baseline instead of accumulating rounded dy
            // operands through SVG tspans.
            let _ = writeln!(
                group,
                "<g id=\"d2-rs-static-appendix-icon-{}\" transform=\"translate(0 {})\" class=\"appendix-icon\" style=\"filter:drop-shadow(0px 0px 32px rgba(31,36,58,0.1));\"><circle cx=\"16\" cy=\"0\" r=\"16\" fill=\"white\" stroke=\"#DEE1EB\"/><text class=\"{}\" x=\"16\" y=\"5\" fill=\"#000000\" font-weight=\"bold\" style=\"font-size:16px;text-anchor:middle;\">{}</text></g>",
                index + 1,
                number(y),
                badge_class,
                index + 1
            );
            let lines = entry.split('\n').collect::<Vec<_>>();
            let line_height = measured_height / lines.len() as f64;
            for (line_index, line) in lines.iter().enumerate() {
                let baseline = y + 5.0 + line_index as f64 * line_height;
                let content = if line.is_empty() && lines.len() > 1 {
                    " ".to_owned()
                } else {
                    escape(line)
                };
                let _ = writeln!(
                    group,
                    "<text id=\"d2-rs-native-appendix-text-{}-{}\" class=\"{}\" x=\"48\" y=\"{}\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\" style=\"font-size:16px;\">{}</text>",
                    index,
                    line_index,
                    entry_class,
                    number(baseline),
                    48.0_f64.to_bits(),
                    baseline.to_bits(),
                    content
                );
            }
            continue;
        }
        let _ = writeln!(
            group,
            "<g id=\"d2-rs-static-appendix-icon-{}\" transform=\"translate(0 {})\" class=\"appendix-icon\" style=\"filter:drop-shadow(0px 0px 32px rgba(31,36,58,0.1));\"><circle cx=\"16\" cy=\"0\" r=\"16\" fill=\"white\" stroke=\"#DEE1EB\"/><text class=\"{}\" x=\"16\" y=\"5\" fill=\"#000000\" font-weight=\"bold\" style=\"font-size:16px;text-anchor:middle;\">{}</text></g><text class=\"{}\" x=\"48\" y=\"{}\" style=\"font-size:16px;\">{}</text>",
            index + 1,
            number(y),
            badge_class,
            index + 1,
            entry_class,
            number(y + 5.0),
            text
        );
    }
    group.push_str("</g>\n");
    // Source Append adds regular and bold Source Sans Pro appendix faces
    // independently of the diagram's font subset. Metadata and badge digits
    // may be absent from that subset; retain their real glyphs and metrics.
    let mut appendix_css = String::from(
        ".d2-rs-theme .appendix > text{font-family:\"d2-rs-appendix-font-regular\";}\
         .d2-rs-theme .appendix-icon > text{font-family:\"d2-rs-appendix-font-bold\";}",
    );
    let selected_regular: &[u8] = match svg_primary {
        Some(BundledFamily::Mono) => {
            include_bytes!("../../d2-rs-text/assets/SourceCodePro-Regular.ttf")
        }
        Some(BundledFamily::HandDrawn) => {
            include_bytes!("../../d2-rs-text/assets/FuzzyBubbles-Regular.ttf")
        }
        _ => include_bytes!("../../d2-rs-text/assets/SourceSansPro-Regular.ttf"),
    };
    let subset = svg_primary
        .and_then(|_| d2_rs_text::font_subset::subset_ttf(selected_regular, &entries.concat()));
    for (family, weight, data) in [
        (
            "d2-rs-appendix-font-regular",
            400,
            subset.as_deref().unwrap_or(selected_regular),
        ),
        (
            "d2-rs-appendix-font-bold",
            700,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../d2-rs-text/assets/SourceSansPro-Bold.ttf"
            ))
            .as_slice(),
        ),
    ] {
        appendix_css.push_str(&font_face_rule_format(
            family, weight, "normal", "font/ttf", "truetype", data,
        ));
    }
    let _ = write!(group, "<style>{appendix_css}</style>");
    output.splice(insertion..insertion, group.as_bytes().iter().copied());
    output
}

fn replace_static_appendix_badges(svg: &[u8], numbers: &[Option<usize>], visible: bool) -> Vec<u8> {
    replace_static_appendix_badges_with_theme(svg, numbers, visible, false)
}

fn replace_static_appendix_badges_with_theme(
    svg: &[u8],
    numbers: &[Option<usize>],
    visible: bool,
    theme_mono: bool,
) -> Vec<u8> {
    let mut output = Vec::with_capacity(svg.len());
    let mut cursor = 0;
    let mut badge_index = 0;
    let marker = b"class=\"appendix-icon\"";
    while let Some(relative_marker) = svg[cursor..]
        .windows(marker.len())
        .position(|window| window == marker)
    {
        let marker_start = cursor + relative_marker;
        let Some(relative_open_end) = svg[marker_start + marker.len()..]
            .iter()
            .position(|byte| *byte == b'>')
        else {
            break;
        };
        let body_start = marker_start + marker.len() + relative_open_end + 1;
        let Some(relative_group_end) = svg[body_start..]
            .windows(4)
            .position(|window| window == b"</g>")
        else {
            break;
        };
        let group_end = body_start + relative_group_end + 4;
        let icon_body = &svg[body_start..group_end - 4];
        let icon_svg_marker = b"<svg width=\"32\"";
        if !icon_body
            .windows(icon_svg_marker.len())
            .any(|window| window == icon_svg_marker)
        {
            // Ignore unrelated SVG groups that happen to reuse this class.
            output.extend_from_slice(&svg[cursor..body_start]);
            cursor = body_start;
            continue;
        }

        output.extend_from_slice(&svg[cursor..marker_start]);
        output.extend_from_slice(
            format!("id=\"d2-rs-static-appendix-icon-{}\" ", badge_index + 1).as_bytes(),
        );
        output.extend_from_slice(&svg[marker_start..body_start]);
        if visible {
            if let Some(Some(number)) = numbers.get(badge_index) {
                let badge_class = if theme_mono {
                    "text-mono-bold"
                } else {
                    "text-bold"
                };
                output.extend_from_slice(
                    format!(
                        "<circle cx=\"16\" cy=\"16\" r=\"16\" fill=\"white\" stroke=\"#DEE1EB\"/><text class=\"{badge_class}\" x=\"16\" y=\"21\" fill=\"#000000\" font-weight=\"bold\" style=\"font-size:16px;text-anchor:middle;\">{number}</text>"
                    )
                    .as_bytes(),
                );
            } else {
                output.extend_from_slice(icon_body);
            }
        }
        output.extend_from_slice(b"</g>");
        cursor = group_end;
        badge_index += 1;
    }
    output.extend_from_slice(&svg[cursor..]);
    output
}

fn replace_first_matching_attr(
    output: &mut Vec<u8>,
    prefix: &[u8],
    old_value: &str,
    new_value: &str,
    limit: usize,
) {
    if old_value == new_value || limit == 0 {
        return;
    }
    let mut cursor = 0;
    let mut replaced = 0;
    let old = format!("{}{}\"", String::from_utf8_lossy(prefix), old_value);
    let replacement = format!("{}{}\"", String::from_utf8_lossy(prefix), new_value);
    while replaced < limit {
        let Some(relative) = output[cursor..]
            .windows(old.len())
            .position(|window| window == old.as_bytes())
        else {
            break;
        };
        let start = cursor + relative;
        output.splice(
            start..start + old.len(),
            replacement.as_bytes().iter().copied(),
        );
        cursor = start + replacement.len();
        replaced += 1;
    }
}

#[derive(Serialize)]
struct HashPoint {
    x: GoFloat,
    y: GoFloat,
}

/// Go's `encoding/json` writes an integral `float64` as `1`, while Rust's
/// default serializer preserves the decimal marker as `1.0`. Target hashes
/// include these bytes, so retain the v0.9.0 number spelling at this boundary.
struct GoFloat(f64);

impl Serialize for GoFloat {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // encoding/json float64 text, including Go's ties-to-even shortest
        // digits (serde's shortest formatter rounds exact ties upward).
        if !self.0.is_finite() {
            return serializer.serialize_f64(self.0);
        }
        serde_json::value::RawValue::from_string(d2_rs_core::go_json_float(self.0))
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

fn target_hash_route_float(value: f64) -> GoFloat {
    GoFloat(target_route_coordinate(value))
}

#[derive(Serialize)]
struct HashIntPoint {
    x: i64,
    y: i64,
}

/// The v0.9.0 target stores icons as `*url.URL`, not as the source spelling.
/// Keep the exported Go field names and order so local paths, HTTP URLs, and
/// data URLs contribute the same bytes to Diagram.Bytes.
#[derive(Serialize, serde::Deserialize)]
struct HashUrl {
    #[serde(rename = "Scheme")]
    scheme: String,
    #[serde(rename = "Opaque")]
    opaque: String,
    #[serde(rename = "User")]
    user: Option<Value>,
    #[serde(rename = "Host")]
    host: String,
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "RawPath")]
    raw_path: String,
    #[serde(rename = "OmitHost")]
    omit_host: bool,
    #[serde(rename = "ForceQuery")]
    force_query: bool,
    #[serde(rename = "RawQuery")]
    raw_query: String,
    #[serde(rename = "Fragment")]
    fragment: String,
    #[serde(rename = "RawFragment")]
    raw_fragment: String,
}

fn parse_hash_url(raw: Option<&str>) -> Option<HashUrl> {
    let value = d2_rs_language::compiler::import_urls::graph_url_metadata(raw?.as_bytes()).ok()?;
    serde_json::from_value(value).ok()
}

#[derive(Serialize)]
struct HashClassField {
    name: String,
    #[serde(rename = "type")]
    type_text: String,
    visibility: String,
    underline: bool,
}

#[derive(Serialize)]
struct HashClassMethod {
    name: String,
    #[serde(rename = "return")]
    return_type: String,
    visibility: String,
    underline: bool,
}

// compileClass strips precisely one ASCII UML visibility prefix. The graph's
// paint rows retain it for their visibility token; target records do not.
fn target_class_member(name: &str) -> (&str, &str) {
    if let Some(name) = name.strip_prefix('+') {
        (name, "public")
    } else if let Some(name) = name.strip_prefix('-') {
        (name, "private")
    } else if let Some(name) = name.strip_prefix('#') {
        (name, "protected")
    } else {
        (name, "public")
    }
}

#[derive(Serialize)]
struct HashShape {
    id: String,
    #[serde(rename = "type")]
    shape_type: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    classes: Vec<String>,
    pos: HashIntPoint,
    width: i64,
    height: i64,
    opacity: GoFloat,
    #[serde(rename = "strokeDash")]
    stroke_dash: GoFloat,
    #[serde(rename = "strokeWidth")]
    stroke_width: i64,
    #[serde(rename = "borderRadius")]
    border_radius: i64,
    fill: String,
    #[serde(rename = "fillPattern", skip_serializing_if = "Option::is_none")]
    fill_pattern: Option<String>,
    stroke: String,
    animated: bool,
    shadow: bool,
    #[serde(rename = "3d")]
    three_dee: bool,
    multiple: bool,
    #[serde(rename = "double-border")]
    double_border: bool,
    tooltip: String,
    link: String,
    #[serde(rename = "prettyLink", skip_serializing_if = "Option::is_none")]
    pretty_link: Option<String>,
    icon: Option<HashUrl>,
    #[serde(rename = "iconBorderRadius", skip_serializing_if = "i64_is_zero")]
    icon_border_radius: i64,
    #[serde(rename = "iconPosition")]
    icon_position: String,
    blend: bool,
    fields: Option<Vec<HashClassField>>,
    methods: Option<Vec<HashClassMethod>>,
    columns: Option<Vec<SourceSqlColumn>>,
    #[serde(rename = "contentAspectRatio", skip_serializing_if = "Option::is_none")]
    content_aspect_ratio: Option<GoFloat>,
    label: String,
    #[serde(rename = "fontSize")]
    font_size: i64,
    #[serde(rename = "fontFamily")]
    font_family: String,
    language: String,
    color: String,
    italic: bool,
    bold: bool,
    underline: bool,
    #[serde(rename = "labelWidth")]
    label_width: i64,
    #[serde(rename = "labelHeight")]
    label_height: i64,
    #[serde(rename = "labelFill", skip_serializing_if = "Option::is_none")]
    label_fill: Option<String>,
    #[serde(rename = "labelPosition", skip_serializing_if = "Option::is_none")]
    label_position: Option<String>,
    #[serde(rename = "tooltipPosition", skip_serializing_if = "Option::is_none")]
    tooltip_position: Option<String>,
    #[serde(rename = "zIndex")]
    z_index: i64,
    level: i64,
    #[serde(flatten)]
    target_accents: Option<SourceTargetAccents>,
}

#[derive(Serialize)]
struct HashConnection {
    id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    classes: Vec<String>,
    src: String,
    #[serde(rename = "srcArrow")]
    src_arrow: String,
    #[serde(rename = "srcLabel", skip_serializing_if = "Option::is_none")]
    src_label: Option<d2_rs_language::compiler::SourceSqlText>,
    dst: String,
    #[serde(rename = "dstArrow")]
    dst_arrow: String,
    #[serde(rename = "dstLabel", skip_serializing_if = "Option::is_none")]
    dst_label: Option<d2_rs_language::compiler::SourceSqlText>,
    opacity: GoFloat,
    #[serde(rename = "strokeDash")]
    stroke_dash: GoFloat,
    #[serde(rename = "strokeWidth")]
    stroke_width: i64,
    stroke: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill: Option<String>,
    #[serde(rename = "borderRadius", skip_serializing_if = "go_float_is_zero")]
    border_radius: GoFloat,
    label: String,
    #[serde(rename = "fontSize")]
    font_size: i64,
    #[serde(rename = "fontFamily")]
    font_family: String,
    language: String,
    color: String,
    italic: bool,
    bold: bool,
    underline: bool,
    #[serde(rename = "labelWidth")]
    label_width: i64,
    #[serde(rename = "labelHeight")]
    label_height: i64,
    #[serde(rename = "labelPosition")]
    label_position: String,
    #[serde(rename = "labelPercentage")]
    label_percentage: GoFloat,
    link: String,
    route: Vec<HashPoint>,
    #[serde(rename = "isCurve", skip_serializing_if = "is_false_hash")]
    is_curve: bool,
    animated: bool,
    tooltip: String,
    icon: Option<HashUrl>,
    #[serde(rename = "iconPosition", skip_serializing_if = "Option::is_none")]
    icon_position: Option<String>,
    #[serde(rename = "iconBorderRadius", skip_serializing_if = "go_float_is_zero")]
    icon_border_radius: GoFloat,
    #[serde(rename = "zIndex")]
    z_index: i64,
}

#[derive(Serialize)]
struct HashConfig {
    sketch: bool,
    #[serde(rename = "themeID")]
    theme_id: i64,
    #[serde(rename = "darkThemeID")]
    dark_theme_id: Option<i64>,
    pad: Option<i64>,
    center: Option<bool>,
    #[serde(rename = "layoutEngine")]
    layout_engine: Option<String>,
    #[serde(rename = "themeOverrides", skip_serializing_if = "Option::is_none")]
    theme_overrides: Option<HashThemeOverrides>,
    #[serde(rename = "darkThemeOverrides", skip_serializing_if = "Option::is_none")]
    dark_theme_overrides: Option<HashThemeOverrides>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    data: BTreeMap<String, Value>,
}

/// The Go target config keeps an allocated ThemeOverrides object as a full
/// object of nullable fields.  Preserve that field order and null spelling in
/// the hash stream instead of treating the Rust map as an unordered JSON map.
#[derive(Serialize)]
struct HashThemeOverrides {
    n1: Option<String>,
    n2: Option<String>,
    n3: Option<String>,
    n4: Option<String>,
    n5: Option<String>,
    n6: Option<String>,
    n7: Option<String>,
    b1: Option<String>,
    b2: Option<String>,
    b3: Option<String>,
    b4: Option<String>,
    b5: Option<String>,
    b6: Option<String>,
    aa2: Option<String>,
    aa4: Option<String>,
    aa5: Option<String>,
    ab4: Option<String>,
    ab5: Option<String>,
}

impl HashThemeOverrides {
    fn from_map(values: &BTreeMap<String, String>) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let get = |name: &str| {
            values
                .get(name)
                .cloned()
                .or_else(|| values.get(&name.to_ascii_uppercase()).cloned())
        };
        Some(Self {
            n1: get("n1"),
            n2: get("n2"),
            n3: get("n3"),
            n4: get("n4"),
            n5: get("n5"),
            n6: get("n6"),
            n7: get("n7"),
            b1: get("b1"),
            b2: get("b2"),
            b3: get("b3"),
            b4: get("b4"),
            b5: get("b5"),
            b6: get("b6"),
            aa2: get("aa2"),
            aa4: get("aa4"),
            aa5: get("aa5"),
            ab4: get("ab4"),
            ab5: get("ab5"),
        })
    }
}

fn target_shape_name(shape: ShapeKind) -> &'static str {
    match shape {
        // The source compiler keeps `circle` as a graph hint, but the target
        // exporter publishes it as the canonical oval shape.  This distinction
        // is visible in Diagram.Bytes and therefore in every SVG hash.
        ShapeKind::Circle | ShapeKind::Oval => "oval",
        ShapeKind::RealSquare | ShapeKind::Square => "rectangle",
        ShapeKind::SqlTable => "sql_table",
        ShapeKind::C4Person => "c4-person",
        ShapeKind::Parallelogram => "parallelogram",
        ShapeKind::StoredData => "stored_data",
        ShapeKind::Person => "person",
        ShapeKind::Diamond => "diamond",
        ShapeKind::Hexagon => "hexagon",
        ShapeKind::Cloud => "cloud",
        ShapeKind::Class => "class",
        ShapeKind::Text => "text",
        ShapeKind::Code => "code",
        ShapeKind::Image => "image",
        ShapeKind::Rectangle => "rectangle",
        ShapeKind::Document => "document",
        ShapeKind::Cylinder => "cylinder",
        ShapeKind::Queue => "queue",
        ShapeKind::Page => "page",
        ShapeKind::Package => "package",
        ShapeKind::Step => "step",
        ShapeKind::Callout => "callout",
    }
}

fn is_false_hash(value: &bool) -> bool {
    !*value
}

fn escape_go_hash_json(raw: &[u8]) -> Vec<u8> {
    let mut escaped = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        let replacement = match raw[index] {
            b'&' => Some(br"\u0026".as_slice()),
            b'<' => Some(br"\u003c".as_slice()),
            b'>' => Some(br"\u003e".as_slice()),
            _ if raw[index..].starts_with("\u{2028}".as_bytes()) => Some(br"\u2028".as_slice()),
            _ if raw[index..].starts_with("\u{2029}".as_bytes()) => Some(br"\u2029".as_slice()),
            _ => None,
        };
        if let Some(replacement) = replacement {
            escaped.extend_from_slice(replacement);
            index += if raw[index] < 0x80 { 1 } else { 3 };
        } else {
            escaped.push(raw[index]);
            index += 1;
        }
    }
    escaped
}

fn parse_hash_f64(value: Option<&str>, default: f64) -> f64 {
    value
        .and_then(d2_rs_core::parse_d2_float)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn parse_hash_bool(value: Option<&str>) -> bool {
    value
        .and_then(|raw| d2_rs_core::parse_d2_bool(raw))
        .unwrap_or(false)
}

fn target_node_level(graph: &Graph, id: NodeId) -> i64 {
    let mut level = 1;
    let mut parent = graph.node(id).and_then(|node| node.parent);
    let mut guard = 0usize;
    while let Some(parent_id) = parent {
        level += 1;
        parent = graph.node(parent_id).and_then(|node| node.parent);
        guard += 1;
        if guard > graph.nodes.len() {
            break;
        }
    }
    level
}

fn target_arrow_name(graph: &Graph, edge_id: EdgeId, target: bool) -> String {
    let arrows = graph.edge_arrows(edge_id);
    let enabled = if target { arrows.target } else { arrows.source };
    if !enabled {
        return "none".to_owned();
    }
    let arrowheads = graph.edge_arrowheads(edge_id);
    let explicit = if target {
        arrowheads.target.as_deref()
    } else {
        arrowheads.source.as_deref()
    };
    // canonical_marker_name maps `none` to "no marker"; the target keeps
    // d2target.NoArrowhead's own name.
    canonical_marker_name(explicit).unwrap_or("none").to_owned()
}

fn target_theme_rules(graph: &Graph) -> d2_rs_core::theme::SpecialRules {
    graph
        .target_theme_id
        .and_then(d2_rs_core::theme::palette)
        .map(|p| p.special_rules)
        .unwrap_or_default()
}
fn target_default_font(graph: &Graph) -> String {
    if target_theme_rules(graph).mono {
        "mono"
    } else {
        "DEFAULT"
    }
    .to_owned()
}
fn i64_is_zero(value: &i64) -> bool {
    *value == 0
}
fn go_float_is_zero(value: &GoFloat) -> bool {
    value.0 == 0.0
}
fn target_arrow_text(
    label: Option<d2_rs_core::ArrowheadLabel>,
) -> Option<d2_rs_language::compiler::SourceSqlText> {
    label
        .filter(|v| !v.text.is_empty())
        .map(|label| d2_rs_language::compiler::SourceSqlText {
            label: label.text,
            color: label.color.unwrap_or_default(),
            label_width: label.size.width.round() as u32,
            label_height: label.size.height.round() as u32,
            ..Default::default()
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetShapeStage {
    Unmeasured,
    LaidOut,
}

// d2exporter consumes Graph.Objects publication order. Arena order is storage;
// compiler/wire adapters retain the owning graph's serialized order separately.
/// Original Graph.Objects publication order, shared with source metadata.
pub fn source_graph_object_order(graph: &Graph) -> Vec<NodeId> {
    target_node_ids(graph)
}

/// Sequence layout's committed object draw order.
pub fn source_graph_object_z_index(graph: &Graph, id: NodeId) -> i32 {
    sequence_shape_z_index(graph, id)
}

/// Span scalar mutations that survive nested-diagram reinjection. Go's
/// InjectNested restores extracted roots' original attributes afterward.
pub fn source_graph_object_has_sequence_span_scalars(graph: &Graph, id: NodeId) -> bool {
    is_sequence_span_like(graph, id)
}

/// Notes receive a Page scalar during sequence layout. Extracted grid
/// contents and reinjected roots keep their own original shape scalars.
pub fn source_graph_object_has_sequence_note_shape(graph: &Graph, id: NodeId) -> bool {
    is_sequence_note_like(graph, id)
}

/// Sequence layout's committed connection draw order.
pub fn source_graph_edge_z_index(graph: &Graph, edge: &d2_rs_core::Edge) -> i32 {
    sequence_connection_z_index(graph, edge)
}

fn target_node_ids(graph: &Graph) -> Vec<NodeId> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::with_capacity(graph.nodes.len());
    for id in graph
        .node_order_override
        .iter()
        .copied()
        .chain(graph.nodes().map(|(id, _)| id))
    {
        if graph.node(id).is_some() && seen.insert(id) {
            ids.push(id);
        }
    }
    ids
}

fn target_hash_shape(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    id: NodeId,
) -> Option<HashShape> {
    target_hash_shape_for_stage(graph, result, labels, styles, id, TargetShapeStage::LaidOut)
}

fn target_hash_shape_for_stage(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    id: NodeId,
    stage: TargetShapeStage,
) -> Option<HashShape> {
    let node = graph.node(id)?;
    let mut style = styles.get(&id).cloned().unwrap_or_default();
    // exporter.applyTheme's non-color special rules; the later applyStyles
    // pass lets explicit style values win.
    let rules = target_theme_rules(graph);
    let has_children = node_has_children(graph, id);
    if rules.outer_container_double_border
        && node.parent.is_none()
        && has_children
        && style.double_border.is_none()
    {
        style.double_border = Some("true".into());
    }
    if style.fill_pattern.is_none() {
        if rules.container_dots {
            if has_children {
                style.fill_pattern = Some("dots".into());
            }
        } else if rules.all_paper {
            style.fill_pattern = Some("paper".into());
        }
    }
    if rules.c4 {
        let implicit_text_fill = node.shape == ShapeKind::Text && style.fill.is_none();
        apply_c4_node_style(
            &mut style,
            target_shape_for_node(graph, id).unwrap_or(node.shape),
            node_has_children(graph, id),
            node_level(graph, id),
        );
        // toShape runs applyStyles again after applyTheme.
        if implicit_text_fill {
            style.fill = Some("transparent".to_owned());
        }
    }
    let rect = result.boxes.get(&id).copied().unwrap_or(Rect {
        origin: Point { x: 0.0, y: 0.0 },
        size: node.size,
    });
    let label_state = result.node_labels.get(&id);
    let label_size = label_state
        .and_then(|state| state.size)
        .or(node.label_size)
        .unwrap_or_default();
    let label_text = if stage == TargetShapeStage::LaidOut
        && source_graph_object_has_sequence_span_scalars(graph, id)
    {
        String::new()
    } else {
        labels.get(&id).cloned().unwrap_or_default()
    };
    let sequence_group = is_sequence_group_like(graph, id);
    // Export the resolved source pointer independently of label text. Layout
    // can assign it to a blank actor, group, note or container. Unmeasured
    // legend compilation has not published any position pointer yet.
    let label_position = if stage == TargetShapeStage::Unmeasured {
        None
    } else {
        label_state
            .map(|state| state.position)
            .filter(|position| *position != LabelPosition::Unset)
            .or_else(|| {
                (node.label_position != LabelPosition::Unset).then_some(node.label_position)
            })
            .map(|position| position.d2_name().to_owned())
    };
    // d2exporter.toShape converts geometry to Go int (toward zero).
    let width = rect.size.width as i64;
    let height = rect.size.height as i64;
    let icon = parse_hash_url(style.icon.as_deref());
    let icon_position = if stage == TargetShapeStage::Unmeasured {
        String::new()
    } else if icon.is_some() {
        label_state
            .and_then(|state| state.icon_position)
            .or(node.icon_position)
            .map(|position| position.d2_name().to_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    // Diagram.Bytes serializes d2graph.Object.GetFill, which depends on the
    // object's hierarchy and shape.  Do not collapse every implicit fill to
    // B6: top-level containers are B4 and their first-level children are B5
    // in the v0.9.0 target model.
    let fill = style.fill.clone().unwrap_or_else(|| match node.shape {
        // applyStyles makes text transparent. Other shapes keep GetFill's
        // ordered sequence/group/span rules before ordinary shape defaults.
        ShapeKind::Text => "transparent".to_owned(),
        _ => default_node_fill(graph, id).to_owned(),
    });
    let stroke = style.stroke.unwrap_or_else(|| {
        if matches!(node.shape, ShapeKind::Text | ShapeKind::Code) {
            "N1".to_owned()
        } else if matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable) {
            "N7".to_owned()
        } else if parse_hash_f64(style.stroke_dash.as_deref(), 0.0) != 0.0 {
            // Object.GetStroke(shape.StrokeDash): dashed shapes default to B2.
            "B2".to_owned()
        } else {
            "B1".to_owned()
        }
    });
    let fields = if node.shape == ShapeKind::Class && !style.class_fields.is_empty() {
        Some(
            style
                .class_fields
                .iter()
                .enumerate()
                .map(|(index, (raw_name, type_text))| {
                    let (name, visibility) = target_class_member(raw_name);
                    HashClassField {
                        name: name.to_owned(),
                        type_text: type_text.clone(),
                        visibility: visibility.to_owned(),
                        underline: style
                            .class_field_underlines
                            .get(index)
                            .copied()
                            .unwrap_or(false),
                    }
                })
                .collect(),
        )
    } else {
        None
    };
    let methods = if node.shape == ShapeKind::Class && !style.class_methods.is_empty() {
        Some(
            style
                .class_methods
                .iter()
                .enumerate()
                .map(|(index, (raw_name, return_type))| {
                    let (name, visibility) = target_class_member(raw_name);
                    HashClassMethod {
                        name: name.to_owned(),
                        return_type: return_type.clone(),
                        visibility: visibility.to_owned(),
                        underline: style
                            .class_method_underlines
                            .get(index)
                            .copied()
                            .unwrap_or(false),
                    }
                })
                .collect(),
        )
    } else {
        None
    };
    let label_fill = (sequence_group && label_position.is_some()).then(|| fill.clone());
    // The pinned Go1.27 build selects encoding/json v2, whose invalid UTF-8
    // presentation matches ByteString rather than v1 raw JSON escape tokens.
    let pretty_link = style
        .pretty_link_bytes
        .as_ref()
        .map(|value| value.to_text().into_owned())
        .or_else(|| style.pretty_link.clone())
        .or_else(|| style.link.clone())
        .filter(|value| !value.is_empty());
    Some(HashShape {
        id: node.external_id.clone(),
        classes: graph
            .target_node_classes
            .get(&id)
            .cloned()
            .unwrap_or_default(),
        shape_type: if graph.is_sequence_diagram(id) {
            "sequence_diagram".to_owned()
        } else {
            let shape = if stage == TargetShapeStage::Unmeasured {
                node.shape
            } else {
                target_shape_for_node(graph, id).unwrap_or(node.shape)
            };
            target_shape_name(shape).to_owned()
        },
        pos: HashIntPoint {
            x: rect.origin.x as i64,
            y: rect.origin.y as i64,
        },
        width,
        height,
        opacity: GoFloat(parse_hash_f64(style.opacity.as_deref(), 1.0)),
        stroke_dash: GoFloat(parse_hash_f64(style.stroke_dash.as_deref(), 0.0)),
        stroke_width: parse_hash_f64(
            style.stroke_width.as_deref(),
            if graph.is_sequence_diagram(id) || sequence_group {
                0.0
            } else {
                2.0
            },
        )
        .round() as i64,
        border_radius: parse_hash_f64(style.border_radius.as_deref(), 0.0).round() as i64,
        fill,
        fill_pattern: style.fill_pattern,
        stroke,
        animated: parse_hash_bool(style.animated.as_deref()),
        shadow: parse_hash_bool(style.shadow.as_deref()),
        three_dee: parse_hash_bool(style.three_dee.as_deref()),
        multiple: parse_hash_bool(style.multiple.as_deref()),
        double_border: parse_hash_bool(style.double_border.as_deref()),
        tooltip: style.tooltip.unwrap_or_default(),
        link: style.link.unwrap_or_default(),
        pretty_link,
        icon,
        icon_position,
        icon_border_radius: style
            .icon_border_radius
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0),
        blend: sequence_group,
        fields,
        methods,
        columns: if node.shape == ShapeKind::SqlTable && !style.table_target_columns.is_empty() {
            Some(style.table_target_columns)
        } else {
            None
        },
        content_aspect_ratio: style
            .content_aspect_ratio_bits
            .map(f64::from_bits)
            .map(GoFloat),
        label: label_text,
        font_size: style
            .target_font_size
            .or_else(|| {
                matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable)
                    .then_some(style.font_size.unwrap_or(20))
            })
            .or_else(|| label_state.and_then(|state| state.font_size))
            .or(node.font_size)
            .unwrap_or(16) as i64,
        font_family: style
            .font
            .clone()
            .unwrap_or_else(|| target_default_font(graph)),
        language: style.language.clone().unwrap_or_default(),
        color: style
            .font_color
            .unwrap_or_else(|| if style.italic { "N2" } else { "N1" }.to_owned()),
        italic: style.italic,
        bold: style.target_bold.unwrap_or(style.bold),
        underline: style.underline,
        label_width: label_size.width.round() as i64,
        label_height: label_size.height.round() as i64,
        label_fill,
        label_position,
        tooltip_position: style
            .tooltip_position
            .filter(|position| !position.is_empty()),
        z_index: i64::from(sequence_shape_z_index(graph, id)),
        level: target_node_level(graph, id),
        target_accents: style.target_accents.or_else(|| {
            matches!(node.shape, ShapeKind::Class | ShapeKind::SqlTable)
                .then(SourceTargetAccents::structured)
        }),
    })
}

/// Project the source compiler's unmeasured legend through the same typed
/// exporter as ordinary shapes/connections. Source compileLegend supplies the
/// fixed geometry; it filters transparent objects without deleting endpoints.
pub fn target_legend_metadata(
    source: &d2_rs_core::SourceLegendGraph,
    theme_id: Option<i64>,
) -> Result<Option<Value>, serde_json::Error> {
    let mut graph = source.graph.clone();
    graph.target_theme_id = theme_id;
    let result = LayoutResult {
        removed_edges: Default::default(),
        source_edge_storage: Default::default(),
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        edge_label_percentage_commits: Default::default(),
        boxes: graph
            .nodes()
            .map(|(id, _)| {
                (
                    id,
                    Rect {
                        origin: Point { x: 10.0, y: 10.0 },
                        size: Size {
                            width: 100.0,
                            height: 100.0,
                        },
                    },
                )
            })
            .collect(),
        routes: graph
            .edges()
            .map(|(id, _)| {
                (
                    id,
                    vec![Point { x: 10.0, y: 10.0 }, Point { x: 110.0, y: 10.0 }],
                )
            })
            .collect(),
        edge_labels: BTreeMap::new(),
        node_labels: BTreeMap::new(),
        report: d2_rs_core::LayoutReport {
            selected_seed: 1,
            selected_pass: 0,
            score: d2_rs_core::Score::default(),
            improvement_passes: 0,
            candidates: Vec::new(),
            warnings: Vec::new(),
        },
    };
    let shapes = target_node_ids(&graph)
        .into_iter()
        .filter(|id| {
            source
                .styles
                .get(id)
                .and_then(|s| s.opacity.as_deref())
                .and_then(d2_rs_core::parse_d2_float)
                != Some(0.0)
        })
        .filter_map(|id| {
            target_hash_shape_for_stage(
                &graph,
                &result,
                &source.labels,
                &source.styles,
                id,
                TargetShapeStage::Unmeasured,
            )
        })
        .collect::<Vec<_>>();
    let connections = graph
        .edges()
        .filter_map(|(id, _)| target_hash_connection(&graph, &result, id))
        .collect::<Vec<_>>();
    if shapes.is_empty() && connections.is_empty() {
        return Ok(None);
    }
    let mut legend = serde_json::Map::new();
    if !source.label.is_empty() {
        legend.insert("label".to_owned(), Value::String(source.label.clone()));
    }
    if !shapes.is_empty() {
        legend.insert("shapes".to_owned(), serde_json::to_value(shapes)?);
    }
    if !connections.is_empty() {
        legend.insert("connections".to_owned(), serde_json::to_value(connections)?);
    }
    Ok(Some(Value::Object(legend)))
}

/// Native Go-target shape projection shared by hashing and the public facade.
/// Serialization errors propagate to callers; a missing node remains absent.
pub fn target_shape_metadata(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    id: NodeId,
) -> Result<Option<Value>, serde_json::Error> {
    target_hash_shape(graph, result, labels, styles, id)
        .map(serde_json::to_value)
        .transpose()
}

/// Exports all target shapes in the original graph publication order.
/// Stable arena IDs remain available to private graph/layout metadata.
pub fn target_shapes_metadata(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Vec<Value>, serde_json::Error> {
    target_node_ids(graph)
        .into_iter()
        .filter_map(|id| target_hash_shape(graph, result, labels, styles, id))
        .map(serde_json::to_value)
        .collect()
}

fn target_hash_connection(
    graph: &Graph,
    result: &LayoutResult,
    id: EdgeId,
) -> Option<HashConnection> {
    if result.removed_edges.contains(&id) {
        return None;
    }
    let edge = graph.edges.get(id.0 as usize)?;
    let source = graph.endpoint_node(edge.source)?.external_id.clone();
    let target = graph.endpoint_node(edge.target)?.external_id.clone();
    let style = graph.edge_style(id).clone();
    let label = graph.edge_label(id);
    let laid_out_label = result.edge_labels.get(&id);
    let route = result.routes.get(&id).cloned().unwrap_or_default();
    // Object presence matters: net/url.Parse("") returns a non-nil URL.
    // The paint accessor intentionally suppresses empty icons; export must not.
    let icon = graph
        .edge_icons
        .get(id.0 as usize)
        .and_then(Option::as_deref);
    let source_arrow = target_arrow_name(graph, id, false);
    let target_arrow = target_arrow_name(graph, id, true);
    let rules = target_theme_rules(graph);
    let arrow_labels = graph.edge_arrowhead_labels(id);
    let dash = parse_hash_f64(
        style.stroke_dash.as_deref(),
        if rules.c4 { 5.0 } else { 0.0 },
    );
    Some(HashConnection {
        id: edge_svg_id(graph, id)?,
        classes: graph
            .target_edge_classes
            .get(&id)
            .cloned()
            .unwrap_or_default(),
        src: source,
        src_arrow: source_arrow,
        src_label: target_arrow_text(arrow_labels.source),
        dst: target,
        dst_arrow: target_arrow,
        dst_label: target_arrow_text(arrow_labels.target),
        opacity: GoFloat(parse_hash_f64(style.opacity.as_deref(), 1.0)),
        stroke_dash: GoFloat(dash),
        stroke_width: parse_hash_f64(style.stroke_width.as_deref(), 2.0).round() as i64,
        stroke: style.stroke.unwrap_or_else(|| {
            if rules.c4 {
                "AA4"
            } else if dash != 0.0 {
                "B2"
            } else {
                "B1"
            }
            .to_owned()
        }),
        fill: style.fill.filter(|v| !v.is_empty()),
        border_radius: GoFloat(parse_hash_f64(
            style.border_radius.as_deref(),
            if rules.no_corner_radius { 0.0 } else { 10.0 },
        )),
        label: label.map(|value| value.text.clone()).unwrap_or_default(),
        font_size: graph.edge_font_size(id) as i64,
        font_family: style.font.unwrap_or_else(|| target_default_font(graph)),
        language: style.language.unwrap_or_default(),
        color: style.font_color.unwrap_or_else(|| {
            if rules.c4
                || style
                    .italic
                    .as_deref()
                    .and_then(d2_rs_core::parse_d2_bool)
                    .unwrap_or(true)
            {
                "N2"
            } else {
                "N1"
            }
            .to_owned()
        }),
        italic: style
            .italic
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(true),
        bold: graph.edge_label_bold(id),
        underline: style
            .underline
            .as_deref()
            .and_then(|value| d2_rs_core::parse_d2_bool(value))
            .unwrap_or(false),
        label_width: label
            .map(|value| value.size.width.round() as i64)
            .unwrap_or(0),
        label_height: label
            .map(|value| value.size.height.round() as i64)
            .unwrap_or(0),
        label_position: laid_out_label
            .or(label)
            .filter(|value| value.position != LabelPosition::Unset)
            .map(|value| value.position.d2_name().to_owned())
            .unwrap_or_default(),
        // d2talalayout.buildEdgePatch commits the selected percentage to the
        // source edge. d2exporter.toConnection exports that value through f32,
        // including 0.5; absence is tracked explicitly, not by a numeric sentinel.
        label_percentage: GoFloat(
            if result.edge_label_percentage_commits.contains(&id) {
                laid_out_label
            } else {
                label.filter(|_| graph.edge_label_percentage_present(id))
            }
            .map_or(0.0, |value| f64::from(value.percentage as f32)),
        ),
        link: graph.edge_link(id).unwrap_or_default().to_owned(),
        route: route
            .into_iter()
            .map(|point| HashPoint {
                x: target_hash_route_float(point.x),
                y: target_hash_route_float(point.y),
            })
            .collect(),
        // d2exporter copies edge.IsCurve verbatim; only engines that set it
        // (dagre's spline conversion, ELK) produce curves. TALA routes are
        // never curves regardless of their point count.
        is_curve: result.route_is_curve(graph, id),
        animated: parse_hash_bool(style.animated.as_deref()),
        tooltip: graph.edge_tooltip(id).unwrap_or_default().to_owned(),
        icon: parse_hash_url(icon),
        icon_position: icon.map(|_| {
            graph
                .edge_icon_position(id)
                .unwrap_or(LabelPosition::InsideMiddleCenter)
                .d2_name()
                .to_owned()
        }),
        icon_border_radius: GoFloat(graph.edge_icon_border_radius(id)),
        z_index: i64::from(sequence_connection_z_index(graph, edge)),
    })
}

// Source addLifelineEdges publishes these after authored connections. They
// have real target identities/styles but do not belong to the obstacle arena.
fn target_hash_lifelines(
    graph: &Graph,
    result: &LayoutResult,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Vec<HashConnection> {
    let fallback;
    let lines = if result.sequence_lifelines.is_empty() {
        fallback = sequence_lifelines(graph, result)
            .into_iter()
            .map(|line| {
                let node = graph.node(line.actor).expect("lifeline actor");
                let prefix = node
                    .parent
                    .and_then(|parent| graph.node(parent))
                    .map(|parent| format!("{}.", parent.external_id))
                    .unwrap_or_default();
                let local = node
                    .external_id
                    .strip_prefix(&prefix)
                    .unwrap_or(&node.external_id);
                let base = format!("{local}-lifeline-end");
                let hash = base.bytes().fold(2_166_136_261u32, |hash, byte| {
                    (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
                });
                d2_rs_core::SequenceLifeline {
                    actor: line.actor,
                    target_id: format!("{base}-{hash}"),
                    route: vec![line.start, line.end],
                }
            })
            .collect::<Vec<_>>();
        &fallback
    } else {
        &result.sequence_lifelines
    };
    lines
        .iter()
        .map(|line| {
            let actor = graph.node(line.actor).expect("lifeline actor exists");
            let style = styles.get(&line.actor);
            HashConnection {
                classes: Vec::new(),
                src_label: None,
                dst_label: None,
                fill: None,
                icon_position: None,
                icon_border_radius: GoFloat(0.0),
                id: format!("({} -- )[0]", actor.external_id),
                src: actor.external_id.clone(),
                src_arrow: "none".to_owned(),
                dst: line.target_id.clone(),
                dst_arrow: "none".to_owned(),
                opacity: GoFloat(1.0),
                stroke_dash: GoFloat(parse_hash_f64(
                    style.and_then(|style| style.stroke_dash.as_deref()),
                    6.0,
                )),
                stroke_width: 2,
                stroke: style
                    .and_then(|style| style.stroke.clone())
                    .unwrap_or_else(|| "B2".to_owned()),
                border_radius: GoFloat(if target_theme_rules(graph).no_corner_radius {
                    0.0
                } else {
                    10.0
                }),
                label: String::new(),
                font_size: 16,
                font_family: target_default_font(graph),
                language: String::new(),
                color: "N2".to_owned(),
                italic: true,
                bold: false,
                underline: false,
                label_width: 0,
                label_height: 0,
                label_position: String::new(),
                label_percentage: GoFloat(0.0),
                link: String::new(),
                route: line
                    .route
                    .iter()
                    .map(|point| HashPoint {
                        x: target_hash_route_float(point.x),
                        y: target_hash_route_float(point.y),
                    })
                    .collect(),
                is_curve: false,
                animated: false,
                tooltip: String::new(),
                icon: None,
                z_index: 1,
            }
        })
        .collect()
}

fn target_hash_root(graph: &Graph, options: &RenderOptions) -> HashShape {
    let mut root = HashShape {
        classes: Vec::new(),
        id: String::new(),
        shape_type: String::new(),
        pos: HashIntPoint { x: 0, y: 0 },
        width: 0,
        height: 0,
        opacity: GoFloat(0.0),
        stroke_dash: GoFloat(0.0),
        stroke_width: 0,
        border_radius: 0,
        fill: if options.transparent_background {
            "transparent".to_owned()
        } else {
            "N7".to_owned()
        },
        fill_pattern: None,
        stroke: String::new(),
        animated: false,
        shadow: false,
        three_dee: false,
        multiple: false,
        double_border: false,
        tooltip: String::new(),
        link: String::new(),
        pretty_link: None,
        icon: None,
        icon_position: String::new(),
        icon_border_radius: 0,
        blend: false,
        fields: None,
        methods: None,
        columns: None,
        content_aspect_ratio: None,
        label: options.target_root_label.clone().unwrap_or_default(),
        font_size: 0,
        font_family: String::new(),
        language: String::new(),
        color: String::new(),
        italic: false,
        bold: false,
        underline: false,
        label_width: 0,
        label_height: 0,
        label_fill: None,
        label_position: None,
        tooltip_position: None,
        z_index: 0,
        level: 0,
        target_accents: None,
    };
    if let Some(style) = &graph.root_style {
        if !options.transparent_background {
            if let Some(fill) = &style.fill {
                root.fill = fill.clone();
            }
        }
        root.stroke = style.stroke.clone().unwrap_or_default();
        root.fill_pattern = style.fill_pattern.clone().filter(|s| !s.is_empty());
        root.stroke_width = style
            .stroke_width
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        root.border_radius = style
            .border_radius
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        root.icon_border_radius = style
            .icon_border_radius
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        root.opacity = GoFloat(
            style
                .opacity
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                .unwrap_or(0.0),
        );
        root.stroke_dash = GoFloat(
            style
                .stroke_dash
                .as_deref()
                .and_then(d2_rs_core::parse_d2_float)
                .unwrap_or(0.0),
        );
        root.shadow = style
            .shadow
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
        root.three_dee = style
            .three_dee
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
        root.multiple = style
            .multiple
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
        root.double_border = style
            .double_border
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
        root.color = style.font_color.clone().unwrap_or_default();
        root.font_family = style.font.clone().unwrap_or_default();
        root.bold = style
            .bold
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
        root.italic = style
            .italic
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
        root.underline = style
            .underline
            .as_deref()
            .and_then(d2_rs_core::parse_d2_bool)
            .unwrap_or(false);
    }
    root
}

/// Complete authored and synthetic connection projection, shared with hashing.
pub fn target_connections_metadata(
    graph: &Graph,
    result: &LayoutResult,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Value, serde_json::Error> {
    let mut connections = graph
        .edges()
        .filter_map(|(id, _)| target_hash_connection(graph, result, id))
        .collect::<Vec<_>>();
    connections.extend(target_hash_lifelines(graph, result, styles));
    serde_json::to_value(connections)
}

/// Source root target record; it has no inferred canvas dimensions.
pub fn target_root_metadata(
    graph: &Graph,
    options: &RenderOptions,
) -> Result<Value, serde_json::Error> {
    serde_json::to_value(target_hash_root(graph, options))
}

/// Source root config retains source-only optional settings while resolved
/// appearance axes are fed back by d2lib.Compile.
pub fn target_config_metadata(
    config: Option<&d2_rs_language::compiler::DiagramConfig>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Value, serde_json::Error> {
    let empty = Default::default();
    let config = config.unwrap_or(&empty);
    serde_json::to_value(HashConfig {
        sketch,
        theme_id: options.theme_id.unwrap_or(0),
        dark_theme_id: options.dark_theme_id,
        pad: config.pad,
        center: config.center,
        layout_engine: config.layout_engine.clone(),
        theme_overrides: HashThemeOverrides::from_map(&config.theme_overrides),
        dark_theme_overrides: HashThemeOverrides::from_map(&config.dark_theme_overrides),
        data: config.data.clone(),
    })
}

/// Reconstructs the v0.9.0 target hash input for the fields represented by the
/// native graph/scene boundary.  This is deliberately kept separate from SVG
/// serialization: D2 hashes compact target JSON, then appends the optional
/// salt before applying FNV-1a 32.
fn target_hash_bytes(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Vec<u8> {
    let shapes = target_node_ids(graph)
        .into_iter()
        .filter_map(|id| target_hash_shape(graph, result, labels, styles, id))
        .collect::<Vec<_>>();
    let mut connections = graph
        .edges()
        .filter_map(|(id, _)| target_hash_connection(graph, result, id))
        .collect::<Vec<_>>();
    connections.extend(target_hash_lifelines(graph, result, styles));
    let root = target_hash_root(graph, options);
    let config = HashConfig {
        sketch,
        theme_id: options.theme_id.unwrap_or(0),
        dark_theme_id: options.dark_theme_id,
        pad: graph.d2_config_pad,
        center: graph.d2_config_center,
        layout_engine: graph.d2_config_layout_engine.clone(),
        theme_overrides: HashThemeOverrides::from_map(&options.theme_overrides),
        dark_theme_overrides: HashThemeOverrides::from_map(&options.dark_theme_overrides),
        data: graph.d2_config_data.clone(),
    };
    let mut bytes = Vec::new();
    for value in [
        serde_json::to_vec(&shapes).unwrap_or_default(),
        serde_json::to_vec(&connections).unwrap_or_default(),
        serde_json::to_vec(&root).unwrap_or_default(),
    ] {
        bytes.extend(escape_go_hash_json(&value));
    }
    if options.include_config_in_hash {
        bytes.extend(escape_go_hash_json(
            &serde_json::to_vec(&config).unwrap_or_default(),
        ));
    }
    #[cfg(feature = "diagnostic-traces")]
    if let Some(path) = std::env::var_os("D2_RS_TRACE_TARGET_BYTES") {
        if let Err(error) = std::fs::write(path, &bytes) {
            eprintln!("target byte trace failed: {error}");
        }
    }
    bytes
}

/// The public d2target.Diagram JSON for one laid-out board, built from the
/// same exporter-equivalent pieces as [`target_hash_input`] (so its
/// Diagram.Bytes reproduces the target hash). Legends are not yet exported
/// through this path and are rejected rather than dropped.
pub fn target_diagram_json(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, String> {
    let legend = match graph.legend.as_ref() {
        None => None,
        Some(legend) => match legend.source_graph.as_deref() {
            Some(source) => {
                target_legend_metadata(source, options.theme_id).map_err(|e| e.to_string())?
            }
            None => return Err("d2svg target export: legend without its source graph".into()),
        },
    };
    let shapes = target_node_ids(graph)
        .into_iter()
        .filter_map(|id| target_hash_shape(graph, result, labels, styles, id))
        .collect::<Vec<_>>();
    let mut connections = graph
        .edges()
        .filter_map(|(id, _)| target_hash_connection(graph, result, id))
        .collect::<Vec<_>>();
    connections.extend(target_hash_lifelines(graph, result, styles));
    let root = target_hash_root(graph, options);
    // d2exporter.Export: sketch compiles with HandDrawn; a mono theme
    // replaces the family with Source Code Pro.
    let font_family = if target_theme_rules(graph).mono {
        "SourceCodePro"
    } else if sketch || options.hand_drawn_font {
        "HandDrawn"
    } else {
        "SourceSansPro"
    };
    let mut diagram = serde_json::Map::new();
    diagram.insert("name".into(), Value::String(String::new()));
    if options.include_config_in_hash {
        let config = HashConfig {
            sketch,
            theme_id: options.theme_id.unwrap_or(0),
            dark_theme_id: options.dark_theme_id,
            pad: graph.d2_config_pad,
            center: graph.d2_config_center,
            layout_engine: graph.d2_config_layout_engine.clone(),
            theme_overrides: HashThemeOverrides::from_map(&options.theme_overrides),
            dark_theme_overrides: HashThemeOverrides::from_map(&options.dark_theme_overrides),
            data: graph.d2_config_data.clone(),
        };
        diagram.insert(
            "config".into(),
            serde_json::to_value(&config).map_err(|e| e.to_string())?,
        );
    }
    diagram.insert("isFolderOnly".into(), Value::Bool(false));
    diagram.insert("fontFamily".into(), Value::String(font_family.into()));
    diagram.insert(
        "monoFontFamily".into(),
        Value::String("SourceCodePro".into()),
    );
    diagram.insert(
        "shapes".into(),
        serde_json::to_value(&shapes).map_err(|e| e.to_string())?,
    );
    diagram.insert(
        "connections".into(),
        serde_json::to_value(&connections).map_err(|e| e.to_string())?,
    );
    diagram.insert(
        "root".into(),
        serde_json::to_value(&root).map_err(|e| e.to_string())?,
    );
    if let Some(legend) = legend {
        diagram.insert("legend".into(), legend);
    }
    serde_json::to_vec(&Value::Object(diagram)).map_err(|e| e.to_string())
}

/// Render one laid-out board through the byte-identical d2svg port (the
/// planned replacement for [`render_svg_with_sketch_checked`]; see PLAN S6).
pub fn render_d2svg(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, String> {
    let diagram = target::from_graph::export(graph, result, labels, styles, options, sketch)?;
    target::d2svg::render_cli_target(&diagram, options, sketch)
}

/// d2cli's `--force-appendix` for a d2svg board: appendix.Append.
pub fn d2svg_append_appendix(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    svg: &[u8],
) -> Result<Vec<u8>, String> {
    let json = target_diagram_json(graph, result, labels, styles, options, sketch)?;
    target::d2svg::append_cli_appendix(&json, options, svg)
}

/// Render a board tree through d2svg as d2cli does for animated SVG:
/// MasterID = HashID(root), RenderMultiboard, then d2animate.Wrap. Each frame
/// is `(board path, target JSON, renderable)`; paths nest like the hash's
/// board traversal (`layers.x`, `scenarios.y.steps.z`), and a non-renderable
/// board is folder-only.
pub fn render_d2svg_animated(
    root_path: &str,
    frames: &[(String, Vec<u8>, bool)],
    options: &RenderOptions,
    sketch: bool,
    interval_ms: u64,
) -> Result<Vec<u8>, String> {
    let tree = target_diagram_tree_json(root_path, frames)?;
    target::d2svg::render_cli_animated(&tree, options, sketch, interval_ms as i64)
}

/// Assemble exported boards into their ordered target tree. Source adapters
/// share this with CLI animation rather than reconstructing SVG or hash bytes.
pub fn target_diagram_tree_json(
    root_path: &str,
    frames: &[(String, Vec<u8>, bool)],
) -> Result<Vec<u8>, String> {
    let parts = |path: &str| -> Result<Vec<String>, String> {
        if path.is_empty() {
            return Ok(Vec::new());
        }
        d2_rs_language::parse_key(path.as_bytes())
            .ok()
            .and_then(|key| key.string_ida())
            .ok_or_else(|| format!("invalid board path {path:?}"))
    };
    // Paths are relative to the animated subtree's root board.
    let root_parts = parts(root_path)?;
    let mut nodes: Vec<(Vec<String>, Value)> = Vec::new();
    for (path, json, renderable) in frames {
        let mut value: Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let full = parts(path)?;
        if !full.starts_with(&root_parts) {
            continue;
        }
        let parts = full[root_parts.len()..].to_vec();
        if let Some(object) = value.as_object_mut() {
            object.insert("isFolderOnly".into(), Value::Bool(!renderable));
            if let Some(name) = parts.last() {
                object.insert("name".into(), Value::String(name.clone()));
            }
        }
        nodes.push((parts, value));
    }
    fn build(nodes: &[(Vec<String>, Value)], index: usize) -> Value {
        let (parts, value) = &nodes[index];
        let mut value = value.clone();
        for kind in ["layers", "scenarios", "steps"] {
            let children: Vec<Value> = nodes
                .iter()
                .enumerate()
                .filter(|(_, (child, _))| {
                    child.len() == parts.len() + 2
                        && child[..parts.len()] == parts[..]
                        && child[parts.len()] == kind
                })
                .map(|(i, _)| build(nodes, i))
                .collect();
            if !children.is_empty() {
                value[kind] = Value::Array(children);
            }
        }
        value
    }
    let root = nodes
        .iter()
        .position(|(parts, _)| parts.is_empty())
        .ok_or("animated SVG without a root board")?;
    serde_json::to_vec(&build(&nodes, root)).map_err(|e| e.to_string())
}

/// Returns the compact target bytes used by D2 v0.9.0 for a single board.
///
/// The animated SVG wrapper hashes the root diagram bytes followed by each
/// descendant board's bytes.  Keep this boundary public so the source/WASM
/// adapter can assemble that recursive stream without parsing SVG or making
/// serialization details part of the animation layer.
pub fn target_hash_input(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Vec<u8> {
    target_hash_bytes(graph, result, labels, styles, options, sketch)
}

/// Hashes a complete target byte stream with D2's FNV-1a 32 contract.
pub fn target_hash_id_from_input(bytes: &[u8], salt: Option<&str>) -> String {
    let mut hash = 2_166_136_261u32;
    for byte in bytes.iter().copied().chain(salt.unwrap_or("").bytes()) {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
    }
    format!("d2-{hash}")
}

/// Reconstructs the byte stream used by D2's `Diagram.Bytes` for a board
/// tree. A diagram serializes its own target bytes first, then appends each
/// direct child recursively in source/compiler order. The flat concatenation
/// used by the first animation implementation only happened to work for one
/// level of boards and produced a different hash for nested layers/scenarios.
pub fn recursive_target_hash_input(boards: &[(String, Vec<u8>)]) -> Vec<u8> {
    fn path_parts(path: &str) -> Option<Vec<String>> {
        if path.is_empty() {
            return Some(Vec::new());
        }
        d2_rs_language::parse_key(path.as_bytes())
            .ok()?
            .string_ida()
    }

    fn append_board(
        index: usize,
        boards: &[(String, Vec<u8>)],
        paths: &[Option<Vec<String>>],
        children: &BTreeMap<Vec<String>, Vec<usize>>,
        output: &mut Vec<u8>,
    ) {
        output.extend(&boards[index].1);
        if let Some(parts) = &paths[index]
            && let Some(indices) = children.get(parts)
        {
            for child in indices {
                append_board(*child, boards, paths, children, output);
            }
        }
    }

    // Parse each key once. Parent lookup must respect quoted key components;
    // reparsing every candidate during every recursive visit is unnecessary.
    let paths = boards
        .iter()
        .map(|(path, _)| path_parts(path))
        .collect::<Vec<_>>();
    let mut children: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
    for (index, parts) in paths.iter().enumerate() {
        if let Some(parts) = parts
            && parts.len() >= 2
        {
            children
                .entry(parts[..parts.len() - 2].to_vec())
                .or_default()
                .push(index);
        }
    }
    let mut output = Vec::new();
    if !boards.is_empty() {
        let root = boards
            .iter()
            .position(|(path, _)| path.is_empty())
            .unwrap_or(0);
        append_board(root, boards, &paths, &children, &mut output);
    }
    output
}

fn target_hash_id(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    salt: &str,
) -> u32 {
    let mut bytes = target_hash_bytes(graph, result, labels, styles, options, sketch);
    bytes.extend(salt.as_bytes());
    let mut hash = 2_166_136_261u32;
    for byte in bytes {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
    }
    hash
}

/// Appends a stable, XML-safe suffix to every generated SVG ID and to all
/// references to those IDs.  The target emits IDs only in the forms handled
/// here (`id="…"`, `url(#…)`, and fragment hrefs), so doing the rewrite once
/// after scene construction keeps the hot drawing loops allocation-free for
/// ordinary renders.
fn salt_svg_ids(svg: &mut String, salt: &str) {
    let suffix = format!("-{}", salt_id_suffix(salt));
    let mut output = String::with_capacity(svg.len() + suffix.len() * 8);
    let bytes = svg.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor..].starts_with(b"id=\"") {
            output.push_str("id=\"");
            cursor += 4;
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'\"' {
                cursor += 1;
            }
            let id = &svg[start..cursor];
            output.push_str(id);
            if !is_upstream_hash_id(id) {
                output.push_str(&suffix);
            }
            if cursor < bytes.len() {
                output.push('"');
                cursor += 1;
            }
            continue;
        }
        if bytes[cursor..].starts_with(b"url(#") {
            output.push_str("url(#");
            cursor += 5;
            let start = cursor;
            while cursor < bytes.len()
                && !matches!(
                    bytes[cursor],
                    b')' | b'"' | b'\'' | b' ' | b'\n' | b'\r' | b'\t'
                )
            {
                cursor += 1;
            }
            let id = &svg[start..cursor];
            output.push_str(id);
            if !is_upstream_hash_id(id) {
                output.push_str(&suffix);
            }
            continue;
        }
        if bytes[cursor..].starts_with(b"href=\"#") {
            output.push_str("href=\"#");
            cursor += 7;
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'\"' {
                cursor += 1;
            }
            let id = &svg[start..cursor];
            output.push_str(id);
            if !is_upstream_hash_id(id) {
                output.push_str(&suffix);
            }
            if cursor < bytes.len() {
                output.push('"');
                cursor += 1;
            }
            continue;
        }
        let character = svg[cursor..].chars().next().expect("UTF-8 cursor");
        output.push(character);
        cursor += character.len_utf8();
    }
    *svg = output;
}

fn is_upstream_hash_id(id: &str) -> bool {
    id.strip_prefix("mk-d2-")
        .is_some_and(|rest| !rest.is_empty())
        || id
            .strip_prefix("d2-")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()))
}

fn salt_id_suffix(salt: &str) -> String {
    // Keep arbitrary user input out of XML identifiers while preserving a
    // readable value for the common alphanumeric case.
    if salt
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return salt.to_owned();
    }
    let mut encoded = String::with_capacity(salt.len() * 2);
    for byte in salt.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    #[test]
    fn markdown_target_emphasis_and_underline_follow_original_svg_contract() {
        for (mono, sketch) in [(false, false), (true, true), (false, true)] {
            for text in ["Agm", "`Agm`", "plain `Agm` text", "**bold** *italic*"] {
                let draw = |bold, italic, underline, private| {
                    let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
                    super::draw_markdown_label(
                        &mut svg,
                        text,
                        Rect {
                            origin: Point { x: 0.0, y: 0.0 },
                            size: Size {
                                width: 200.0,
                                height: 40.0,
                            },
                        },
                        16,
                        LabelPosition::InsideMiddleCenter,
                        "#0A0F25",
                        None,
                        None,
                        None,
                        super::TextStyle {
                            bold,
                            italic,
                            underline,
                            mono,
                        },
                        None,
                        private,
                        if sketch {
                            super::MarkdownFamily::HandDrawn
                        } else {
                            super::MarkdownFamily::Sans
                        },
                    );
                    svg.push_str("</svg>");
                    svg
                };
                let regular = draw(false, false, false, false);
                assert!(!regular.contains("foreignObject"));
                for (bold, italic) in [(true, false), (false, true), (true, true)] {
                    assert_eq!(
                        draw(bold, italic, false, false),
                        regular,
                        "Go's Markdown layout context does not inherit target emphasis"
                    );
                }
                for private in [false, true] {
                    let plain = draw(false, false, false, private);
                    let doc = roxmltree::Document::parse(&plain).unwrap();
                    for text in doc.descendants().filter(|n| n.has_tag_name("text")) {
                        assert_eq!(text.attribute("data-d2-rs-text-x-bits").is_some(), private);
                        assert_eq!(text.attribute("data-d2-rs-text-y-bits").is_some(), private);
                        assert!(text.attribute("text-decoration").is_none());
                    }
                    assert_eq!(plain.contains("data-d2-rs-markdown-reference"), private);
                    let underlined = draw(true, true, true, private);
                    let doc = roxmltree::Document::parse(&underlined).unwrap();
                    let texts = doc
                        .descendants()
                        .filter(|n| n.has_tag_name("text"))
                        .collect::<Vec<_>>();
                    assert!(!texts.is_empty());
                    for text in texts {
                        assert_eq!(text.attribute("text-decoration"), Some("underline"));
                        assert_eq!(text.attribute("data-d2-rs-text-x-bits").is_some(), private);
                        assert_eq!(text.attribute("data-d2-rs-text-y-bits").is_some(), private);
                    }
                }
            }
        }
        let mut decorated = String::from(
            "<text id=\"existing\" x=\"0\" y=\"16\" text-decoration=\"line-through\">Agm</text>",
        );
        super::prepare_native_markdown_text(&mut decorated, 0, true, true);
        let doc = roxmltree::Document::parse(&decorated).unwrap();
        assert_eq!(
            doc.root_element().attribute("text-decoration"),
            Some("underline line-through")
        );
        assert_eq!(doc.root_element().attribute("id"), Some("existing"));
    }
    use super::{
        FontNeeds, RenderOptions, append_static_appendix, append_static_appendix_with_theme,
        arrowhead_adjustment, arrowhead_label_bounds, cloud_content_aspect, default_node_fill,
        default_node_stroke, draw_native_markdown_subset, font_needs_from_svg, marker_hash,
        marker_id, marker_reference, modifier_adjusted_label_rect, node_label_box,
        recursive_target_hash_input, render_svg, render_svg_with_options, render_svg_with_sketch,
        replace_static_appendix_badges, rounded_render_rect, route_path_data_with_radius,
        salt_svg_ids, svg_object_class, target_hash_bytes, target_hash_id, themed_node_style,
        validate_markdown_labels,
    };
    use d2_rs_core::{
        ArrowheadLabel, Edge, EdgeArrowheadLabels, EdgeArrows, EdgeStyle, Graph, LabelPosition,
        LayoutReport, LayoutResult, Node, NodeId, Point, Rect, Score, ShapeKind, Size,
    };
    use d2_rs_language::compiler::{SourceNodeStyle, SourceSqlColumn, SourceTargetAccents};
    use d2_tala_rs::{Engine, LayoutOptions};
    use std::collections::BTreeMap;

    #[test]
    fn go_opacity_spellings_share_export_value_and_native_svg() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "node",
            Size {
                width: 100.0,
                height: 60.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::from([(node, "Opacity text".to_owned())]);
        let mut expected = None;
        for value in [
            "0.5",
            "0x1p-1",
            "0x_1.0p-1",
            "0.5_0",
            "5e-1",
            "+0.5",
            "0x.8p0",
        ] {
            let styles = BTreeMap::from([(
                node,
                SourceNodeStyle {
                    opacity: Some(value.into()),
                    ..Default::default()
                },
            )]);
            let (svg, _) = super::render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
            )
            .unwrap();
            assert_eq!(
                super::parse_hash_f64(Some(value), 1.0).to_bits(),
                0.5f64.to_bits()
            );
            if let Some(expected) = &expected {
                assert_eq!(&svg, expected, "{value}");
            } else {
                expected = Some(svg);
            }
        }
    }

    #[test]
    fn caps_lock_applies_only_exact_source_text_transform_spellings() {
        let overrides = BTreeMap::new();
        let palette = super::PaletteView::new(super::active_palette(Some(300)), &overrides);
        for (transform, expected) in [
            ("none", "MiXeD Words"),
            ("NONE", "MIXED WORDS"),
            ("uppercase", "MIXED WORDS"),
            ("UPPERCASE", "MIXED WORDS"),
            ("lowercase", "mixed words"),
            ("LOWERCASE", "MIXED WORDS"),
            ("capitalize", "Mixed Words"),
            ("CAPITALIZE", "MIXED WORDS"),
        ] {
            let node_style = SourceNodeStyle {
                text_transform: Some(transform.into()),
                ..Default::default()
            };
            let edge_style = EdgeStyle {
                text_transform: Some(transform.into()),
                ..Default::default()
            };
            assert_eq!(
                super::theme_label(
                    "MiXeD Words",
                    Some(&node_style),
                    ShapeKind::Rectangle,
                    palette
                ),
                expected,
                "node: {transform}"
            );
            assert_eq!(
                super::theme_edge_label("MiXeD Words", &edge_style, palette),
                expected,
                "edge: {transform}"
            );
        }
    }

    #[test]
    fn static_appendix_replaces_interactive_badges_with_numbers() {
        let svg = b"<svg><g transform=\"translate(1 2)\" class=\"appendix-icon\" style=\"filter:drop-shadow(0px 0px 32px rgba(31,36,58,0.1));\"><svg width=\"32\" height=\"32\"><path d=\"M0 0\"/></svg></g></svg>";
        let numbered = replace_static_appendix_badges(svg, &[Some(3)], true);
        assert!(String::from_utf8_lossy(&numbered).contains(">3</text>"));
        assert!(String::from_utf8_lossy(&numbered).contains("id=\"d2-rs-static-appendix-icon-1\""));
        assert!(String::from_utf8_lossy(&numbered).contains("font-weight=\"bold\""));
        assert!(String::from_utf8_lossy(&numbered).contains("fill=\"#000000\""));
        assert!(!String::from_utf8_lossy(&numbered).contains("text-bold d2-rs-text-N1"));
        assert!(!String::from_utf8_lossy(&numbered).contains("<svg width=\"32\""));

        let hidden = replace_static_appendix_badges(svg, &[Some(1)], false);
        assert!(!String::from_utf8_lossy(&hidden).contains("<svg width=\"32\""));
    }

    #[test]
    fn static_appendix_keeps_definition_badges_and_measured_blank_lines() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new(
            "parent",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let mut child_node = Node::new(
            "child",
            Size {
                width: 40.0,
                height: 40.0,
            },
        );
        child_node.parent = Some(parent);
        let child = graph.add_node(child_node);
        let other = graph.add_node(Node::new(
            "other",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let styles = BTreeMap::from([
            (
                child,
                SourceNodeStyle {
                    tooltip: Some("first\n\nthird<&>".into()),
                    ..Default::default()
                },
            ),
            (
                other,
                SourceNodeStyle {
                    tooltip: Some("second".into()),
                    ..Default::default()
                },
            ),
        ]);
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let interactive = render_svg(&graph, &result, &BTreeMap::new(), &styles);
        let interactive = String::from_utf8(interactive).unwrap();
        let rendered = String::from_utf8(append_static_appendix(
            interactive.as_bytes(),
            &graph,
            &result,
            &styles,
        ))
        .unwrap();
        let document = roxmltree::Document::parse(&rendered).unwrap();
        let appendix = document
            .descendants()
            .find(|node| node.attribute("class") == Some("appendix"))
            .unwrap();
        let rows = appendix
            .children()
            .filter(|node| node.has_tag_name("text"))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].text(), Some("second"));
        let spans = rows[0]
            .children()
            .filter(|node| node.has_tag_name("tspan"))
            .collect::<Vec<_>>();
        assert_eq!(
            spans
                .iter()
                .map(|node| node.text().unwrap())
                .collect::<Vec<_>>(),
            vec!["first", " ", "third<&>"]
        );
        assert_eq!(spans[1].attribute("dy"), Some("17.666667"));
        let first_y: f64 = rows[0].attribute("y").unwrap().parse().unwrap();
        let second_y: f64 = rows[1].attribute("y").unwrap().parse().unwrap();
        assert_eq!(second_y - first_y, 73.0); // Original Go control: 53px +20.
        let badges = document
            .descendants()
            .filter(|node| {
                node.attribute("id")
                    .is_some_and(|id| id.starts_with("d2-rs-static-appendix-icon-"))
            })
            .filter_map(|node| {
                node.children()
                    .find(|child| child.has_tag_name("text"))
                    .and_then(|child| child.text())
            })
            .take(2)
            .collect::<Vec<_>>();
        assert_eq!(badges, vec!["2", "1"]);
        let before = roxmltree::Document::parse(&interactive).unwrap();
        let height = |doc: &roxmltree::Document<'_>| {
            doc.root_element()
                .attribute("viewBox")
                .unwrap()
                .split_whitespace()
                .nth(3)
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        assert_eq!(height(&document) - height(&before), 195.0);
    }

    #[test]
    fn static_appendix_supplies_metadata_and_digit_glyphs_outside_label_subsets() {
        use super::{TrueTypeFont, bundled_source_sans_pro_bold, bundled_source_sans_pro_regular};
        use base64::Engine as _;
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "A",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                tooltip: Some("ρν".into()),
                ..Default::default()
            },
        )]);
        let interactive = render_svg(&graph, &result, &BTreeMap::new(), &styles);
        let rendered = String::from_utf8(append_static_appendix(
            &interactive,
            &graph,
            &result,
            &styles,
        ))
        .unwrap();
        for (family, expected, glyphs) in [
            (
                "d2-rs-appendix-font-regular",
                bundled_source_sans_pro_regular().unwrap(),
                "ρν",
            ),
            (
                "d2-rs-appendix-font-bold",
                bundled_source_sans_pro_bold().unwrap(),
                "1234567890",
            ),
        ] {
            let start = rendered
                .find(&format!("@font-face{{font-family:\"{family}\";"))
                .unwrap();
            let face = &rendered[start..];
            let encoded = face
                .split("data:font/ttf;base64,")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap();
            let data = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            let actual = TrueTypeFont::parse(&data).unwrap();
            for glyph in glyphs.chars() {
                assert_eq!(actual.advance(glyph, 16.0), expected.advance(glyph, 16.0));
                assert_ne!(
                    actual.advance(glyph, 16.0),
                    actual.advance('\u{10ffff}', 16.0)
                );
            }
        }
        assert!(
            rendered.contains(".appendix > text{font-family:\"d2-rs-appendix-font-regular\";}")
        );
        assert!(
            rendered.contains(".appendix-icon > text{font-family:\"d2-rs-appendix-font-bold\";}")
        );
    }

    #[test]
    fn svg_appendix_without_rows_preserves_interactive_link_icons() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "linked",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                link: Some("https://example.com".into()),
                pretty_link: Some(String::new()),
                ..Default::default()
            },
        )]);
        let svg = render_svg(&graph, &result, &BTreeMap::new(), &styles);
        assert_eq!(append_static_appendix(&svg, &graph, &result, &styles), svg);
    }
    #[test]
    fn static_appendix_separator_uses_the_theme_b2_color() {
        let mut graph = Graph::default();
        let node_id = graph.add_node(Node::new(
            "linked",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let styles = BTreeMap::from([(
            node_id,
            SourceNodeStyle {
                link: Some("https://example.com".into()),
                pretty_link: Some("example.com".into()),
                ..SourceNodeStyle::default()
            },
        )]);
        let interactive = render_svg(&graph, &result, &BTreeMap::new(), &styles);
        let static_svg = append_static_appendix(&interactive, &graph, &result, &styles);
        let static_svg = String::from_utf8(static_svg).unwrap();

        assert!(static_svg.contains("class=\"d2-rs-stroke-B2\""));
        assert!(static_svg.contains("id=\"d2-rs-static-appendix-icon-1\""));
        assert!(static_svg.contains("class=\"text-bold\" x=\"16\" y=\"5\" fill=\"#000000\""));
        assert!(!static_svg.contains("stroke=\"#9499AB\""));

        // Go appendix.Append does not apply RenderOpts theme font rules.
        // New pinned render-only terminal controls retain the text/text-bold
        // families; theme application belongs to export, before raw Render.
        let terminal_svg =
            append_static_appendix_with_theme(&interactive, &graph, &result, &styles, Some(300));
        assert!(
            String::from_utf8_lossy(&terminal_svg)
                .contains("class=\"text d2-rs-text-N1\" x=\"48\"")
        );
        assert!(
            String::from_utf8_lossy(&terminal_svg)
                .contains("class=\"text-bold\" x=\"16\" y=\"21\" fill=\"#000000\"")
        );
    }

    #[test]
    fn hexagon_label_painting_uses_target_integer_half_offset() {
        let rect = Rect {
            origin: Point { x: 10.0, y: 20.0 },
            size: Size {
                width: 100.0,
                height: 80.0,
            },
        };
        for position in [
            LabelPosition::OutsideTopCenter,
            LabelPosition::OutsideRightMiddle,
            LabelPosition::BorderRightMiddle,
        ] {
            let painted =
                modifier_adjusted_label_rect(rect, ShapeKind::Hexagon, position, true, true);
            assert_eq!(painted.origin, Point { x: 10.0, y: 13.0 });
            assert_eq!(
                painted.size,
                Size {
                    width: 115.0,
                    height: 87.0
                }
            );
        }
        assert_eq!(
            modifier_adjusted_label_rect(
                rect,
                ShapeKind::Hexagon,
                LabelPosition::InsideMiddleCenter,
                true,
                true
            ),
            rect
        );
        // BoundingBox uses a side-specific translation of the original label,
        // not the renderer's expanded box or TALA's fractional modifier.
        let label = Rect {
            origin: Point { x: 115.0, y: 30.5 },
            size: Size {
                width: 20.0,
                height: 15.0,
            },
        };
        let right = super::target_modifier_label_bounds(
            label,
            ShapeKind::Hexagon,
            LabelPosition::OutsideRightMiddle,
            true,
        );
        assert_eq!(right.origin, Point { x: 122.0, y: 30.5 });
        let top = super::target_modifier_label_bounds(
            label,
            ShapeKind::Hexagon,
            LabelPosition::OutsideTopCenter,
            true,
        );
        assert_eq!(top.origin, Point { x: 115.0, y: 23.5 });
        assert_eq!(
            super::target_modifier_label_bounds(
                label,
                ShapeKind::Hexagon,
                LabelPosition::BorderTopCenter,
                true
            ),
            label
        );
    }

    #[test]
    fn combined_hexagon_modifiers_each_expand_the_target_canvas() {
        let mut graph = Graph::default();
        let mut node = Node::new(
            "hexagon",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        node.shape = ShapeKind::Hexagon;
        let id = graph.add_node(node);
        let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        result.boxes.insert(
            id,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 100.0,
                    height: 80.0,
                },
            },
        );
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                three_dee: Some("true".into()),
                multiple: Some("true".into()),
                ..SourceNodeStyle::default()
            },
        )]);
        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &styles),
            (-1.0, -12.0, 117.0, 81.0)
        );
    }

    #[test]
    fn multiple_outside_label_anchor_includes_rear_shape() {
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 44.0,
                height: 66.0,
            },
        };
        let adjusted = modifier_adjusted_label_rect(
            rect,
            ShapeKind::Rectangle,
            LabelPosition::OutsideBottomCenter,
            false,
            true,
        );
        assert_eq!(adjusted.origin, Point { x: 0.0, y: -10.0 });
        assert_eq!(
            adjusted.size,
            Size {
                width: 54.0,
                height: 76.0
            }
        );
        assert_eq!(adjusted.center().x, 27.0);
        assert_eq!(adjusted.bottom(), rect.bottom());

        let inside = modifier_adjusted_label_rect(
            rect,
            ShapeKind::Rectangle,
            LabelPosition::InsideMiddleCenter,
            false,
            true,
        );
        assert_eq!(inside, rect);
    }

    #[test]
    fn route_target_coordinates_truncate_before_float32_conversion() {
        for value in [1.0029, -1.0029] {
            let expected = f64::from(if value > 0.0 { 1.002_f32 } else { -1.002_f32 });
            let actual = super::target_route_coordinate(value);
            assert_eq!(actual.to_bits(), expected.to_bits());
            assert_eq!(
                super::target_hash_route_float(value).0.to_bits(),
                expected.to_bits()
            );
        }
        // Applying the boundary twice truncates a second time after f32 lost
        // precision. Callers must always supply the original layout route.
        let converted = super::target_route_coordinate(1.0029);
        assert_ne!(
            super::target_route_coordinate(converted).to_bits(),
            converted.to_bits()
        );
    }

    #[test]
    fn crowfoot_raster_transform_uses_target_rounded_route_endpoint() {
        let raw_route = [
            Point {
                x: 467.333_333_333_333_3,
                y: 22.0,
            },
            Point {
                x: 467.333_333_333_333_3,
                y: 110.0,
            },
        ];
        let route = raw_route.map(super::target_route_point);
        let arrow = super::raster_crowfoot_arrow(
            "trace".into(),
            "cf-one",
            &route,
            &route,
            true,
            Point { x: 0.0, y: -4.0 },
            0.0,
            0.0,
            2.0,
            "#0D32B2",
        )
        .expect("crowfoot raster marker");
        let source_transform = arrow.paths[0]
            .source_transform
            .expect("marker source transform");
        assert_eq!(route[1].x.to_bits(), 467.333_007_812_5_f64.to_bits());
        assert_eq!(
            source_transform[4].to_bits(),
            476.333_007_812_5_f64.to_bits()
        );
    }

    #[test]
    fn arrowhead_label_rectangles_match_v090_horizontal_positions() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new("source", Size::default()));
        let target = graph.add_node(Node::new("target", Size::default()));
        let edge = graph.add_edge(Edge { source, target });
        graph.set_edge_arrows(
            edge,
            EdgeArrows {
                source: true,
                target: true,
            },
        );
        let label = ArrowheadLabel {
            text: "eth1".to_owned(),
            size: Size {
                width: 29.0,
                height: 21.0,
            },
            color: None,
        };
        graph.set_edge_arrowhead_labels(
            edge,
            EdgeArrowheadLabels {
                source: Some(label.clone()),
                target: Some(label),
            },
        );
        let route = [Point { x: 32.0, y: 16.0 }, Point { x: 132.0, y: 16.0 }];
        assert_eq!(
            arrowhead_label_bounds(&graph, edge, &route, false),
            Some(Rect {
                origin: Point { x: 37.0, y: -13.0 },
                size: Size {
                    width: 29.0,
                    height: 21.0,
                },
            })
        );
        assert_eq!(
            arrowhead_label_bounds(&graph, edge, &route, true),
            Some(Rect {
                origin: Point { x: 98.0, y: -13.0 },
                size: Size {
                    width: 29.0,
                    height: 21.0,
                },
            })
        );
    }

    #[test]
    fn edge_text_anchors_in_the_rounded_target_label_box() {
        let label = d2_rs_core::EdgeLabel {
            text: "edge".into(),
            size: Size {
                width: 41.0,
                height: 21.0,
            },
            position: LabelPosition::InsideMiddleCenter,
            // Explicit middle ignores this percentage; unlocked positions use it.
            percentage: 0.75,
        };
        let route = [Point { x: 0.25, y: 0.25 }, Point { x: 100.25, y: 0.25 }];
        let rect = super::edge_label_bounds(&route, &label, 2.0, false).unwrap();
        assert_eq!(rect.origin, Point { x: 30.0, y: -10.0 });
        let mut svg = String::new();
        super::draw_label(
            &mut svg,
            &label.text,
            rect,
            16,
            "black",
            None,
            false,
            false,
            true,
            false,
            None,
        );
        assert!(svg.contains("<text x=\"50.5\" y=\"6\" text-anchor=\"middle\""));
    }

    #[test]
    fn raster_single_line_node_and_edge_keep_original_origin_and_identity() {
        let rect = Rect {
            origin: Point {
                x: 10.123456789,
                y: 20.987654321,
            },
            size: Size {
                width: 41.0,
                height: 21.0,
            },
        };
        for node in [false, true] {
            let mut native = String::new();
            if node {
                super::draw_node_text_box(
                    &mut native,
                    "identical",
                    rect,
                    16,
                    "#0a0f25",
                    None,
                    super::TextStyle {
                        mono: true,
                        ..Default::default()
                    },
                    None,
                    Some("source-node"),
                );
            } else {
                super::draw_label(
                    &mut native,
                    "identical",
                    rect,
                    16,
                    "#0a0f25",
                    None,
                    false,
                    false,
                    false,
                    true,
                    Some("source-edge"),
                );
            }
            let lines = raster_line_origins_for_identical_text(&native);
            assert_eq!(lines.len(), 1);
            assert_eq!(
                lines[0].1,
                (rect.origin.x + rect.size.width / 2.0).to_bits()
            );
            assert_eq!(lines[0].2, (rect.origin.y + 16.0).to_bits());
            assert_eq!(
                lines[0].0,
                if node {
                    "source-node-line-0"
                } else {
                    "source-edge-line-0"
                }
            );
        }
        let (public, native) = raster_multiline_origin_witness("identical");
        assert!(!public.contains("data-d2-rs-text-x-bits"));
        assert_eq!(raster_line_origins_for_identical_text(&native).len(), 1);
    }

    #[test]
    fn multiline_node_text_uses_measured_height_and_keeps_blank_lines() {
        for (text, height, expected) in [
            (
                "Ring\nBuffer",
                73.0,
                "<tspan x=\"30\" dy=\"0\">Ring</tspan><tspan x=\"30\" dy=\"36.5\">Buffer</tspan>",
            ),
            (
                "first\n\nthird",
                63.0,
                "<tspan x=\"30\" dy=\"0\">first</tspan><tspan x=\"30\" dy=\"21\"> </tspan><tspan x=\"30\" dy=\"21\">third</tspan>",
            ),
        ] {
            let mut svg = String::new();
            super::draw_node_text_box(
                &mut svg,
                text,
                Rect {
                    origin: Point { x: 10.0, y: 20.0 },
                    size: Size {
                        width: 40.0,
                        height,
                    },
                },
                32,
                "#000000",
                None,
                super::TextStyle {
                    bold: true,
                    mono: true,
                    ..Default::default()
                },
                None,
                None,
            );
            assert!(svg.contains(expected), "{svg}");
            assert!(svg.contains("y=\"52\""));
        }
    }

    #[test]
    fn multiline_edge_text_uses_the_measured_label_height() {
        let mut svg = String::new();
        super::draw_label(
            &mut svg,
            "first\n\nthird",
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 40.0,
                    height: 63.0,
                },
            },
            16,
            "black",
            None,
            false,
            false,
            true,
            false,
            None,
        );
        assert!(svg.contains("<text x=\"30\" y=\"36\""));
        assert!(svg.contains("<tspan x=\"30\" dy=\"0\">first</tspan><tspan x=\"30\" dy=\"21\"> </tspan><tspan x=\"30\" dy=\"21\">third</tspan>"));
    }

    fn raster_multiline_origin_witness(text: &str) -> (String, String) {
        raster_multiline_origin_witness_with_options(text, RenderOptions::default())
    }

    fn raster_multiline_origin_witness_with_options(
        text: &str,
        options: RenderOptions,
    ) -> (String, String) {
        let mut graph = Graph::default();
        let size = Size {
            width: 40.0,
            height: 30.0,
        };
        let source = graph.add_node(Node::new("source", size));
        let target = graph.add_node(Node::new("target", size));
        let edge = graph.add_edge(Edge { source, target });
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: text.to_owned(),
                size: Size {
                    width: 56.0,
                    height: 53.0,
                },
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    source,
                    Rect {
                        origin: Point { x: -40.0, y: 85.0 },
                        size,
                    },
                ),
                (
                    target,
                    Rect {
                        origin: Point { x: 200.0, y: 85.0 },
                        size,
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                edge,
                vec![Point { x: 0.0, y: 100.0 }, Point { x: 200.0, y: 100.0 }],
            )]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let public = super::render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            options.clone(),
        );
        let (native, _) = super::render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            options,
            false,
        )
        .expect("raster scene builds");
        (
            String::from_utf8(public).unwrap(),
            String::from_utf8(native).unwrap(),
        )
    }

    #[test]
    fn raster_canvas_reference_marks_only_native_main_viewport() {
        let (public, native) = raster_multiline_origin_witness("identical\nidentical\nidentical");
        assert!(!public.contains("data-d2-rs-canvas-reference"));
        assert!(!public.contains("d2-rs-native-canvas-viewport-"));
        let document = roxmltree::Document::parse(&native).unwrap();
        let marked = document
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.attribute("data-d2-rs-canvas-reference") == Some("ordinary")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            marked.len(),
            1,
            "only the renderer-authored main viewport is qualified"
        );
        let viewport = marked[0];
        assert_eq!(viewport.tag_name().name(), "svg");
        assert!(
            viewport
                .attribute("id")
                .unwrap()
                .starts_with("d2-rs-native-canvas-viewport-d2-")
        );
        assert!(
            viewport
                .attribute("class")
                .unwrap()
                .split_whitespace()
                .any(|class| class == "d2-svg")
        );
        assert_eq!(viewport.parent_element(), Some(document.root_element()));
    }

    #[test]
    fn raster_canvas_reference_marker_keeps_final_salted_identity() {
        let (public, native) = raster_multiline_origin_witness_with_options(
            "identical\nidentical\nidentical",
            RenderOptions {
                salt: Some("worker-2".to_owned()),
                ..RenderOptions::default()
            },
        );
        assert!(!public.contains("data-d2-rs-canvas-reference"));
        assert!(!public.contains("d2-rs-native-canvas-viewport-"));
        let document = roxmltree::Document::parse(&native).unwrap();
        let viewport = document
            .descendants()
            .find(|node| node.attribute("data-d2-rs-canvas-reference") == Some("ordinary"))
            .unwrap();
        let diagram_hash = viewport
            .attribute("class")
            .unwrap()
            .split_whitespace()
            .find(|class| class.starts_with("d2-"))
            .unwrap();
        assert_eq!(
            viewport.attribute("id").unwrap(),
            format!(
                "d2-rs-native-canvas-viewport-{diagram_hash}-{}",
                super::salt_id_suffix(diagram_hash)
            ),
            "the export parser will see the final salted wrapper identity"
        );
    }

    fn raster_line_origins_for_identical_text(svg: &str) -> Vec<(String, u64, u64)> {
        let document = roxmltree::Document::parse(svg).expect("native SVG parses");
        document
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == "text"
                    && node.text() == Some("identical")
                    && !node.children().any(|child| child.is_element())
            })
            .map(|node| {
                let id = node.attribute("id").expect("stable native line identity");
                let x = node
                    .attribute("data-d2-rs-text-x-bits")
                    .expect("exact scene X metadata");
                let y = node
                    .attribute("data-d2-rs-text-y-bits")
                    .expect("exact scene Y metadata");
                (
                    id.to_owned(),
                    u64::from_str_radix(x, 16).unwrap(),
                    u64::from_str_radix(y, 16).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn raster_multiline_origins_keep_go_f64_line_baselines() {
        let (public, native) = raster_multiline_origin_witness("identical\nidentical\nidentical");
        assert!(!public.contains("data-d2-rs-text-y-bits"));
        assert!(public.contains("dy=\"17.666667\""));
        let lines = raster_line_origins_for_identical_text(&native);
        assert_eq!(lines.len(), 3, "native scene retains each independent line");
        // Go centeredTextRuns: target top-left Y=74, font=16, height=53.
        // Route placement rounds the centered top-left 73.5 before painting.
        // These values remain in float64 instead of accumulating SVG dy.
        let expected_y = [90.0_f64, 107.666_666_666_666_67, 125.333_333_333_333_34];
        for (line, expected_y) in lines.iter().zip(expected_y) {
            assert_eq!(line.1, 100.0_f64.to_bits());
            assert_eq!(line.2, expected_y.to_bits());
        }
        assert_eq!(
            lines
                .iter()
                .map(|line| &line.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn raster_multiline_origins_preserve_blank_middle_line_index() {
        let (public, native) = raster_multiline_origin_witness("identical\n\nidentical");
        assert!(!public.contains("data-d2-rs-text-y-bits"));
        let lines = raster_line_origins_for_identical_text(&native);
        assert_eq!(
            lines.len(),
            2,
            "both non-empty lines retain their identities"
        );
        assert_eq!(lines[0].2, 90.0_f64.to_bits());
        assert_eq!(lines[1].2, 125.333_333_333_333_34_f64.to_bits());
        assert_ne!(lines[0].0, lines[1].0);
    }

    fn assert_native_ordinary_dash_operands(svg: &str, expected_primitives: usize) {
        let document = roxmltree::Document::parse(svg).expect("native SVG parses");
        let dashed = document
            .descendants()
            .filter(|node| node.is_element() && node.attribute("stroke-dasharray").is_some())
            .collect::<Vec<_>>();
        assert_eq!(dashed.len(), expected_primitives);
        let mut identities = std::collections::BTreeSet::new();
        for primitive in dashed {
            let operands = primitive
                .ancestors()
                .find(|node| node.attribute("data-d2-rs-dash-gap-bits").is_some())
                .expect("ordinary native stroke retains its source float64 operands");
            let bits = |name| {
                u64::from_str_radix(operands.attribute(name).expect("exact operand"), 16)
                    .expect("hexadecimal float64 bits")
            };
            assert_eq!(bits("data-d2-rs-dash-size-bits"), 10.0_f64.to_bits());
            // Original Go1.27.0 GetStrokeDashAttributes(2,5), captured by
            // the independent frozen Go native raster overlay.
            assert_eq!(bits("data-d2-rs-dash-gap-bits"), 0x4023_bb35_11f4_5fb3);
            assert_eq!(bits("data-d2-rs-dash-offset-bits"), 0.0_f64.to_bits());
            let id = primitive
                .attribute("id")
                .expect("stable primitive identity");
            assert!(!id.is_empty());
            assert!(identities.insert(id.to_owned()), "distinct primitive IDs");
        }
    }

    #[test]
    fn raster_ordinary_dash_shapes_keep_source_operands_and_distinct_primitives() {
        let mut graph = Graph::default();
        let size = Size {
            width: 100.0,
            height: 80.0,
        };
        let rectangle = graph.add_node(Node::new("rectangle", size));
        let page = graph.add_node(Node {
            shape: ShapeKind::Page,
            ..Node::new("page", size)
        });
        let style = SourceNodeStyle {
            stroke_width: Some("2".to_owned()),
            stroke_dash: Some("5".to_owned()),
            ..SourceNodeStyle::default()
        };
        let styles = BTreeMap::from([(rectangle, style.clone()), (page, style)]);
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let public = render_svg(&graph, &result, &BTreeMap::new(), &styles);
        assert!(!String::from_utf8_lossy(&public).contains("data-d2-rs-dash-"));
        let (native, _) = super::render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            RenderOptions::default(),
            false,
        )
        .unwrap();
        // Original shape_page.GetSVGPathData returns outer and inner paths;
        // shape_document returns only one. One rectangle and the two page
        // paths must remain independently identifiable with the same tuple.
        assert_native_ordinary_dash_operands(std::str::from_utf8(&native).unwrap(), 3);
    }

    #[test]
    fn raster_connection_opacity_covers_labels_and_geometry_once() {
        let mut graph = Graph::default();
        let size = Size {
            width: 40.0,
            height: 30.0,
        };
        let source = graph.add_node(Node::new("source", size));
        let target = graph.add_node(Node::new("target", size));
        let edge = graph.add_edge(Edge { source, target });
        graph.set_edge_style(
            edge,
            EdgeStyle {
                opacity: Some("0.35".into()),
                ..EdgeStyle::default()
            },
        );
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "owner label".into(),
                size: Size {
                    width: 80.0,
                    height: 20.0,
                },
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let (native, _) = super::render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
        )
        .unwrap();
        let document = roxmltree::Document::parse(std::str::from_utf8(&native).unwrap()).unwrap();
        let owners = document
            .descendants()
            .filter(|n| n.attribute("opacity") == Some("0.35"))
            .collect::<Vec<_>>();
        assert_eq!(owners.len(), 1);
        let owner = owners[0];
        assert!(owner.has_tag_name("g"));
        assert!(
            owner
                .descendants()
                .any(|n| n.has_tag_name("text") && n.text() == Some("owner label"))
        );
        assert!(owner.descendants().any(|n| n.has_tag_name("path")));
        assert_eq!(graph.edge_style(edge).opacity.as_deref(), Some("0.35"));
    }

    #[test]
    fn raster_ordinary_dash_connection_keeps_existing_path_identity() {
        let mut graph = Graph::default();
        let size = Size {
            width: 40.0,
            height: 30.0,
        };
        let source = graph.add_node(Node::new("source", size));
        let target = graph.add_node(Node::new("target", size));
        let edge = graph.add_edge(Edge { source, target });
        graph.set_edge_style(
            edge,
            EdgeStyle {
                stroke_width: Some("2".to_owned()),
                stroke_dash: Some("5".to_owned()),
                ..EdgeStyle::default()
            },
        );
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let public = render_svg(&graph, &result, &BTreeMap::new(), &BTreeMap::new());
        assert!(!String::from_utf8_lossy(&public).contains("data-d2-rs-dash-"));
        let (native, _) = super::render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
        )
        .unwrap();
        let native = std::str::from_utf8(&native).unwrap();
        assert_native_ordinary_dash_operands(native, 1);
        let document = roxmltree::Document::parse(native).unwrap();
        let path = document
            .descendants()
            .find(|node| node.attribute("stroke-dasharray").is_some())
            .unwrap();
        assert!(
            path.attribute("id")
                .unwrap()
                .contains("native-raster-edge-0")
        );
    }

    #[test]
    fn raster_straight_route_source_retains_captured_go_endpoint_bits() {
        // Original v0.9.0 connectionPath emits float64 MoveTo/LineTo even
        // without a cubic. These endpoints were captured from the actual
        // Jupyter red stroke, before scanline reference-origin rounding.
        let route = [
            Point {
                x: 1015.845_524_948_431,
                y: 602.878_382_803_169_3,
            },
            Point {
                x: 1061.577_237_525_784_5,
                y: 758.560_808_598_415_3,
            },
        ];
        let commands = super::route_path_source_commands(
            &route,
            Point::default(),
            Point::default(),
            10.0,
            false,
        )
        .expect("a straight scene route must retain its original commands");
        let [
            super::RasterPathCommand::MoveTo(start),
            super::RasterPathCommand::LineTo(end),
        ] = commands.as_slice()
        else {
            panic!("the original two-point scene path contains only a move and line")
        };
        for (actual, expected) in [(*start, route[0]), (*end, route[1])] {
            assert_eq!(actual.x.to_bits(), expected.x.to_bits());
            assert_eq!(actual.y.to_bits(), expected.y.to_bits());
            assert_ne!(actual.x, f64::from(actual.x as f32));
            assert_ne!(actual.y, f64::from(actual.y as f32));
        }
    }

    #[test]
    fn raster_straight_route_source_retains_adjustments_and_repeated_points() {
        // Source connectionPath keeps a line at a repeated corner, then
        // applies the endpoint arrow adjustments. Neither requires a cubic
        // or permission to narrow the scene coordinates to SVG/f32 values.
        let route = [
            Point { x: 10.0, y: 20.0 },
            Point { x: 10.0, y: 20.0 },
            Point { x: 80.0, y: 90.0 },
        ];
        let source_adjustment = Point {
            x: 1.0 / 3.0,
            y: -2.0 / 3.0,
        };
        let target_adjustment = Point {
            x: -4.0 / 7.0,
            y: 3.0 / 7.0,
        };
        let commands = super::route_path_source_commands(
            &route,
            source_adjustment,
            target_adjustment,
            10.0,
            false,
        )
        .expect("a repeated corner preserves the original line-only commands");
        let [
            super::RasterPathCommand::MoveTo(start),
            super::RasterPathCommand::LineTo(corner),
            super::RasterPathCommand::LineTo(end),
        ] = commands.as_slice()
        else {
            panic!("source emits one line for the repeated corner and one for the endpoint")
        };
        assert_eq!(*corner, route[1]);
        assert_eq!(
            start.x.to_bits(),
            (route[0].x + source_adjustment.x).to_bits()
        );
        assert_eq!(
            start.y.to_bits(),
            (route[0].y + source_adjustment.y).to_bits()
        );
        assert_eq!(
            end.x.to_bits(),
            (route[2].x + target_adjustment.x).to_bits()
        );
        assert_eq!(
            end.y.to_bits(),
            (route[2].y + target_adjustment.y).to_bits()
        );
    }

    fn view_box(svg: &[u8]) -> String {
        let text = std::str::from_utf8(svg).expect("SVG must be UTF-8");
        let start = text.find("viewBox=\"").expect("viewBox attribute");
        let value = &text[start + 10..];
        value[..value.find('"').expect("viewBox terminator")].to_owned()
    }

    #[test]
    fn sketch_keeps_diagram_and_appendix_canvas_bounds() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "a",
            Size {
                width: 140.0,
                height: 80.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::from([(node, "A".to_owned())]);
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                tooltip: Some("metadata AVgy 0123456789".to_owned()),
                link: Some("https://example.org/appendix".to_owned()),
                ..SourceNodeStyle::default()
            },
        )]);
        let canvas = |svg: &[u8]| {
            let document = roxmltree::Document::parse(std::str::from_utf8(svg).unwrap()).unwrap();
            let elements = document
                .descendants()
                .filter(|node| node.has_tag_name("svg"))
                .take(2)
                .collect::<Vec<_>>();
            let mut dimensions = elements
                .iter()
                .map(|node| {
                    ["width", "height", "viewBox"]
                        .map(|key| node.attribute(key).unwrap().to_owned())
                        .to_vec()
                })
                .collect::<Vec<_>>();
            let background = elements[1]
                .children()
                .find(|node| node.has_tag_name("rect"))
                .unwrap();
            dimensions.push(
                ["x", "y", "width", "height"]
                    .map(|key| background.attribute(key).unwrap().to_owned())
                    .to_vec(),
            );
            dimensions
        };
        for pad in [0.0, 20.0, 100.0] {
            for appendix in [false, true] {
                for raster in [false, true] {
                    let paint = |sketch| {
                        let options = RenderOptions {
                            pad,
                            scale: Some(1.5),
                            force_appendix: appendix,
                            ..RenderOptions::default()
                        };
                        if raster {
                            super::render_svg_with_raster_sources_checked(
                                &graph, &result, &labels, &styles, options, sketch,
                            )
                            .unwrap()
                            .0
                        } else {
                            render_svg_with_sketch(
                                &graph, &result, &labels, &styles, options, sketch,
                            )
                        }
                    };
                    assert_eq!(
                        canvas(&paint(false)),
                        canvas(&paint(true)),
                        "pad={pad} appendix={appendix} raster={raster}"
                    );
                }
            }
        }
    }

    #[test]
    fn svg_shape_boxes_use_d2target_truncation() {
        let rect = rounded_render_rect(Rect {
            origin: Point { x: 12.9, y: 7.9 },
            size: Size {
                width: 183.333,
                height: 166.667,
            },
        });
        assert_eq!(rect.origin, Point { x: 12.0, y: 7.0 });
        assert_eq!(
            rect.size,
            Size {
                width: 183.0,
                height: 166.0
            }
        );
    }

    #[test]
    fn canvas_bounds_use_the_truncated_target_rectangle() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "fractional",
            Size {
                width: 66.75,
                height: 66.75,
            },
        ));
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([(
                node,
                Rect {
                    origin: Point { x: -0.5, y: -0.5 },
                    size: Size {
                        width: 66.75,
                        height: 66.75,
                    },
                },
            )]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &BTreeMap::new()),
            (-1.0, -1.0, 67.0, 67.0)
        );
    }

    #[test]
    fn canvas_multiple_label_bounds_use_target_box_before_paint_offset() {
        let mut graph = Graph::default();
        let size = Size {
            width: 100.0,
            height: 80.0,
        };
        let node = graph.add_node(Node {
            label_position: LabelPosition::OutsideTopCenter,
            label_size: Some(Size {
                width: 20.0,
                height: 20.0,
            }),
            ..Node::new("multiple", size)
        });
        let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        result.boxes.insert(
            node,
            Rect {
                origin: Point::default(),
                size,
            },
        );
        result.node_labels.clear();
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                multiple: Some("true".into()),
                ..SourceNodeStyle::default()
            },
        )]);
        // d2target.BoundingBox reserves the rear silhouette (-12) but
        // computes label bounds from the original box (-20 - padding5).
        // SVG painting separately shifts that outside label by ten pixels.
        assert_eq!(
            super::bounds(
                &graph,
                &result,
                &BTreeMap::from([(node, "label".into())]),
                &styles
            ),
            (-1.0, -25.0, 112.0, 81.0)
        );
        let painted = modifier_adjusted_label_rect(
            result.boxes[&node],
            ShapeKind::Rectangle,
            LabelPosition::OutsideTopCenter,
            false,
            true,
        );
        assert_eq!(painted.origin.y, -10.0);
    }

    #[test]
    fn canvas_bounds_include_v090_shadow_translation() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "shadow",
            Size {
                width: 44.0,
                height: 66.0,
            },
        ));
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([(
                node,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 44.0,
                        height: 66.0,
                    },
                },
            )]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                shadow: Some("true".to_owned()),
                ..SourceNodeStyle::default()
            },
        )]);

        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &styles),
            (-1.0, -1.0, 48.0, 72.0)
        );
    }

    #[test]
    fn shape_opacity_covers_label_and_geometry_once() {
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new(
            "a",
            Size {
                width: 80.0,
                height: 60.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                opacity: Some("0.5".into()),
                fill: Some("red".into()),
                multiple: Some("true".into()),
                ..SourceNodeStyle::default()
            },
        )]);
        let svg = render_svg(
            &graph,
            &result,
            &BTreeMap::from([(id, "label".into())]),
            &styles,
        );
        let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
        let label = document
            .descendants()
            .find(|node| node.has_tag_name("text") && node.text() == Some("label"))
            .unwrap();
        let owner = label
            .ancestors()
            .find(|node| node.attribute("opacity").is_some())
            .unwrap();
        assert_eq!(owner.attribute("opacity"), Some("0.5"));
        assert!(owner.descendants().any(|node| node.has_tag_name("rect")));
        assert_eq!(
            owner
                .descendants()
                .filter(|node| node.attribute("opacity").is_some())
                .count(),
            1,
            "overlapping paints must share one opacity layer"
        );
        assert_eq!(
            styles[&id].opacity.as_deref(),
            Some("0.5"),
            "source style is retained"
        );
    }

    #[test]
    fn ordinary_label_bold_face_precedes_italic_like_source_svg() {
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new(
            "a",
            Size {
                width: 60.0,
                height: 60.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                bold: true,
                italic: true,
                ..SourceNodeStyle::default()
            },
        )]);
        let svg = render_svg(
            &graph,
            &result,
            &BTreeMap::from([(id, "label".to_owned())]),
            &styles,
        );
        let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
        let label = document
            .descendants()
            .find(|node| node.has_tag_name("text") && node.text() == Some("label"))
            .unwrap();
        assert_eq!(label.attribute("font-weight"), Some("bold"));
        assert_eq!(label.attribute("font-style"), None);
        assert_eq!(label.attribute("fill"), Some("#676C7E"));
        assert!(
            label
                .attribute("class")
                .unwrap()
                .split_ascii_whitespace()
                .any(|class| class == "text-bold")
        );
    }

    #[test]
    fn rendering_uses_live_layout_font_size_after_tala_prescale() {
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new(
            "step",
            Size {
                width: 127.0,
                height: 127.0,
            },
        ));
        graph.node_mut(id).unwrap().font_size = Some(16);
        let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        result.boxes.get_mut(&id).unwrap().origin = Point::default();
        result.node_labels.insert(
            id,
            d2_rs_core::NodeLabelState {
                font_size_committed: false,
                size: Some(Size {
                    width: 10.0,
                    height: 27.0,
                }),
                font_size: Some(20),
                position: LabelPosition::InsideMiddleCenter,
                icon_position: None,
            },
        );
        let svg = render_svg(
            &graph,
            &result,
            &BTreeMap::from([(id, "a".to_owned())]),
            &BTreeMap::new(),
        );
        let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
        let label = document
            .descendants()
            .find(|node| node.has_tag_name("text") && node.text() == Some("a"))
            .unwrap();
        assert_eq!(label.attribute("font-size"), Some("20"));
        assert_eq!(label.attribute("y"), Some("70"));
        assert_eq!(graph.node(id).unwrap().font_size, Some(16));
    }

    #[test]
    fn canvas_bounds_truncate_node_label_coordinates_like_the_go_target() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node {
            shape: ShapeKind::Circle,
            ..Node::new(
                "fractional-label",
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            )
        });
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([(
                node,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 10.0,
                        height: 10.0,
                    },
                },
            )]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::from([(
                node,
                d2_rs_core::NodeLabelState {
                    size: Some(Size {
                        width: 9.0,
                        height: 21.0,
                    }),
                    position: LabelPosition::InsideMiddleCenter,
                    ..d2_rs_core::NodeLabelState::default()
                },
            )]),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };

        assert_eq!(
            super::bounds(
                &graph,
                &result,
                &BTreeMap::from([(node, "a".into())]),
                &BTreeMap::new()
            ),
            (-1.0, -5.0, 11.0, 16.0)
        );
    }

    #[test]
    fn multiple_shape_canvas_bounds_reserve_full_rear_stroke() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "multiple",
            Size {
                width: 44.0,
                height: 66.0,
            },
        ));
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([(
                node,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 44.0,
                        height: 66.0,
                    },
                },
            )]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                multiple: Some("true".to_owned()),
                ..SourceNodeStyle::default()
            },
        )]);
        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &styles),
            (-1.0, -12.0, 56.0, 67.0)
        );
    }

    #[test]
    fn node_icons_follow_target_size_limits_and_box_alignment() {
        let rect = Rect {
            origin: Point { x: 10.0, y: 20.0 },
            size: Size {
                width: 100.0,
                height: 92.0,
            },
        };
        let outside = super::icon_rect(rect, ShapeKind::Rectangle, LabelPosition::OutsideTopLeft);
        assert_eq!(outside.origin, Point { x: 5.0, y: -31.0 });
        assert_eq!(
            outside.size,
            Size {
                width: 46.0,
                height: 46.0
            }
        );
        let inside = super::icon_rect(rect, ShapeKind::Rectangle, LabelPosition::InsideTopCenter);
        assert_eq!(inside.origin, Point { x: 37.0, y: 25.0 });
        let diamond = super::icon_rect(rect, ShapeKind::Diamond, LabelPosition::InsideMiddleCenter);
        assert_eq!(diamond.size.width, 23.0);
        let tiny = Rect {
            size: Size {
                width: 8.0,
                height: 8.0,
            },
            ..rect
        };
        assert_eq!(super::icon_size(tiny, LabelPosition::InsideTopLeft), 0.0);
        assert_eq!(super::icon_size(tiny, LabelPosition::OutsideTopLeft), 8.0);
    }

    #[test]
    fn image_outside_icon_envelope_uses_resolved_layout_position() {
        let mut graph = Graph::default();
        let id = graph.add_node(Node {
            shape: ShapeKind::Image,
            has_icon: true,
            ..Node::new(
                "image",
                Size {
                    width: 128.0,
                    height: 128.0,
                },
            )
        });
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([(
                id,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 128.0,
                        height: 128.0,
                    },
                },
            )]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::from([(
                id,
                d2_rs_core::NodeLabelState {
                    icon_position: Some(LabelPosition::OutsideBottomCenter),
                    ..d2_rs_core::NodeLabelState::default()
                },
            )]),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                icon: Some("image.svg".to_owned()),
                ..SourceNodeStyle::default()
            },
        )]);
        // BoundingBox retains this target icon envelope for image shapes,
        // although drawShape uses the asset as the image instead of drawing
        // another icon. 128 + 5px padding + 64px icon is the lower extent.
        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &styles),
            (-1.0, -1.0, 129.0, 197.0)
        );
    }

    #[test]
    fn cloud_label_uses_content_aspect_ratio_before_shape_aspect() {
        let node = Node {
            shape: ShapeKind::Cloud,
            ..Node::new(
                "cloud",
                Size {
                    width: 104.0,
                    height: 84.0,
                },
            )
        };
        let label_size = Size {
            width: 40.0,
            height: 21.0,
        };
        let label_box = node_label_box(
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: node.size,
            },
            ShapeKind::Cloud,
            LabelPosition::InsideMiddleCenter,
            label_size,
            cloud_content_aspect(&node, label_size),
        );
        assert!((label_box.origin.x - 31.588).abs() < 1e-9);
        assert!((label_box.origin.y - 47.516).abs() < 1e-9);
        let center = label_box.center();
        assert!((center.x - 51.588).abs() < 1e-9);
        assert!((center.y - 58.016).abs() < 1e-9);
    }

    #[test]
    fn native_markdown_subset_matches_d2_block_structure() {
        let source = "## Raster rendering\n\n- first item\n- **second** item\n\n`code` and [a link](https://example.com)";
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            source,
            Rect {
                origin: Point { x: 101.0, y: 101.0 },
                size: Size {
                    width: 174.0,
                    height: 147.0,
                },
            },
            16,
            "#0A0F25",
            Some("d2-rs-text-N1"),
        ));
        assert!(svg.contains("<svg x=\"101\" y=\"101\" width=\"174\" height=\"147\""));
        assert!(svg.contains("class=\"md md-native\""));
        assert!(svg.contains("text-semibold"));
        assert!(svg.contains("<rect x=\"15.000\" y=\"65.200\""));
        assert!(svg.contains("href=\"https://example.com\""));
        assert!(!svg.contains("foreignObject"));
    }

    #[test]
    fn latex_labels_use_in_process_mathjax_paths() {
        let mut svg = String::new();
        super::draw_text(
            &mut svg,
            r"\frac{1}{2}+x_i^2",
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 160.0,
                    height: 80.0,
                },
            },
            16,
            LabelPosition::InsideMiddleCenter,
            "#0A0F25",
            Some("latex"),
            super::TextStyle::default(),
            Some("d2-rs-text-N1"),
        );
        assert!(svg.contains("data-d2-language=\"latex\""));
        assert!(svg.contains("data-mml-node=\"math\""));
        assert!(svg.contains("<path data-c=\"31\""));
        assert!(
            !svg.contains("<text "),
            "latex should not fall back to text"
        );
    }

    #[test]
    fn native_markdown_inline_code_paints_chip_before_glyphs() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "`code` and [a link](https://example.com)",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 114.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        let chip = svg.find("class=\"fill-N6\"").expect("inline code chip");
        let glyph = svg.find(">code</text>").expect("inline code glyph");
        assert!(chip < glyph, "the chip must be painted before its glyphs");
        assert!(svg.contains("rx=\"6.000\""));
        assert!(svg.contains("font-family=\"Source Code Pro, monospace\""));
        assert!(svg.contains("href=\"https://example.com\""));
    }

    #[test]
    fn native_markdown_rejects_dangerous_link_schemes() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "# title\n\n- item\n\n[jump](javascript:alert(1))",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 120.0,
                    height: 100.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("javascript:"));
        assert!(!svg.contains("<a href="));
    }

    #[test]
    fn native_markdown_paints_list_only_and_table_blocks() {
        for source in [
            "- first\n- second",
            "| Status | Count |\n| ------ | ----- |\n| Done | 42 |",
        ] {
            let mut svg = String::new();
            assert!(draw_native_markdown_subset(
                &mut svg,
                source,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size {
                        width: 142.0,
                        height: 76.0,
                    },
                },
                16,
                "#0A0F25",
                None,
            ));
            assert!(!svg.contains("foreignObject"));
            assert!(svg.contains("class=\"md md-native\""));
        }
    }

    #[test]
    fn native_markdown_paints_paragraphs_and_ordered_lists() {
        let cases = [
            ("Plain paragraph.", 111.0, 24.0),
            ("Plain *italic*.", 74.0, 24.0),
            ("1. first\n2. second", 81.0, 52.0),
        ];
        for (source, width, height) in cases {
            let mut svg = String::new();
            assert!(draw_native_markdown_subset(
                &mut svg,
                source,
                Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: Size { width, height },
                },
                16,
                "#0A0F25",
                None,
            ));
            assert!(!svg.contains("foreignObject"));
            assert!(svg.contains("class=\"md md-native\""));
        }
    }

    #[test]
    fn native_markdown_paints_strikethrough_as_foreground_rule() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "before ~~strikethrough~~ after",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 220.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("foreignObject"));
        assert!(svg.contains(
            "<rect x=\"46.812\" y=\"12.000\" width=\"91.250\" height=\"1.000\" rx=\"0.000\" class=\"\" fill=\"#0A0F25\"/>"
        ));
    }

    #[test]
    fn native_markdown_paints_uri_autolinks_without_html_fallback() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "visit <https://example.com> now",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 205.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("foreignObject"));
        assert!(
            svg.contains(
                "<a href=\"https://example.com\" xlink:href=\"https://example.com\"><text"
            )
        );
        assert!(svg.contains(">https://example.com</text></a>"));
    }

    #[test]
    fn raster_markdown_retains_source_text_origin_precision() {
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 205.0,
                height: 24.0,
            },
        };
        let source = "visit <https://example.com> now";
        let mut public_svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut public_svg,
            source,
            rect,
            16,
            "#0A0F25",
            None,
        ));
        assert!(public_svg.contains("<text x=\"30.656\" y=\"18.000\""));

        let mut raster_svg = String::new();
        assert!(super::draw_native_markdown_subset_with_precision(
            &mut raster_svg,
            source,
            rect,
            16,
            "#0A0F25",
            None,
            true,
        ));
        assert!(raster_svg.contains("<text x=\"30.65625\" y=\"18\""));
    }

    #[test]
    fn native_markdown_paints_email_autolinks_with_mailto_href() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "email <test@example.com> now",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 203.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("foreignObject"));
        assert!(svg.contains(
            "<a href=\"mailto:test@example.com\" xlink:href=\"mailto:test@example.com\"><text"
        ));
        assert!(svg.contains(">test@example.com</text></a>"));
    }

    #[test]
    fn native_markdown_decodes_go_html_character_references() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "&copy; &nbsp; &#x1F642; &amp;copy;",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 220.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("&copy;"));
        assert!(svg.contains("©   🙂 &amp;copy;"));
    }

    #[test]
    fn native_markdown_uses_complete_go_entity_table_and_preserves_unknowns() {
        assert_eq!(
            super::native_markdown_unescape("&NotEqualTilde; &bogus; &#0;"),
            Some("≂̸ &bogus; �".to_owned())
        );
    }

    #[test]
    fn native_markdown_accepts_go_supported_raw_inline_elements() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "<b>bold</b> <em>italic</em> <s>gone</s> <strike>old</strike>",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 220.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("foreignObject"));
        // Concrete HTML emphasis uses the D2 role class; the v0.9.0 painter
        // reserves font-weight for synthetic emphasis axes.
        assert!(svg.contains("class=\"md-text text-bold"));
        assert!(svg.contains("text-italic"));
        assert!(svg.matches("height=\"1.000\"").count() >= 2);
    }

    #[test]
    fn native_markdown_paints_raw_html_line_breaks() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "<b>bold</b><br />line",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 32.0,
                    height: 48.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(svg.contains("y=\"18.000\""));
        assert!(svg.contains("y=\"42.000\""));
        assert!(!svg.contains("foreignObject"));
    }

    #[test]
    fn native_markdown_omits_unfetched_images() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "before ![build passing](https://example.invalid/status.svg) after",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 83.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(svg.contains(">before </text>"));
        assert!(svg.contains(">after</text>"));
        assert!(!svg.contains("example.invalid"));
        assert!(!svg.contains("build passing"));
        assert!(!svg.contains("foreignObject"));
    }

    #[test]
    fn checked_source_render_rejects_unsupported_markdown_html() {
        let node = NodeId(1);
        let mut labels = BTreeMap::new();
        labels.insert(node, "<u>under</u>".to_owned());
        let mut styles = BTreeMap::new();
        styles.insert(
            node,
            SourceNodeStyle {
                markdown: true,
                ..Default::default()
            },
        );
        let error = validate_markdown_labels(&labels, &styles).unwrap_err();
        assert_eq!(
            error.to_string(),
            "native Markdown SVG does not support HTML element <u>"
        );
    }

    #[test]
    fn native_markdown_preserves_nested_bold_italic_style() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "***bold italic*** and **_both_**",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 137.0,
                    height: 24.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        // Nested emphasis uses the source painter's concrete face plus
        // synthetic CSS axes.
        assert!(svg.contains("font-weight=\"bold\""));
        assert!(svg.contains("font-style=\"italic\""));
        assert!(!svg.contains("stroke-width=\"0.5\""));
        assert!(!svg.contains("skewX(-12)"));
        assert!(!svg.contains("foreignObject"));
    }

    #[test]
    fn native_markdown_paints_v090_special_blocks() {
        let mut svg = String::new();
        assert!(draw_native_markdown_subset(
            &mut svg,
            "> quoted text\n\n---\n\n```rust\nlet x = 1;\nprintln!(\"{x}\");\n```",
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 163.0,
                    height: 166.0,
                },
            },
            16,
            "#0A0F25",
            None,
        ));
        assert!(!svg.contains("foreignObject"));
        assert!(svg.contains("<rect x=\"0.000\" y=\"48.000\" width=\"163.000\" height=\"4.000\""));
        assert!(svg.contains("<rect x=\"0.000\" y=\"76.000\" width=\"163.000\" height=\"71.440\""));
        assert!(svg.contains("y=\"106.360\""));
        assert!(svg.contains("y=\"126.080\""));
    }

    #[test]
    fn default_padding_is_kept_by_compatibility_wrapper() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::new();
        let svg = render_svg(&graph, &result, &labels, &BTreeMap::new());
        assert!(!String::from_utf8_lossy(&svg).contains("\\\\"));
        assert!(!String::from_utf8_lossy(&svg).contains("rx=\"4\""));
        assert_eq!(
            view_box(&svg),
            view_box(&render_svg_with_options(
                &graph,
                &result,
                &labels,
                &BTreeMap::new(),
                RenderOptions::default(),
            ))
        );
    }

    #[test]
    fn edge_path_matches_upstream_arrow_adjustment_for_straight_route() {
        let route = [Point { x: 10.0, y: 10.0 }, Point { x: 10.0, y: 110.0 }];
        let source = arrowhead_adjustment(route[1], route[0], false, 2.0, 2.0);
        let target = arrowhead_adjustment(route[0], route[1], true, 2.0, 2.0);
        assert_eq!(
            route_path_data_with_radius(&route, source, target, 10.0, false),
            "M 10 12 L 10 106"
        );
    }

    #[test]
    fn labelled_connections_emit_target_masks() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "source",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "target",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "masked".to_owned(),
                size: Size {
                    width: 48.0,
                    height: 16.0,
                },
                position: d2_rs_core::LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    source,
                    Rect {
                        origin: Point { x: 20.0, y: 10.0 },
                        size: graph.node(source).unwrap().size,
                    },
                ),
                (
                    target,
                    Rect {
                        origin: Point { x: 20.0, y: 110.0 },
                        size: graph.node(target).unwrap().size,
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                edge,
                vec![Point { x: 40.0, y: 40.0 }, Point { x: 40.0, y: 110.0 }],
            )]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let svg = String::from_utf8(render_svg(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
        ))
        .expect("SVG output is UTF-8");
        assert!(svg.contains("<mask id=\"d2-"));
        assert!(svg.contains("mask=\"url(#d2-"));
        assert!(svg.contains("fill=\"black\""));
    }

    // d2scenebuild.buildConnectionMask uses the label position when text is
    // present, otherwise the icon position. lib/label.IsOnEdge distinguishes
    // opaque middle-band holes from 75%-black holes beside the route.
    fn connection_mask_contract_svg(
        label_position: Option<LabelPosition>,
        icon_position: Option<LabelPosition>,
    ) -> String {
        let mut graph = Graph::default();
        let size = Size {
            width: 20.0,
            height: 20.0,
        };
        let source = graph.add_node(Node::new("source", size));
        let target = graph.add_node(Node::new("target", size));
        let edge = graph.add_edge(Edge { source, target });
        if let Some(position) = label_position {
            graph.set_edge_label(
                edge,
                Some(d2_rs_core::EdgeLabel {
                    text: "label".to_owned(),
                    size: Size {
                        width: 40.0,
                        height: 20.0,
                    },
                    position,
                    percentage: 0.5,
                }),
            );
        }
        if let Some(position) = icon_position {
            graph.set_edge_icon(edge, Some("icon.svg".to_owned()), Some(position), 0.0);
        }
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    source,
                    Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        size,
                    },
                ),
                (
                    target,
                    Rect {
                        origin: Point { x: 80.0, y: 0.0 },
                        size,
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                edge,
                vec![Point { x: 20.0, y: 10.0 }, Point { x: 80.0, y: 10.0 }],
            )]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let svg = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions {
                asset_data: BTreeMap::from([("icon.svg".to_owned(), b"<svg/>".to_vec())]),
                ..RenderOptions::default()
            },
        ))
        .expect("SVG output is UTF-8");
        let mask_start = svg.find("<mask ").expect("connection-label mask exists");
        let mask_end = svg[mask_start..]
            .find("</mask>")
            .expect("connection-label mask closes");
        svg[mask_start..mask_start + mask_end].to_owned()
    }

    #[test]
    fn raster_connection_mask_includes_shape_border_labels_and_omits_an_empty_mask() {
        for (position, expected) in [
            (LabelPosition::BorderTopCenter, Some([38.0, 9.0, 44.0, 2.0])),
            (
                LabelPosition::BorderBottomCenter,
                Some([38.0, 89.0, 44.0, 2.0]),
            ),
            (
                LabelPosition::BorderLeftMiddle,
                Some([9.0, 38.0, 2.0, 24.0]),
            ),
            (
                LabelPosition::BorderRightMiddle,
                Some([109.0, 38.0, 2.0, 24.0]),
            ),
            (LabelPosition::InsideMiddleCenter, None),
        ] {
            let mut graph = Graph::default();
            let size = Size {
                width: 100.0,
                height: 80.0,
            };
            let mut node = Node::new("source", size);
            node.label_size = Some(Size {
                width: 40.0,
                height: 20.0,
            });
            node.label_position = position;
            let source = graph.add_node(node);
            let target = graph.add_node(Node::new("target", size));
            let edge = graph.add_edge(Edge { source, target });
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes: BTreeMap::from([
                    (
                        source,
                        Rect {
                            origin: Point { x: 10.0, y: 10.0 },
                            size,
                        },
                    ),
                    (
                        target,
                        Rect {
                            origin: Point { x: 210.0, y: 10.0 },
                            size,
                        },
                    ),
                ]),
                routes: BTreeMap::from([(
                    edge,
                    vec![Point { x: 110.0, y: 50.0 }, Point { x: 210.0, y: 50.0 }],
                )]),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let (svg, _) = super::render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &BTreeMap::from([(source, "border label".to_owned())]),
                &BTreeMap::new(),
                RenderOptions::default(),
                false,
            )
            .unwrap();
            let svg = String::from_utf8(svg).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let mask = document
                .descendants()
                .find(|node| node.has_tag_name("mask"))
                .unwrap();
            let holes = mask
                .children()
                .filter(|node| node.attribute("fill") == Some("black"))
                .map(|node| {
                    ["x", "y", "width", "height"]
                        .map(|key| node.attribute(key).unwrap().parse::<f64>().unwrap())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                holes,
                expected.into_iter().collect::<Vec<_>>(),
                "{position:?}"
            );
            let route = document
                .descendants()
                .find(|node| {
                    node.attribute("id")
                        .is_some_and(|id| id.ends_with("native-raster-edge-0"))
                })
                .unwrap();
            assert_eq!(
                route
                    .ancestors()
                    .any(|node| node.attribute("mask").is_some()),
                expected.is_some(),
                "only a real diagram-wide hole creates the retained route layer"
            );
        }
    }

    #[test]
    fn connection_mask_off_edge_label_uses_partial_black() {
        for position in [
            LabelPosition::OutsideTopCenter,
            LabelPosition::OutsideBottomCenter,
        ] {
            let mask = connection_mask_contract_svg(Some(position), None);
            assert!(
                mask.contains("fill=\"rgba(0,0,0,0.75)\""),
                "off-edge label {position:?} must leave 25% connection luminance: {mask}"
            );
        }
    }

    #[test]
    fn connection_mask_on_edge_labels_remain_opaque() {
        for position in [
            LabelPosition::InsideMiddleLeft,
            LabelPosition::InsideMiddleCenter,
            LabelPosition::InsideMiddleRight,
            LabelPosition::UnlockedMiddle,
        ] {
            let mask = connection_mask_contract_svg(Some(position), None);
            assert!(mask.contains("fill=\"black\""), "{position:?}: {mask}");
            assert!(!mask.contains("rgba(0,0,0,0.75)"), "{position:?}: {mask}");
        }
    }

    #[test]
    fn connection_mask_icon_only_uses_icon_position() {
        let outside = connection_mask_contract_svg(None, Some(LabelPosition::OutsideTopCenter));
        assert!(
            outside.contains("fill=\"rgba(0,0,0,0.75)\""),
            "off-edge icon must use partial black: {outside}"
        );
        let middle = connection_mask_contract_svg(None, Some(LabelPosition::InsideMiddleCenter));
        assert!(middle.contains("fill=\"black\""), "{middle}");
        assert!(!middle.contains("rgba(0,0,0,0.75)"), "{middle}");
    }

    #[test]
    fn connection_mask_label_position_takes_precedence_over_icon_position() {
        let mask = connection_mask_contract_svg(
            Some(LabelPosition::InsideMiddleCenter),
            Some(LabelPosition::OutsideTopCenter),
        );
        assert!(mask.contains("fill=\"black\""), "{mask}");
        assert!(!mask.contains("rgba(0,0,0,0.75)"), "{mask}");
        let mask = connection_mask_contract_svg(
            Some(LabelPosition::OutsideTopCenter),
            Some(LabelPosition::InsideMiddleCenter),
        );
        assert!(
            mask.contains("fill=\"rgba(0,0,0,0.75)\""),
            "off-edge label must override on-edge icon: {mask}"
        );
    }

    #[test]
    fn edge_icons_follow_d2_label_gap_and_embed_assets() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "source",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "target",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "route".to_owned(),
                size: Size {
                    width: 40.0,
                    height: 21.0,
                },
                position: d2_rs_core::LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );
        graph.set_edge_icon(edge, Some("route.svg".to_owned()), None, 4.0);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    source,
                    Rect {
                        origin: Point { x: 20.0, y: 10.0 },
                        size: graph.node(source).unwrap().size,
                    },
                ),
                (
                    target,
                    Rect {
                        origin: Point { x: 20.0, y: 110.0 },
                        size: graph.node(target).unwrap().size,
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                edge,
                vec![Point { x: 40.0, y: 40.0 }, Point { x: 40.0, y: 110.0 }],
            )]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let svg = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions {
                asset_data: BTreeMap::from([("route.svg".to_owned(), b"<svg/>".to_vec())]),
                ..RenderOptions::default()
            },
        ))
        .expect("SVG output is UTF-8");
        assert!(svg.contains("width=\"32\" height=\"32\""));
        assert!(svg.contains("href=\"data:image/svg+xml;base64,PHN2Zy8+\""));
        let document = roxmltree::Document::parse(&svg).unwrap();
        let image = document
            .descendants()
            .find(|n| n.has_tag_name("image"))
            .unwrap();
        let clip_id = image
            .attribute("clip-path")
            .unwrap()
            .strip_prefix("url(#")
            .unwrap()
            .strip_suffix(')')
            .unwrap();
        let clip = document
            .descendants()
            .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some(clip_id))
            .unwrap();
        let path = clip
            .children()
            .find(|n| n.has_tag_name("path"))
            .unwrap()
            .attribute("d")
            .unwrap();
        assert_eq!(
            path.matches(" A 4 4 ").count(),
            4,
            "source ordinary-image clip uses four circular corners"
        );
        assert!(svg.contains("<mask id=\"d2-"));
    }

    #[test]
    fn orthogonal_route_uses_v090_corner_rounding() {
        let route = [
            Point { x: 55.0, y: 76.0 },
            Point { x: 89.0, y: 76.0 },
            Point { x: 89.0, y: 21.0 },
            Point { x: 122.0, y: 21.0 },
        ];
        assert_eq!(
            route_path_data_with_radius(
                &route,
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: 0.0 },
                10.0,
                false,
            ),
            "M 55 76 L 79 76 S 89 76 89 66 L 89 31 S 89 21 99 21 L 122 21"
        );
    }

    #[test]
    fn curved_route_preserves_cubic_control_points() {
        let route = [
            Point { x: 10.0, y: 10.0 },
            Point { x: 40.0, y: 10.0 },
            Point { x: 60.0, y: 20.0 },
            Point { x: 90.0, y: 30.0 },
        ];
        assert_eq!(
            route_path_data_with_radius(
                &route,
                Point { x: -2.0, y: 0.0 },
                Point { x: 0.0, y: 3.0 },
                10.0,
                true,
            ),
            "M 8 10 C 40 10 60 20 90 33"
        );
    }

    #[test]
    fn custom_arrowhead_markers_follow_v090_dimensions_and_color() {
        let overrides = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        let id = marker_reference(
            Some("circle"),
            true,
            4.0,
            "red",
            "red",
            "#0D32B2",
            None,
            super::MarkerTheme {
                palette: super::PaletteView::new(super::active_palette(None), &overrides),
                inline: true,
            },
            &mut definitions,
        )
        .expect("circle marker");
        let definition = definitions.get(&id).expect("dynamic marker definition");
        assert_eq!(id, "d2-rs-marker-circle-end-2167055256");
        assert!(definition.contains("markerWidth=\"28\" markerHeight=\"28\" refX=\"22\""));
        assert!(definition.contains("<circle r=\"10\" cx=\"16\" cy=\"14\" stroke=\"red\""));
        assert_eq!(marker_id(None, true), Some("triangle"));
        assert_eq!(marker_id(Some("arrow"), true), Some("arrow"));
    }

    #[test]
    fn crowfoot_arrowhead_markers_match_v090_geometry() {
        let cases = [
            (
                "cf-one",
                true,
                "<circle cx=\"7.1\" cy=\"13.5\" r=\"5.1\"",
                "<path d=\"M24,13.5 37.2,13.5 M20.4,0 20.4,27\"",
            ),
            (
                "cf-many",
                true,
                "<circle cx=\"7.1\" cy=\"13.5\" r=\"5.1\"",
                "<path d=\"M24,13.5 37.2,13.5 M13.2,13.5 37.2,0 M13.2,13.5 37.2,27\"",
            ),
            (
                "cf-one-required",
                false,
                "<path d=\"M10.2,0 10.2,27\"",
                "<path d=\"M24,13.5 37.2,13.5 M20.4,0 20.4,27\"",
            ),
            (
                "cf-many-required",
                false,
                "<path d=\"M10.2,0 10.2,27\"",
                "<path d=\"M24,13.5 37.2,13.5 M13.2,13.5 37.2,0 M13.2,13.5 37.2,27\"",
            ),
        ];

        let overrides = BTreeMap::new();
        for (name, is_target, modifier, branches) in cases {
            let mut definitions = BTreeMap::new();
            let id = marker_reference(
                Some(name),
                is_target,
                4.0,
                "red",
                "red",
                "#0D32B2",
                None,
                super::MarkerTheme {
                    palette: super::PaletteView::new(super::active_palette(None), &overrides),
                    inline: true,
                },
                &mut definitions,
            )
            .unwrap_or_else(|| panic!("{name} marker was not recognized"));
            let definition = definitions.get(&id).expect("crowfoot marker definition");
            let ref_x = if is_target { 21.0 } else { 6.0 };
            let geometry = format!(
                "markerWidth=\"27\" markerHeight=\"27\" refX=\"{}\" refY=\"13.5\" viewBox=\"0 0 27 27\"",
                super::number(ref_x)
            );
            assert!(definition.contains(&geometry), "{name}: {definition}");
            assert!(definition.contains(modifier), "{name}: {definition}");
            assert!(definition.contains(branches), "{name}: {definition}");
            if !is_target {
                assert!(
                    definition.contains("transform=\"scale(-1) translate(-27, -27)\""),
                    "{name}: {definition}"
                );
            }
        }

        assert_eq!(marker_id(Some("cf-one"), true), Some("cf-one"));
        assert_eq!(
            marker_id(Some("cf-many-required"), false),
            Some("cf-many-required-start")
        );
    }

    #[test]
    fn marker_hash_matches_v090_default_tuple() {
        // The bundled `a -> b` witness uses the default B1 stroke.  Go's
        // d2svg hash is 3488378134 for the target triangle tuple.
        assert_eq!(marker_hash("triangle", true, 2.0, "B1"), 3_488_378_134);
    }

    #[test]
    fn marker_paint_keeps_raw_theme_identity_when_colors_match() {
        let overrides = BTreeMap::new();
        let marker_theme = super::MarkerTheme {
            palette: super::PaletteView::new(super::active_palette(None), &overrides),
            inline: true,
        };
        let mut definitions = BTreeMap::new();
        let ids = ["B1", "#0D32B2"].map(|raw_stroke| {
            marker_reference(
                Some("triangle"),
                true,
                2.0,
                "#0D32B2",
                raw_stroke,
                "#0D32B2",
                Some("theme-contract"),
                marker_theme,
                &mut definitions,
            )
            .expect("triangle marker")
        });
        assert_ne!(ids[0], ids[1]);
        let symbolic = &definitions[&ids[0]];
        let literal = &definitions[&ids[1]];
        assert!(symbolic.contains("fill=\"#0D32B2\" class=\"connection fill-B1\""));
        assert!(literal.contains("fill=\"#0D32B2\" class=\"connection\""));
        assert!(!literal.contains("fill-B1"));
        assert!(!symbolic.contains(" stroke=\""));

        // A literal with the default RGB must also bypass the legacy unsalted
        // symbolic-marker shortcut.
        let literal_id = marker_reference(
            Some("triangle"),
            true,
            2.0,
            "#0D32B2",
            "#0D32B2",
            "#0D32B2",
            None,
            marker_theme,
            &mut definitions,
        )
        .expect("literal marker");
        assert!(definitions.contains_key(&literal_id));

        // Go recognizes only the exact symbolic codes, even when an override
        // happens to have the same key as another authored color.
        let overrides = BTreeMap::from([("RED".to_owned(), "#0D32B2".to_owned())]);
        let marker_theme = super::MarkerTheme {
            palette: super::PaletteView::new(super::active_palette(None), &overrides),
            inline: true,
        };
        for raw in ["red", "b1"] {
            let definition = super::dynamic_marker_definition(
                "literal",
                "triangle",
                true,
                2.0,
                raw,
                marker_theme,
            );
            assert!(definition.contains(&format!("fill=\"{raw}\" class=\"connection\"")));
        }
    }

    #[test]
    fn marker_classes_follow_v090_shape_paint_roles() {
        let overrides = BTreeMap::new();
        let marker_theme = super::MarkerTheme {
            palette: super::PaletteView::new(super::active_palette(None), &overrides),
            inline: true,
        };
        let cases = [
            ("arrow", "connection fill-B1", "#0D32B2", false),
            ("triangle", "connection fill-B1", "#0D32B2", false),
            ("filled-diamond", "connection fill-B1", "#0D32B2", false),
            ("filled-circle", "connection fill-B1", "#0D32B2", false),
            ("filled-box", "connection fill-B1", "#0D32B2", false),
            (
                "unfilled-triangle",
                "connection stroke-B1 fill-N7",
                "#FFFFFF",
                true,
            ),
            ("diamond", "connection stroke-B1 fill-N7", "#FFFFFF", true),
            ("box", "connection stroke-B1 fill-N7", "#FFFFFF", true),
            ("circle", " stroke-B1 fill-N7", "#FFFFFF", true),
            ("line", "connection stroke-B1", "none", true),
        ];
        for is_target in [false, true] {
            for (shape, class, fill, has_stroke) in cases {
                let definition = super::dynamic_marker_definition(
                    "paint-role",
                    shape,
                    is_target,
                    2.0,
                    "B1",
                    marker_theme,
                );
                assert!(
                    definition.contains(&format!("class=\"{class}\"")),
                    "{shape}: {definition}"
                );
                assert!(
                    definition.contains(&format!("fill=\"{fill}\"")),
                    "{shape}: {definition}"
                );
                assert_eq!(
                    definition.contains(" stroke=\"#0D32B2\""),
                    has_stroke,
                    "{shape}: {definition}"
                );
            }
        }
    }

    #[test]
    fn hollow_marker_background_uses_active_palette_and_overrides() {
        for (overrides, background) in [
            (BTreeMap::new(), "#1E1E2E"),
            (
                BTreeMap::from([("N7".to_owned(), "#123456".to_owned())]),
                "#123456",
            ),
        ] {
            let marker_theme = super::MarkerTheme {
                palette: super::PaletteView::new(super::active_palette(Some(200)), &overrides),
                inline: true,
            };
            for shape in [
                "unfilled-triangle",
                "diamond",
                "circle",
                "box",
                "cross",
                "cf-one",
                "cf-many",
                "cf-one-required",
                "cf-many-required",
            ] {
                let definition = super::dynamic_marker_definition(
                    "background",
                    shape,
                    true,
                    2.0,
                    "B1",
                    marker_theme,
                );
                assert!(
                    definition.contains(&format!("fill=\"{background}\"")),
                    "{shape}: {definition}"
                );
                assert!(definition.contains("fill-N7"), "{shape}: {definition}");
                assert!(!definition.contains("#FFFFFF"), "{shape}: {definition}");
            }
        }
    }

    #[test]
    fn dark_marker_paint_uses_css_and_preserves_literal_attributes() {
        let overrides = BTreeMap::new();
        let palette = super::PaletteView::new(super::active_palette(None), &overrides);
        let marker_theme = super::MarkerTheme {
            palette,
            inline: false,
        };
        for shape in [
            "triangle",
            "diamond",
            "circle",
            "line",
            "cross",
            "cf-many-required",
        ] {
            let symbolic =
                super::dynamic_marker_definition("dark", shape, true, 2.0, "B1", marker_theme);
            assert!(!symbolic.contains(" stroke=\""), "{shape}: {symbolic}");
            assert_eq!(
                symbolic.contains(" fill=\""),
                shape == "line",
                "{shape}: {symbolic}"
            );
            let literal =
                super::dynamic_marker_definition("literal", shape, true, 2.0, "red", marker_theme);
            let property = if shape == "triangle" {
                "fill"
            } else {
                "stroke"
            };
            assert!(
                literal.contains(&format!("{property}=\"red\"")),
                "{shape}: {literal}"
            );
            assert!(!literal.contains("-B1"), "{shape}: {literal}");
            if shape == "circle" {
                assert!(literal.contains("class=\" fill-N7\""));
            }
        }

        let dark_overrides = BTreeMap::from([("N7".to_owned(), "#123456".to_owned())]);
        let dark_palette =
            super::PaletteView::new(super::active_palette(Some(200)), &dark_overrides);
        let css = super::theme_css(palette, Some(dark_palette), FontNeeds::default(), false);
        for (code, property, light_color, dark_color) in [
            ("B1", "fill", "#0D32B2", "#CBA6f7"),
            ("B1", "stroke", "#0D32B2", "#CBA6f7"),
            ("N7", "fill", "#FFFFFF", "#123456"),
        ] {
            for color in [light_color, dark_color] {
                assert!(css.contains(&format!(
                    ".d2-rs-theme .d2-rs-{property}-{code},.d2-rs-theme .{property}-{code}{{{property}:{color} !important;}}"
                )));
            }
        }
        assert!(css.contains("@media screen and (prefers-color-scheme:dark)"));
    }

    #[test]
    fn compound_marker_children_inherit_v090_group_paint() {
        let overrides = BTreeMap::new();
        for inline in [false, true] {
            let marker_theme = super::MarkerTheme {
                palette: super::PaletteView::new(super::active_palette(None), &overrides),
                inline,
            };
            for is_target in [false, true] {
                for shape in [
                    "cross",
                    "cf-one",
                    "cf-many",
                    "cf-one-required",
                    "cf-many-required",
                ] {
                    let definition = super::dynamic_marker_definition(
                        "compound",
                        shape,
                        is_target,
                        2.0,
                        "B1",
                        marker_theme,
                    );
                    let painted_count = if shape == "cross" { 1 } else { 2 };
                    assert_eq!(
                        definition
                            .matches("class=\"connection stroke-B1 fill-N7\"")
                            .count(),
                        painted_count,
                        "{shape}: {definition}"
                    );
                    let spine = definition
                        .rsplit("<path ")
                        .next()
                        .expect("marker spine")
                        .split('>')
                        .next()
                        .expect("spine attributes");
                    assert!(
                        !spine.contains("fill=")
                            && !spine.contains("stroke=")
                            && !spine.contains("class="),
                        "{shape}: {definition}"
                    );
                    if shape == "cross" {
                        let polygon = definition
                            .split("<polygon ")
                            .nth(1)
                            .expect("cross polygon")
                            .split('>')
                            .next()
                            .expect("polygon attributes");
                        assert!(
                            !polygon.contains("fill=")
                                && !polygon.contains("stroke=")
                                && !polygon.contains("class=")
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn svg_object_classes_escape_before_url_base64() {
        assert_eq!(svg_object_class("a"), "YQ==");
        assert_eq!(svg_object_class("(a -> b)[0]"), "KGEgLSZndDsgYilbMF0=");
    }

    #[test]
    fn salted_objects_follow_v090_shape_then_connection_order() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });
        graph.set_edge_arrows(
            edge,
            d2_rs_core::EdgeArrows {
                target: true,
                source: false,
            },
        );
        let result = Engine
            .layout(&graph, &LayoutOptions::default())
            .expect("layout");
        let svg = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions {
                salt: Some("worker-2".to_owned()),
                ..RenderOptions::default()
            },
        ))
        .expect("SVG output is UTF-8");
        let first_shape = svg.find("<g class=\"YQ==\">").expect("source shape");
        let second_shape = svg.find("<g class=\"Yg==\">").expect("target shape");
        let connection = svg
            .find("<g class=\"KGEgLSZndDsgYilbMF0=\">")
            .expect("connection group");
        let marker = svg[connection..]
            .find("<marker id=\"mk-")
            .map(|offset| connection + offset)
            .expect("inline salted marker");
        let path = svg[marker..]
            .find("<path ")
            .map(|offset| marker + offset)
            .expect("connection path");
        assert!(svg.contains(" d2-svg\" width="));
        assert!(svg.contains("viewBox=\"-"));
        assert!(svg.contains("<mask id=\"d2-") && svg.contains(" x=\"-"));
        assert!(first_shape < second_shape && second_shape < connection);
        assert!(connection < marker && marker < path);
        assert!(!svg[connection..].contains("<defs><marker id=\"mk-"));
    }

    #[test]
    fn renderer_sorts_parent_shape_before_child_shape() {
        let mut graph = Graph::default();
        // Materialize the child first to make insertion order disagree with
        // d2svg.sortObjects; the target must still paint the container first.
        let child = graph.add_node(Node::new(
            "child",
            Size {
                width: 24.0,
                height: 18.0,
            },
        ));
        let parent = graph.add_node(Node::new(
            "parent",
            Size {
                width: 80.0,
                height: 60.0,
            },
        ));
        graph.node_mut(child).expect("child").parent = Some(parent);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    parent,
                    Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        size: Size {
                            width: 80.0,
                            height: 60.0,
                        },
                    },
                ),
                (
                    child,
                    Rect {
                        origin: Point { x: 10.0, y: 10.0 },
                        size: Size {
                            width: 24.0,
                            height: 18.0,
                        },
                    },
                ),
            ]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let svg = String::from_utf8(render_svg(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
        ))
        .expect("SVG output is UTF-8");
        let parent_offset = svg.find("<g class=\"cGFyZW50\">").expect("parent shape");
        let child_offset = svg.find("<g class=\"Y2hpbGQ=\">").expect("child shape");
        assert!(parent_offset < child_offset);
    }

    #[test]
    fn target_hash_matches_v090_simple_edge_witness() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        for id in [source, target] {
            let node = graph.node_mut(id).expect("node");
            node.label_size = Some(Size {
                width: 8.0,
                height: 21.0,
            });
            node.font_size = Some(16);
        }
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });
        graph.set_edge_arrows(
            edge,
            d2_rs_core::EdgeArrows {
                target: true,
                source: false,
            },
        );
        let labels = BTreeMap::from([(source, "a".to_owned()), (target, "b".to_owned())]);
        let styles = BTreeMap::from([
            (
                source,
                SourceNodeStyle {
                    bold: true,
                    ..SourceNodeStyle::default()
                },
            ),
            (
                target,
                SourceNodeStyle {
                    bold: true,
                    ..SourceNodeStyle::default()
                },
            ),
        ]);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            // Dagre's spline conversion marks this route as a curve.
            route_curves: BTreeMap::from([(edge, true)]),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    source,
                    Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        size: Size {
                            width: 53.0,
                            height: 66.0,
                        },
                    },
                ),
                (
                    target,
                    Rect {
                        origin: Point { x: 0.0, y: 166.0 },
                        size: Size {
                            width: 53.0,
                            height: 66.0,
                        },
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                edge,
                vec![
                    Point { x: 26.5, y: 66.0 },
                    Point { x: 26.5, y: 106.0 },
                    Point { x: 26.5, y: 126.0 },
                    Point { x: 26.5, y: 166.0 },
                ],
            )]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::from([
                (
                    source,
                    d2_rs_core::NodeLabelState {
                        font_size_committed: false,
                        size: Some(Size {
                            width: 8.0,
                            height: 21.0,
                        }),
                        font_size: Some(16),
                        position: d2_rs_core::LabelPosition::InsideMiddleCenter,
                        icon_position: None,
                    },
                ),
                (
                    target,
                    d2_rs_core::NodeLabelState {
                        font_size_committed: false,
                        size: Some(Size {
                            width: 8.0,
                            height: 21.0,
                        }),
                        font_size: Some(16),
                        position: d2_rs_core::LabelPosition::InsideMiddleCenter,
                        icon_position: None,
                    },
                ),
            ]),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let hash_bytes = target_hash_bytes(
            &graph,
            &result,
            &labels,
            &styles,
            &RenderOptions::default(),
            false,
        );
        let hash = target_hash_id(
            &graph,
            &result,
            &labels,
            &styles,
            &RenderOptions::default(),
            false,
            "",
        );
        assert!(hash_bytes.windows(6).any(|window| window == br"\u003e"));
        assert_eq!(hash, 2_044_524_636);
        assert_eq!(
            target_hash_id(
                &graph,
                &result,
                &labels,
                &styles,
                &RenderOptions::default(),
                false,
                "worker-2",
            ),
            3_545_618_205
        );
        let mut override_options = RenderOptions::default();
        override_options
            .theme_overrides
            .insert("N1".to_owned(), "#111111".to_owned());
        assert_eq!(
            target_hash_id(
                &graph,
                &result,
                &labels,
                &styles,
                &override_options,
                false,
                "worker-2",
            ),
            985_722_238
        );
        // The pinned Go source changes the target hash when the configured
        // theme changes, while output-only pad/center options remain outside
        // the target diagram bytes.  Keep that distinction explicit so this
        // witness catches accidental serialization of renderer-only options.
        let theme_options = RenderOptions {
            theme_id: Some(1),
            ..RenderOptions::default()
        };
        assert_eq!(
            target_hash_id(
                &graph,
                &result,
                &labels,
                &styles,
                &theme_options,
                false,
                "worker-2",
            ),
            3_486_127_812
        );
        let canvas_options = RenderOptions {
            pad: 80.0,
            center: true,
            ..RenderOptions::default()
        };
        assert_eq!(
            target_hash_id(
                &graph,
                &result,
                &labels,
                &styles,
                &canvas_options,
                false,
                "worker-2",
            ),
            3_545_618_205
        );
    }

    #[test]
    fn recursive_target_hash_input_includes_folder_only_descendants() {
        let bytes = recursive_target_hash_input(&[
            ("".to_owned(), b"root".to_vec()),
            ("layers.n.layers.inner".to_owned(), b"inner".to_vec()),
            ("layers.n".to_owned(), b"folder".to_vec()),
            ("scenarios.alt".to_owned(), b"scenario".to_vec()),
        ]);
        assert_eq!(bytes, b"rootfolderinnerscenario");
    }

    #[test]
    fn recursive_target_hash_input_preserves_quoted_board_components() {
        let bytes = recursive_target_hash_input(&[
            ("".to_owned(), b"root".to_vec()),
            (
                "layers.\"one.dot\".layers.\"two.dot\"".to_owned(),
                b"inner".to_vec(),
            ),
            ("layers.\"one.dot\"".to_owned(), b"folder".to_vec()),
            ("layers.other".to_owned(), b"sibling".to_vec()),
        ]);
        assert_eq!(bytes, b"rootfolderinnersibling");
    }

    #[test]
    fn diamond_marker_refx_is_based_on_unscaled_body() {
        let overrides = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        let id = marker_reference(
            Some("diamond"),
            true,
            4.0,
            "#7b2cbf",
            "#7b2cbf",
            "#0D32B2",
            None,
            super::MarkerTheme {
                palette: super::PaletteView::new(super::active_palette(None), &overrides),
                inline: true,
            },
            &mut definitions,
        )
        .expect("diamond marker");
        let definition = definitions.get(&id).expect("dynamic marker definition");
        assert!(definition.contains("markerWidth=\"36.3\" markerHeight=\"27\" refX=\"30.6\""));
    }

    #[test]
    fn supplied_asset_bytes_are_embedded_as_svg_data_uris() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node {
            shape: ShapeKind::Image,
            size: Size {
                width: 128.0,
                height: 128.0,
            },
            ..Node::new("asset", Size::default())
        });
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut styles = BTreeMap::new();
        styles.insert(
            node,
            SourceNodeStyle {
                icon: Some("asset.svg".into()),
                ..SourceNodeStyle::default()
            },
        );
        let svg = render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            RenderOptions {
                asset_data: BTreeMap::from([("asset.svg".into(), b"<svg></svg>".to_vec())]),
                ..RenderOptions::default()
            },
        );
        let svg = String::from_utf8(svg).unwrap();
        assert!(svg.contains("href=\"data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=\""));
    }

    #[test]
    fn synthetic_sequence_roots_render_actor_lifelines() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let first = graph.add_node(Node::new(
            "a",
            Size {
                width: 100.0,
                height: 66.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "b",
            Size {
                width: 100.0,
                height: 66.0,
            },
        ));
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    first,
                    Rect {
                        origin: Point { x: 12.0, y: 12.0 },
                        size: graph.node(first).unwrap().size,
                    },
                ),
                (
                    second,
                    Rect {
                        origin: Point { x: 162.0, y: 12.0 },
                        size: graph.node(second).unwrap().size,
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                d2_rs_core::EdgeId(0),
                vec![Point { x: 62.0, y: 120.0 }, Point { x: 212.0, y: 120.0 }],
            )]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let styles = BTreeMap::from([(
            first,
            SourceNodeStyle {
                stroke: Some("red".to_owned()),
                stroke_dash: Some("3".to_owned()),
                ..SourceNodeStyle::default()
            },
        )]);
        let svg = String::from_utf8(render_svg(&graph, &result, &BTreeMap::new(), &styles))
            .expect("SVG output is UTF-8");
        assert_eq!(svg.matches("<line ").count(), 2);
        let default_dash = super::stroke_dasharray("2", Some("6")).unwrap();
        let actor_dash = super::stroke_dasharray("2", Some("3")).unwrap();
        assert!(svg.contains(&format!("stroke-dasharray=\"{default_dash}\"")));
        assert!(svg.contains(&format!(
            "stroke=\"red\" stroke-width=\"2\" stroke-dasharray=\"{actor_dash}\""
        )));
        let lifeline_lines = svg
            .lines()
            .filter(|line| line.contains("<line "))
            .collect::<Vec<_>>();
        assert!(lifeline_lines[0].contains("id=\"d2rs-exact-dash-"));
        assert!(lifeline_lines[0].contains("mask=\"url(#"));
        assert!(
            lifeline_lines[0].contains("x1=\"62\" y1=\"80\" x2=\"62\" y2=\"147\""),
            "unexpected lifeline output: {lifeline_lines:?}"
        );
        assert!(svg.contains("fill-B5"));
        assert!(svg.contains("stroke-width=\"2\""));
    }

    #[test]
    fn sequence_lifeline_dash_values_keep_go_float64_precision() {
        let (dash, gap) = super::stroke_dash_values("2", Some("6")).unwrap();
        assert_eq!(dash.to_bits(), 0x4028_0000_0000_0000);
        assert_eq!(gap.to_bits(), 0x4027_ad72_e258_72d6);
    }

    #[test]
    fn root_sequence_actor_descendants_render_as_unlabeled_spans() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let actor = graph.add_node(Node::new(
            "actor",
            Size {
                width: 100.0,
                height: 66.0,
            },
        ));
        let span = graph.add_node(Node {
            parent: Some(actor),
            ..Node::new(
                "actor.work",
                Size {
                    width: 24.0,
                    height: 24.0,
                },
            )
        });
        let note = graph.add_node(Node {
            parent: Some(actor),
            ..Node::new(
                "actor.note",
                Size {
                    width: 60.0,
                    height: 28.0,
                },
            )
        });
        let recipient = graph.add_node(Node::new(
            "recipient",
            Size {
                width: 100.0,
                height: 66.0,
            },
        ));
        let actor_message = graph.add_edge(d2_rs_core::Edge {
            source: actor,
            target: recipient,
        });
        let span_message = graph.add_edge(d2_rs_core::Edge {
            source: span,
            target: recipient,
        });
        let boxes = [
            (
                actor,
                Point { x: 12.0, y: 12.0 },
                graph.node(actor).unwrap().size,
            ),
            (
                span,
                Point { x: 30.0, y: 96.0 },
                graph.node(span).unwrap().size,
            ),
            (
                note,
                Point { x: 30.0, y: 132.0 },
                graph.node(note).unwrap().size,
            ),
            (
                recipient,
                Point { x: 162.0, y: 12.0 },
                graph.node(recipient).unwrap().size,
            ),
        ]
        .into_iter()
        .map(|(id, origin, size)| (id, Rect { origin, size }))
        .collect();
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes,
            routes: BTreeMap::from([
                (
                    actor_message,
                    vec![Point { x: 62.0, y: 45.0 }, Point { x: 162.0, y: 45.0 }],
                ),
                (
                    span_message,
                    vec![Point { x: 42.0, y: 108.0 }, Point { x: 162.0, y: 45.0 }],
                ),
            ]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let labels = BTreeMap::from([
            (actor, "actor label".to_owned()),
            (span, "span label must be suppressed".to_owned()),
            (note, "note label".to_owned()),
            (recipient, "recipient label".to_owned()),
        ]);

        assert!(super::is_sequence_span_like(&graph, span));
        assert!(!super::is_sequence_span_like(&graph, note));
        assert!(!super::is_sequence_group_like(&graph, actor));
        assert_eq!(
            super::target_shape_for_node(&graph, span),
            Some(ShapeKind::Square)
        );
        assert_eq!(
            super::target_shape_for_node(&graph, note),
            Some(ShapeKind::Page)
        );
        let svg = String::from_utf8(render_svg(&graph, &result, &labels, &BTreeMap::new()))
            .expect("SVG output is UTF-8");
        assert!(svg.contains("actor label"));
        assert!(svg.contains("note label"));
        assert!(svg.contains("recipient label"));
        assert!(!svg.contains("span label must be suppressed"));
        let exported =
            super::target_shape_metadata(&graph, &result, &labels, &BTreeMap::new(), span)
                .unwrap()
                .unwrap();
        assert_eq!(exported["label"], "");
        for (id, text) in [
            (actor, "actor label"),
            (note, "note label"),
            (recipient, "recipient label"),
        ] {
            assert_eq!(
                super::target_shape_metadata(&graph, &result, &labels, &BTreeMap::new(), id)
                    .unwrap()
                    .unwrap()["label"],
                text
            );
        }
        let unmeasured = super::target_hash_shape_for_stage(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            span,
            super::TargetShapeStage::Unmeasured,
        )
        .unwrap();
        assert_eq!(unmeasured.label, "span label must be suppressed");
        // Original BoundingBox only admits nonempty exported target labels.
        // Sequence layout clears a span's text but preserves measured size.
        graph.node_mut(span).unwrap().label_size = Some(Size {
            width: 1000.0,
            height: 80.0,
        });
        let with_cleared_span = super::bounds(&graph, &result, &labels, &BTreeMap::new());
        let mut without_span_label = labels.clone();
        without_span_label.remove(&span);
        assert_eq!(
            with_cleared_span,
            super::bounds(&graph, &result, &without_span_label, &BTreeMap::new()),
            "cleared span dimensions cannot reserve canvas space"
        );
    }

    #[test]
    fn nested_sequence_grid_cells_follow_sequence_note_shape_rules() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let actor = graph.add_node(Node {
            grid_columns: Some(2),
            ..Node::new(
                "actor",
                Size {
                    width: 240.0,
                    height: 150.0,
                },
            )
        });
        let cell = graph.add_node(Node {
            parent: Some(actor),
            ..Node::new(
                "actor.cell",
                Size {
                    width: 80.0,
                    height: 60.0,
                },
            )
        });

        assert!(super::is_nested_sequence_grid_descendant(&graph, cell));
        assert!(!super::is_sequence_note_like(&graph, cell));
        assert!(super::has_sequence_note_fill(&graph, cell));
        assert_eq!(super::default_node_fill(&graph, cell), "N7");
        assert!(!super::is_sequence_span_like(&graph, cell));
        assert_eq!(
            super::target_shape_for_node(&graph, cell),
            Some(ShapeKind::Rectangle)
        );
    }

    #[test]
    fn root_sequence_unconnected_actors_keep_authored_shapes_and_white_fill() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let actor = graph.add_node(Node::new("actor", Size::default()));
        let note = graph.add_node(Node {
            parent: Some(actor),
            ..Node::new("actor.note", Size::default())
        });
        let isolated = graph.add_node(Node::new("isolated", Size::default()));
        assert_eq!(super::default_node_fill(&graph, isolated), "N7");
        assert_eq!(
            super::target_shape_for_node(&graph, isolated),
            Some(ShapeKind::Rectangle)
        );
        assert_eq!(super::default_node_fill(&graph, note), "N7");
        assert_eq!(
            super::target_shape_for_node(&graph, note),
            Some(ShapeKind::Page)
        );
    }

    #[test]
    fn sequence_grid_leaf_fill_obeys_final_edges_and_authored_override() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let actor = graph.add_node(Node {
            grid_columns: Some(2),
            ..Node::new("actor", Size::default())
        });
        let cell = graph.add_node(Node {
            parent: Some(actor),
            ..Node::new("actor.cell", Size::default())
        });
        let recipient = graph.add_node(Node::new("recipient", Size::default()));
        assert_eq!(super::default_node_fill(&graph, cell), "N7");
        let palette_overrides = BTreeMap::new();
        let palette = super::PaletteView::new(super::active_palette(None), &palette_overrides);
        let authored = SourceNodeStyle {
            fill: Some("#123456".into()),
            ..Default::default()
        };
        assert_eq!(
            super::themed_node_style(&graph, cell, &authored, palette)
                .fill
                .as_deref(),
            Some("#123456")
        );
        graph.add_edge(d2_rs_core::Edge {
            source: cell,
            target: recipient,
        });
        assert!(!super::has_sequence_note_fill(&graph, cell));
        assert_eq!(super::default_node_fill(&graph, cell), "B4");
        assert_eq!(
            super::target_shape_for_node(&graph, cell),
            Some(ShapeKind::Rectangle)
        );
    }

    #[test]
    fn nested_sequence_lifelines_ignore_outer_edges_and_use_zero_stroke_bounds() {
        let mut graph = Graph::default();
        let scope = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 170.0,
                height: 351.0,
            },
        ));
        graph.set_sequence_diagram(scope, true);
        let actor = graph.add_node(Node::new(
            "sequence.worker",
            Size {
                width: 100.0,
                height: 66.0,
            },
        ));
        graph.node_mut(actor).unwrap().parent = Some(scope);
        let complete = graph.add_node(Node::new(
            "complete",
            Size {
                width: 114.0,
                height: 66.0,
            },
        ));
        let message = graph.add_edge(d2_rs_core::Edge {
            source: actor,
            target: actor,
        });
        let outer = graph.add_edge(d2_rs_core::Edge {
            source: scope,
            target: complete,
        });
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    scope,
                    Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        size: graph.node(scope).unwrap().size,
                    },
                ),
                (
                    actor,
                    Rect {
                        origin: Point { x: 12.0, y: 88.0 },
                        size: graph.node(actor).unwrap().size,
                    },
                ),
                (
                    complete,
                    Rect {
                        origin: Point { x: 28.0, y: 451.0 },
                        size: graph.node(complete).unwrap().size,
                    },
                ),
            ]),
            routes: BTreeMap::from([
                (
                    message,
                    vec![
                        Point { x: 62.0, y: 224.0 },
                        Point { x: 142.0, y: 224.0 },
                        Point { x: 142.0, y: 269.0 },
                        Point { x: 62.0, y: 269.0 },
                    ],
                ),
                (
                    outer,
                    vec![Point { x: 85.0, y: 351.0 }, Point { x: 85.0, y: 451.0 }],
                ),
            ]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let lifelines = super::sequence_lifelines(&graph, &result);
        assert_eq!(lifelines.len(), 1);
        assert_eq!(lifelines[0].end, Point { x: 62.0, y: 339.0 });
        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &BTreeMap::new()),
            (0.0, 0.0, 170.0, 518.0)
        );
        let styles = BTreeMap::from([(
            scope,
            SourceNodeStyle {
                stroke_width: Some("4".to_owned()),
                ..SourceNodeStyle::default()
            },
        )]);
        assert_eq!(
            super::bounds(&graph, &result, &BTreeMap::new(), &styles),
            (-2.0, -2.0, 172.0, 518.0)
        );
    }

    #[test]
    fn sequence_lifelines_ignore_edges_internal_to_extracted_actor_grids() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let grid_actor = graph.add_node(Node {
            grid_columns: Some(2),
            ..Node::new(
                "grid actor",
                Size {
                    width: 200.0,
                    height: 130.0,
                },
            )
        });
        let first_cell = graph.add_node(Node {
            parent: Some(grid_actor),
            ..Node::new(
                "grid actor.1",
                Size {
                    width: 60.0,
                    height: 40.0,
                },
            )
        });
        let second_cell = graph.add_node(Node {
            parent: Some(grid_actor),
            ..Node::new(
                "grid actor.2",
                Size {
                    width: 60.0,
                    height: 40.0,
                },
            )
        });
        let other_actor = graph.add_node(Node::new(
            "other",
            Size {
                width: 100.0,
                height: 50.0,
            },
        ));
        let message = graph.add_edge(d2_rs_core::Edge {
            source: grid_actor,
            target: other_actor,
        });
        let extracted_grid_edge = graph.add_edge(d2_rs_core::Edge {
            source: first_cell,
            target: second_cell,
        });
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    grid_actor,
                    Rect {
                        origin: Point { x: 12.0, y: 20.0 },
                        size: Size {
                            width: 200.0,
                            height: 130.0,
                        },
                    },
                ),
                (
                    first_cell,
                    Rect {
                        origin: Point { x: 20.0, y: 30.0 },
                        size: Size {
                            width: 60.0,
                            height: 40.0,
                        },
                    },
                ),
                (
                    second_cell,
                    Rect {
                        origin: Point { x: 100.0, y: 30.0 },
                        size: Size {
                            width: 60.0,
                            height: 40.0,
                        },
                    },
                ),
                (
                    other_actor,
                    Rect {
                        origin: Point { x: 300.0, y: 10.0 },
                        size: Size {
                            width: 100.0,
                            height: 50.0,
                        },
                    },
                ),
            ]),
            routes: BTreeMap::from([
                (
                    message,
                    vec![Point { x: 112.0, y: 140.0 }, Point { x: 300.0, y: 140.0 }],
                ),
                (
                    extracted_grid_edge,
                    vec![Point { x: 80.0, y: 800.0 }, Point { x: 100.0, y: 800.0 }],
                ),
            ]),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };

        let lifelines = super::sequence_lifelines(&graph, &result);
        assert_eq!(lifelines.len(), 2);
        assert_eq!(lifelines[0].end.y, 220.0);
        assert_eq!(lifelines[1].end.y, 220.0);
    }

    #[test]
    fn sequence_groups_use_reference_scopes_for_the_upstream_blend_wrapper() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let group = graph.add_node(Node::new(
            "group",
            Size {
                width: 120.0,
                height: 80.0,
            },
        ));
        let child = graph.add_node(Node {
            parent: Some(group),
            ..Node::new(
                "child",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            )
        });
        let actor = graph.add_node(Node::new(
            "actor",
            Size {
                width: 60.0,
                height: 40.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: child,
            target: actor,
        });
        graph.sequence_group_edges.insert(group, vec![edge]);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    group,
                    Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        size: graph.node(group).unwrap().size,
                    },
                ),
                (
                    child,
                    Rect {
                        origin: Point { x: 10.0, y: 10.0 },
                        size: graph.node(child).unwrap().size,
                    },
                ),
                (
                    actor,
                    Rect {
                        origin: Point { x: 180.0, y: 0.0 },
                        size: graph.node(actor).unwrap().size,
                    },
                ),
            ]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let svg = String::from_utf8(render_svg(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
        ))
        .expect("SVG output is UTF-8");
        // Source child.ContainsAnyEdge follows the edge's reference scope.
        // A parent-scoped edge touching this child does not qualify a group.
        assert_eq!(svg.matches("<g class=\"shape blend\">").count(), 0);
        assert_eq!(super::default_node_fill(&graph, group), "B5");
        graph.sequence_group_edges.clear();
        graph.sequence_group_edges.insert(child, vec![edge]);
        let svg = String::from_utf8(render_svg(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
        ))
        .unwrap();
        assert_eq!(svg.matches("<g class=\"shape blend\">").count(), 1);
        assert!(svg.contains("fill-N5"));
    }

    #[test]
    fn default_target_colors_follow_d2graph_getfill() {
        let mut graph = Graph::default();
        let outer = graph.add_node(Node {
            shape: ShapeKind::Rectangle,
            ..Node::new("outer", Size::default())
        });
        let inner = graph.add_node(Node {
            parent: Some(outer),
            shape: ShapeKind::Rectangle,
            ..Node::new("inner", Size::default())
        });
        let cylinder = graph.add_node(Node {
            shape: ShapeKind::Cylinder,
            ..Node::new("cylinder", Size::default())
        });
        let person = graph.add_node(Node {
            shape: ShapeKind::Person,
            ..Node::new("person", Size::default())
        });
        let diamond = graph.add_node(Node {
            shape: ShapeKind::Diamond,
            ..Node::new("diamond", Size::default())
        });
        let class = graph.add_node(Node {
            shape: ShapeKind::Class,
            ..Node::new("class", Size::default())
        });
        let sql_table = graph.add_node(Node {
            shape: ShapeKind::SqlTable,
            ..Node::new("sql_table", Size::default())
        });

        assert_eq!(default_node_fill(&graph, outer), "B4");
        assert_eq!(default_node_fill(&graph, inner), "B5");
        assert_eq!(default_node_fill(&graph, cylinder), "AA4");
        assert_eq!(default_node_fill(&graph, person), "B3");
        assert_eq!(default_node_fill(&graph, diamond), "N4");
        assert_eq!(default_node_fill(&graph, class), "N1");
        assert_eq!(default_node_fill(&graph, sql_table), "N1");
        assert_eq!(
            default_node_stroke(&SourceNodeStyle::default(), ShapeKind::Class),
            "N7"
        );
        assert_eq!(
            default_node_stroke(&SourceNodeStyle::default(), ShapeKind::SqlTable),
            "N7"
        );

        let overrides = BTreeMap::new();
        let palette = super::PaletteView::new(
            d2_rs_core::theme::palette(0).expect("default palette"),
            &overrides,
        );
        let style = themed_node_style(&graph, cylinder, &SourceNodeStyle::default(), palette);
        assert_eq!(style.fill.as_deref(), Some("AA4"));
        assert_eq!(style.stroke.as_deref(), Some("B1"));
        assert_eq!(style.stroke_width.as_deref(), Some("2"));

        assert_eq!(
            super::themed_edge_style(
                EdgeStyle {
                    stroke_dash: Some("3".into()),
                    ..EdgeStyle::default()
                },
                palette,
            )
            .stroke
            .as_deref(),
            Some("B2")
        );
        assert_eq!(
            super::themed_edge_style(
                EdgeStyle {
                    stroke_dash: Some("2 3".into()),
                    ..EdgeStyle::default()
                },
                palette,
            )
            .stroke,
            None
        );
    }

    #[test]
    fn c4_edges_use_v090_theme_defaults_without_replacing_source_styles() {
        let overrides = BTreeMap::new();
        let palette = super::PaletteView::new(
            d2_rs_core::theme::palette(303).expect("C4 palette"),
            &overrides,
        );

        let defaults = super::themed_edge_style(EdgeStyle::default(), palette);
        assert_eq!(defaults.stroke_dash.as_deref(), Some("5"));
        assert_eq!(defaults.stroke.as_deref(), Some("AA4"));

        let authored = super::themed_edge_style(
            EdgeStyle {
                stroke: Some("#123456".into()),
                stroke_dash: Some("3".into()),
                ..EdgeStyle::default()
            },
            palette,
        );
        assert_eq!(authored.stroke.as_deref(), Some("#123456"));
        assert_eq!(authored.stroke_dash.as_deref(), Some("3"));

        let authored_stroke = super::themed_edge_style(
            EdgeStyle {
                stroke: Some("#123456".into()),
                ..EdgeStyle::default()
            },
            palette,
        );
        assert_eq!(authored_stroke.stroke.as_deref(), Some("#123456"));
        assert_eq!(authored_stroke.stroke_dash.as_deref(), Some("5"));
    }

    #[test]
    fn c4_authored_dash_keeps_theme_stroke_and_explicit_stroke_precedence() {
        let overrides = BTreeMap::from([
            ("AA4".into(), "#abcdef".into()),
            ("B2".into(), "#654321".into()),
        ]);
        let palette = super::PaletteView::new(
            d2_rs_core::theme::palette(303).expect("C4 palette"),
            &overrides,
        );

        for dash in ["0", "3"] {
            let style = super::themed_edge_style(
                EdgeStyle {
                    stroke_dash: Some(dash.into()),
                    ..EdgeStyle::default()
                },
                palette,
            );
            assert_eq!(style.stroke_dash.as_deref(), Some(dash));
            assert_eq!(style.stroke.as_deref(), Some("AA4"));
            assert_eq!(
                palette.explicit(style.stroke.as_deref().unwrap(), ""),
                "#abcdef"
            );
        }

        for (stroke, expected_color) in [("B2", "#654321"), ("#123456", "#123456")] {
            let style = super::themed_edge_style(
                EdgeStyle {
                    stroke: Some(stroke.into()),
                    stroke_dash: Some("3".into()),
                    ..EdgeStyle::default()
                },
                palette,
            );
            assert_eq!(style.stroke.as_deref(), Some(stroke));
            assert_eq!(style.stroke_dash.as_deref(), Some("3"));
            assert_eq!(
                palette.explicit(style.stroke.as_deref().unwrap(), ""),
                expected_color
            );
        }

        let ordinary_palette = super::PaletteView::new(
            d2_rs_core::theme::palette(0).expect("default palette"),
            &overrides,
        );
        let ordinary = super::themed_edge_style(
            EdgeStyle {
                stroke_dash: Some("3".into()),
                ..EdgeStyle::default()
            },
            ordinary_palette,
        );
        assert_eq!(ordinary.stroke.as_deref(), Some("B2"));
    }

    #[test]
    fn animated_dash_gap_matches_v090_stroke_dash_attributes() {
        assert_eq!(
            super::stroke_dasharray("5", Some("9")).as_deref(),
            Some("45,42.318306")
        );
    }

    #[test]
    fn explicit_padding_changes_only_the_canvas() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::new();
        let default_box = view_box(&render_svg(&graph, &result, &labels, &BTreeMap::new()));
        let zero_box = view_box(&render_svg_with_options(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions {
                pad: 0.0,
                ..RenderOptions::default()
            },
        ));
        let default_width = default_box
            .split_whitespace()
            .nth(2)
            .unwrap()
            .parse::<f64>()
            .unwrap();
        let zero_width = zero_box
            .split_whitespace()
            .nth(2)
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert_eq!(default_width - zero_width, 200.0);
    }

    #[test]
    fn salt_rewrites_ids_and_all_fragment_references() {
        let mut svg = String::from(
            r###"<svg><defs><linearGradient id="gradient"><stop/></linearGradient></defs><path style="fill:url(#gradient)" href="#gradient" xlink:href="#gradient"/></svg>"###,
        );
        salt_svg_ids(&mut svg, "two diagrams");
        assert!(svg.contains("id=\"gradient-74776f206469616772616d73\""));
        assert!(svg.matches("#gradient-74776f206469616772616d73").count() == 3);
        assert!(!svg.contains("#gradient)"));
    }

    #[test]
    fn explicit_scale_ceil_rounds_logical_viewport_without_scaling_scene_twice() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::new();
        let default_svg = render_svg(&graph, &result, &labels, &BTreeMap::new());
        let default_text = std::str::from_utf8(&default_svg).unwrap();
        let default_document = roxmltree::Document::parse(default_text).unwrap();
        let base = default_document
            .root_element()
            .attribute("viewBox")
            .unwrap();
        let values: Vec<f64> = base
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        for scale in [0.125, 0.333333333, 1.0010000001, 1.25, 2.0] {
            let svg = render_svg_with_options(
                &graph,
                &result,
                &labels,
                &BTreeMap::new(),
                RenderOptions {
                    scale: Some(scale),
                    ..RenderOptions::default()
                },
            );
            let text = std::str::from_utf8(&svg).unwrap();
            let document = roxmltree::Document::parse(text).unwrap();
            let root = document.root_element();
            assert_eq!(root.attribute("viewBox"), Some(base));
            assert_eq!(
                root.attribute("width").unwrap().parse::<f64>().unwrap(),
                (values[2] * scale).ceil()
            );
            assert_eq!(
                root.attribute("height").unwrap().parse::<f64>().unwrap(),
                (values[3] * scale).ceil()
            );
            let inner = root
                .children()
                .find(|node| {
                    node.attribute("class")
                        .is_some_and(|value| value.contains("d2-svg"))
                })
                .unwrap();
            assert_eq!(
                inner.attribute("width").unwrap().parse::<f64>().unwrap(),
                values[2]
            );
            assert_eq!(
                inner.attribute("height").unwrap().parse::<f64>().unwrap(),
                values[3]
            );
            assert!(!text.contains("<g transform=\"scale("));
        }
    }

    #[test]
    fn center_changes_only_svg_viewport_alignment() {
        let mut graph = Graph::default();
        graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::new();
        let default_svg =
            String::from_utf8(render_svg(&graph, &result, &labels, &BTreeMap::new())).unwrap();
        let centered_svg = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions {
                center: true,
                ..RenderOptions::default()
            },
        ))
        .unwrap();
        assert!(default_svg.contains("preserveAspectRatio=\"xMinYMin meet\""));
        assert!(centered_svg.contains("preserveAspectRatio=\"xMidYMid meet\""));
        assert_eq!(
            view_box(default_svg.as_bytes()),
            view_box(centered_svg.as_bytes())
        );
    }

    #[test]
    fn dark_theme_emits_media_palette_without_changing_geometry() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(Edge { source, target });
        graph.set_edge_arrows(
            edge,
            EdgeArrows {
                source: true,
                target: true,
            },
        );
        graph.set_edge_arrowheads(
            edge,
            d2_rs_core::EdgeArrowheads {
                source: Some("line".to_owned()),
                target: Some("diamond".to_owned()),
            },
        );
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::new();
        let light_output =
            String::from_utf8(render_svg(&graph, &result, &labels, &BTreeMap::new())).unwrap();
        let output = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions {
                dark_theme_id: Some(200),
                ..RenderOptions::default()
            },
        ))
        .unwrap();
        assert!(output.contains("@media screen and (prefers-color-scheme:dark)"));
        assert!(output.contains("#CDD6F4"));
        assert!(output.contains("d2-rs-fill-B6"));
        assert_eq!(
            view_box(output.as_bytes()),
            view_box(light_output.as_bytes())
        );
        let markers = output
            .split("<marker ")
            .skip(1)
            .map(|marker| marker.split("</marker>").next().expect("marker body"))
            .collect::<Vec<_>>();
        assert_eq!(markers.len(), 2);
        assert!(
            markers
                .iter()
                .any(|marker| marker.contains("class=\"connection stroke-B1 fill-N7\""))
        );
        assert!(
            markers
                .iter()
                .any(|marker| marker.contains("fill=\"none\" class=\"connection stroke-B1\""))
        );
        for marker in markers {
            assert!(!marker.contains(" stroke=\""));
            assert!(!marker.contains(" fill=\"#"));
        }
    }

    #[test]
    fn special_theme_rules_follow_upstream_target_defaults() {
        let mut graph = Graph::default();
        let outer = graph.add_node(Node::new(
            "outer",
            Size {
                width: 120.0,
                height: 90.0,
            },
        ));
        let inner = graph.add_node(Node::new(
            "inner",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.node_mut(inner).expect("inner node").parent = Some(outer);
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(outer, "outer label".into());
        labels.insert(inner, "inner label".into());
        let output = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions {
                theme_id: Some(300),
                ..RenderOptions::default()
            },
        ))
        .unwrap();
        assert!(output.contains("url(#d2-rs-pattern-dots)"));
        assert!(output.contains("Source Code Pro, monospace"));
        // Terminal themes add the outer container's second border and remove
        // corner rounding through the target theme pass.
        assert!(output.matches("<rect ").count() >= 3);
        assert!(output.contains("rx=\"0\""));

        let person = Node {
            shape: ShapeKind::Person,
            ..Node::new(
                "person",
                Size {
                    width: 50.0,
                    height: 70.0,
                },
            )
        };
        let mut c4_graph = Graph::default();
        c4_graph.add_node(person);
        let c4_result = Engine.layout(&c4_graph, &LayoutOptions::default()).unwrap();
        let c4_output = String::from_utf8(render_svg_with_options(
            &c4_graph,
            &c4_result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions {
                theme_id: Some(303),
                ..RenderOptions::default()
            },
        ))
        .unwrap();
        assert!(c4_output.contains("#08427b"));
        assert!(c4_output.contains("#073b6f"));
    }

    #[test]
    fn built_in_dot_and_line_patterns_keep_upstream_geometry_and_blend_groups() {
        let mut graph = Graph::default();
        let dots = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let lines = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut styles = BTreeMap::new();
        styles.insert(
            dots,
            SourceNodeStyle {
                fill_pattern: Some("dots".into()),
                ..SourceNodeStyle::default()
            },
        );
        styles.insert(
            lines,
            SourceNodeStyle {
                fill_pattern: Some("lines".into()),
                ..SourceNodeStyle::default()
            },
        );
        let output = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            RenderOptions::default(),
        ))
        .unwrap();
        assert!(output.contains(
            r##"<g style="mix-blend-mode:multiply" opacity="0.1"><rect x="2" y="7" width="1" height="1" fill="#0A0F25"/></g>"##
        ));
        assert!(output.contains(
            r##"<g style="mix-blend-mode:multiply" opacity="0.05"><rect y="12" width="15" height="1" fill="#0A0F25"/></g>"##
        ));
        assert!(output.contains("class=\"dots-overlay\""));
        assert!(output.contains("class=\"lines-overlay\""));
        assert!(output.contains("fill=\"url(#d2-rs-pattern-dots)\""));
        assert!(output.contains("d2-rs-fill-B6"));
        assert!(output.contains(".d2-rs-theme .dots-overlay"));

        let mut grain_styles = BTreeMap::new();
        grain_styles.insert(
            dots,
            SourceNodeStyle {
                fill_pattern: Some("grain".into()),
                ..SourceNodeStyle::default()
            },
        );
        let grain_output = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &grain_styles,
            RenderOptions::default(),
        ))
        .unwrap();
        assert!(grain_output.contains("fill=\"url(#grain-d2-rs)\""));
        assert!(grain_output.contains("data:image/png;base64,iVBOR"));
    }

    #[test]
    fn connection_label_bold_takes_precedence_over_italic_for_all_faces_and_line_modes() {
        // d2svg connection font class and d2scenebuild::font both select
        // bold before italic. Markdown emphasis has a separate contract.
        for mono in [false, true] {
            for bold in [false, true] {
                for italic in [false, true] {
                    for (text, geometry) in [
                        ("marked label", None),
                        ("marked\nlabel", None),
                        ("marked\nlabel", Some("native-label")),
                    ] {
                        let mut output = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
                        super::draw_label(
                            &mut output,
                            text,
                            Rect {
                                origin: Point { x: 10.0, y: 20.0 },
                                size: Size {
                                    width: 100.0,
                                    height: 40.0,
                                },
                            },
                            16,
                            "#123456",
                            None,
                            false,
                            bold,
                            italic,
                            mono,
                            geometry,
                        );
                        output.push_str("</svg>");
                        let document = roxmltree::Document::parse(&output).unwrap();
                        let labels = document
                            .descendants()
                            .filter(|node| node.tag_name().name() == "text")
                            .collect::<Vec<_>>();
                        assert!(!labels.is_empty());
                        for label in labels {
                            assert_eq!(
                                label.attribute("font-style"),
                                if italic && !bold {
                                    Some("italic")
                                } else {
                                    None
                                },
                                "mono={mono} bold={bold} italic={italic} geometry={geometry:?}"
                            );
                            assert_eq!(
                                label.attribute("font-weight"),
                                if bold { Some("bold") } else { None }
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn native_grain_tile_matches_source_scene_image_placement() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "grain",
            Size {
                width: 300.0,
                height: 300.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                fill_pattern: Some("grain".into()),
                ..Default::default()
            },
        )]);
        let (raster, _) = super::render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            RenderOptions::default(),
            false,
        )
        .unwrap();
        let raster = String::from_utf8(raster).unwrap();
        let document = roxmltree::Document::parse(&raster).unwrap();
        let pattern = document
            .descendants()
            .find(|n| n.attribute("id") == Some("grain-d2-rs"))
            .unwrap();
        let image = pattern
            .descendants()
            .find(|n| n.tag_name().name() == "image")
            .unwrap();
        // Pinned patterns.go: a300x300 Image with AlignNone, opacity.9,
        // inside an opacity.8 root. Public SVG uses the original wrapper.
        assert_eq!(image.attribute("width"), Some("300"));
        assert_eq!(image.attribute("height"), Some("300"));
        assert_eq!(image.attribute("preserveAspectRatio"), Some("none"));
        assert_eq!(image.parent().unwrap().attribute("opacity"), Some("0.9"));
        assert_eq!(
            image
                .parent()
                .unwrap()
                .attribute("data-d2-rs-exact-opacity"),
            Some("0.9")
        );
        assert_eq!(
            image
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .attribute("opacity"),
            Some("0.8")
        );
        let public = String::from_utf8(render_svg_with_options(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            RenderOptions::default(),
        ))
        .unwrap();
        assert!(public.contains("id=\"pattern0-d2-rs\""));
        assert!(public.contains("width=\"466\" height=\"349\""));
    }

    #[test]
    fn svg_embeds_only_the_used_bundled_d2_font_face() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "font",
            Size {
                width: 120.0,
                height: 40.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "font".to_owned());
        let svg =
            String::from_utf8(render_svg(&graph, &result, &labels, &BTreeMap::new())).unwrap();
        assert!(svg.contains("@font-face"));
        assert_eq!(svg.matches("@font-face").count(), 1);
        assert!(svg.contains("font-family:\"Source Sans Pro\""));
        assert!(svg.contains("font-family:\"Source Code Pro\""));
        assert!(svg.contains("data:font/ttf;base64,"));
        assert!(!svg.contains("data:font/woff2;base64,"));
    }

    #[test]
    fn markdown_font_subset_tracks_css_face_with_synthetic_italic() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text class="md-text text-bold" font-family="Source Sans Pro, sans-serif" font-style="italic">bold italic</text><text class="md-text text-italic" font-family="Source Sans Pro, sans-serif">italic</text></svg>"#;
        let needs = font_needs_from_svg(svg, FontNeeds::default());
        assert!(needs.subset_corpus_valid);
        assert_eq!(needs.markdown_corpora[2], "bold italic");
        assert_eq!(needs.markdown_corpora[3], "italic");
    }

    #[test]
    fn primary_mono_markdown_subsets_follow_resolved_regular_and_emphasis_faces() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g class="md md-primary-mono"><text class="md-text text" font-family="Source Sans Pro, sans-serif">regular</text><text class="md-text text-bold" font-family="Source Sans Pro, sans-serif">bold</text><text class="md-text text-italic" font-family="Source Sans Pro, sans-serif">italic</text><text class="md-text text-mono">code</text></g></svg>"#;
        let needs = super::font_needs_from_svg(svg, super::FontNeeds::default());
        assert_eq!(needs.markdown_corpora[4], "regularcode");
        assert_eq!(needs.markdown_corpora[6], "bold");
        assert_eq!(needs.markdown_corpora[7], "italic");
        assert!(needs.markdown_corpora[..4].iter().all(String::is_empty));
        let css = super::font_face_css(needs, true);
        for family in [
            "d2-rs-font-mono-regular",
            "d2-rs-font-mono-bold",
            "d2-rs-font-mono-italic",
        ] {
            assert!(css.contains(&format!("@font-face{{font-family:\"{family}\"")));
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn paragraph_primary_fonts_match_complete_original_go_primitive_geometry() {
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-font-layout-source-v090.json"))
        .unwrap();
        for control in controls.as_array().unwrap() {
            let family = match control["family"].as_str().unwrap() {
                "SourceSansPro" => super::MarkdownFamily::Sans,
                "SourceCodePro" => super::MarkdownFamily::Mono,
                "HandDrawn" => super::MarkdownFamily::HandDrawn,
                other => panic!("unknown Source primary {other}"),
            };
            let mut svg = String::new();
            assert!(super::draw_native_markdown_paragraph(
                &mut svg,
                control["source"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: control["width"].as_f64().unwrap(),
                        height: control["height"].as_f64().unwrap()
                    }
                },
                control["size"].as_u64().unwrap() as u32,
                "#112233",
                None,
                true,
                family
            ));
            let document = roxmltree::Document::parse(&svg).unwrap();
            let primitives = document
                .descendants()
                .filter(|node| node.has_tag_name("rect") || node.has_tag_name("text"))
                .collect::<Vec<_>>();
            let source = control["layout"]["Primitives"].as_array().unwrap();
            assert_eq!(
                primitives.len(),
                source.len(),
                "complete Source paint order {control}"
            );
            for (node, expected) in primitives.iter().zip(source) {
                assert_eq!(
                    node.tag_name().name(),
                    expected["Kind"].as_str().unwrap(),
                    "{control}"
                );
                for (attribute, field) in [("x", "X"), ("y", "Y")] {
                    let actual: f64 = node.attribute(attribute).unwrap().parse().unwrap();
                    assert_eq!(
                        actual.to_bits(),
                        expected[field].as_f64().unwrap().to_bits(),
                        "{attribute} Source {control}"
                    );
                }
                if node.has_tag_name("rect") {
                    for (attribute, field) in
                        [("width", "Width"), ("height", "Height"), ("rx", "Radius")]
                    {
                        let actual: f64 = node.attribute(attribute).unwrap().parse().unwrap();
                        assert_eq!(
                            actual.to_bits(),
                            expected[field].as_f64().unwrap().to_bits(),
                            "{attribute} Source {control}"
                        );
                    }
                } else {
                    assert_eq!(
                        node.text().unwrap_or_default(),
                        expected["Text"].as_str().unwrap()
                    );
                    let actual: f64 = node
                        .attribute("font-size")
                        .unwrap()
                        .trim_end_matches("px")
                        .parse()
                        .unwrap();
                    assert_eq!(
                        actual.to_bits(),
                        expected["FontSize"].as_f64().unwrap().to_bits()
                    );
                    if expected["SyntheticBold"].as_bool().unwrap() {
                        let stroke: f64 = node
                            .attribute("stroke-width")
                            .expect("Source synthetic bold stroke")
                            .parse()
                            .unwrap();
                        assert_eq!(stroke.to_bits(), (actual / 32.0).max(0.35).to_bits());
                        assert_eq!(node.attribute("stroke-linejoin"), Some("round"));
                        assert_eq!(node.attribute("font-weight"), None);
                    } else {
                        assert_eq!(node.attribute("stroke-width"), None);
                    }
                    assert_eq!(
                        node.attribute("font-style") == Some("italic"),
                        expected["SyntheticItalic"].as_bool().unwrap()
                    );
                }
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn heading_primary_fonts_match_complete_original_go_primitive_geometry() {
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-heading-layout-source-v090.json"))
        .unwrap();
        for control in controls.as_array().unwrap() {
            let family = match control["family"].as_str().unwrap() {
                "SourceSansPro" => super::MarkdownFamily::Sans,
                "SourceCodePro" => super::MarkdownFamily::Mono,
                "HandDrawn" => super::MarkdownFamily::HandDrawn,
                other => panic!("unknown Source primary {other}"),
            };
            let mut svg = String::new();
            assert!(super::draw_native_markdown_subset_in_family(
                &mut svg,
                control["source"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: control["width"].as_f64().unwrap(),
                        height: control["height"].as_f64().unwrap()
                    }
                },
                control["size"].as_u64().unwrap() as u32,
                "#112233",
                None,
                true,
                family
            ));
            let document = roxmltree::Document::parse(&svg).unwrap();
            let primitives = document
                .descendants()
                .filter(|node| node.has_tag_name("rect") || node.has_tag_name("text"))
                .collect::<Vec<_>>();
            let source = control["layout"]["Primitives"].as_array().unwrap();
            assert_eq!(
                primitives.len(),
                source.len(),
                "complete Source paint order {control}"
            );
            for (node, expected) in primitives.iter().zip(source) {
                assert_eq!(
                    node.tag_name().name(),
                    expected["Kind"].as_str().unwrap(),
                    "{control}"
                );
                for (attribute, field) in [("x", "X"), ("y", "Y")] {
                    let actual: f64 = node.attribute(attribute).unwrap().parse().unwrap();
                    assert_eq!(
                        actual.to_bits(),
                        expected[field].as_f64().unwrap().to_bits(),
                        "{attribute} Source {control}"
                    );
                }
                if node.has_tag_name("rect") {
                    for (attribute, field) in
                        [("width", "Width"), ("height", "Height"), ("rx", "Radius")]
                    {
                        let actual: f64 = node.attribute(attribute).unwrap().parse().unwrap();
                        assert_eq!(
                            actual.to_bits(),
                            expected[field].as_f64().unwrap().to_bits(),
                            "{attribute} Source {control}"
                        );
                    }
                } else {
                    assert_eq!(
                        node.text().unwrap_or_default(),
                        expected["Text"].as_str().unwrap()
                    );
                    let actual: f64 = node
                        .attribute("font-size")
                        .unwrap()
                        .trim_end_matches("px")
                        .parse()
                        .unwrap();
                    assert_eq!(
                        actual.to_bits(),
                        expected["FontSize"].as_f64().unwrap().to_bits()
                    );
                    if expected["SyntheticBold"].as_bool().unwrap() {
                        let stroke: f64 = node
                            .attribute("stroke-width")
                            .expect("Source synthetic bold stroke")
                            .parse()
                            .unwrap();
                        assert_eq!(stroke.to_bits(), (actual / 32.0).max(0.35).to_bits());
                        assert_eq!(node.attribute("stroke-linejoin"), Some("round"));
                        assert_eq!(node.attribute("font-weight"), None);
                    } else {
                        assert_eq!(node.attribute("stroke-width"), None);
                    }
                    assert_eq!(
                        node.attribute("font-style") == Some("italic"),
                        expected["SyntheticItalic"].as_bool().unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn sketch_svg_scopes_fuzzy_bubbles_over_markdown_face_rules() {
        let mut graph = Graph::default();
        let node = graph.add_node(Node::new(
            "font",
            Size {
                width: 120.0,
                height: 40.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "**font**".to_owned());
        let svg = String::from_utf8(render_svg_with_sketch(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions::default(),
            true,
        ))
        .unwrap();
        assert!(svg.contains("class=\"d2-rs-theme d2-rs-sketch\""));
        assert!(svg.contains(
            ".d2-rs-theme.d2-rs-sketch text.md-text.text-bold{font-family:\"d2-rs-sketch-font-bold\""
        ));
        assert!(svg.contains(
            ".d2-rs-theme.d2-rs-sketch text[font-weight=\"bold\"]{font-family:\"d2-rs-sketch-font-bold\""
        ));
        assert!(svg.contains("font-family:\"d2-rs-sketch-font-bold\";font-style:normal"));
        assert!(svg.contains(".d2-rs-theme.d2-rs-sketch text.text-italic{font-style:normal;}"));
        assert!(svg.contains("data:font/ttf;base64,"));
    }

    #[test]
    fn target_connection_exports_committed_and_authored_label_percentages() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new("a", Size::default()));
        let target = graph.add_node(Node::new("b", Size::default()));
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });
        let label = d2_rs_core::EdgeLabel {
            text: "label".to_owned(),
            size: Size::default(),
            position: LabelPosition::UnlockedTop,
            percentage: 0.5,
        };
        graph.set_edge_label(edge, Some(label.clone()));
        let mut result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::new(),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::from([(edge, label)]),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        for (authored, committed, percentage, expected) in [
            (false, false, 0.4, 0.0),
            (true, false, 0.4, 0.5),
            (false, true, 0.4, f64::from(0.4_f32)),
            (true, true, 0.0, 0.0),
            (false, true, 0.5, 0.5),
        ] {
            graph.set_edge_label_percentage_present(edge, authored);
            result.edge_labels.get_mut(&edge).unwrap().percentage = percentage;
            result.edge_label_percentage_commits.clear();
            if committed {
                result.edge_label_percentage_commits.insert(edge);
            }
            let target = super::target_hash_connection(&graph, &result, edge).unwrap();
            assert_eq!(target.label_percentage.0, expected);
        }
    }

    #[test]
    fn target_connection_ids_use_scope_arrows_and_equivalent_edge_indices() {
        let mut graph = Graph::default();
        let group = graph.add_node(Node::new("group", Size::default()));
        let a = graph.add_node(Node::new("group.a", Size::default()));
        let b = graph.add_node(Node::new("group.b", Size::default()));
        let other = graph.add_node(Node::new("other", Size::default()));
        graph.node_mut(a).unwrap().parent = Some(group);
        graph.node_mut(b).unwrap().parent = Some(group);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::new(),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        for (source, target, source_arrow, target_arrow, expected) in [
            (a, b, false, true, "group.(a -> b)[0]"),
            (a, other, false, true, "(group.a -> other)[0]"),
            (b, a, true, false, "group.(b <- a)[1]"),
            (b, a, false, true, "group.(b -> a)[0]"),
            (a, b, false, true, "group.(a -> b)[2]"),
            (a, b, true, true, "group.(a <-> b)[0]"),
            (b, a, true, true, "group.(b <-> a)[1]"),
            (a, b, false, false, "group.(a -- b)[0]"),
            (b, a, false, false, "group.(b -- a)[1]"),
        ] {
            let edge = graph.add_edge(d2_rs_core::Edge { source, target });
            graph.set_edge_arrows(
                edge,
                d2_rs_core::EdgeArrows {
                    source: source_arrow,
                    target: target_arrow,
                },
            );
            assert_eq!(super::edge_svg_id(&graph, edge).as_deref(), Some(expected));
            let target = super::target_hash_connection(&graph, &result, edge).unwrap();
            assert_eq!(target.id, expected);
        }
    }

    #[test]
    fn target_connection_ids_preserve_quoted_dots_and_nested_scope() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new("\"scope.name\"", Size::default()));
        let child = graph.add_node(Node::new("\"scope.name\".inner", Size::default()));
        graph.node_mut(child).unwrap().parent = Some(parent);
        let a = graph.add_node(Node::new("\"scope.name\".inner.\"a.b\"", Size::default()));
        let b = graph.add_node(Node::new("\"scope.name\".inner.b", Size::default()));
        graph.node_mut(a).unwrap().parent = Some(child);
        graph.node_mut(b).unwrap().parent = Some(child);
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });
        graph.set_edge_arrows(
            edge,
            d2_rs_core::EdgeArrows {
                source: false,
                target: true,
            },
        );
        assert_eq!(
            super::edge_svg_id(&graph, edge).as_deref(),
            Some("\"scope.name\".inner.(\"a.b\" -> b)[0]")
        );
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: child,
            target: a,
        });
        assert_eq!(
            super::edge_svg_id(&graph, edge).as_deref(),
            Some("\"scope.name\".(inner -- inner.\"a.b\")[0]")
        );
    }
}

#[cfg(test)]
mod live_external_label_source_contract {
    use super::render_svg;
    use d2_rs_core::{
        ExternalAlignment, ExternalLabel, ExternalSide, Graph, LabelPosition, LayoutReport,
        LayoutResult, Node, NodeLabelState, Point, Rect, Score, ShapeKind, Size,
    };
    use d2_rs_language::compiler::{SourceNodeStyle, SourceSqlColumn, SourceTargetAccents};
    use std::collections::BTreeMap;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn post_layout_external_label_dimensions_match_pristine_svg() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/live-external-label-source-v090.json"))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            for automatic in [true, false] {
                let position = LabelPosition::from_d2_name(row["position"].as_str().unwrap());
                let variant = row["variant"].as_str().unwrap();
                let mut graph = Graph::default();
                let old_size = Size {
                    width: 103.0,
                    height: 21.0,
                };
                let mut external = ExternalLabel {
                    size: old_size,
                    side: ExternalSide::Top,
                    alignment: ExternalAlignment::Center,
                    automatic: true,
                    reserve_space: true,
                }
                .at_position(position);
                external.automatic = automatic;
                let id = graph.add_node(Node {
                    shape: if variant.starts_with("person") {
                        ShapeKind::Person
                    } else {
                        ShapeKind::Rectangle
                    },
                    label_size: Some(old_size),
                    font_size: Some(16),
                    label_position: position,
                    external_label: Some(external),
                    ..Node::new(
                        "probe",
                        Size {
                            width: 160.0,
                            height: 160.0,
                        },
                    )
                });
                let size = Size {
                    width: row["width"].as_f64().unwrap(),
                    height: row["height"].as_f64().unwrap(),
                };
                let result = LayoutResult {
                    removed_edges: Default::default(),
                    source_edge_storage: Default::default(),
                    synthetic_root: None,
                    route_curves: Default::default(),
                    sequence_lifelines: Vec::new(),
                    edge_label_percentage_commits: Default::default(),
                    boxes: BTreeMap::from([(
                        id,
                        Rect {
                            origin: Point { x: 15.0, y: 26.0 },
                            size: Size {
                                width: 160.0,
                                height: 160.0,
                            },
                        },
                    )]),
                    routes: BTreeMap::new(),
                    edge_labels: BTreeMap::new(),
                    node_labels: BTreeMap::from([(
                        id,
                        NodeLabelState {
                            font_size_committed: false,
                            size: Some(size),
                            font_size: Some(row["font_size"].as_u64().unwrap() as u32),
                            position,
                            icon_position: None,
                        },
                    )]),
                    report: LayoutReport {
                        selected_seed: 0,
                        selected_pass: 0,
                        score: Score::default(),
                        improvement_passes: 0,
                        candidates: Vec::new(),
                        warnings: Vec::new(),
                    },
                };
                let styles = BTreeMap::from([(
                    id,
                    SourceNodeStyle {
                        bold: true,
                        fill: Some("B3".into()),
                        stroke: Some("B1".into()),
                        stroke_width: Some("2".into()),
                        multiple: variant.ends_with("multiple").then(|| "true".into()),
                        three_dee: variant.ends_with("3d").then(|| "true".into()),
                        ..Default::default()
                    },
                )]);
                let svg = render_svg(
                    &graph,
                    &result,
                    &BTreeMap::from([(id, "Probe".into())]),
                    &styles,
                );
                let svg = std::str::from_utf8(&svg).unwrap();
                let document = roxmltree::Document::parse(svg).unwrap();
                let text = document
                    .descendants()
                    .find(|n| n.has_tag_name("text") && n.text() == Some("Probe"))
                    .unwrap();
                let x = text.attribute("x").unwrap().parse::<f64>().unwrap();
                let y = text.attribute("y").unwrap().parse::<f64>().unwrap();
                let viewbox = document.root_element().attribute("viewBox").unwrap();
                if x != row["x"].as_f64().unwrap()
                    || y != row["y"].as_f64().unwrap()
                    || viewbox != row["viewbox"].as_str().unwrap()
                {
                    differences.push((row.clone(), automatic, x, y, viewbox.to_owned()));
                }
                assert_eq!(
                    graph.node(id).unwrap().external_label.unwrap().size,
                    old_size
                );
            }
        }
        assert!(
            differences.is_empty(),
            "pristine live external-label differences: {differences:#?}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod badge_anchor_source_contract {
    use super::*;
    use d2_rs_core::{LayoutReport, LayoutResult, Node, Score};
    #[test]
    fn badge_anchors_and_complete_svg_viewboxes_match_pristine_shapes() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/badge-anchor-source-v090.json"))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let shape = match row["shape"].as_str().unwrap() {
                "rectangle" => ShapeKind::Rectangle,
                "square" => ShapeKind::Square,
                "page" => ShapeKind::Page,
                "parallelogram" => ShapeKind::Parallelogram,
                "document" => ShapeKind::Document,
                "cylinder" => ShapeKind::Cylinder,
                "queue" => ShapeKind::Queue,
                "package" => ShapeKind::Package,
                "step" => ShapeKind::Step,
                "callout" => ShapeKind::Callout,
                "stored_data" => ShapeKind::StoredData,
                "person" => ShapeKind::Person,
                "diamond" => ShapeKind::Diamond,
                "oval" => ShapeKind::Oval,
                "circle" => ShapeKind::Circle,
                "hexagon" => ShapeKind::Hexagon,
                "cloud" => ShapeKind::Cloud,
                "c4-person" => ShapeKind::C4Person,
                _ => panic!("unsupported Source shape"),
            };
            let size = Size {
                width: row["width"].as_f64().unwrap(),
                height: row["height"].as_f64().unwrap(),
            };
            let mut graph = Graph::default();
            let id = graph.add_node(Node {
                shape,
                ..Node::new("probe", size)
            });
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                boxes: BTreeMap::from([(
                    id,
                    Rect {
                        origin: Point {
                            x: row["x"].as_f64().unwrap(),
                            y: row["y"].as_f64().unwrap(),
                        },
                        size,
                    },
                )]),
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                edge_label_percentage_commits: Default::default(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 0,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let metadata = row["metadata"].as_str().unwrap();
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    fill: Some("B3".into()),
                    stroke: Some("B1".into()),
                    stroke_width: Some(row["stroke"].as_u64().unwrap().to_string()),
                    link: (metadata == "link" || metadata == "both")
                        .then(|| "https://example.com".into()),
                    tooltip: (metadata == "tooltip" || metadata == "both").then(|| "Tip".into()),
                    ..Default::default()
                },
            )]);
            let svg = render_svg(&graph, &result, &BTreeMap::new(), &styles);
            let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
            let anchors: Vec<_> = document
                .descendants()
                .filter(|node| {
                    node.attribute("class")
                        .is_some_and(|class| class.split_whitespace().any(|c| c == "appendix-icon"))
                })
                .map(|node| node.attribute("transform").unwrap())
                .collect();
            let viewboxes: Vec<_> = document
                .descendants()
                .filter(|node| node.has_tag_name("svg"))
                .filter_map(|node| node.attribute("viewBox"))
                .collect();
            if serde_json::json!(anchors) != row["anchors"]
                || serde_json::json!(viewboxes) != row["viewboxes"]
            {
                differences.push(serde_json::json!({"name":row["name"],"expected_anchors":row["anchors"],"actual_anchors":anchors,"expected_viewboxes":row["viewboxes"],"actual_viewboxes":viewboxes}));
            }
        }
        assert!(
            differences.is_empty(),
            "pristine badge anchor/viewbox differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
mod tooltip_target_handoff_source_contract {
    use super::*;
    use d2_rs_core::{LayoutReport, Score, ast::Shared};
    use d2_rs_language::{
        compiler::{compile_ir, lower_graph_with_measurer_and_labels},
        parse,
    };

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn tooltip_metadata_and_positions_reach_target_bytes() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-language"),
            "tests/fixtures/tooltip-handoff-source-v090.json",
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                "tooltip.d2",
                row["source"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none());
            let (compiler, root) = compile_ir(Shared::new(parsed.map)).unwrap();
            let lowered = lower_graph_with_measurer_and_labels(&compiler, root, &|_: &str| Size {
                width: 160.0,
                height: 160.0,
            });
            if !row["error"].as_str().unwrap().is_empty() {
                assert_eq!(
                    lowered.unwrap_err().to_string(),
                    row["error"].as_str().unwrap()
                );
                continue;
            }
            let (graph, labels, styles) = lowered.unwrap();
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                boxes: BTreeMap::new(),
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                edge_label_percentage_commits: Default::default(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 0,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let actual:Vec<_> = graph.nodes().map(|(id,_)| {
                let shape = target_hash_shape(&graph,&result,&labels,&styles,id).unwrap();
                let bytes = serde_json::to_value(&shape).unwrap();
                // Original Diagram.Bytes omits a nil position and carries
                // nonempty positions even when tooltip text itself is absent.
                assert_eq!(bytes.get("tooltipPosition").and_then(Value::as_str),shape.tooltip_position.as_deref());
                serde_json::json!({"id":shape.id,"tooltip":shape.tooltip,"position":shape.tooltip_position.unwrap_or_default()})
            }).collect();
            if serde_json::json!(actual) != row["targets"] {
                differences.push(serde_json::json!({"name":row["name"],"actual":actual,"expected":row["targets"]}));
            }
        }
        assert!(
            differences.is_empty(),
            "pristine target tooltip handoff differences: {differences:#?}"
        );
    }
}

fn native_markdown_measurer(
    family: MarkdownFamily,
) -> Option<markdown_measure::MarkdownMeasurer<'static>> {
    use d2_rs_text::measure as fonts;
    static REGULAR: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static BOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static ITALIC: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static SEMIBOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static MONO: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static MONO_BOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static MONO_ITALIC: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static MONO_SEMIBOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static HAND: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    static HAND_BOLD: OnceLock<Option<TrueTypeFont>> = OnceLock::new();
    let mono = native_markdown_font(&MONO, fonts::bundled_source_code_pro_regular)?;
    let mono_bold = native_markdown_font(&MONO_BOLD, fonts::bundled_source_code_pro_bold)?;
    let mono_italic = native_markdown_font(&MONO_ITALIC, fonts::bundled_source_code_pro_italic)?;
    let mono_semibold = native_markdown_font(&MONO_SEMIBOLD, fonts::bundled_source_code_pro_semibold)?;
    let (regular, bold, italic, semibold) = match family.primary_kind() {
        BundledFamily::Sans => (
            native_markdown_font(&REGULAR, fonts::bundled_source_sans_pro_regular)?,
            native_markdown_font(&BOLD, fonts::bundled_source_sans_pro_bold)?,
            native_markdown_font(&ITALIC, fonts::bundled_source_sans_pro_italic)?,
            native_markdown_font(&SEMIBOLD, fonts::bundled_source_sans_pro_semibold)?,
        ),
        BundledFamily::Mono => (mono, mono_bold, mono_italic, mono_semibold),
        BundledFamily::HandDrawn => {
            let regular = native_markdown_font(&HAND, fonts::bundled_fuzzy_bubbles_regular)?;
            let bold = native_markdown_font(&HAND_BOLD, fonts::bundled_fuzzy_bubbles_bold)?;
            (regular, bold, regular, bold)
        }
    };
    let (mono, mono_bold, mono_italic, mono_semibold) = match family.mono_kind() {
        BundledFamily::Mono => (mono, mono_bold, mono_italic, mono_semibold),
        BundledFamily::Sans => (
            native_markdown_font(&REGULAR, fonts::bundled_source_sans_pro_regular)?,
            native_markdown_font(&BOLD, fonts::bundled_source_sans_pro_bold)?,
            native_markdown_font(&ITALIC, fonts::bundled_source_sans_pro_italic)?,
            native_markdown_font(&SEMIBOLD, fonts::bundled_source_sans_pro_semibold)?,
        ),
        BundledFamily::HandDrawn => {
            let regular = native_markdown_font(&HAND, fonts::bundled_fuzzy_bubbles_regular)?;
            let bold = native_markdown_font(&HAND_BOLD, fonts::bundled_fuzzy_bubbles_bold)?;
            (regular, bold, regular, bold)
        }
    };
    Some(markdown_measure::MarkdownMeasurer {
        font: regular,
        bold_font: bold,
        italic_font: italic,
        semibold_font: semibold,
        mono_font: mono,
        mono_bold_font: mono_bold,
        mono_italic_font: mono_italic,
        mono_semibold_font: mono_semibold,
    })
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod positioned_tooltip_measure_source_contract {
    use super::*;
    #[test]
    fn tooltip_markdown_dimensions_match_pristine_layout() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/positioned-tooltip-source-v090.json"))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let text = row["shape"]["tooltip"].as_str().unwrap();
            let family = if row["shape"]["fontFamily"] == "mono" {
                MarkdownFamily::Mono
            } else {
                MarkdownFamily::Sans
            };
            if !row["layout_error"].as_str().unwrap().is_empty() {
                assert!(
                    first_unsupported_markdown_html(text).is_some(),
                    "{}",
                    row["name"]
                );
                continue;
            }
            let dimensions = native_markdown_measurer(family)
                .unwrap()
                .markdown_dimensions(text, 16, false, false, family.uses_mono_css());
            let expected = &row["layout"];
            if dimensions.width != expected["Width"].as_f64().unwrap()
                || dimensions.height != expected["Height"].as_f64().unwrap()
            {
                differences.push(serde_json::json!({"name":row["name"],"actual":[dimensions.width,dimensions.height],"expected":[expected["Width"],expected["Height"]]}));
            }
        }
        assert!(
            differences.is_empty(),
            "pristine tooltip measurement differences: {differences:#?}"
        );
    }
}

fn positioned_tooltip_rect(
    shape: Rect,
    style: &SourceNodeStyle,
    family: MarkdownFamily,
) -> Option<Rect> {
    let position = style
        .tooltip_position
        .as_deref()
        .filter(|value| !value.is_empty())?;
    let tooltip = style.tooltip.as_deref().filter(|value| !value.is_empty())?;
    let family = if style
        .font
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case("mono"))
    {
        family.for_mono_text()
    } else {
        family
    };
    let size = if let Some(measurer) = native_markdown_measurer(family)
        .filter(|_| first_unsupported_markdown_html(tooltip).is_none())
    {
        measurer.markdown_dimensions(tooltip, 16, false, false, family.uses_mono_css())
    } else {
        // Original target bounding fallback; checked SVG font construction is
        // backed by the bundled immutable faces in normal native operation.
        Size {
            width: (tooltip.len() * 8).min(180) as f64,
            height: 10.0,
        }
    };
    let size = Size {
        width: size.width + 20.0,
        height: size.height + 20.0,
    };
    let x = match position {
        "top-left" | "bottom-left" => shape.origin.x,
        "top-right" | "bottom-right" => shape.right() - size.width,
        "center-left" => shape.origin.x - size.width - 10.0,
        "center-right" => shape.right() + 10.0,
        _ => shape.origin.x + shape.size.width / 2.0 - size.width / 2.0,
    };
    let y = match position {
        "bottom-left" | "bottom-center" | "bottom-right" => shape.bottom() + 10.0,
        "center-left" | "center-right" => {
            shape.origin.y + shape.size.height / 2.0 - size.height / 2.0
        }
        _ => shape.origin.y - size.height - 10.0,
    };
    Some(Rect {
        origin: Point { x, y },
        size,
    })
}

fn draw_positioned_tooltip(
    svg: &mut String,
    shape: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    family: MarkdownFamily,
    precision: bool,
) {
    let Some(rect) = positioned_tooltip_rect(shape, style, family) else {
        return;
    };
    let position = style.tooltip_position.as_deref().unwrap();
    let width = rect.size.width;
    let height = rect.size.height;
    let (x, y, direction) = match position {
        "top-left" => (20.0, height, "bottom"),
        "top-right" => (width - 20.0, height, "bottom"),
        "center-left" => (width, height / 2.0, "right"),
        "center-right" => (0.0, height / 2.0, "left"),
        "bottom-left" => (20.0, 0.0, "top"),
        "bottom-center" => (width / 2.0, 0.0, "top"),
        "bottom-right" => (width - 20.0, 0.0, "top"),
        _ => (width / 2.0, height, "bottom"),
    };
    let x = x + rect.origin.x;
    let y = y + rect.origin.y;
    let points = match direction {
        "top" => [(x - 4.0, y), (x + 4.0, y), (x, y - 8.0)],
        "bottom" => [(x - 4.0, y), (x + 4.0, y), (x, y + 8.0)],
        "left" => [(x, y - 4.0), (x, y + 4.0), (x - 8.0, y)],
        _ => [(x, y - 4.0), (x, y + 4.0), (x + 8.0, y)],
    };
    let fill = themed_color(None, "N7", palette);
    let stroke = themed_color(None, "N5", palette);
    let _ = write!(
        svg,
        "<g class=\"positioned-tooltip\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" ry=\"4\" class=\"fill-N7 stroke-N5\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/><path d=\"M {} {} L {} {} L {} {} Z\" class=\"fill-N7 stroke-N5\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
        number(rect.origin.x),
        number(rect.origin.y),
        number(width),
        number(height),
        fill,
        stroke,
        number(points[0].0),
        number(points[0].1),
        number(points[1].0),
        number(points[1].1),
        number(points[2].0),
        number(points[2].1),
        fill,
        stroke
    );
    let font_family = if style
        .font
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case("mono"))
    {
        MarkdownFamily::Mono
    } else {
        family
    };
    let start = svg.len();
    draw_native_markdown_subset_in_family(
        svg,
        style.tooltip.as_deref().unwrap(),
        Rect {
            origin: Point {
                x: rect.origin.x + 10.0,
                y: rect.origin.y + 10.0,
            },
            size: Size {
                width: width - 20.0,
                height: height - 20.0,
            },
        },
        16,
        &palette.color("N1", palette.base.neutrals.n1),
        Some("d2-rs-text-N1"),
        precision,
        font_family,
    );
    if precision {
        prepare_native_markdown_text(svg, start, false, true);
    }
    svg.push_str("</g>");
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod positioned_tooltip_svg_source_contract {
    use super::*;
    use d2_rs_core::{LayoutReport, Score};
    #[test]
    fn positioned_tooltip_box_tail_bounds_content_and_errors_match_source() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/positioned-tooltip-source-v090.json"))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let shape = &row["shape"];
            let mut graph = Graph::default();
            let size = Size {
                width: shape["width"].as_f64().unwrap(),
                height: shape["height"].as_f64().unwrap(),
            };
            let id = graph.add_node(Node::new("probe", size));
            let rect = Rect {
                origin: Point {
                    x: shape["pos"]["x"].as_f64().unwrap(),
                    y: shape["pos"]["y"].as_f64().unwrap(),
                },
                size,
            };
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    fill: Some("B3".into()),
                    stroke: Some("B1".into()),
                    stroke_width: Some("3".into()),
                    tooltip: Some(shape["tooltip"].as_str().unwrap().into()),
                    tooltip_position: Some(shape["tooltipPosition"].as_str().unwrap().into()),
                    link: shape
                        .get("link")
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                        .map(str::to_owned),
                    font: Some(shape["fontFamily"].as_str().unwrap().into()),
                    ..Default::default()
                },
            )]);
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                boxes: BTreeMap::from([(id, rect)]),
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                edge_label_percentage_commits: Default::default(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 0,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let rendered = render_svg_with_sketch_checked(
                &graph,
                &result,
                &BTreeMap::new(),
                &styles,
                RenderOptions::default(),
                false,
            );
            if !row["render_error"].as_str().unwrap().is_empty() {
                assert_eq!(
                    rendered.unwrap_err().to_string(),
                    row["render_error"].as_str().unwrap()
                );
                continue;
            }
            let svg = rendered.unwrap();
            let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
            let groups: Vec<_> = document
                .descendants()
                .filter(|n| n.attribute("class") == Some("positioned-tooltip"))
                .collect();
            let actual:Vec<_>=groups.iter().map(|group|{
                let rect=group.children().find(|n|n.has_tag_name("rect")).unwrap();let tail=group.children().find(|n|n.has_tag_name("path")).unwrap();
                let coords:Vec<f64>=["x","y","width","height","rx","ry"].iter().map(|name|rect.attribute(*name).unwrap().parse().unwrap()).collect();
                let texts:Vec<_>=group.descendants().filter(|n|n.has_tag_name("text")).map(|n|n.descendants().filter(|n|n.is_text()).filter_map(|n|n.text()).collect::<String>()).collect();
                serde_json::json!({"box":coords,"tail":tail.attribute("d").unwrap(),"text":texts})
            }).collect();
            let expected: Vec<_> = row["callouts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| serde_json::json!({"box":v["box"],"tail":v["tail"],"text":v["text"]}))
                .collect();
            let viewboxes: Vec<_> = document
                .descendants()
                .filter(|n| n.has_tag_name("svg"))
                .filter_map(|n| n.attribute("viewBox"))
                .collect();
            let bounds = bounds(&graph, &result, &BTreeMap::new(), &styles);
            let expected_bounds = (
                row["top_left"]["x"].as_f64().unwrap(),
                row["top_left"]["y"].as_f64().unwrap(),
                row["bottom_right"]["x"].as_f64().unwrap(),
                row["bottom_right"]["y"].as_f64().unwrap(),
            );
            if actual != expected
                || serde_json::json!(viewboxes) != row["svg_viewboxes"]
                || bounds != expected_bounds
            {
                differences.push(serde_json::json!({"name":row["name"],"actual":actual,"expected":expected,"viewboxes":viewboxes,"expected_viewboxes":row["svg_viewboxes"],"bounds":bounds,"expected_bounds":expected_bounds}));
            }
        }
        assert!(
            differences.is_empty(),
            "Source callout SVG contract differences: {differences:#?}"
        );
    }
}

// Original Go markdownPainter.paintFlow consumes parsed block siblings, rather
// than joining source lines before deciding whether a list interrupts a p.
// Reuse the pinned native Goldmark projection and existing inline painters.
fn draw_native_markdown_flow(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    precision: bool,
    family: MarkdownFamily,
) -> bool {
    let html = format!(
        "<body>{}</body>",
        markdown_measure::legacy_inline::measurement_html(&render_html(text.as_bytes()))
    );
    let Ok(document) = roxmltree::Document::parse(&html) else {
        return false;
    };
    let blocks = document
        .root_element()
        .children()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>();
    if blocks.len() < 2 {
        return false;
    }
    let mut content = String::new();
    let mut y = 0.0;
    let base = f64::from(font_size);
    for (index, block) in blocks.iter().enumerate() {
        let heading_level = block
            .tag_name()
            .name()
            .strip_prefix('h')
            .and_then(|level| level.parse::<usize>().ok())
            .filter(|level| (1..=6).contains(level));
        // Go paintFlow collapses the preceding 16px bottom margin with
        // a heading's 24px top margin. Root first-child margins are omitted.
        if index > 0 {
            y += if heading_level.is_some() { 24.0 } else { 16.0 };
        }
        let inner = |node: roxmltree::Node<'_, '_>| -> String {
            node.children()
                .map(|child| html[child.range()].to_owned())
                .collect::<String>()
        };
        let mut child = String::new();
        let mut height = 0.0;
        if let Some(level) = heading_level {
            let markup = inner(*block);
            let Some(runs) = native_markdown_html_inline(&format!("<p>{markup}</p>\n")) else {
                return false;
            };
            if !draw_native_markdown_heading_at(
                &mut child,
                &runs,
                level,
                Rect {
                    origin: Point::default(),
                    size: rect.size,
                },
                font_size,
                fill,
                class,
                precision,
                family,
                Some(&mut height),
                y,
            ) {
                return false;
            }
        } else if block.has_tag_name("p") {
            let markup = inner(*block);
            let Some(runs) = native_markdown_html_inline(&format!("<p>{markup}</p>\n")) else {
                return false;
            };
            if !draw_native_markdown_inline_context(
                &mut child,
                &split_native_markdown_lines(&runs),
                Rect {
                    origin: Point::default(),
                    size: rect.size,
                },
                font_size,
                fill,
                class,
                precision,
                family,
                Point { x: 0.0, y },
                false,
                "",
                Some(&mut height),
            ) {
                return false;
            }
        } else if block.has_tag_name("table") {
            let Some(table_height) = markdown_table::paint_node(
                &mut child,
                *block,
                &html,
                Point { x: 0.0, y },
                rect.size.width,
                font_size,
                fill,
                class,
                precision,
                family,
            ) else {
                return false;
            };
            height = table_height;
        } else if block.has_tag_name("ul") || block.has_tag_name("ol") {
            let ordered = block.has_tag_name("ol");
            let start = block
                .attribute("start")
                .and_then(|n| n.parse::<i64>().ok())
                .unwrap_or(1);
            let items = block
                .children()
                .filter(|n| n.is_element())
                .collect::<Vec<_>>();
            if items.is_empty() {
                return false;
            }
            for (item_index, item) in items.iter().enumerate() {
                if !item.has_tag_name("li")
                    || item.children().any(|n| {
                        n.is_element()
                            && matches!(
                                n.tag_name().name(),
                                "ul" | "ol" | "p" | "blockquote" | "pre"
                            )
                    })
                {
                    return false;
                }
                let markup = inner(*item).trim().to_owned();
                let marker = if ordered {
                    format!("{}. ", start + item_index as i64)
                } else {
                    "- ".to_owned()
                };
                let mut item_svg = String::new();
                if item_index > 0 {
                    height += base * 0.25;
                }
                if !draw_native_markdown_list_at(
                    &mut item_svg,
                    &format!("{marker}{markup}"),
                    Rect {
                        origin: Point::default(),
                        size: rect.size,
                    },
                    font_size,
                    fill,
                    class,
                    precision,
                    family,
                    y + height,
                ) {
                    return false;
                }
                let Some(runs) = native_markdown_html_inline(&format!("<p>{markup}</p>\n")) else {
                    return false;
                };
                if runs.iter().any(|r| r.line_break) {
                    return false;
                }
                let inside =
                    &item_svg[item_svg.find('>').unwrap() + 1..item_svg.len() - "</svg>".len()];
                child.push_str(inside);
                height += base * 1.5;
            }
        } else {
            return false;
        }
        let inside = if block.has_tag_name("p") || heading_level.is_some() {
            &child[child.find('>').unwrap() + 1..child.len() - "</svg>".len()]
        } else {
            &child
        };
        let _ = write!(
            content,
            "<g transform=\"translate(0 {})\">{}</g>",
            native_markdown_text_number(0.0, precision),
            inside
        );
        y += height;
    }
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">{}</g></svg>",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width),
        number(rect.size.height),
        number(rect.size.width),
        number(rect.size.height),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
        content
    );
    true
}

fn draw_native_markdown_paragraph_document(
    svg: &mut String,
    text: &str,
    rect: Rect,
    font_size: u32,
    fill: &str,
    class: Option<&str>,
    precision: bool,
    family: MarkdownFamily,
) -> bool {
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    if paragraphs.len() < 2 {
        return false;
    }
    let Some(measurer) = native_markdown_measurer(family) else {
        return false;
    };
    let mut content = String::new();
    let mut y = 0.0;
    for (index, paragraph) in paragraphs.iter().enumerate() {
        let dimensions = measurer.markdown_dimensions(
            paragraph,
            font_size,
            false,
            false,
            family.uses_mono_css(),
        );
        let mut child = String::new();
        if !draw_native_markdown_paragraph(
            &mut child,
            paragraph,
            Rect {
                origin: Point::default(),
                size: dimensions,
            },
            font_size,
            fill,
            class,
            precision,
            family,
        ) {
            return false;
        }
        if index > 0 {
            y += 16.0;
        }
        let inner = &child[child.find('>').unwrap() + 1..child.len() - "</svg>".len()];
        let _ = write!(
            content,
            "<g transform=\"translate(0 {})\">{}</g>",
            number(y),
            inner
        );
        y += dimensions.height;
    }
    let _ = write!(
        svg,
        "<svg x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\"><g class=\"md md-native{}\">{}</g></svg>",
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width),
        number(rect.size.height),
        number(rect.size.width),
        number(rect.size.height),
        if family.uses_mono_css() {
            " md-primary-mono"
        } else {
            ""
        },
        content
    );
    true
}

#[cfg(all(test, target_arch = "aarch64", feature = "oracle-fixtures"))]
mod circle_arrow_source_contract {
    use super::*;
    #[test]
    fn circle_marker_matrices_radii_and_stroke_modes_match_pristine_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/circle-arrow-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 672);
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let route = row["route"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| Point {
                    x: p[0].as_f64().unwrap(),
                    y: p[1].as_f64().unwrap(),
                })
                .collect::<Vec<_>>();
            let arrow = raster_circle_arrow(
                "probe".to_owned(),
                row["kind"].as_str().unwrap(),
                &route,
                &route,
                row["target"].as_bool().unwrap(),
                Point { x: 0.0, y: 0.0 },
                0.0,
                0.0,
                row["width"].as_f64().unwrap(),
                "#0D32B2",
                "#FFFFFF",
            )
            .unwrap();
            let path = &arrow.paths[0];
            let RasterPathCommand::MoveTo(right) = path.commands[0] else {
                panic!("ellipse start")
            };
            let RasterPathCommand::CubicTo(_, _, bottom) = path.commands[1] else {
                panic!("ellipse quarter")
            };
            let mut values = path.source_transform.unwrap().to_vec();
            values.extend([bottom.x, right.y, right.x - bottom.x, right.x - bottom.x]);
            let source = row["bits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>();
            let actual = values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            if actual != source {
                differences.push(
                    serde_json::json!({"name":row["name"],"expected":source,"actual":actual}),
                );
            }
            let hollow = row["kind"] == "circle";
            assert_eq!(path.stroke.is_some(), hollow);
            assert_eq!(path.stroke_style.is_some(), hollow);
            assert_eq!(path.fill, if hollow { "#FFFFFF" } else { "#0D32B2" });
        }
        assert!(
            differences.is_empty(),
            "circle Source geometry differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
mod markdown_flow_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn parsed_paragraph_list_flow_matches_original_go_complete_primitives() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-flow-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 72);
        check_flow_rows(&rows);
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn heading_paragraph_list_flow_matches_original_go_complete_primitives() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-heading-flow-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 324);
        check_flow_rows(&rows);
    }
    fn check_flow_rows(rows: &Value) {
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let family = match row["family"].as_str().unwrap() {
                "SourceSansPro" => MarkdownFamily::Sans,
                "SourceCodePro" => MarkdownFamily::Mono,
                "HandDrawn" => MarkdownFamily::HandDrawn,
                _ => panic!("family"),
            };
            assert_eq!(
                render_html(row["source"].as_str().unwrap().as_bytes()),
                row["html"].as_str().unwrap(),
                "Source block projection {}",
                row["name"]
            );
            let measured = native_markdown_measurer(family)
                .unwrap()
                .markdown_dimensions(
                    row["source"].as_str().unwrap(),
                    row["size"].as_u64().unwrap() as u32,
                    false,
                    false,
                    family.uses_mono_css(),
                );
            let wanted_size = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            if measured != wanted_size {
                differences.push(format!(
                    "{} viewport: {measured:?} vs {wanted_size:?}",
                    row["name"]
                ));
            }
            let mut svg = String::new();
            assert!(
                draw_native_markdown_flow(
                    &mut svg,
                    row["source"].as_str().unwrap(),
                    Rect {
                        origin: Point::default(),
                        size: Size {
                            width: row["layout"]["Width"].as_f64().unwrap(),
                            height: row["layout"]["Height"].as_f64().unwrap()
                        }
                    },
                    row["size"].as_u64().unwrap() as u32,
                    "#112233",
                    None,
                    true,
                    family
                ),
                "flow rejected {}",
                row["name"]
            );
            let document = roxmltree::Document::parse(&svg).unwrap();
            let nodes = document
                .descendants()
                .filter(|n| n.has_tag_name("text") || n.has_tag_name("rect"))
                .collect::<Vec<_>>();
            let source = row["layout"]["Primitives"].as_array().unwrap();
            assert_eq!(
                nodes.len(),
                source.len(),
                "complete inventory {}",
                row["name"]
            );
            for (node, expected) in nodes.iter().zip(source) {
                assert_eq!(node.tag_name().name(), expected["Kind"].as_str().unwrap());
                let mut dy = 0.0;
                let mut dx = 0.0;
                for parent in node.ancestors() {
                    if let Some(t) = parent.attribute("transform") {
                        let values = t
                            .strip_prefix("translate(")
                            .unwrap()
                            .strip_suffix(')')
                            .unwrap()
                            .split_whitespace()
                            .map(|v| v.parse::<f64>().unwrap())
                            .collect::<Vec<_>>();
                        assert_eq!(values.len(), 2);
                        dx += values[0];
                        dy += values[1];
                    }
                }
                for (attr, field) in [("x", "X"), ("y", "Y")] {
                    let actual = node.attribute(attr).unwrap().parse::<f64>().unwrap()
                        + if attr == "y" { dy } else { dx };
                    let wanted = expected[field].as_f64().unwrap();
                    if actual != wanted {
                        differences.push(format!(
                            "{} {} {}: {actual} vs {wanted}",
                            row["name"], expected["Text"], attr
                        ));
                    }
                }
                if node.has_tag_name("text") {
                    assert_eq!(
                        node.text().unwrap_or_default(),
                        expected["Text"].as_str().unwrap()
                    );
                    assert_eq!(
                        node.attribute("font-size").unwrap().parse::<f64>().unwrap(),
                        expected["FontSize"].as_f64().unwrap()
                    );
                } else {
                    for (attr, field) in
                        [("width", "Width"), ("height", "Height"), ("rx", "Radius")]
                    {
                        let actual = node.attribute(attr).unwrap().parse::<f64>().unwrap();
                        let wanted = expected[field].as_f64().unwrap();
                        if actual != wanted {
                            differences
                                .push(format!("{} {attr}: {actual} vs {wanted}", row["name"]));
                        }
                    }
                }
            }
        }
        assert!(
            differences.is_empty(),
            "Source primitive differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
mod appendix_target_bounds_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn appendix_uses_complete_original_go_target_bounds_and_excludes_legend_card() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/appendix-bounds-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 96);
        for row in rows.as_array().unwrap() {
            let offset = row["offset"].as_f64().unwrap();
            let width = row["stroke"].as_u64().unwrap().to_string();
            let mut graph = Graph::default();
            let a = graph.add_node(Node::new(
                "a",
                Size {
                    width: 100.0,
                    height: 80.0,
                },
            ));
            let b = graph.add_node(Node::new(
                "b",
                Size {
                    width: 80.0,
                    height: 80.0,
                },
            ));
            let edge = graph.add_edge(d2_rs_core::Edge {
                source: a,
                target: b,
            });
            graph.set_edge_style(
                edge,
                EdgeStyle {
                    stroke_width: Some(width.clone()),
                    ..EdgeStyle::default()
                },
            );
            let mut a_style = SourceNodeStyle {
                stroke_width: Some(width.clone()),
                ..SourceNodeStyle::default()
            };
            match row["profile"].as_str().unwrap() {
                "shadow" => a_style.shadow = Some("true".into()),
                "multiple" => a_style.multiple = Some("true".into()),
                "3d" => a_style.three_dee = Some("true".into()),
                "plain" => (),
                _ => panic!("profile"),
            };
            let styles = BTreeMap::from([
                (a, a_style),
                (
                    b,
                    SourceNodeStyle {
                        stroke_width: Some(width),
                        tooltip: Some("details".into()),
                        ..SourceNodeStyle::default()
                    },
                ),
            ]);
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes: BTreeMap::from([
                    (
                        a,
                        Rect {
                            origin: Point {
                                x: 10.0 + offset,
                                y: 20.0 + offset,
                            },
                            size: Size {
                                width: 100.0,
                                height: 80.0,
                            },
                        },
                    ),
                    (
                        b,
                        Rect {
                            origin: Point {
                                x: 150.0 + offset,
                                y: 150.0 + offset,
                            },
                            size: Size {
                                width: 80.0,
                                height: 80.0,
                            },
                        },
                    ),
                ]),
                routes: BTreeMap::from([(
                    edge,
                    serde_json::from_value(row["route"].clone()).unwrap(),
                )]),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::new(),
                report: d2_rs_core::LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Default::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let expected = (
                row["bounds"]["left"].as_f64().unwrap(),
                row["bounds"]["top"].as_f64().unwrap(),
                row["bounds"]["right"].as_f64().unwrap(),
                row["bounds"]["bottom"].as_f64().unwrap(),
            );
            for legend in [false, true] {
                graph.legend = legend.then(|| Legend {
                    label: "Key".into(),
                    shapes: vec![d2_rs_core::LegendShape {
                        source_node: None,
                        font: None,
                        label: "Card".into(),
                        shape: ShapeKind::Rectangle,
                        fill: None,
                        stroke: None,
                    }],
                    connections: Vec::new(),
                    source_graph: None,
                });
                let actual = bounds_with_markdown_family_and_legend(
                    &graph,
                    &result,
                    &BTreeMap::new(),
                    &styles,
                    MarkdownFamily::Sans,
                    false,
                );
                assert_eq!(
                    actual, expected,
                    "Source target bounds {} legend={legend}",
                    row["name"]
                );
                for direct_api in [false, true] {
                    let svg = if direct_api {
                        let interactive = render_svg(&graph, &result, &BTreeMap::new(), &styles);
                        append_static_appendix(&interactive, &graph, &result, &styles)
                    } else {
                        render_svg_with_options(
                            &graph,
                            &result,
                            &BTreeMap::new(),
                            &styles,
                            RenderOptions {
                                force_appendix: true,
                                ..RenderOptions::default()
                            },
                        )
                    };
                    let svg = String::from_utf8(svg).unwrap();
                    let document = roxmltree::Document::parse(&svg).unwrap();
                    let line = document
                        .descendants()
                        .find(|n| {
                            n.has_tag_name("line")
                                && n.attribute("class") == Some("d2-rs-stroke-B2")
                        })
                        .unwrap();
                    for (attr, wanted) in [
                        ("x1", expected.0 - 40.0),
                        (
                            "x2",
                            expected
                                .2
                                .max(row["bounds"]["appendix_max_width"].as_f64().unwrap())
                                + 40.0,
                        ),
                        ("y1", expected.3 + 50.0),
                        ("y2", expected.3 + 50.0),
                    ] {
                        assert_eq!(
                            line.attribute(attr).unwrap().parse::<f64>().unwrap(),
                            wanted,
                            "Source separator {} {attr} API={direct_api} legend={legend}",
                            row["name"]
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod markdown_list_width_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn styled_list_items_match_original_go_complete_viewports() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-list-width-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 144);
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let family = match row["family"].as_str().unwrap() {
                "SourceSansPro" => MarkdownFamily::Sans,
                "SourceCodePro" => MarkdownFamily::Mono,
                "HandDrawn" => MarkdownFamily::HandDrawn,
                _ => panic!("family"),
            };
            let actual = native_markdown_measurer(family)
                .unwrap()
                .markdown_dimensions(
                    row["source"].as_str().unwrap(),
                    row["size"].as_u64().unwrap() as u32,
                    false,
                    false,
                    family.uses_mono_css(),
                );
            let expected = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            if actual != expected {
                differences.push(format!("{} {actual:?} vs {expected:?}", row["name"]));
            }
        }
        assert!(
            differences.is_empty(),
            "Source viewport differences {differences:#?}"
        );
    }
}

#[cfg(test)]
mod markdown_html_case_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn html_name_folding_preserves_source_measurement_paint_and_rejections() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-html-case-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 96);
        for row in rows.as_array().unwrap() {
            let text = row["source"].as_str().unwrap();
            let error = row["error"].as_str().unwrap();
            let unsupported = first_unsupported_markdown_html(text);
            if !error.is_empty() {
                assert_eq!(
                    RenderError::UnsupportedMarkdownHtml {
                        element: unsupported.expect("Source rejection")
                    }
                    .to_string(),
                    error,
                    "{}",
                    row["name"]
                );
                continue;
            }
            assert_eq!(unsupported, None, "Source accepted {}", row["name"]);
            assert_eq!(
                render_html(text.as_bytes()),
                row["html"].as_str().unwrap(),
                "Goldmark projection remains Source exact"
            );
            let family = match row["family"].as_str().unwrap() {
                "SourceSansPro" => MarkdownFamily::Sans,
                "SourceCodePro" => MarkdownFamily::Mono,
                "HandDrawn" => MarkdownFamily::HandDrawn,
                _ => panic!("family"),
            };
            let size = row["size"].as_u64().unwrap() as u32;
            let expected_size = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            assert_eq!(
                native_markdown_measurer(family)
                    .unwrap()
                    .markdown_dimensions(text, size, false, false, family.uses_mono_css()),
                expected_size,
                "Source viewport {}",
                row["name"]
            );
            let mut svg = String::new();
            assert!(draw_native_markdown_subset_in_family(
                &mut svg,
                text,
                Rect {
                    origin: Point::default(),
                    size: expected_size
                },
                size,
                "#112233",
                None,
                true,
                family
            ));
            let wrapped = format!("<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\">{svg}</svg>");
            let document = roxmltree::Document::parse(&wrapped).unwrap();
            let nodes = document
                .descendants()
                .filter(|n| n.has_tag_name("text") || n.has_tag_name("rect"))
                .collect::<Vec<_>>();
            let source = row["layout"]["Primitives"].as_array().unwrap();
            assert_eq!(
                nodes.len(),
                source.len(),
                "complete inventory {}",
                row["name"]
            );
            for (node, wanted) in nodes.iter().zip(source) {
                assert_eq!(node.tag_name().name(), wanted["Kind"].as_str().unwrap());
                let mut dy = 0.0;
                let mut dx = 0.0;
                for parent in node.ancestors() {
                    if let Some(t) = parent.attribute("transform") {
                        let values = t
                            .strip_prefix("translate(")
                            .unwrap()
                            .strip_suffix(')')
                            .unwrap()
                            .split_whitespace()
                            .map(|v| v.parse::<f64>().unwrap())
                            .collect::<Vec<_>>();
                        assert_eq!(values.len(), 2);
                        dx += values[0];
                        dy += values[1];
                    }
                }
                for (attr, field) in [("x", "X"), ("y", "Y")] {
                    let actual = node.attribute(attr).unwrap().parse::<f64>().unwrap()
                        + if attr == "y" { dy } else { dx };
                    assert_eq!(
                        actual,
                        wanted[field].as_f64().unwrap(),
                        "{} {attr}",
                        row["name"]
                    );
                }
                if node.has_tag_name("text") {
                    assert_eq!(
                        node.text().unwrap_or_default(),
                        wanted["Text"].as_str().unwrap()
                    );
                    assert_eq!(
                        node.attribute("font-size").unwrap().parse::<f64>().unwrap(),
                        wanted["FontSize"].as_f64().unwrap()
                    );
                    let link = node
                        .ancestors()
                        .find(|p| p.has_tag_name("a"))
                        .and_then(|p| p.attribute("href"))
                        .unwrap_or("");
                    assert_eq!(
                        link,
                        wanted["Link"].as_str().unwrap(),
                        "Source attribute value {}",
                        row["name"]
                    );
                } else {
                    for (attr, field) in
                        [("width", "Width"), ("height", "Height"), ("rx", "Radius")]
                    {
                        assert_eq!(
                            node.attribute(attr).unwrap().parse::<f64>().unwrap(),
                            wanted[field].as_f64().unwrap(),
                            "{} {attr}",
                            row["name"]
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod root_canvas_source_contract {
    use super::*;
    use d2_rs_core::ast::Shared;
    use d2_rs_language::{compiler, parse};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_root_styles_hashes_and_canvas_bounds() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/root-canvas-style-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 121);
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                "root-style.d2",
                row["source"].as_str().unwrap().as_bytes(),
                None,
            );
            if let Some(error) = parsed.error {
                assert_eq!(
                    error.to_string(),
                    row["compile_error"].as_str().unwrap(),
                    "{} parser",
                    row["name"]
                );
                continue;
            }
            let (compiler, mut root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            if let Some(target) = row["target"].as_str().filter(|t| !t.is_empty()) {
                root = compiler::resolve_board_target(&compiler, root, &format!("layers.{target}"))
                    .unwrap();
            }
            let lowered =
                compiler::lower_graph_with_measurer_and_labels(&compiler, root, &|_: &str| Size {
                    width: 0.0,
                    height: 0.0,
                });
            if let Some(expected) = row["compile_error"].as_str() {
                assert_eq!(
                    lowered.unwrap_err().to_string(),
                    expected,
                    "{} compiler",
                    row["name"]
                );
                continue;
            }
            let (graph, labels, styles) = lowered.unwrap();
            // Canvas controls have one empty fixed rectangle. Child-board
            // controls retain their measured geometry through the native
            // layout adapter rather than hand-written fixture dimensions.
            let layout = d2_tala_rs::Engine
                .layout(&graph, &d2_rs_core::LayoutOptions::default())
                .unwrap_or_else(|error| panic!("{} layout: {error}", row["name"]));
            let options = RenderOptions {
                pad: 20.0,
                target_root_label: row["target"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                include_config_in_hash: row["target"].as_str().unwrap_or("").is_empty(),
                ..RenderOptions::default()
            };
            let input = target_hash_input(&graph, &layout, &labels, &styles, &options, false);
            let mut stream = serde_json::Deserializer::from_slice(&input).into_iter::<Value>();
            let actual = stream.nth(2).unwrap().unwrap();
            assert_eq!(
                actual, row["root"],
                "{} complete exported root",
                row["name"]
            );
            let rendered = render_svg_with_raster_sources_checked(
                &graph, &layout, &labels, &styles, options, false,
            );
            if let Some(error) = row["error"].as_str() {
                assert_eq!(
                    rendered.unwrap_err().to_string(),
                    error,
                    "{} scene error",
                    row["name"]
                );
                continue;
            }
            let (svg, _) = rendered.unwrap();
            let svg = String::from_utf8(svg).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let inner = document
                .descendants()
                .find(|n| {
                    n.has_tag_name("svg")
                        && n.attribute("viewBox").is_some_and(|s| s.starts_with('-'))
                })
                .unwrap();
            let bounds = inner
                .attribute("viewBox")
                .unwrap()
                .split_whitespace()
                .map(|s| s.parse::<f64>().unwrap())
                .collect::<Vec<_>>();
            for (i, key) in ["X", "Y", "Width", "Height"].iter().enumerate() {
                assert_eq!(
                    bounds[i],
                    row["viewbox"][key].as_f64().unwrap(),
                    "{} {key}",
                    row["name"]
                );
            }
            let root_nodes = document
                .descendants()
                .filter(|n| n.attribute("data-d2-rs-root").is_some())
                .collect::<Vec<_>>();
            assert_eq!(
                root_nodes.len(),
                row["backgrounds"].as_array().unwrap().len(),
                "{} complete background inventory",
                row["name"]
            );
            for (node, wanted) in root_nodes
                .iter()
                .zip(row["backgrounds"].as_array().unwrap())
            {
                for (attr, key) in [("rx", "RadiusX"), ("ry", "RadiusY")] {
                    assert_eq!(
                        node.attribute(attr)
                            .or_else(|| (attr == "ry").then(|| node.attribute("rx")).flatten())
                            .unwrap_or("0")
                            .parse::<f64>()
                            .unwrap(),
                        wanted["primitive"][key].as_f64().unwrap(),
                        "{} {attr}",
                        row["name"]
                    );
                }
                for (attr, key) in [
                    ("x", "X"),
                    ("y", "Y"),
                    ("width", "Width"),
                    ("height", "Height"),
                ] {
                    assert_eq!(
                        node.attribute(attr).unwrap().parse::<f64>().unwrap(),
                        wanted["primitive"]["Box"][key].as_f64().unwrap(),
                        "{} {attr}",
                        row["name"]
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod markdown_background_source_contract {
    use super::*;
    use d2_rs_core::ast::Shared;
    use d2_rs_language::{compiler, parse};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn target_background_is_inside_markdown_clip_before_primitives() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-background-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 64);
        let mut accepted = 0;
        let mut rejected = 0;
        for row in rows.as_array().unwrap() {
            let parsed = parse("", row["source"].as_str().unwrap().as_bytes(), None);
            assert!(parsed.error.is_none());
            let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            let size = row["shape"]["fontSize"].as_u64().unwrap_or(40) as u32;
            let lowered =
                compiler::lower_graph_with_measurer_and_labels(&compiler, root, &|label: &str| {
                    native_markdown_measurer(MarkdownFamily::Sans)
                        .unwrap()
                        .markdown_dimensions(label, size, false, false, false)
                });
            if let Some(error) = row["error"].as_str() {
                assert_eq!(
                    lowered.unwrap_err().to_string(),
                    error,
                    "{} complete rejection",
                    row["name"]
                );
                rejected += 1;
                continue;
            }
            let (graph, labels, styles) = lowered.unwrap();
            let layout = d2_tala_rs::Engine
                .layout(&graph, &d2_rs_core::LayoutOptions::default())
                .unwrap();
            let options = RenderOptions {
                theme_id: row["theme"].as_i64(),
                ..RenderOptions::default()
            };
            for private in [false, true] {
                let (svg, _) = render_svg_with_raster_sources_checked(
                    &graph,
                    &layout,
                    &labels,
                    &styles,
                    options.clone(),
                    private,
                )
                .unwrap();
                let document =
                    roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
                let viewport = document
                    .descendants()
                    .find(|n| n.has_tag_name("svg") && n.attribute("overflow") == Some("hidden"))
                    .unwrap();
                let backgrounds: Vec<_> = viewport
                    .children()
                    .filter(|n| n.has_tag_name("rect"))
                    .collect();
                if row["background"].is_null() {
                    assert!(
                        backgrounds.is_empty(),
                        "{} transparent viewport",
                        row["name"]
                    );
                } else {
                    assert_eq!(
                        backgrounds.len(),
                        1,
                        "{} complete background inventory",
                        row["name"]
                    );
                    let background = backgrounds[0];
                    assert_eq!(
                        viewport.children().find(|n| n.is_element()).unwrap(),
                        background
                    );
                    let expected = &row["background"]["Primitive"];
                    for (attr, field) in [
                        ("x", "X"),
                        ("y", "Y"),
                        ("width", "Width"),
                        ("height", "Height"),
                    ] {
                        assert_eq!(
                            background.attribute(attr).unwrap().parse::<f64>().unwrap(),
                            expected["Box"][field].as_f64().unwrap(),
                            "{} {attr}",
                            row["name"]
                        );
                    }
                    let fill = row["shape"]["fill"].as_str().unwrap();
                    let expected_fill = if fill == "B6" {
                        let color = &expected["Fill"]["Color"];
                        format!(
                            "#{:02X}{:02X}{:02X}",
                            color["R"].as_u64().unwrap(),
                            color["G"].as_u64().unwrap(),
                            color["B"].as_u64().unwrap()
                        )
                    } else {
                        fill.to_owned()
                    };
                    assert_eq!(
                        background.attribute("fill").unwrap().to_ascii_lowercase(),
                        expected_fill.to_ascii_lowercase()
                    );
                    assert!(background.attribute("opacity").is_none());
                    assert!(background.attribute("stroke").is_none());
                }
            }
            accepted += 1;
        }
        assert_eq!((accepted, rejected), (48, 16));
    }
}

#[cfg(test)]
mod sequence_group_reference_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_group_inventory_matches_original_go_reference_scope_controls() {
        use d2_rs_core::ast::Shared;
        use d2_rs_language::{compiler, parse};
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/sequence-group-reference-source-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let parsed = parse("", row["source"].as_str().unwrap().as_bytes(), None);
            let (compiler, ir) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            let (graph, _, _) =
                compiler::lower_graph_with_measurer_and_labels(&compiler, ir, &|_: &str| {
                    d2_rs_core::Size {
                        width: 20.0,
                        height: 20.0,
                    }
                })
                .unwrap();
            let mut actual = graph
                .nodes()
                .filter(|(id, _)| super::is_sequence_group_like(&graph, *id))
                .map(|(_, n)| n.external_id.clone())
                .collect::<Vec<_>>();
            let mut expected = row["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected, "{}", row["name"]);
            for (id, node) in graph
                .nodes()
                .filter(|(id, _)| super::is_sequence_group_like(&graph, *id))
            {
                let fill = row["group_fills"][&node.external_id].as_str().unwrap();
                assert_eq!(
                    super::default_node_fill(&graph, id),
                    fill,
                    "{} {}",
                    row["name"],
                    node.external_id
                );
                assert!(
                    !super::has_sequence_note_fill(&graph, id),
                    "a reference-owning group is not a sequence note"
                );
            }
        }
    }
}

#[cfg(test)]
mod sequence_group_label_fill_source_contract {
    use super::*;
    use d2_rs_core::{Edge, LayoutReport, Node, Score};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn ordinary_group_label_repaints_target_fill_outside_geometry_blend_owner() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/sequence-group-label-fill-source-v090.json"))
        .unwrap();
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Default::default()
        };
        let mut boxes = BTreeMap::new();
        let mut labels = BTreeMap::new();
        let mut ids = Vec::new();
        for shape in fixture["shapes"].as_array().unwrap() {
            let mut node = Node::new(
                shape["id"].as_str().unwrap(),
                Size {
                    width: shape["width"].as_f64().unwrap(),
                    height: shape["height"].as_f64().unwrap(),
                },
            );
            node.label_size = Some(Size {
                width: shape["labelWidth"].as_f64().unwrap(),
                height: shape["labelHeight"].as_f64().unwrap(),
            });
            node.font_size = Some(shape["fontSize"].as_u64().unwrap() as u32);
            let size = node.size;
            let id = graph.add_node(node);
            ids.push(id);
            boxes.insert(
                id,
                Rect {
                    origin: Point {
                        x: shape["pos"]["x"].as_f64().unwrap(),
                        y: shape["pos"]["y"].as_f64().unwrap(),
                    },
                    size,
                },
            );
            labels.insert(id, shape["label"].as_str().unwrap().to_owned());
        }
        let edge = graph.add_edge(Edge {
            source: ids[0],
            target: ids[1],
        });
        graph.sequence_group_edges.insert(ids[2], vec![edge]);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes,
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let svg =
            String::from_utf8(render_svg(&graph, &result, &labels, &BTreeMap::new())).unwrap();
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let blend = doc
            .descendants()
            .find(|n| n.attribute("class") == Some("shape blend"))
            .unwrap();
        let owner = blend.parent_element().unwrap();
        let actual: Vec<_> = owner.descendants().filter(|n| n.has_tag_name("rect"))
            .map(|n| serde_json::json!({"x":n.attribute("x"),"y":n.attribute("y"),
                "width":n.attribute("width"),"height":n.attribute("height"),"fill":n.attribute("fill")})).collect();
        assert_eq!(serde_json::json!(actual), fixture["group_rectangles"]);
        assert_eq!(
            blend
                .descendants()
                .filter(|n| n.has_tag_name("rect"))
                .count(),
            1
        );
        assert!(!blend.descendants().any(|n| n.has_tag_name("text")));
        let label_background = blend.next_sibling_element().unwrap();
        assert!(label_background.has_tag_name("rect"));
        assert!(
            label_background
                .next_sibling_element()
                .unwrap()
                .has_tag_name("text")
        );
    }
}

#[cfg(test)]
mod sequence_render_roles_source_contract {
    use super::*;
    use d2_rs_core::{Edge, LayoutReport, Node, Score};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_labels_roles_and_shape_order_match_go_targets() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/sequence-render-roles-source-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut graph = Graph {
                root_sequence_diagram: row["name"] != "grid_nested_sequence",
                ..Default::default()
            };
            let mut ids = BTreeMap::new();
            let mut boxes = BTreeMap::new();
            let mut labels = BTreeMap::new();
            for shape in row["shapes"].as_array().unwrap() {
                let name = shape["id"].as_str().unwrap();
                let mut node = Node::new(
                    name,
                    Size {
                        width: shape["width"].as_f64().unwrap(),
                        height: shape["height"].as_f64().unwrap(),
                    },
                );
                node.parent = name.rsplit_once('.').and_then(|(p, _)| ids.get(p).copied());
                node.label_size = Some(Size {
                    width: shape["labelWidth"].as_f64().unwrap(),
                    height: shape["labelHeight"].as_f64().unwrap(),
                });
                node.font_size = Some(shape["fontSize"].as_u64().unwrap() as u32);
                if row["name"] == "grid_nested_sequence" && name == "grid" {
                    node.grid_columns = Some(1);
                }
                let size = node.size;
                let id = graph.add_node(node);
                ids.insert(name.to_owned(), id);
                graph.set_sequence_diagram(id, shape["type"] == "sequence_diagram");
                boxes.insert(
                    id,
                    Rect {
                        origin: Point {
                            x: shape["pos"]["x"].as_f64().unwrap(),
                            y: shape["pos"]["y"].as_f64().unwrap(),
                        },
                        size,
                    },
                );
                // Native input retains actor-path labels; Source clears them
                // only when converting actor descendants to target spans.
                labels.insert(
                    id,
                    if shape["label"].as_str().unwrap().is_empty() {
                        name.rsplit('.').next().unwrap().to_owned()
                    } else {
                        shape["label"].as_str().unwrap().to_owned()
                    },
                );
            }
            for reference in row["reference_edges"].as_array().unwrap() {
                let id = graph.add_edge(Edge {
                    source: ids[reference["source"].as_str().unwrap()],
                    target: ids[reference["target"].as_str().unwrap()],
                });
                for scope in reference["scopes"].as_array().unwrap() {
                    if let Some(owner) = ids.get(scope.as_str().unwrap()) {
                        graph
                            .sequence_group_edges
                            .entry(*owner)
                            .or_default()
                            .push(id);
                    }
                }
                assert_eq!(
                    sequence_connection_z_index(&graph, &graph.edges[id.0 as usize]),
                    4
                );
            }
            for shape in row["shapes"].as_array().unwrap() {
                let id = ids[shape["id"].as_str().unwrap()];
                assert_eq!(
                    sequence_shape_z_index(&graph, id),
                    shape["zIndex"].as_i64().unwrap() as i32,
                    "{} {}",
                    row["name"],
                    shape["id"]
                );
                assert_eq!(
                    is_sequence_span_like(&graph, id),
                    shape["zIndex"] == 2,
                    "{} {}",
                    row["name"],
                    shape["id"]
                );
            }
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes,
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let svg =
                String::from_utf8(render_svg(&graph, &result, &labels, &BTreeMap::new())).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let mut actual: Vec<_> = document
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .map(|n| n.text().unwrap_or("").to_owned())
                .collect();
            let mut expected: Vec<_> = row["shapes"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|s| {
                    s["label"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .collect();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected, "{} complete label inventory", row["name"]);
            let actual_order: Vec<_> = document
                .descendants()
                .filter_map(|n| {
                    let class = n.attribute("class")?;
                    ids.iter()
                        .find(|(name, _)| svg_object_class(name) == class)
                        .map(|(name, _)| name.clone())
                })
                .collect();
            let mut shapes = row["shapes"].as_array().unwrap().clone();
            shapes.sort_by_key(|s| (s["zIndex"].as_i64().unwrap(), s["level"].as_i64().unwrap()));
            let expected_order: Vec<_> = shapes
                .iter()
                .map(|s| s["id"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(
                actual_order, expected_order,
                "{} complete shape paint order",
                row["name"]
            );
        }
    }

    #[test]
    fn source_object_sort_interleaves_shapes_connections_and_notes_stably() {
        let mut paints = vec![
            ObjectPaint {
                z_index: 5,
                shape_level: Some(2),
                svg: "note".into(),
            },
            ObjectPaint {
                z_index: 4,
                shape_level: None,
                svg: "message-a".into(),
            },
            ObjectPaint {
                z_index: 3,
                shape_level: Some(2),
                svg: "group-child".into(),
            },
            ObjectPaint {
                z_index: 0,
                shape_level: None,
                svg: "ordinary-edge".into(),
            },
            ObjectPaint {
                z_index: 2,
                shape_level: Some(3),
                svg: "span".into(),
            },
            ObjectPaint {
                z_index: 3,
                shape_level: Some(1),
                svg: "group".into(),
            },
            ObjectPaint {
                z_index: 1,
                shape_level: None,
                svg: "lifeline".into(),
            },
            ObjectPaint {
                z_index: 0,
                shape_level: Some(1),
                svg: "actor".into(),
            },
            ObjectPaint {
                z_index: 4,
                shape_level: None,
                svg: "message-b".into(),
            },
        ];
        let mut svg = String::new();
        paint_objects_in_source_order(&mut svg, &mut paints);
        assert_eq!(
            svg,
            "actorordinary-edgelifelinespangroupgroup-childmessage-amessage-bnote"
        );
    }
}

#[cfg(test)]
mod sequence_note_references_source_contract {
    use super::*;
    use d2_rs_core::ast::Shared;
    use d2_rs_language::{
        compiler::{compile_ir, lower_graph},
        parse,
    };

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_reference_state_note_and_group_inventories_match_original_go() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/sequence-note-references-source-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                row["name"].as_str().unwrap(),
                row["source"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none());
            let lowered = compile_ir(Shared::new(parsed.map))
                .map_err(|error| error.to_string())
                .and_then(|(compiler, root)| {
                    if !compiler.errors.is_empty() {
                        Err(compiler
                            .errors
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n"))
                    } else {
                        lower_graph(&compiler, root).map_err(|error| error.to_string())
                    }
                });
            if let Some(error) = row.get("error") {
                assert_eq!(lowered.err().as_deref(), error.as_str());
                continue;
            }
            let graph = lowered.unwrap();
            let expected = row["objects"].as_array().unwrap();
            assert_eq!(
                graph.nodes().count(),
                expected.len(),
                "{} complete inventory",
                row["name"]
            );
            for object in expected {
                let (id, _) = graph
                    .nodes()
                    .find(|(_, node)| node.external_id == object["id"].as_str().unwrap())
                    .unwrap();
                assert_eq!(
                    graph.sequence_node_edge_refs.get(&id),
                    object["hasEdgeRef"].as_bool().as_ref(),
                    "{} {} explicit edge-reference metadata",
                    row["name"],
                    object["id"]
                );
                assert_eq!(
                    graph.sequence_contains_object(id),
                    object["containsObjects"].as_bool().unwrap(),
                    "{} {} object containment",
                    row["name"],
                    object["id"]
                );
                let contains_edges = graph.sequence_group_edges.iter().any(|(owner, edges)| {
                    !edges.is_empty() && super::is_descendant_or_self(&graph, *owner, id)
                });
                assert_eq!(
                    contains_edges,
                    object["containsEdges"].as_bool().unwrap(),
                    "{} {} edge containment",
                    row["name"],
                    object["id"]
                );
                assert_eq!(
                    super::has_sequence_note_fill(&graph, id),
                    object["note"].as_bool().unwrap(),
                    "{} {} note predicate",
                    row["name"],
                    object["id"]
                );
                assert_eq!(
                    super::is_sequence_group_like(&graph, id),
                    object["group"].as_bool().unwrap(),
                    "{} {} group predicate",
                    row["name"],
                    object["id"]
                );
            }
        }
    }
}

// Reads d2-rs-layout's regenerated `layout-sequence` capture (docs/FIXTURES.md).
#[cfg(all(test, feature = "oracle-fixtures"))]
mod lifeline_source_order_fallback_contract {
    use super::*;
    use d2_rs_core::{Edge, LayoutReport, Node, Score};
    #[test]
    fn complete_fallback_lifelines_match_original_go_target_routes() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-layout"),
            "tests/fixtures/sequence-lifeline-source-order-source-v090.json",
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut graph = Graph {
                root_sequence_diagram: true,
                ..Default::default()
            };
            let mut ids = BTreeMap::new();
            let mut boxes = BTreeMap::new();
            let mut routes = BTreeMap::new();
            for shape in row["shapes"].as_array().unwrap() {
                let name = shape["id"].as_str().unwrap();
                let size = Size {
                    width: shape["width"].as_f64().unwrap(),
                    height: shape["height"].as_f64().unwrap(),
                };
                let mut node = Node::new(name, size);
                node.parent = name.rsplit_once('.').and_then(|(p, _)| ids.get(p).copied());
                let id = graph.add_node(node);
                ids.insert(name.to_owned(), id);
                boxes.insert(
                    id,
                    Rect {
                        origin: Point {
                            x: shape["pos"]["x"].as_f64().unwrap(),
                            y: shape["pos"]["y"].as_f64().unwrap(),
                        },
                        size,
                    },
                );
            }
            let messages: Vec<_> = row["connections"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| !c["dst"].as_str().unwrap().contains("-lifeline-end-"))
                .collect();
            for (connection, line) in messages
                .iter()
                .zip(row["lines"]["edges"].as_array().unwrap())
            {
                let id = graph.add_edge(Edge {
                    source: ids[connection["src"].as_str().unwrap()],
                    target: ids[connection["dst"].as_str().unwrap()],
                });
                graph
                    .sequence_edge_lines
                    .insert(id, line["line"].as_i64().unwrap());
                routes.insert(
                    id,
                    connection["route"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| Point {
                            x: p["x"].as_f64().unwrap(),
                            y: p["y"].as_f64().unwrap(),
                        })
                        .collect(),
                );
            }
            assert_eq!(
                messages.len(),
                row["lines"]["edges"].as_array().unwrap().len()
            );
            let result = LayoutResult {
                boxes,
                routes,
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                edge_labels: Default::default(),
                node_labels: Default::default(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let actual = super::sequence_lifelines_in_scope(&graph, &result, None);
            let expected: Vec<_> = row["connections"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["dst"].as_str().unwrap().contains("-lifeline-end-"))
                .collect();
            assert_eq!(actual.len(), expected.len());
            assert_eq!(
                messages.len() + expected.len(),
                row["connections"].as_array().unwrap().len()
            );
            for line in expected {
                let actor = ids[line["src"].as_str().unwrap()];
                let native = actual.iter().find(|n| n.actor == actor).unwrap();
                let source: Vec<_> = line["route"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| Point {
                        x: p["x"].as_f64().unwrap(),
                        y: p["y"].as_f64().unwrap(),
                    })
                    .collect();
                assert_eq!(
                    vec![native.start, native.end],
                    source,
                    "{} lifeline",
                    row["name"]
                );
            }
        }
    }
}

#[cfg(test)]
mod extracted_group_label_source_contract {
    use super::*;
    use d2_rs_core::{LayoutReport, NodeLabelState, Score, ast::Shared};
    use d2_rs_language::{compiler, parse};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_grid_label_inventory_uses_live_positions_and_fonts() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/sequence-extracted-labels-source-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let parsed = parse("", row["source"].as_str().unwrap().as_bytes(), None);
            let (compiler, ir) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            let (mut graph, labels, styles) =
                compiler::lower_graph_with_measurer_and_labels(&compiler, ir, &|_: &str| Size {
                    width: 20.0,
                    height: 20.0,
                })
                .unwrap();
            let ids = graph
                .nodes()
                .map(|(id, node)| (node.external_id.clone(), id))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(ids.len(), row["target"]["shapes"].as_array().unwrap().len());
            let mut result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes: BTreeMap::new(),
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            for shape in row["target"]["shapes"].as_array().unwrap() {
                let id = ids[shape["id"].as_str().unwrap()];
                let size = Size {
                    width: shape["width"].as_f64().unwrap(),
                    height: shape["height"].as_f64().unwrap(),
                };
                graph.node_mut(id).unwrap().size = size;
                let label_size = Some(Size {
                    width: shape["labelWidth"].as_f64().unwrap(),
                    height: shape["labelHeight"].as_f64().unwrap(),
                });
                graph.node_mut(id).unwrap().label_size = label_size;
                result.boxes.insert(
                    id,
                    Rect {
                        origin: Point {
                            x: shape["pos"]["x"].as_f64().unwrap(),
                            y: shape["pos"]["y"].as_f64().unwrap(),
                        },
                        size,
                    },
                );
                result.node_labels.insert(
                    id,
                    NodeLabelState {
                        font_size_committed: false,
                        size: label_size,
                        font_size: Some(shape["fontSize"].as_u64().unwrap() as u32),
                        position: LabelPosition::from_d2_name(
                            shape["labelPosition"].as_str().unwrap(),
                        ),
                        icon_position: None,
                    },
                );
            }
            for connection in row["target"]["connections"].as_array().unwrap() {
                let route = connection["route"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|point| Point {
                        x: point["x"].as_f64().unwrap(),
                        y: point["y"].as_f64().unwrap(),
                    })
                    .collect::<Vec<_>>();
                let source = ids[connection["src"].as_str().unwrap()];
                let target = connection["dst"].as_str().unwrap();
                if target.contains("-lifeline-end-") {
                    result
                        .sequence_lifelines
                        .push(d2_rs_core::SequenceLifeline {
                            actor: source,
                            target_id: target.to_owned(),
                            route,
                        });
                } else {
                    let edge = graph
                        .edges()
                        .find(|(_, edge)| edge.source == source && edge.target == ids[target])
                        .unwrap()
                        .0;
                    result.routes.insert(edge, route);
                }
            }
            assert_eq!(result.routes.len(), graph.edges.len());
            let svg = String::from_utf8(render_svg(&graph, &result, &labels, &styles)).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let texts=doc.descendants().filter(|node| node.has_tag_name("text")).map(|node| serde_json::json!({"text":node.text().unwrap_or(""),"x":node.attribute("x"),"y":node.attribute("y"),"font":node.attribute("font-size")})).collect::<Vec<_>>();
            assert_eq!(serde_json::json!(texts), row["texts"], "{}", row["name"]);
        }
    }
}

#[cfg(test)]
mod connection_label_fill_source_contract {
    use super::*;
    use d2_rs_core::{EdgeLabel, LayoutReport, NodeLabelState, Score, ast::Shared};
    use d2_rs_language::{compiler, parse};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_connection_label_fill_is_a_rounded_sibling_of_masked_geometry() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/connection-label-fill-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 9);
        let mut accepted = 0;
        let mut rejected = 0;
        for row in rows.as_array().unwrap() {
            let parsed = parse("", row["source"].as_str().unwrap().as_bytes(), None);
            let (compiler, ir) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            let lowered =
                compiler::lower_graph_with_measurer_and_labels(&compiler, ir, &|_: &str| Size {
                    width: 20.0,
                    height: 20.0,
                });
            if row["expected_error"] == true {
                assert!(lowered.is_err());
                rejected += 1;
                continue;
            }
            accepted += 1;
            let (mut graph, labels, styles) = lowered.unwrap();
            let ids = graph
                .nodes()
                .map(|(id, n)| (n.external_id.clone(), id))
                .collect::<BTreeMap<_, _>>();
            let shapes = row["target"]["shapes"].as_array().unwrap();
            assert_eq!(ids.len(), shapes.len());
            let mut result = LayoutResult {
                boxes: BTreeMap::new(),
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::new(),
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            for shape in shapes {
                let id = ids[shape["id"].as_str().unwrap()];
                let size = Size {
                    width: shape["width"].as_f64().unwrap(),
                    height: shape["height"].as_f64().unwrap(),
                };
                let label_size = Size {
                    width: shape["labelWidth"].as_f64().unwrap(),
                    height: shape["labelHeight"].as_f64().unwrap(),
                };
                graph.node_mut(id).unwrap().size = size;
                graph.node_mut(id).unwrap().label_size = Some(label_size);
                result.boxes.insert(
                    id,
                    Rect {
                        origin: Point {
                            x: shape["pos"]["x"].as_f64().unwrap(),
                            y: shape["pos"]["y"].as_f64().unwrap(),
                        },
                        size,
                    },
                );
                result.node_labels.insert(
                    id,
                    NodeLabelState {
                        font_size_committed: false,
                        size: Some(label_size),
                        font_size: Some(shape["fontSize"].as_u64().unwrap() as u32),
                        position: LabelPosition::from_d2_name(
                            shape["labelPosition"].as_str().unwrap(),
                        ),
                        icon_position: None,
                    },
                );
            }
            let connections = row["target"]["connections"].as_array().unwrap();
            assert_eq!(graph.edges.len(), connections.len());
            for connection in connections {
                let edge = graph
                    .edges()
                    .find(|(_, e)| {
                        e.source == ids[connection["src"].as_str().unwrap()]
                            && e.target == ids[connection["dst"].as_str().unwrap()]
                    })
                    .unwrap()
                    .0;
                graph.set_edge_font_size(edge, connection["fontSize"].as_u64().unwrap() as u32);
                graph.set_edge_label(
                    edge,
                    Some(EdgeLabel {
                        text: connection["label"].as_str().unwrap().to_owned(),
                        size: Size {
                            width: connection["labelWidth"].as_f64().unwrap(),
                            height: connection["labelHeight"].as_f64().unwrap(),
                        },
                        position: LabelPosition::from_d2_name(
                            connection["labelPosition"].as_str().unwrap(),
                        ),
                        percentage: connection["labelPercentage"].as_f64().unwrap(),
                    }),
                );
                result.routes.insert(
                    edge,
                    connection["route"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| Point {
                            x: p["x"].as_f64().unwrap(),
                            y: p["y"].as_f64().unwrap(),
                        })
                        .collect(),
                );
            }
            for native in [false, true] {
                let svg = if native {
                    render_svg_with_raster_sources_checked(
                        &graph,
                        &result,
                        &labels,
                        &styles,
                        RenderOptions::default(),
                        false,
                    )
                    .unwrap()
                    .0
                } else {
                    render_svg(&graph, &result, &labels, &styles)
                };
                let text = std::str::from_utf8(&svg).unwrap();
                let doc = roxmltree::Document::parse(text).unwrap();
                let actual = doc
                    .descendants()
                    .filter(|n| n.has_tag_name("rect") && n.attribute("rx") == Some("10"))
                    .collect::<Vec<_>>();
                let expected = row["backgrounds"].as_array().unwrap();
                assert_eq!(
                    actual.len(),
                    expected.len(),
                    "{} native{native}",
                    row["name"]
                );
                for (rect, source) in actual.iter().zip(expected) {
                    for attr in ["x", "y", "width", "height", "rx"] {
                        assert_eq!(
                            rect.attribute(attr),
                            source[attr].as_str(),
                            "{} {attr}",
                            row["name"]
                        );
                    }
                    let fill = rect.attribute("fill").unwrap();
                    if source["fill"].as_str().unwrap().starts_with("url(") {
                        assert!(fill.starts_with("url(#"));
                    } else {
                        assert_eq!(fill, source["fill"].as_str().unwrap());
                    }
                    assert!(
                        rect.ancestors().all(|n| n.attribute("mask").is_none()),
                        "label background must not enter geometry mask"
                    );
                }
            }
        }
        assert_eq!((accepted, rejected), (8, 1));
    }
}

#[cfg(test)]
mod original_polygon_arrowhead_contract {
    use super::*;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_go_polygon_commands_and_endpoint_matrices() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/polygon-arrowheads-source-v090.json"))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 210);
        for row in rows {
            let name = row["name"].as_str().unwrap();
            let width = row["width"].as_f64().unwrap();
            let target = row["target"].as_bool().unwrap();
            let route = [
                Point {
                    x: 80.25,
                    y: 190.75,
                },
                Point {
                    x: 80.25 + row["delta"][0].as_f64().unwrap(),
                    y: 190.75 + row["delta"][1].as_f64().unwrap(),
                },
            ];
            let arrow = raster_polygon_arrow(
                "probe".to_owned(),
                name,
                &route,
                &route,
                target,
                Point { x: -0.5, y: 0.25 },
                0.0,
                0.0,
                width,
                "#0D32B2",
                "#FFFFFF",
            )
            .unwrap();
            assert_eq!(arrow.paths.len(), 1);
            let path = &arrow.paths[0];
            let commands = row["commands"].as_array().unwrap();
            assert_eq!(path.commands.len(), commands.len(), "{row}");
            for (actual, expected) in path.commands.iter().zip(commands) {
                let (kind, point) = match actual {
                    RasterPathCommand::MoveTo(p) => (0, Some(p)),
                    RasterPathCommand::LineTo(p) => (1, Some(p)),
                    RasterPathCommand::Close => (5, None),
                    _ => panic!("unexpected polygon command"),
                };
                assert_eq!(kind, expected["kind"].as_u64().unwrap(), "{row}");
                if let Some(p) = point {
                    assert_eq!(
                        p.x.to_bits(),
                        expected["x"].as_f64().unwrap().to_bits(),
                        "{row}"
                    );
                    assert_eq!(
                        p.y.to_bits(),
                        expected["y"].as_f64().unwrap().to_bits(),
                        "{row}"
                    );
                }
            }
            #[cfg(target_arch = "aarch64")]
            for (actual, expected) in path
                .source_transform
                .unwrap()
                .iter()
                .zip(row["matrix_bits"].as_array().unwrap())
            {
                assert_eq!(
                    format!("{:016x}", actual.to_bits()),
                    expected.as_str().unwrap(),
                    "{row}"
                );
            }
            assert_eq!(path.fill != "none", row["fill"].as_bool().unwrap(), "{row}");
            assert_eq!(
                path.stroke.is_some(),
                row["stroke"].as_bool().unwrap(),
                "{row}"
            );
            if path.stroke.is_some() {
                assert_eq!(
                    path.stroke_width.unwrap(),
                    row["stroke_width"].as_f64().unwrap(),
                    "{row}"
                );
                assert_eq!(row["cap"], 0);
                let round = row["join"] == 1;
                assert_eq!(
                    path.stroke_style.unwrap().contains("stroke-linejoin:round"),
                    round,
                    "{row}"
                );
                if !round {
                    assert_eq!(row["miter_limit"], 4.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod original_outside_icon_bounds_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn all_original_target_shape_names_and_outside_icon_positions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/outside-icon-bounds-source-v090.json"))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 96);
        let positions = [
            LabelPosition::OutsideTopLeft,
            LabelPosition::OutsideTopCenter,
            LabelPosition::OutsideTopRight,
            LabelPosition::OutsideBottomLeft,
            LabelPosition::OutsideBottomCenter,
            LabelPosition::OutsideBottomRight,
            LabelPosition::OutsideLeftTop,
            LabelPosition::OutsideLeftMiddle,
            LabelPosition::OutsideLeftBottom,
            LabelPosition::OutsideRightTop,
            LabelPosition::OutsideRightMiddle,
            LabelPosition::OutsideRightBottom,
        ];
        let shape_kinds = [
            ShapeKind::Rectangle,
            ShapeKind::Oval,
            ShapeKind::Diamond,
            ShapeKind::Hexagon,
            ShapeKind::Cloud,
            ShapeKind::Cylinder,
            ShapeKind::Person,
            ShapeKind::Document,
        ];
        let rect = Rect {
            origin: Point { x: 101.0, y: 202.0 },
            size: Size {
                width: 114.0,
                height: 98.0,
            },
        };
        for (index, row) in rows.iter().enumerate() {
            let envelope =
                outside_icon_envelope(rect, shape_kinds[index / 12], positions[index % 12])
                    .unwrap();
            // Original target's stroke bounds surround the node by one pixel.
            let min = Point {
                x: 100.0_f64.min(envelope.origin.x),
                y: 201.0_f64.min(envelope.origin.y),
            };
            let max = Point {
                x: 216.0_f64.max(envelope.right()),
                y: 301.0_f64.max(envelope.bottom()),
            };
            assert_eq!(
                [min.x, min.y, max.x, max.y],
                [
                    row["min"]["x"].as_f64().unwrap(),
                    row["min"]["y"].as_f64().unwrap(),
                    row["max"]["x"].as_f64().unwrap(),
                    row["max"]["y"].as_f64().unwrap()
                ],
                "{row}"
            );
        }
    }
}

#[cfg(test)]
mod original_math_label_box_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_shape_math_origins_and_intrinsic_dimensions() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/math-label-boxes-source-v090.json"))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 84);
        for row in rows {
            let shape = match row["kind"].as_str().unwrap() {
                "rectangle" => ShapeKind::Rectangle,
                "oval" => ShapeKind::Oval,
                "diamond" => ShapeKind::Diamond,
                _ => panic!("unknown kind"),
            };
            let position = match row["position"].as_str().unwrap() {
                "UNSET" => LabelPosition::Unset,
                "INSIDE_MIDDLE_CENTER" => LabelPosition::InsideMiddleCenter,
                "INSIDE_TOP_LEFT" => LabelPosition::InsideTopLeft,
                "OUTSIDE_TOP_RIGHT" => LabelPosition::OutsideTopRight,
                "BORDER_TOP_CENTER" => LabelPosition::BorderTopCenter,
                "BORDER_LEFT_MIDDLE" => LabelPosition::BorderLeftMiddle,
                _ => panic!("unknown position"),
            };
            let label_box = latex_shape_label_box(
                Rect {
                    origin: Point { x: 20.0, y: 30.0 },
                    size: Size {
                        width: 200.0,
                        height: 130.0,
                    },
                },
                shape,
                position,
                Size {
                    width: row["label_width"].as_f64().unwrap(),
                    height: row["label_height"].as_f64().unwrap(),
                },
                None,
            );
            assert_eq!(
                format!("{:016x}", label_box.origin.x.to_bits()),
                row["x_bits"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(
                format!("{:016x}", label_box.origin.y.to_bits()),
                row["y_bits"].as_str().unwrap(),
                "{row}"
            );
            let mut svg = String::new();
            draw_node_text_box(
                &mut svg,
                "1 + 1",
                label_box,
                row["font_size"].as_u64().unwrap() as u32,
                "#123456",
                Some("latex"),
                TextStyle::default(),
                None,
                Some("math"),
            );
            let document = roxmltree::Document::parse(&svg).unwrap();
            let image = document.root_element();
            assert_eq!(image.tag_name().name(), "image");
            assert_eq!(
                image.attribute("width").unwrap().parse::<f64>().unwrap(),
                row["intrinsic_width"].as_f64().unwrap()
            );
            assert_eq!(
                image.attribute("height").unwrap().parse::<f64>().unwrap(),
                row["intrinsic_height"].as_f64().unwrap()
            );
        }
    }
}

#[cfg(test)]
mod math_scene_visibility_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn invisible_math_is_not_imported_after_owner_opacity_is_taken() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            Size {
                width: 240.0,
                height: 180.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        for expression in [r"\ohm", r"\celsius", r"\frac{\micro}{\ohm}"] {
            let labels = BTreeMap::from([(node, expression.to_owned())]);
            for opacity in ["0", "-0", "0x0p0", "0_0"] {
                let styles = BTreeMap::from([(
                    node,
                    SourceNodeStyle {
                        language: Some("latex".into()),
                        opacity: Some(opacity.into()),
                        ..Default::default()
                    },
                )]);
                let (svg, _) = render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false,
                )
                .unwrap();
                assert!(
                    !std::str::from_utf8(&svg)
                        .unwrap()
                        .contains("native-raster-math-")
                );
            }
            let styles = BTreeMap::from([(
                node,
                SourceNodeStyle {
                    language: Some("latex".into()),
                    ..Default::default()
                },
            )]);
            assert!(matches!(
                render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                ),
                Err(RenderError::LatexScene { .. })
            ));
            // The ordinary SVG renderer retains semantic math rather than importing it.
            assert!(
                render_svg_with_sketch_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                )
                .is_ok()
            );
        }
    }
}

#[cfg(test)]
mod raw_rich_label_scene_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn raw_invalid_bytes_are_rejected_before_import_even_when_invisible() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "m",
            Size {
                width: 120.0,
                height: 80.0,
            },
        ));
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        for language in ["latex", "markdown", "rust"] {
            for opacity in ["0", "1"] {
                graph.set_node_label_bytes(node, vec![0x80, 0x80, b'a']);
                let styles = BTreeMap::from([(
                    node,
                    SourceNodeStyle {
                        language: Some(language.into()),
                        opacity: Some(opacity.into()),
                        ..Default::default()
                    },
                )]);
                let error = render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &graph.node_labels,
                    &styles,
                    RenderOptions::default(),
                    false,
                )
                .unwrap_err();
                assert_eq!(
                    error.to_string(),
                    "scene: shape \"m\" field \"label\" must be valid UTF-8"
                );
            }
        }
        // A literal replacement character is valid source text. It cannot be
        // used as an invalid-byte sentinel in the scene path.
        graph.set_node_label(node, "\u{fffd}");
        let styles = BTreeMap::from([(
            node,
            SourceNodeStyle {
                language: Some("markdown".into()),
                ..Default::default()
            },
        )]);
        assert!(
            render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &graph.node_labels,
                &styles,
                RenderOptions::default(),
                false
            )
            .is_ok()
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod markdown_layout_viewport_source_contract {
    use super::*;
    #[test]
    fn original_natural_paint_layout_is_independent_of_target_clip() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-layout-viewport-source-v090.json"))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 90);
        let mut outcomes = Vec::new();
        for row in rows {
            let request = &row["request"];
            let family = match request["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let size = |name: &str, layout: &str| {
                let value = request[name].as_i64().unwrap();
                if value < 0 {
                    row["layout"][layout].as_f64().unwrap()
                } else {
                    value as f64
                }
            };
            let viewport = Rect {
                origin: Point {
                    x: 17.25,
                    y: 29.125,
                },
                size: Size {
                    width: size("Width", "Width"),
                    height: size("Height", "Height"),
                },
            };
            let mut svg = String::new();
            assert!(
                draw_native_markdown_layout_viewport(
                    &mut svg,
                    request["Markdown"].as_str().unwrap(),
                    viewport,
                    request["FontSize"].as_u64().unwrap() as u32,
                    "#0A0F25",
                    None,
                    false,
                    family,
                ),
                "{}",
                request["Name"]
            );
            assert_eq!(
                markdown_wrap::source_contract::primitives(&svg),
                markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap()),
                "Source natural layout {}",
                request["Name"]
            );
            let wrapped = format!("<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\">{svg}</svg>");
            let document = roxmltree::Document::parse(&wrapped).unwrap();
            let target = document.root_element().first_element_child().unwrap();
            for (attr, expected) in [
                ("x", viewport.origin.x),
                ("y", viewport.origin.y),
                ("width", viewport.size.width),
                ("height", viewport.size.height),
            ] {
                assert_eq!(
                    target.attribute(attr).unwrap().parse::<f64>().unwrap(),
                    expected,
                    "Source exported viewport {} {attr}",
                    request["Name"]
                );
            }
            assert_eq!(target.attribute("overflow"), Some("hidden"));
            let mut baseline = String::new();
            let baseline_supported = draw_native_markdown_subset_in_family(
                &mut baseline,
                request["Markdown"].as_str().unwrap(),
                viewport,
                request["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
            );
            let baseline_wrapped =
                format!("<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\">{baseline}</svg>");
            let baseline_document = roxmltree::Document::parse(&baseline_wrapped).unwrap();
            let baseline_target = baseline_document
                .root_element()
                .first_element_child()
                .unwrap();
            let baseline_matched = baseline_supported
                && markdown_wrap::source_contract::primitives(&baseline)
                    == markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap())
                && [
                    ("x", viewport.origin.x),
                    ("y", viewport.origin.y),
                    ("width", viewport.size.width),
                    ("height", viewport.size.height),
                ]
                .iter()
                .all(|(attr, expected)| {
                    baseline_target
                        .attribute(*attr)
                        .and_then(|v| v.parse::<f64>().ok())
                        == Some(*expected)
                });
            outcomes.push(serde_json::json!({"name":request["Name"], "actual_svg":svg,
                "baseline_svg":baseline,"baseline_matched":baseline_matched}));
        }
        if let Some(path) = std::env::var_os("D2_RS_TEST_MD_VIEWPORT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&outcomes).unwrap()).unwrap();
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod table_inline_source_contract {
    use super::*;
    #[test]
    fn complete_original_table_inline_context() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-table-inline-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1512);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let html = render_html(r["Markdown"].as_str().unwrap().as_bytes());
            let runs =
                native_markdown_html_inline_context(&html, r["Header"].as_bool().unwrap()).unwrap();
            let parsed = split_native_markdown_lines(&runs);
            let mut svg = String::new();
            let mut height = 0.0;
            let supported = draw_native_markdown_inline_context(
                &mut svg,
                &parsed,
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: r["Width"].as_f64().unwrap(),
                        height: 1024.0,
                    },
                },
                r["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
                Point::default(),
                true,
                r["Align"].as_str().unwrap(),
                Some(&mut height),
            );
            let actual = markdown_wrap::source_contract::primitives(&svg);
            let expected = markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap());
            let matched = supported
                && actual == expected
                && height == row["painted_height"].as_f64().unwrap();
            let outcome = serde_json::json!({"request":r,"html":html,"height":height,"actual":actual,"expected":expected,"matched":matched});
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(outcome);
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_TABLE_INLINE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "table inline differences: {:?}",
            failures
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod table_paint_source_contract {
    use super::*;
    #[test]
    fn complete_original_single_table_paint_and_dispatch_inventory() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-table-paint-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 252);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            if row.get("error").is_some() {
                outcomes.push(serde_json::json!({"request":r,"error":row["error"]}));
                continue;
            }
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let html = format!(
                "<body>{}</body>",
                markdown_measure::legacy_inline::measurement_html(
                    row["rendered_html"].as_str().unwrap()
                )
            );
            let doc = roxmltree::Document::parse(&html).unwrap();
            let blocks = doc
                .root_element()
                .children()
                .filter(roxmltree::Node::is_element)
                .collect::<Vec<_>>();
            let single = blocks.len() == 1 && blocks[0].has_tag_name("table");
            let mut svg = String::new();
            let supported = draw_native_markdown_table(
                &mut svg,
                r["Markdown"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: row["layout"]["Width"].as_f64().unwrap(),
                        height: row["layout"]["Height"].as_f64().unwrap(),
                    },
                },
                r["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
            );
            let actual = markdown_wrap::source_contract::primitives(&svg);
            let expected = markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap());
            let matched = supported && actual == expected;
            let outcome = serde_json::json!({"request":r,"single_table":single,"supported":supported,"matched":matched,"actual":actual,"expected":expected});
            if single && !matched || !single && supported {
                failures.push(r["Name"].clone());
            }
            outcomes.push(outcome);
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_TABLE_PAINT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "table paint differences: {:?}",
            failures
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod table_document_flow_source_contract {
    use super::*;
    #[test]
    fn complete_original_table_documents() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-table-paint-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 252);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            if row.get("error").is_some() {
                let error = first_unsupported_markdown_html(r["Markdown"].as_str().unwrap())
                    .map(|element| RenderError::UnsupportedMarkdownHtml { element }.to_string())
                    .unwrap_or_default();
                assert_eq!(error, row["error"].as_str().unwrap());
                outcomes.push(serde_json::json!({"request":r,"error":row["error"]}));
                continue;
            }
            let mut svg = String::new();
            let supported = draw_native_markdown_subset_in_family(
                &mut svg,
                r["Markdown"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: row["layout"]["Width"].as_f64().unwrap(),
                        height: row["layout"]["Height"].as_f64().unwrap(),
                    },
                },
                r["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
            );
            let wrapped = format!("<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\">{svg}</svg>");
            let doc = roxmltree::Document::parse(&wrapped).unwrap();
            assert!(
                doc.descendants()
                    .filter_map(|n| n.attribute("transform"))
                    .all(|v| v
                        .strip_prefix("translate(")
                        .and_then(|v| v.strip_suffix(')'))
                        .is_some_and(|v| {
                            let values = v
                                .split_whitespace()
                                .map(str::parse::<f64>)
                                .collect::<Result<Vec<_>, _>>();
                            values == Ok(vec![0.0, 0.0])
                        })),
                "non-identity transforms must be compared in world coordinates"
            );
            let actual = markdown_wrap::source_contract::primitives(&svg);
            let expected = markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap());
            let matched = supported && actual == expected;
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"supported":supported,"matched":matched,"actual":actual,"expected":expected}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_TABLE_DOCUMENT_FLOW_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "table document differences: {failures:?}"
        );
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod mixed_document_flow_source_contract {
    use super::*;
    #[test]
    fn complete_original_mixed_documents() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-mixed-flow-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 126);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            if row.get("error").is_some() {
                let error = first_unsupported_markdown_html(r["Markdown"].as_str().unwrap())
                    .map(|element| RenderError::UnsupportedMarkdownHtml { element }.to_string())
                    .unwrap_or_default();
                assert_eq!(error, row["error"].as_str().unwrap());
                outcomes.push(serde_json::json!({"request":r,"error":row["error"]}));
                continue;
            }
            let mut svg = String::new();
            let supported = draw_native_markdown_subset_in_family(
                &mut svg,
                r["Markdown"].as_str().unwrap(),
                Rect {
                    origin: Point::default(),
                    size: Size {
                        width: row["layout"]["Width"].as_f64().unwrap(),
                        height: row["layout"]["Height"].as_f64().unwrap(),
                    },
                },
                r["FontSize"].as_u64().unwrap() as u32,
                "#0A0F25",
                None,
                false,
                family,
            );
            let wrapped = format!("<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\">{svg}</svg>");
            let doc = roxmltree::Document::parse(&wrapped).unwrap();
            assert!(
                doc.descendants()
                    .filter_map(|n| n.attribute("transform"))
                    .all(|v| v
                        .strip_prefix("translate(")
                        .and_then(|v| v.strip_suffix(')'))
                        .is_some_and(|v| {
                            let values = v
                                .split_whitespace()
                                .map(str::parse::<f64>)
                                .collect::<Result<Vec<_>, _>>();
                            values == Ok(vec![0.0, 0.0])
                        })),
                "non-identity transforms must be compared in world coordinates"
            );
            let actual = markdown_wrap::source_contract::primitives(&svg);
            let expected = markdown_wrap::source_contract::primitives(row["svg"].as_str().unwrap());
            let matched = supported && actual == expected;
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"supported":supported,"matched":matched,"actual":actual,"expected":expected}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MIXED_DOCUMENT_FLOW_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "table document differences: {failures:?}"
        );
    }
}

#[cfg(test)]
mod remote_asset_bundle_contract {
    use super::*;

    #[test]
    fn bundled_http_content_type_survives_empty_bytes_and_xml_aliases() {
        let svg = b"<svg><image href=\"https://example.invalid/a\"/></svg>";
        let assets = BTreeMap::from([("https://example.invalid/a".into(), Vec::new())]);
        let types = BTreeMap::from([(
            "https://example.invalid/a".into(),
            "image/svg+xml; charset=utf-8".into(),
        )]);
        let result = bundle_svg_images_with_content_types(svg, &assets, &types);
        assert_eq!(
            result,
            b"<svg><image href=\"data:image/svg+xml; charset=utf-8;base64,\"/></svg>"
        );
        let types = BTreeMap::from([("https://example.invalid/a".into(), "text/xml".into())]);
        assert!(
            String::from_utf8(bundle_svg_images_with_content_types(svg, &assets, &types))
                .unwrap()
                .contains("data:image/svg+xml;base64,")
        );
    }
}

#[cfg(test)]
mod scene_label_admission_source_contract {
    use super::*;
    use d2_rs_core::ast::Shared;
    use d2_rs_language::{compiler, parse};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn structured_and_code_languages_follow_scene_admission() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/scene-label-admission-v090.json"))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let parsed = parse(
                "fixture.d2",
                row["input"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none());
            let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            let (graph, _, styles) =
                compiler::lower_graph_with_measurer_and_labels(&compiler, root, &AdmissionMeasurer)
                    .unwrap();
            let result = validate_shape_labels(&graph, &styles);
            if row["source_exit"].as_i64().unwrap() == 0 {
                assert!(result.is_ok(), "{}: {:?}", row["name"], result);
            } else {
                let error = result.unwrap_err().to_string();
                // The pinned CLI wrapper is separately compared without edits.
                // This assertion checks its exact scene-stage error component.
                let expected = format!(
                    "err: failed to compile {}: {}\n",
                    row["source_file"].as_str().unwrap(),
                    error
                );
                assert_eq!(
                    expected,
                    row["source_error"].as_str().unwrap(),
                    "{}",
                    row["name"]
                );
            }
        }
    }
    struct AdmissionMeasurer;
    impl compiler::GraphMeasurer for AdmissionMeasurer {
        fn measure(&self, _: &str) -> Size {
            Size {
                width: 100.0,
                height: 100.0,
            }
        }
    }
    #[test]
    fn visible_code_utf8_precedes_language_and_hidden_code_skips_that_check() {
        let mut graph = Graph::default();
        let id = graph.add_node(d2_rs_core::Node::new(
            "n",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        graph.node_mut(id).unwrap().shape = ShapeKind::Code;
        graph.set_node_label_bytes(id, vec![0x80]);
        for (opacity, expected) in [
            (
                "1",
                "scene: shape \"n\" field \"label\" must be valid UTF-8",
            ),
            (
                "0",
                "scene: shape \"n\" uses unsupported label language markdown on a structured or code shape",
            ),
        ] {
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    language: Some("markdown".into()),
                    opacity: Some(opacity.into()),
                    ..Default::default()
                },
            )]);
            assert_eq!(
                validate_shape_labels(&graph, &styles)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }
}

#[cfg(test)]
mod connection_math_scene_source_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn connection_math_is_admitted_even_for_zero_opacity_owner_without_panicking() {
        let mut graph = Graph::default();
        let a = graph.add_node(d2_rs_core::Node::new(
            "a",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let b = graph.add_node(d2_rs_core::Node::new(
            "b",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });
        graph.set_edge_arrows(
            edge,
            d2_rs_core::EdgeArrows {
                source: false,
                target: true,
            },
        );
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: r"\ohm".to_owned(),
                size: Size {
                    width: 10.0,
                    height: 10.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        for opacity in ["0", "1"] {
            graph.set_edge_style(
                edge,
                EdgeStyle {
                    language: Some("latex".into()),
                    opacity: Some(opacity.into()),
                    ..Default::default()
                },
            );
            let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            let error = render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &BTreeMap::new(),
                &BTreeMap::new(),
                RenderOptions::default(),
                false,
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "scene: connection \"(a -> b)[0]\" latex SVG: d2svgimport: connection \"(a -> b)[0]\" latex: root <svg> has invalid MathJax vertical-align: expected a finite ex length"
            );
        }
    }
}

#[cfg(test)]
mod native_rich_connection_link_layer_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn private_scene_link_metadata_does_not_add_a_rich_label_paint_group() {
        let mut graph = Graph::default();
        let a = graph.add_node(d2_rs_core::Node::new(
            "a",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let b = graph.add_node(d2_rs_core::Node::new(
            "b",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
        });
        graph.set_edge_link(edge, Some("https://example.com".into()));
        graph.set_edge_style(
            edge,
            EdgeStyle {
                language: Some("latex".into()),
                ..Default::default()
            },
        );
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: r"\frac{x}{y}".into(),
                size: Size {
                    width: 20.0,
                    height: 30.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let (native, _) = render_svg_with_raster_sources_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
        )
        .unwrap();
        assert!(!std::str::from_utf8(&native).unwrap().contains("<a href="));
        let public = render_svg_with_sketch_checked(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
        )
        .unwrap();
        assert!(std::str::from_utf8(&public).unwrap().contains("<a href="));
    }
}

#[cfg(test)]
mod native_code_input_admission_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn native_code_limits_follow_shape_language_opacity_precedence_and_public_boundary() {
        let mut graph = Graph::default();
        let mut node = Node::new(
            "n",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        node.shape = ShapeKind::Code;
        node.font_size = Some(16);
        let id = graph.add_node(node);
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::from([(id, "x".repeat(100001))]);
        for opacity in ["0", "1"] {
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    language: Some("go".into()),
                    opacity: Some(opacity.into()),
                    ..Default::default()
                },
            )]);
            assert_eq!(
                render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                )
                .unwrap_err()
                .to_string(),
                "scene: shape \"n\" field \"label\" has invalid value 100001: code source runes must not exceed 100000"
            );
            assert!(
                render_svg_with_sketch_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                )
                .is_ok(),
                "public SVG does not apply native scene code input ceilings"
            );
        }
        for (opacity, expected) in [
            (
                "1",
                "scene: shape \"n\" field \"label\" has invalid value 100001: code source runes must not exceed 100000",
            ),
            (
                "0",
                "scene: shape \"n\" uses unsupported label language markdown on a structured or code shape",
            ),
        ] {
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    language: Some("markdown".into()),
                    opacity: Some(opacity.into()),
                    ..Default::default()
                },
            )]);
            assert_eq!(
                render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                )
                .unwrap_err()
                .to_string(),
                expected
            );
        }
        graph.node_mut(id).unwrap().shape = ShapeKind::Class;
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                language: Some("go".into()),
                ..Default::default()
            },
        )]);
        assert_eq!(
            render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false
            )
            .unwrap_err()
            .to_string(),
            "scene: shape \"n\" uses unsupported label language go on a structured shape"
        );
    }
}

#[cfg(test)]
mod native_latex_input_admission_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn math_input_preflight_applies_to_hidden_owners_before_assets_but_not_public_svg() {
        let mut graph = Graph::default();
        let mut node = Node::new(
            "n",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        node.shape = ShapeKind::Text;
        node.font_size = Some(16);
        let id = graph.add_node(node);
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        let labels = BTreeMap::from([(id, format!("x{}y", " ".repeat(4096)))]);
        for opacity in ["0", "1"] {
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    language: Some("latex".into()),
                    opacity: Some(opacity.into()),
                    ..Default::default()
                },
            )]);
            assert_eq!(
                render_svg_with_raster_sources_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                )
                .unwrap_err()
                .to_string(),
                "scene: shape \"n\" latex input is 4098 bytes, exceeding limit 4096"
            );
            assert!(
                render_svg_with_sketch_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    RenderOptions::default(),
                    false
                )
                .is_ok()
            );
        }
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: id,
            target: id,
        });
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "x".repeat(4097),
                size: Size {
                    width: 10.0,
                    height: 10.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        graph.set_edge_style(
            edge,
            EdgeStyle {
                language: Some("latex".into()),
                opacity: Some("0".into()),
                ..Default::default()
            },
        );
        let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(
            render_svg_with_raster_sources_checked(
                &graph,
                &result,
                &BTreeMap::new(),
                &BTreeMap::new(),
                RenderOptions::default(),
                false
            )
            .unwrap_err()
            .to_string(),
            "scene: connection \"(n -- n)[0]\" latex input is 4097 bytes, exceeding limit 4096"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        let mut budget = d2_rs_text::code::CodeBudget::default();
        assert_eq!(
            render_svg_with_raster_sources_controlled(
                &graph,
                &result,
                &labels,
                &BTreeMap::new(),
                RenderOptions::default(),
                false,
                &mut budget,
                Some(&token)
            )
            .unwrap_err()
            .to_string(),
            "context canceled"
        );
    }
}

#[cfg(test)]
mod raster_vector_scene_placement_contract {
    #[test]
    fn vector_and_raster_images_keep_scene_placement_for_native_export() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg"><image id="vector" x="1.25" y="2.5" width="62" height="46" preserveAspectRatio="xMidYMid meet" href="data:image/svg+xml;base64,PHN2Zy8+"/><image id="raster" preserveAspectRatio="none" href="data:image/png;base64,AAAA"/><image id="unsupported" preserveAspectRatio="defer xMidYMid" href="data:image/svg+xml;base64,PHN2Zy8+"/></svg>"#;
        let tagged = super::tag_raster_effect_opacities(input);
        let document = roxmltree::Document::parse(&tagged).unwrap();
        for (id, expected) in [
            ("vector", Some("ordinary")),
            ("raster", Some("ordinary")),
            ("unsupported", None),
        ] {
            let node = document
                .descendants()
                .find(|n| n.attribute("id") == Some(id))
                .unwrap();
            assert_eq!(node.attribute("data-d2-rs-scene-image"), expected, "{id}");
        }
    }
}

#[cfg(test)]
mod public_svg_language_admission_contract {
    use super::*;
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn public_svg_retains_legacy_language_dispatch_without_native_scene_restrictions() {
        for (shape, language) in [
            (ShapeKind::Code, "markdown"),
            (ShapeKind::Code, "latex"),
            (ShapeKind::Class, "markdown"),
            (ShapeKind::Class, "golang"),
            (ShapeKind::SqlTable, "markdown"),
            (ShapeKind::SqlTable, "golang"),
        ] {
            let mut graph = Graph::default();
            let mut node = Node::new(
                "n",
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            );
            node.shape = shape;
            node.font_size = Some(16);
            let id = graph.add_node(node);
            let result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            let labels = BTreeMap::from([(id, "x".into())]);
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    language: Some(language.into()),
                    markdown: language == "markdown",
                    ..Default::default()
                },
            )]);
            assert!(
                matches!(
                    validate_shape_labels(&graph, &styles),
                    Err(RenderError::UnsupportedLabelLanguage { .. })
                ),
                "native scene {shape:?}/{language}"
            );
            let svg = render_svg_with_sketch_checked(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions::default(),
                false,
            )
            .unwrap();
            assert!(std::str::from_utf8(&svg).unwrap().contains("<svg"));
        }
    }
}

#[cfg(test)]
mod xml_render_text_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_xml_escaping_and_render_text_preserve_controls_and_empty_lines() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/xml-render-text-source-v090.json"))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 32);
        for row in cases {
            let text = row["text"].as_str().unwrap();
            assert_eq!(
                escape_xml_text(text),
                row["escaped"].as_str().unwrap(),
                "{row}"
            );
            assert_eq!(
                render_text_content(
                    text,
                    row["x"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                row["rendered"].as_str().unwrap(),
                "{row}"
            );
        }
    }
}

#[cfg(test)]
mod appendix_raw_pretty_link_source_contract {
    use super::*;
    #[test]
    fn truncated_url_bytes_are_rejected_only_for_source_appendix_work() {
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new(
            "x",
            Size {
                width: 100.0,
                height: 60.0,
            },
        ));
        let style = SourceNodeStyle {
            link: Some(
                "https://example.com/prefix/雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪".into(),
            ),
            pretty_link_bytes: Some(d2_rs_core::ast::ByteString::from(vec![0xe9, 0x9b])),
            ..Default::default()
        };
        let mut styles = BTreeMap::from([(id, style)]);
        assert!(validate_appendix_metadata(&graph, &styles, false).is_ok());
        assert_eq!(
            validate_appendix_metadata(&graph, &styles, true)
                .unwrap_err()
                .to_string(),
            "scene: shape[0] \"x\" field \"prettyLink\" must be valid UTF-8"
        );
        styles.get_mut(&id).unwrap().pretty_link_bytes =
            Some(d2_rs_core::ast::ByteString::from("��"));
        assert!(validate_appendix_metadata(&graph, &styles, true).is_ok());
    }
}

#[cfg(test)]
mod empty_config_target_bytes_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_non_renderable_empty_targets_preserve_original_go_config_bytes() {
        let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/target-empty-config-bytes-source-v090.json"))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let parsed = d2_rs_language::parse(
                "empty.d2",
                case["source"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none());
            let (compiler, root) =
                d2_rs_language::compiler::compile_ir(d2_rs_core::ast::Shared::new(parsed.map))
                    .unwrap();
            let graph = d2_rs_language::compiler::lower_graph(&compiler, root).unwrap();
            let cfg = d2_rs_language::compiler::extract_config(&compiler, root).unwrap_or_default();
            let options = RenderOptions {
                theme_id: cfg.theme_id,
                dark_theme_id: cfg.dark_theme_id,
                ..Default::default()
            };
            let actual = target_hash_input(
                &graph,
                &LayoutResult {
                    removed_edges: Default::default(),
                    source_edge_storage: Default::default(),
                    synthetic_root: None,
                    route_curves: Default::default(),
                    sequence_lifelines: Vec::new(),
                    boxes: BTreeMap::new(),
                    routes: BTreeMap::new(),
                    edge_labels: BTreeMap::new(),
                    edge_label_percentage_commits: Default::default(),
                    node_labels: BTreeMap::new(),
                    report: d2_rs_core::LayoutReport {
                        selected_seed: 0,
                        selected_pass: 0,
                        score: Default::default(),
                        improvement_passes: 0,
                        candidates: Vec::new(),
                        warnings: Vec::new(),
                    },
                },
                &BTreeMap::new(),
                &BTreeMap::new(),
                &options,
                false,
            );
            let expected = base64::engine::general_purpose::STANDARD
                .decode(case["bytes"].as_str().unwrap())
                .unwrap();
            assert_eq!(actual, expected, "{}", case["name"]);
        }
    }
}

#[cfg(test)]
mod edge_icon_position_source_contract {
    use super::*;
    use d2_rs_core::{Edge, EdgeLabel, EdgeStyle, LayoutReport, Node, Score};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_icon_positions_follow_original_exported_route_and_label_precedence() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/edge-icon-positions-source-v090.json"))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 64);
        for row in cases {
            let c = &row["connection"];
            let mut graph = Graph::default();
            let source = graph.add_node(Node::new(
                "a",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
            let target = graph.add_node(Node::new(
                "b",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
            let edge = graph.add_edge(Edge { source, target });
            graph.set_edge_icon(
                edge,
                row["has_icon"]
                    .as_bool()
                    .unwrap()
                    .then(|| "icon.svg".to_owned()),
                Some(LabelPosition::from_d2_name(
                    c["iconPosition"].as_str().unwrap(),
                )),
                0.0,
            );
            graph.set_edge_style(
                edge,
                EdgeStyle {
                    stroke_width: Some(c["strokeWidth"].to_string()),
                    ..Default::default()
                },
            );
            graph.set_edge_label(
                edge,
                Some(EdgeLabel {
                    text: c["label"].as_str().unwrap().to_owned(),
                    size: Size {
                        width: c["labelWidth"].as_f64().unwrap(),
                        height: c["labelHeight"].as_f64().unwrap(),
                    },
                    position: LabelPosition::from_d2_name(c["labelPosition"].as_str().unwrap()),
                    percentage: c["labelPercentage"].as_f64().unwrap(),
                }),
            );
            let route: Vec<Point> = serde_json::from_value(row["raw_route"].clone()).unwrap();
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes: BTreeMap::new(),
                routes: BTreeMap::from([(edge, route.clone())]),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let expected: Option<Point> = serde_json::from_value(row["point"].clone()).unwrap();
            let translated = expected.map(|p| Point {
                x: p.x + 13.125,
                y: p.y - 8.5,
            });
            let actual =
                edge_icon_rect(&graph, &result, edge, &route, 13.125, -8.5).map(|r| r.origin);
            assert_eq!(actual, translated, "{}", row["name"]);
        }
    }
}

#[cfg(test)]
mod legend_target_source_contract {
    use super::*;
    use d2_rs_core::ast::Shared;
    use d2_rs_language::{compiler, parse};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn unmeasured_legend_roots_match_complete_original_go_target_records() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-wasm"),
            "tests/fixtures/legend-target-source-v090.json",
        ))
        .unwrap();
        let rows = fixture["cases"].as_array().unwrap();
        assert_eq!(rows.len(), 24);
        let mut positives = 0;
        for row in rows {
            if row["error"].is_string() {
                continue;
            } // Error checked at whole compiler/WASM boundary.
            positives += 1;
            let parsed = parse(
                row["path"].as_str().unwrap(),
                row["source"].as_str().unwrap().as_bytes(),
                Default::default(),
            );
            assert!(
                parsed.error.is_none(),
                "{}: {:?}",
                row["name"],
                parsed.error
            );
            let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
            let source = compiler::lower_legend_source_graph(&compiler, root).unwrap();
            let actual = source
                .map(|s| {
                    target_legend_metadata(&s, row["diagram"]["config"]["themeID"].as_i64())
                        .unwrap()
                })
                .flatten();
            let expected = row["diagram"].get("legend").cloned();
            assert_eq!(actual, expected, "{}", row["name"]);
        }
        assert_eq!(positives, 23);
    }
}

#[cfg(test)]
mod scene_route_tangent_contract {
    use super::*;
    use d2_rs_core::{Edge, EdgeArrows, Node, Size};
    use d2_tala_rs::{Engine, LayoutOptions};
    #[test]
    fn only_explicitly_removed_edges_bypass_route_validation_and_export() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 100.,
                height: 60.,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 100.,
                height: 60.,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        result.routes.remove(&edge);
        assert!(validate_connection_routes(&graph, &result).is_err());
        assert!(target_hash_connection(&graph, &result, edge).is_some());
        result.removed_edges.insert(edge);
        assert!(validate_connection_routes(&graph, &result).is_ok());
        assert!(target_hash_connection(&graph, &result, edge).is_none());
    }

    #[test]
    fn source_scene_checks_exported_tangents_and_only_enabled_arrow_ends() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 100.,
                height: 60.,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 100.,
                height: 60.,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        result.routes.insert(
            edge,
            vec![
                Point {
                    x: 1.2341,
                    y: 2.3451,
                },
                Point {
                    x: 1.2349,
                    y: 2.3459,
                },
            ],
        );
        assert!(validate_connection_routes(&graph, &result).is_ok());
        for (source, target, endpoint) in [
            (true, true, "source"),
            (false, true, "destination"),
            (true, false, "source"),
        ] {
            graph.set_edge_arrows(edge, EdgeArrows { source, target });
            let error = validate_scene_route_tangents(&graph, &result).unwrap_err();
            assert!(error.to_string().ends_with(&format!(
                "must have a non-zero {endpoint} arrowhead tangent"
            )));
        }
        graph.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: false,
            },
        );
        assert!(validate_scene_route_tangents(&graph, &result).is_ok());
        result.routes.insert(
            edge,
            vec![
                Point { x: 1., y: 2. },
                Point { x: 1., y: 2. },
                Point { x: 7., y: 2. },
            ],
        );
        graph.set_edge_arrows(
            edge,
            EdgeArrows {
                source: false,
                target: true,
            },
        );
        assert!(validate_scene_route_tangents(&graph, &result).is_ok());
    }
    #[test]
    fn source_scene_does_not_export_already_exported_target_routes_twice() {
        let mut graph = Graph::default();
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 100.,
                height: 60.,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 100.,
                height: 60.,
            },
        ));
        let edge = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        graph.set_edge_arrows(
            edge,
            d2_rs_core::EdgeArrows {
                source: false,
                target: true,
            },
        );
        let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
        graph.target_routes_exported = true;
        result.routes.insert(
            edge,
            vec![Point { x: 1.2341, y: 2. }, Point { x: 1.2349, y: 2. }],
        );
        assert!(validate_scene_route_tangents(&graph, &result).is_ok());
    }
}

#[cfg(test)]
mod sequence_lifeline_export_contract {
    use super::*;
    use d2_rs_core::{Node, Size};
    use d2_tala_rs::{Engine, LayoutOptions};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_fractional_lifelines_cross_target_boundary_once_before_paint() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/spore-sequence-lifeline-export-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2);
        for row in rows.as_array().unwrap() {
            let mut graph = Graph::default();
            let actor = graph.add_node(Node::new(
                "actor",
                Size {
                    width: 100.,
                    height: 66.,
                },
            ));
            let mut result = Engine.layout(&graph, &LayoutOptions::default()).unwrap();
            let raw: Vec<Point> = serde_json::from_value(row["raw"].clone()).unwrap();
            let target: Vec<Point> = serde_json::from_value(row["target"].clone()).unwrap();
            result.sequence_lifelines = vec![d2_rs_core::SequenceLifeline {
                actor,
                target_id: "end".into(),
                route: raw.clone(),
            }];
            for already_exported in [false, true] {
                graph.target_routes_exported = already_exported;
                result.sequence_lifelines[0].route = if already_exported {
                    target.clone()
                } else {
                    raw.clone()
                };
                let mut svg = String::new();
                draw_sequence_lifelines(
                    &graph,
                    &result,
                    &BTreeMap::new(),
                    &mut svg,
                    0.,
                    0.,
                    PaletteView::new(active_palette(None), &BTreeMap::new()),
                    None,
                    None,
                    false,
                    false,
                    None,
                );
                let xml = format!("<svg>{svg}</svg>");
                let doc = roxmltree::Document::parse(&xml).unwrap();
                let line = doc.descendants().find(|n| n.has_tag_name("line")).unwrap();
                assert_eq!(
                    line.attribute("y1"),
                    Some(number(target[0].y + 2.).as_str())
                );
                assert_eq!(
                    line.attribute("y2"),
                    Some(number(target[1].y - 1.).as_str())
                );
                assert_eq!(
                    result.sequence_lifelines[0].route,
                    if already_exported {
                        target.clone()
                    } else {
                        raw.clone()
                    }
                );
            }
        }
    }
}

/// Render an upstream e2e `board.exp.json` with the d2svg port (S1 harness;
/// see docs/PLAN.md). Not yet the CLI/WASM SVG path.
pub fn render_d2svg_e2e(json: &[u8], theme_id: Option<i64>) -> Result<Vec<u8>, String> {
    target::d2svg::render_e2e(json, theme_id)
}
