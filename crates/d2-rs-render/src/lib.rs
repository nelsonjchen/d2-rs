// SPDX-License-Identifier: MPL-2.0
//! The SVG and sketch renderers: a Rust port of upstream D2's `d2renderers/d2svg`
//! and `d2renderers/d2sketch` packages,
//! with the `d2themes`, `lib/shape`, `lib/svg` and `lib/color` support it uses,
//! producing SVG byte-identical with Go D2.
//!
//! Isometric output is dispatched by `d2-rs-isometric`; this crate exposes the
//! flat renderer. The `diagnostic-traces` feature enables renderer traces;
//! `oracle-fixtures` compiles tests that read regenerated Go-oracle artifacts.

mod image_mime;

/// Go net/http.DetectContentType without filename-extension precedence.
/// Shared by imageasset's malformed-resource diagnostics.
pub fn detect_image_mime(bytes: &[u8]) -> &'static str {
    image_mime::detect(bytes)
}
pub mod markdown_scene;
mod native_markdown_fonts;
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
use d2_rs_core::theme::resolve_color;
use d2_rs_core::{
    EdgeId, EdgeStyle, Graph, LabelPosition, Node, NodeId, Point, Rect, ShapeKind, Size, theme,
};
use d2_rs_language::compiler::{SourceNodeStyle, SourceSqlColumn, SourceTargetAccents};
use d2_rs_tala::LayoutResult;
use d2_rs_text::markdown::{
    html::render_markdown_html as render_html, html_names::normalize_html_names,
};
use d2_rs_text::measure::{TrueTypeFont, bundled_source_sans_pro_regular};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};
use std::sync::OnceLock;

pub mod markdown_measure;
mod patternassets;
/// Source D2 pattern assets, shared with the scene builder.
pub mod assets {
    pub const PAPER_SVG: &str = include_str!("../assets/paper.svg");
    pub const GRAIN_PNG: &[u8] = include_bytes!("../assets/grain.png");
    pub const STREAK_PATH_TXT: &str = include_str!("../assets/streak_path.txt");
}

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

/// Errors raised by the checked source-render path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// A source renderer dereferences a nil connection label point.
    SourceNilPointer {
        source_site: &'static str,
    },
    /// A source renderer indexes an incomplete connection route.
    SourceRouteIndex {
        index: usize,
        length: usize,
    },
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
    UnsupportedMarkdownHtml {
        element: String,
    },
    /// Original scene builder rejects these effects on a synthetic root.
    UnsupportedRootEffect {
        effect: &'static str,
    },
    /// Original native scene math source admission.
    LatexInput {
        object: String,
        bytes: usize,
    },
    LatexInvalidUtf8 {
        object: String,
    },
    LatexNesting {
        object: String,
        depth: usize,
    },
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
    CodeContext {
        deadline: bool,
    },
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
    InvalidLabelUtf8 {
        object: String,
    },
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
            Self::LatexInvalidUtf8 { object } => {
                write!(formatter, "scene: {object} latex input must be valid UTF-8")
            }
            Self::LatexNesting { object, depth } => write!(
                formatter,
                "scene: {object} latex group nesting depth {depth} exceeds limit 128"
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

fn node_level(graph: &Graph, node_id: NodeId) -> usize {
    let mut level = 1;
    let mut current = graph.node(node_id).and_then(|node| node.parent);
    while let Some(parent) = current {
        level += 1;
        current = graph.node(parent).and_then(|node| node.parent);
    }
    level
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

fn stroke_width(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(2.0)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn edge_svg_id(graph: &Graph, edge_id: EdgeId) -> Option<String> {
    d2_rs_core::edge_abs_id(graph, edge_id)
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

fn style_flag(
    style: Option<&SourceNodeStyle>,
    value: impl Fn(&SourceNodeStyle) -> Option<&String>,
) -> bool {
    style
        .and_then(value)
        .and_then(|value| d2_rs_core::parse_d2_bool(value))
        .unwrap_or(false)
}

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
                if is_sequence_panel(graph, *node_id) || is_sequence_group_like(graph, *node_id) {
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
                // d2target.Diagram.BoundingBox uses the exported position
                // verbatim. An unset Markdown position therefore leaves the
                // label box at the shape's top-left even though the Markdown
                // painter centers the viewport inside the shape.
                let rendered_position = position;
                target_modifier_label_bounds(
                    node_label_box(
                        target_rect,
                        // d2target.Diagram.BoundingBox places every label
                        // on GetBox, independently of the painter's inner
                        // content box (including collapsed short shapes).
                        ShapeKind::Rectangle,
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
    // LayoutNested extracts special descendants before classifying actors.
    // Each actor then gets a lifeline, which keeps IsSequenceDiagramGroup
    // false even after InjectNested restores the inner messages. Lifelines
    // live outside Graph.edges here, so use that same extracted inventory.
    let actor_candidate = graph.node(id).is_some_and(|node| node.parent == scope);
    let live_object = |object| {
        // Only direct actors acquire a lifeline. Other descendants use the
        // post-reinjection reference inventory, just like the Go exporter.
        if !actor_candidate {
            return true;
        }
        let mut parent = graph.node(object).and_then(|node| node.parent);
        while let Some(ancestor) = parent {
            if Some(ancestor) == scope {
                break;
            }
            if graph.is_sequence_diagram(ancestor)
                || graph
                    .node(ancestor)
                    .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
            {
                return false;
            }
            parent = graph.node(ancestor).and_then(|node| node.parent);
        }
        true
    };
    let contains_reference_edge = |container| {
        graph.sequence_group_edges.iter().any(|(owner, edges)| {
            is_descendant_or_self(graph, *owner, container)
                && edges.iter().any(|edge| {
                    graph
                        .edges
                        .get(edge.0 as usize)
                        .is_some_and(|edge| live_object(edge.source) && live_object(edge.target))
                })
        })
    };
    let contains_object = if graph.sequence_object_scopes.is_empty() {
        graph
            .nodes()
            .any(|(child, node)| node.parent == Some(id) && live_object(child))
    } else {
        graph.sequence_object_scopes.iter().any(|(object, scopes)| {
            live_object(*object)
                && scopes
                    .iter()
                    .any(|scope| is_descendant_or_self(graph, *scope, id))
        })
    };
    if !contains_reference_edge(id) && !contains_object {
        return false;
    }
    // Source checks child.ContainsAnyEdge using reference scopes. Compiled
    // identities already reflect EnsureChild's actor lookup; only legacy
    // measured graphs need to infer actor aliases from terminal names.
    graph
        .nodes()
        .filter(|(child, node)| node.parent == Some(id) && live_object(*child))
        .all(|(child, node)| {
            let actor_proxy = !graph.sequence_node_edge_refs.contains_key(&id)
                && !is_nested_sequence_grid_descendant(graph, id)
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

// newSequenceDiagram rewrites empty sequence descendants into pages or
// spans. Populated diagrams recover their authored shape during reinjection.
fn is_sequence_panel(graph: &Graph, id: NodeId) -> bool {
    graph.is_sequence_diagram(id)
        && !is_sequence_note_like(graph, id)
        && !is_sequence_span_like(graph, id)
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
        ShapeKind::Circle | ShapeKind::Oval => {
            // d2target.Shape.SetType exports circles as ovals. Grids can
            // stretch them, so label/icon placement must use shapeOval's
            // separate radii and float32 angle, not the layout circle inset.
            let rx = rect.size.width / 2.0;
            let ry = rect.size.height / 2.0;
            let theta = f64::from(go_float::gomath::atan2(ry, rx) as f32);
            let sin = go_float::gomath::sin(theta);
            let cos = go_float::gomath::cos(theta);
            let radius = rx * ry / ((rx * sin).powi(2) + (ry * cos).powi(2)).sqrt();
            let offset_x = (rx - cos * radius).ceil();
            let offset_y = (ry - sin * radius).ceil();
            inner.origin.x += offset_x;
            inner.origin.y += offset_y;
            inner.size.width = rect.size.width - 2.0 * offset_x;
            inner.size.height = rect.size.height - 2.0 * offset_y;
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
            let body_top = go_float::go_fmadd(rect.size.width, 0.22, head_radius * 0.8);
            let horizontal = rect.size.width * 0.05;
            let vertical = rect.size.height * 0.03;
            inner.origin.x = go_float::go_fmadd(rect.size.width, 0.05, inner.origin.x);
            inner.origin.y += go_float::go_fmadd(rect.size.height, 0.03, body_top);
            inner.size.width =
                rect.size.width - go_float::go_fmadd(rect.size.width, 0.05, horizontal);
            inner.size.height =
                rect.size.height - body_top - go_float::go_fmadd(rect.size.height, 0.03, vertical);
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
            inner.size.height = rect.size.height - 3.0 * arc;
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

fn native_markdown_font(
    slot: &OnceLock<Option<TrueTypeFont>>,
    load: fn() -> Result<TrueTypeFont, d2_rs_text::measure::FontError>,
) -> Option<&TrueTypeFont> {
    slot.get_or_init(|| load().ok()).as_ref()
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
                        |fonts| font.measure_with_fallback(entry, 16.0, fonts.measurer().mono_font),
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
        Some(BundledFamily::Mono) => d2_rs_text::assets::SOURCE_CODE_PRO_REGULAR_TTF,
        Some(BundledFamily::HandDrawn) => d2_rs_text::assets::FUZZY_BUBBLES_REGULAR_TTF,
        _ => d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF,
    };
    let subset = svg_primary
        .filter(|_| !precision)
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
            d2_rs_text::assets::SOURCE_SANS_PRO_BOLD_TTF,
        ),
    ] {
        if !precision {
            appendix_css.push_str(&font_face_rule_format(
                family, weight, "normal", "font/ttf", "truetype", data,
            ));
        }
    }
    let _ = write!(group, "<style>{appendix_css}</style>");
    output.splice(insertion..insertion, group.as_bytes().iter().copied());
    output
}

#[cfg(test)]
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
    value.and_then(d2_rs_core::parse_d2_bool).unwrap_or(false)
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
    // The pinned Go 1.27 build selects encoding/json v2, whose invalid UTF-8
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
        shape_type: if graph.is_sequence_diagram(id)
            && (stage == TargetShapeStage::Unmeasured || is_sequence_panel(graph, id))
        {
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
            if (graph.is_sequence_diagram(id) && stage == TargetShapeStage::Unmeasured)
                || is_sequence_panel(graph, id)
                || sequence_group
            {
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
            .and_then(d2_rs_core::parse_d2_bool)
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
            let rules = target_theme_rules(graph);
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
                    .unwrap_or_else(|| {
                        // addLifelineEdges only copies an explicitly authored
                        // actor stroke. d2exporter.exportConnection then applies
                        // C4's default to the synthetic edge like any other.
                        if rules.c4 { "AA4" } else { "B2" }.to_owned()
                    }),
                border_radius: GoFloat(if rules.no_corner_radius { 0.0 } else { 10.0 }),
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

/// Render one laid-out board through the byte-identical d2svg port (PLAN S6).
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

/// Render and process each board before composing the animated SVG.
pub fn render_d2svg_animated_with(
    root_path: &str,
    frames: &[(String, Vec<u8>, bool)],
    options: &RenderOptions,
    sketch: bool,
    interval_ms: u64,
    process: &mut impl FnMut(Vec<u8>) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let tree = target_diagram_tree_json(root_path, frames)?;
    target::d2svg::render_cli_animated_with(&tree, options, sketch, interval_ms as i64, process)
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

#[cfg(test)]
mod tests {

    use super::{
        arrowhead_label_bounds, cloud_content_aspect, node_label_box, recursive_target_hash_input,
        replace_static_appendix_badges, rounded_render_rect, validate_markdown_labels,
    };
    use d2_rs_core::{
        ArrowheadLabel, Edge, EdgeArrowheadLabels, EdgeArrows, Graph, LabelPosition, LayoutReport,
        LayoutResult, Node, NodeId, Point, Rect, Score, ShapeKind, Size,
    };
    use d2_rs_language::compiler::SourceNodeStyle;

    use std::collections::BTreeMap;

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

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tooltip_target_handoff_source_contract {
    #[cfg(feature = "oracle-fixtures")]
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

use native_markdown_fonts::native_markdown_measurer;

fn bundled_markdown_measurer(
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
    let mono_semibold =
        native_markdown_font(&MONO_SEMIBOLD, fonts::bundled_source_code_pro_semibold)?;
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
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/positioned-tooltip-source-v090.json"
        ))
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
                .measurer()
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
        measurer
            .measurer()
            .markdown_dimensions(tooltip, 16, false, false, family.uses_mono_css())
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

#[cfg(test)]
mod markdown_list_width_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn styled_list_items_match_original_go_complete_viewports() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-list-width-source-v090.json"
        ))
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
                .measurer()
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
mod sequence_group_reference_contract {

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_group_inventory_matches_original_go_reference_scope_controls() {
        use d2_rs_core::ast::Shared;
        use d2_rs_language::{compiler, parse};
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-group-reference-source-v090.json"
        ))
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

#[cfg(all(test, feature = "oracle-fixtures"))]
mod sequence_note_references_source_contract {

    #[cfg(feature = "oracle-fixtures")]
    use d2_rs_core::ast::Shared;
    use d2_rs_language::{
        compiler::{compile_ir, lower_graph},
        parse,
    };

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_reference_state_note_and_group_inventories_match_original_go() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-note-references-source-v090.json"
        ))
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
mod original_outside_icon_bounds_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn all_original_target_shape_names_and_outside_icon_positions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/outside-icon-bounds-source-v090.json"
        ))
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
mod empty_config_target_bytes_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_non_renderable_empty_targets_preserve_original_go_config_bytes() {
        let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/target-empty-config-bytes-source-v090.json"
        ))
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

#[cfg(all(test, feature = "oracle-fixtures"))]
mod legend_target_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
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
            let actual = source.and_then(|s| {
                target_legend_metadata(&s, row["diagram"]["config"]["themeID"].as_i64()).unwrap()
            });
            let expected = row["diagram"].get("legend").cloned();
            assert_eq!(actual, expected, "{}", row["name"]);
        }
        assert_eq!(positives, 23);
    }
}

/// Render an upstream e2e `board.exp.json` with the d2svg port, isolating SVG
/// parity from layout parity (`scripts/d2svg-golden.py`).
pub fn render_d2svg_e2e(json: &[u8], theme_id: Option<i64>) -> Result<Vec<u8>, String> {
    target::d2svg::render_e2e(json, theme_id)
}
