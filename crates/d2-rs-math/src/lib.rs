// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Source-derived fraction, class-spacing, numeric and script-style portions:
// Copyright 2017–2022 The MathJax Consortium, Apache-2.0.
// Translated and modified from pinned mathjax-go v0.1.0; see
// THIRD_PARTY_NOTICES.md for source mapping and retained third-party terms.

//! LaTeX math labels: a Rust port of upstream D2's `d2latex` package and the
//! `mathjax-go` TeX renderer it uses, producing the same measurements and SVG
//! as Go D2 without a JavaScript runtime or browser.
//!
//! The `oracle-fixtures` feature compiles tests that read regenerated Go-oracle
//! artifacts.

use d2_rs_core::Size;
use thiserror::Error;

mod input_limits;
mod internal_text;
pub use input_limits::{MAX_GROUP_NESTING_DEPTH, MAX_INPUT_BYTES, validate_input};

mod ams;
mod array_body;
mod array_rules;
mod buildrel;
mod cases;
mod cd;
mod compound_glyphs;
mod glyphs;
mod large_glyphs;
mod length;
mod limits;
mod named_colors;
mod radical_glyphs;
mod root_index;
mod rule;
mod scene;
mod size3_glyphs;
mod size4_glyphs;
mod source_symbols;
mod stack_annotations;
mod stretch_glyphs;
mod table;
mod vertical_delimiters;
// Native mhchem expansion, inserted into the owning TeX parser.
#[cfg(all(test, feature = "oracle-fixtures"))]
mod aboxed_contract_tests;
mod accents;
mod adjust_limits;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod adjust_limits_contract_tests;
mod alias_font_glyphs;
mod alias_fonts;
mod alignment;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod alignment_marks_contract_tests;
mod bold_glyphs;
mod brackets;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod brackets_contract_tests;
mod braket;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod braket_contract_tests;
mod classes;
mod command_registry;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod continued_fraction_contract_tests;
mod declaration_font_glyphs;
mod declaration_fonts;
mod double_struck;
mod enclose;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod enclose_contract_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod fixed_accents_contract_tests;
mod font_bindings;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod font_declarations_contract_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod font_wrappers_contract_tests;
mod framebox;
mod general_fraction;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod general_fraction_contract_tests;
mod genfrac_palette;
mod hfill;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod line_accents_contract_tests;
mod math_minwidth;
mod mhchem;
mod mml_spacing;
mod mml_token;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod multiscript_contract_tests;
mod multiscripts;
mod nonscript;
mod operator_dictionary;
mod padded;
mod paired_delimiters;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod paired_delimiters_contract_tests;
mod pending_dots;
mod pending_not;
mod physics;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod physics_contract_tests;
mod physics_matrix_quantity;
mod position;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod set_macros_contract_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod size_contract_tests;
mod smallop_glyphs;
mod source_macros;
mod split_fraction;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod split_fraction_contract_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod stacks_contract_tests;
mod style_classes;
mod styles;
mod tex_variant_glyphs;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod under_over_contract_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod under_over_marks_contract_tests;
mod wrapper_font_glyphs;
mod xarrow;
use length::{Dimension, Minimums};
mod calligraphic_glyphs;
mod italic_glyphs;
mod letter_unicode;
mod mathit_glyphs;
mod number_unicode;
pub use scene::import_scene_svg;
#[doc(hidden)]
pub mod svg_bounds;
#[doc(hidden)]
pub mod svg_budget;
#[doc(hidden)]
pub mod svg_clip;
#[doc(hidden)]
pub mod svg_color;
#[doc(hidden)]
pub mod svg_compile;
#[doc(hidden)]
pub mod svg_declarations;
#[doc(hidden)]
pub mod svg_element;
#[doc(hidden)]
pub mod svg_frozen_text;
#[doc(hidden)]
pub mod svg_geometry;
#[doc(hidden)]
pub mod svg_gradient;
#[doc(hidden)]
pub mod svg_gradient_stop;
#[doc(hidden)]
pub mod svg_gradient_tree;
#[doc(hidden)]
pub mod svg_image;
#[doc(hidden)]
pub mod svg_import;
#[doc(hidden)]
pub mod svg_length;
#[doc(hidden)]
pub mod svg_path;
#[doc(hidden)]
pub mod svg_primitive;
#[doc(hidden)]
pub mod svg_resources;
#[doc(hidden)]
pub mod svg_scene;
#[doc(hidden)]
pub mod svg_style;
#[doc(hidden)]
pub mod svg_stylesheet;
#[doc(hidden)]
pub mod svg_transform;
mod svg_trig;
#[doc(hidden)]
pub mod svg_use;
#[doc(hidden)]
pub mod svg_viewport;
#[doc(hidden)]
pub mod svg_xml_admission;
#[doc(hidden)]
pub mod svg_xml_budget;
#[doc(hidden)]
pub mod svg_xml_document;

/// Math parsing or rendering failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MathError {
    /// Input rejected before the recursive MathJax parser.
    #[error("{0}")]
    InputLimit(String),
    /// An original MathJax parser leaves a nil child which SVG wrapping
    /// dereferences. Native context is not a fabricated Go runtime stack.
    #[error(
        "panic recovered: runtime error: invalid memory address or nil pointer dereference\nnative source context: mathjax-go/internal/svg/renderer.wrap:61"
    )]
    SourceNilChild,
    /// The pinned parser failed before constructing a renderable TeX error.
    #[error("{0}")]
    SourceConversion(String),
    /// No expression was supplied.
    #[error("math expression is empty")]
    Empty,
    /// The emitted SVG did not expose finite positive intrinsic dimensions.
    #[error("mathjax-go: SVG dimensions not found")]
    InvalidDimensions,
    /// The expression is outside the translated MathJax surface; no exact
    /// SVG can be produced.
    #[error("unsupported: TeX outside the translated mathjax-go surface")]
    Unsupported,
    /// A brace group was not balanced.
    #[error("math expression has an unbalanced {delimiter} at byte {byte}")]
    Unbalanced { delimiter: char, byte: usize },
}

/// Measures the actual emitted SVG using original D2's serialized intrinsic units.
pub fn measure(expression: &str) -> Result<Size, MathError> {
    let svg = render(expression)?;
    let doc = roxmltree::Document::parse(&svg).map_err(|_| MathError::InvalidDimensions)?;
    let root = doc.root_element();
    let dimension = |name| {
        let value = root.attribute(name).ok_or(MathError::InvalidDimensions)?;
        let (numeric, unit) = value.strip_suffix("ex").map_or((value, 1.0), |v| (v, 8.0));
        let number = numeric
            .parse::<f64>()
            .map_err(|_| MathError::InvalidDimensions)?
            * unit;
        if !number.is_finite() || number <= 0.0 {
            return Err(MathError::InvalidDimensions);
        }
        Ok(number.ceil())
    };
    Ok(Size {
        width: dimension("width")?,
        height: dimension("height")?,
    })
}

// Unsupported parser paths remain a visible text fallback, without recursion
// through the public render-then-measure boundary.
fn fallback_measure(expression: &str) -> Result<Size, MathError> {
    validate(expression)?;
    let mut width: f64 = 0.0;
    let mut height: f64 = 18.0;
    let mut escaped = false;
    let mut script_depth = 0_u32;
    let mut command = false;
    for (byte, rune) in expression.char_indices() {
        if escaped {
            escaped = false;
            if rune.is_ascii_alphabetic() {
                command = true;
                continue;
            }
            width += 8.0;
            continue;
        }
        match rune {
            '\\' => {
                escaped = true;
                command = false;
            }
            '^' | '_' => {
                script_depth = script_depth.saturating_add(1);
                height = height.max(18.0 + f64::from(script_depth) * 8.0);
            }
            '{' | '}' => {}
            c if c.is_whitespace() => width += 3.0,
            _ => width += if script_depth > 0 { 6.0 } else { 9.0 },
        }
        if command && !rune.is_ascii_alphabetic() {
            command = false;
        }
        let _ = byte;
    }
    if escaped {
        width += 8.0;
    }
    Ok(Size {
        width: width.max(1.0).ceil(),
        height: height.ceil(),
    })
}

/// Renders TeX exactly as D2 v0.9.0's `d2latex.Render` (mathjax-go v0.1.0
/// `mathjax.Render`) does, or refuses.
///
/// Unlike [`render`], this never returns the visible-text fallback: inputs
/// outside the translated MathJax parser/layout surface are reported as
/// [`MathError::Unsupported`] so callers never embed approximate output.
pub fn render_exact(expression: &str) -> Result<String, MathError> {
    validate_input(expression)?;
    if let Some(ast) = parse(expression) {
        if let Some(error) = cd::first_error_node(&ast) {
            return match error {
                Atom::SourceError(message) => Err(MathError::SourceConversion(message.clone())),
                Atom::ErrorDisplay(message) => Ok(cd::render_error(message)),
                _ => unreachable!(),
            };
        }
        if cd::first_nil_child(&ast).is_some() {
            return Err(MathError::SourceNilChild);
        }
        if let Some(layout) = layout(&ast) {
            return Ok(render_layout(&layout));
        }
    }
    Err(MathError::Unsupported)
}

/// Renders a deterministic semantic SVG for a TeX label.
///
/// The fallback keeps the original expression in a text node so unsupported
/// MathJax commands remain visible. Exact MathJax paths, glyph metrics, and
/// accessibility markup are a separate parity task.
pub fn render(expression: &str) -> Result<String, MathError> {
    validate_input(expression)?;
    // Recoverable parser errors precede structural fallback validation, as in
    // original MathJax. This also preserves the earliest padded-argument error.
    if let Some(ast) = parse(expression) {
        if let Some(error) = cd::first_error_node(&ast) {
            return match error {
                Atom::SourceError(message) => Err(MathError::SourceConversion(message.clone())),
                Atom::ErrorDisplay(message) => Ok(cd::render_error(message)),
                _ => unreachable!(),
            };
        }
        if cd::first_nil_child(&ast).is_some() {
            return Err(MathError::SourceNilChild);
        }
        if let Some(layout) = layout(&ast) {
            return Ok(render_layout(&layout));
        }
    }
    validate(expression)?;
    let size = fallback_measure(expression)?;
    let escaped = escape_text(expression);
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><text x=\"0\" y=\"{}\" font-family=\"serif\">{}</text></svg>",
        number(size.width),
        number(size.height),
        number(size.width),
        number(size.height),
        number((size.height * 0.78).max(1.0)),
        escaped
    ))
}

#[derive(Clone, Debug)]
enum Atom {
    ErrorDisplay(String),
    SourceError(String),
    // Source structural leaves carry no glyph or advance.
    EmptyNode(&'static str),
    RuleSpace {
        dimensions: [Dimension; 3],
        background: Option<String>,
        mathsize: f64,
    },
    // Height/depth-only source node (CD mpadded strut, xarrow mspace); it
    // still emits its own empty SVG node.
    Strut(f64, f64, &'static str),
    CdArrow {
        code: u32,
        texclass: Option<usize>,
        over: Option<Box<Atom>>,
        under: Option<Box<Atom>>,
        minimums: Minimums,
    },
    Glyph(u32),
    FontGlyph {
        original: u32,
        paint: u32,
        bold: bool,
        font_variant: &'static str,
    },
    VariantGlyph(u32),
    CalligraphicGlyph(u32),
    ItalicGlyph(u32),
    // Physical Unicode math characters retain their identity under font commands.
    LiteralGlyph(u32),
    Unit(u32),
    Reference {
        body: Box<Atom>,
        href: String,
    },
    TaggedTable {
        spacing: Option<f64>,
        rows: Vec<Vec<Atom>>,
        tags: Vec<Option<ams::Tag>>,
        environment: String,
        container_align: &'static str,
    },
    Boxed(Box<Atom>),
    NotToken {
        first: Box<Atom>,
        kind: &'static str,
        class: usize,
    },
    Space(f64),
    // A source filter candidate; inherited level is recorded before removal.
    NonscriptSpace {
        body: Box<Atom>,
        remove: bool,
    },
    Kern(Dimension),
    Sequence(Vec<Atom>),
    InferredRow(Vec<Atom>),
    ExplicitRow(Vec<Atom>),
    Mrow(Box<Atom>),
    NamedFunction(Box<Atom>),
    // A literal normal mi with svg/texclass.go autoOP, distinct from
    // native Identifier rows that may represent separate font tokens.
    AutoOperator(Box<Atom>),
    // Explicit mo spacing is owned by the outermost embellished wrapper.
    MathMLSpace {
        body: Box<Atom>,
        left: Option<Dimension>,
        right: Option<Dimension>,
    },
    ApplyFunction,
    Identifier(Vec<Atom>),
    OperatorToken(Vec<Atom>),
    RawOperator {
        attributes: &'static [(&'static str, &'static str)],
        cleanup: bool,
        code: u32,
        body: Box<Atom>,
        definition: operator_dictionary::Definition,
        // An explicit source texClass property, which
        // applyOperatorInheritance does not replace.
        texclass: Option<i8>,
    },
    Number(Vec<Atom>),
    Fraction(Box<Atom>, Box<Atom>),
    GeneralFraction(Box<Atom>, Box<Atom>, general_fraction::Spec),
    FractionStrut(f64),
    HorizontalMark {
        body: Box<Atom>,
        code: u32,
        stretchy: bool,
        mathaccent: bool,
    },
    BracketMark {
        body: Box<Atom>,
        thickness: f64,
        height: String,
        under: bool,
        mathsize: f64,
    },
    SplitFraction(Box<Atom>, Box<Atom>, f64),
    Sqrt(Box<Atom>),
    // \textbf/\textit: an mtext token whose characters take the math font
    // variant mapping (only alphanumeric content is admitted).
    StyledText(Vec<Atom>),
    // Original tex/commands.go sqrt with an optional index: mroot(base, index).
    // mathsize is resolved by styles::resolve_sizes; only 1 is laid out.
    Root {
        body: Box<Atom>,
        index: Box<Atom>,
        mathsize: f64,
    },
    SizedSqrt {
        body: Box<Atom>,
        mathsize: f64,
    },
    Operator(Box<Atom>),
    MovesScripts(Box<Atom>),
    ScriptPolicy {
        body: Box<Atom>,
        moves: bool,
        movable: Option<bool>,
        movable_property: bool,
    },
    Ord(Box<Atom>),
    Class {
        body: Box<Atom>,
        class: usize,
    },
    Phantom(Box<Atom>),
    Padded {
        body: Box<Atom>,
        padding: padded::Padding,
    },
    // mathtools \math*lap / \cramped*lap: an mstyle carrying an explicit
    // data-cramped attribute, which svg handleAttributes copies.
    CrampedStyle(Box<Atom>, bool),
    DeclaredStyle {
        body: Box<Atom>,
        display: bool,
        level: usize,
    },
    DeclaredSize {
        body: Box<Atom>,
        factor: f64,
    },
    Choice([Box<Atom>; 4]),
    XArrow {
        code: u32,
        over: Box<Atom>,
        under: Option<Box<Atom>>,
    },
    Text(String),
    ColorBox {
        body: Box<Atom>,
        background: String,
        mathsize: f64,
    },
    Color {
        body: Box<Atom>,
        color: String,
    },
    Cancel {
        body: Box<Atom>,
        up: bool,
        down: bool,
        arrow: bool,
        options: enclose::Options,
    },
    FenceDelimiter {
        code: u32,
        class: usize,
        color: Option<String>,
    },
    FixedDelimiter {
        code: u32,
        size: f64,
    },
    Fenced {
        open: u32,
        body: Vec<Atom>,
        close: u32,
        close_color: Option<String>,
        stretchy: bool,
        inner: bool,
    },
    Matrix {
        spacing: Option<f64>,
        options: Option<table::MultlineOptions>,
        rows: Vec<Vec<Atom>>,
        environment: String,
        column_spec: String,
    },
    Multiscripts {
        base: Box<Atom>,
        sub: Box<Atom>,
        sup: Box<Atom>,
        post: Option<[Box<Atom>; 2]>,
        prescript: bool,
        mathsize: f64,
    },
    Scripts {
        // Original parser distinguishes msub/msup from moving under/over nodes.
        limits: bool,
        // tex/script_attachment.go: parser-built compact families can be reused.
        eager: bool,
        accent: Option<accents::Spec>,
        leading_space: f64,
        base: Box<Atom>,
        sub: Option<Box<Atom>>,
        sup: Option<Box<Atom>>,
    },
}

#[derive(Clone, Copy)]
struct PositionedGlyph {
    font_original: Option<u32>,
    font_variant: &'static str,
    bold: bool,
    small_operator: bool,
    variant: bool,
    codepoint: u32,
    unit: bool,
    calligraphic: bool,
    italic: bool,
    x: f64,
    baseline: f64,
    scale: f64,
    large: bool,
    stretch: bool,
    radical_variant: Option<usize>,
    assembly_height: Option<(f64, f64)>,
}

#[derive(Clone, Copy)]
struct TableLine {
    x: f64,
    x2: Option<f64>,
    frame: bool,
    y1: f64,
    y2: f64,
    dashed: bool,
}
struct LayoutGroup {
    // Local SVG x and its paint-frame scale, before any scale round trip.
    x_frame: Option<(f64, f64)>,
    data_cramped: Option<bool>,
    // moToSVG's own "translate(0 u)" (u in the node's units, already x1000)
    // for symmetric/largeop centering; it replaces standardSVG's scale.
    own_translate: Option<f64>,
    accent_offset: Option<f64>,
    box_border: Option<f64>,
    reference: Option<String>,
    tagged: Option<ams::TaggedLayout>,
    percentage: Option<table::Percentage>,
    table_line: Option<TableLine>,
    mathsize: Option<f64>,
    tex_class: usize,
    before_glyph: usize,
    scale: f64,
    x: f64,
    baseline: f64,
    color: Option<String>,
    background: Option<(String, f64, f64, f64)>,
    kind: &'static str,
    delimiter: Option<cd::Delimiter>,
    arrow: Option<enclose::Arrow>,
    body: Box<Layout>,
    lines: Vec<(f64, f64, f64, f64, f64)>,
}

struct Layout {
    bounds_em: Option<(f64, f64)>,
    // Source BBox width in ems, separate from SVG paint-plan coordinates.
    width_em: Option<f64>,
    // BBox.W in the unscaled wrapper frame. Retain this before multiplying
    // by the absolute paint scale; tables apply their row RScale separately.
    intrinsic_width_em: Option<f64>,
    width: f64,
    ascent: f64,
    depth: f64,
    glyphs: Vec<PositionedGlyph>,
    rules: Vec<(f64, f64, f64, f64)>,
    groups: Vec<LayoutGroup>,
}

#[derive(Clone, Copy)]
struct GlyphData {
    height: f64,
    depth: f64,
    width: f64,
    path: &'static str,
}

thread_local! {
    // The mathcolor inherited at the current layout position.
    static INHERITED_COLOR: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn is_large_operator(code: u32) -> bool {
    matches!(code, 0x220F | 0x2211 | 0x222B)
        || compound(code).is_some_and(|g| g.large.is_some())
        // Other operator-dictionary largeop characters with -largeop and
        // -smallop glyphs (tex/operator_dictionary_generated.go, font data).
        || (operator_dictionary::lookup(code, operator_dictionary::Form::Prefix).flag("largeop")
            && large_glyphs::LARGE_GLYPHS
                .binary_search_by_key(&code, |g| g.codepoint)
                .is_ok()
            && smallop_glyphs::SMALL_OP_GLYPHS
                .binary_search_by_key(&code, |g| g.codepoint)
                .is_ok())
}

fn compound(code: u32) -> Option<&'static compound_glyphs::Compound> {
    compound_glyphs::COMPOUNDS
        .binary_search_by_key(&code, |g| g.codepoint)
        .ok()
        .map(|i| &compound_glyphs::COMPOUNDS[i])
}

fn uses_normal_text_fallback(codepoint: u32) -> bool {
    matches!(codepoint, 0xB5 | 0x2103 | 0x2030)
        || (compound(codepoint).is_none()
            && glyphs::GLYPHS
                .binary_search_by_key(&codepoint, |g| g.codepoint)
                .is_err())
}

// Original svg/wrapper.go isCJK: unknown wide glyphs use one em;
// other unknown glyphs use .6em, with common .75/.2 height/depth.
fn unknown_glyph_width(codepoint: u32) -> f64 {
    if matches!(codepoint, 0x1100..=0x115F | 0x2329 | 0x232A | 0x2E80..=0x303E
        | 0x3040..=0x3247 | 0x3250..=0x4DBF | 0x4E00..=0xA4C6 | 0xA960..=0xA97C
        | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE6B
        | 0xFF01..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1B000..=0x1B001 | 0x1F200..=0x1F251
        | 0x20000..=0x3FFFD)
    {
        1.0
    } else {
        0.6
    }
}

fn glyph(codepoint: u32) -> Option<GlyphData> {
    if let Some(g) = compound(codepoint) {
        return Some(GlyphData {
            height: g.height,
            depth: g.depth,
            width: g.width,
            path: "",
        });
    }
    // Original gensymb identities missing from the normal font use the
    // SVG wrapper's non-CJK unknownText metrics, not mathematical mu or C.
    if uses_normal_text_fallback(codepoint) {
        return Some(GlyphData {
            height: 0.75,
            depth: 0.2,
            width: unknown_glyph_width(codepoint),
            path: "",
        });
    }
    glyphs::GLYPHS
        .binary_search_by_key(&codepoint, |glyph| glyph.codepoint)
        .ok()
        .map(|index| {
            let glyph = glyphs::GLYPHS[index];
            GlyphData {
                height: glyph.height,
                depth: glyph.depth,
                width: glyph.width,
                path: glyph.path,
            }
        })
}

fn smallop_glyph(codepoint: u32) -> Option<GlyphData> {
    if compound(codepoint).is_some_and(|g| g.large.is_some()) {
        return glyph(codepoint);
    }

    smallop_glyphs::SMALL_OP_GLYPHS
        .binary_search_by_key(&codepoint, |g| g.codepoint)
        .ok()
        .map(|i| {
            let g = smallop_glyphs::SMALL_OP_GLYPHS[i];
            GlyphData {
                height: g.height,
                depth: g.depth,
                width: g.width,
                path: g.path,
            }
        })
}
fn variant_glyph(codepoint: u32) -> Option<GlyphData> {
    tex_variant_glyphs::TEX_VARIANT_GLYPHS
        .binary_search_by_key(&codepoint, |g| g.codepoint)
        .ok()
        .map(|i| {
            let g = tex_variant_glyphs::TEX_VARIANT_GLYPHS[i];
            GlyphData {
                height: g.height,
                depth: g.depth,
                width: g.width,
                path: g.path,
            }
        })
        .or_else(|| glyph(codepoint))
}
fn glyph_for(item: PositionedGlyph) -> Option<GlyphData> {
    if declaration_fonts::extended(item.font_variant) && !item.large && !item.small_operator {
        return declaration_fonts::lookup(item.codepoint, item.font_variant);
    }
    if item.bold {
        return double_struck::bold(item.codepoint).map(|r| r.0);
    }
    if item.large {
        if let Some((h, d, w)) = compound(item.codepoint).and_then(|g| g.large) {
            return Some(GlyphData {
                height: h,
                depth: d,
                width: w,
                path: "",
            });
        }
    }

    if item.small_operator {
        return smallop_glyph(item.codepoint);
    }
    if item.variant {
        return variant_glyph(item.codepoint);
    }
    if item.calligraphic {
        return calligraphic_glyph(item.codepoint);
    }
    if item.italic {
        return italic_glyph(item.codepoint);
    }
    if let Some((total, depth)) = item.assembly_height {
        return Some(GlyphData {
            height: total - depth,
            depth,
            width: 1.056,
            path: "",
        });
    }
    if let Some(index) = item.radical_variant {
        return radical_glyphs::RADICAL_GLYPHS.get(index).copied();
    }
    if item.stretch {
        return stretch_glyphs::STRETCH_GLYPHS.iter().find_map(|glyph| {
            (glyph.codepoint == item.codepoint).then_some(GlyphData {
                height: glyph.height,
                depth: glyph.depth,
                width: glyph.width,
                path: glyph.path,
            })
        });
    }
    if item.large {
        // Font.Lookup keeps the requested variant for Content descendants,
        // even when the parent character falls back to normal-font data.
        if let Some((height, depth, width)) = compound(item.codepoint).and_then(|g| g.large) {
            return Some(GlyphData {
                height,
                depth,
                width,
                path: "",
            });
        }
        return large_glyphs::LARGE_GLYPHS
            .binary_search_by_key(&item.codepoint, |glyph| glyph.codepoint)
            .ok()
            .map(|index| {
                let glyph = large_glyphs::LARGE_GLYPHS[index];
                GlyphData {
                    height: glyph.height,
                    depth: glyph.depth,
                    width: glyph.width,
                    path: glyph.path,
                }
            })
            .or_else(|| glyph(item.codepoint));
    }
    glyph(item.codepoint)
}

// Source parser.go parseCharacter and wrapper.go getVariant select italic for
// one-rune mi. Exact own entries precede SMP remaps and normal inheritance.
fn automatic_italic_atom(character: char) -> Atom {
    let code = character as u32;
    if italic_glyphs::ITALIC_GLYPHS
        .binary_search_by_key(&code, |g| g.codepoint)
        .is_ok()
    {
        return Atom::ItalicGlyph(code);
    }
    let code = italic_font_code(code);
    if uses_normal_text_fallback(code) {
        Atom::ItalicGlyph(character as u32)
    } else {
        Atom::Glyph(code)
    }
}

// Original base/AMS mathchar0mi maps; default one-rune mi is italic unless
// the source entry declares normal. hbar's variantForm is dispatched separately.
fn source_identifier(name: &str) -> Option<(u32, bool)> {
    Some((
        match name {
            "alpha" => 0x3B1,
            "beta" => 0x3B2,
            "gamma" => 0x3B3,
            "delta" => 0x3B4,
            "epsilon" => 0x3F5,
            "zeta" => 0x3B6,
            "eta" => 0x3B7,
            "theta" => 0x3B8,
            "iota" => 0x3B9,
            "kappa" => 0x3BA,
            "lambda" => 0x3BB,
            "mu" => 0x3BC,
            "nu" => 0x3BD,
            "xi" => 0x3BE,
            "omicron" => 0x3BF,
            "pi" => 0x3C0,
            "rho" => 0x3C1,
            "varsigma" => 0x3C2,
            "sigma" => 0x3C3,
            "tau" => 0x3C4,
            "upsilon" => 0x3C5,
            "phi" => 0x3D5,
            "chi" => 0x3C7,
            "psi" => 0x3C8,
            "omega" => 0x3C9,
            "varepsilon" => 0x3B5,
            "vartheta" => 0x3D1,
            "varpi" => 0x3D6,
            "varrho" => 0x3F1,
            "varphi" => 0x3C6,
            "varkappa" => 0x3F0,
            "digamma" => 0x3DD,
            "imath" => 0x131,
            "jmath" => 0x237,
            "hslash" => 0x210F,
            "partial" => 0x2202,
            "nabla" => 0x2207,
            "backslash" => 0x2216,
            "varGamma" => 0x393,
            "varDelta" => 0x394,
            "varTheta" => 0x398,
            "varLambda" => 0x39B,
            "varXi" => 0x39E,
            "varPi" => 0x3A0,
            "varSigma" => 0x3A3,
            "varUpsilon" => 0x3A5,
            "varPhi" => 0x3A6,
            "varPsi" => 0x3A8,
            "varOmega" => 0x3A9,
            _ => return None,
        },
        matches!(name, "nabla" | "backslash"),
    ))
}

fn command_codepoint(name: &str) -> Option<u32> {
    let greek = [
        ("alpha", 0x1D6FC),
        ("beta", 0x1D6FD),
        ("gamma", 0x1D6FE),
        ("delta", 0x1D6FF),
        ("epsilon", 0x1D700),
        ("zeta", 0x1D701),
        ("eta", 0x1D702),
        ("theta", 0x1D703),
        ("iota", 0x1D704),
        ("kappa", 0x1D705),
        ("lambda", 0x1D706),
        ("mu", 0x1D707),
        ("nu", 0x1D708),
        ("xi", 0x1D709),
        ("omicron", 0x1D70A),
        ("pi", 0x1D70B),
        ("rho", 0x1D70C),
        ("varsigma", 0x1D70D),
        ("sigma", 0x1D70E),
        ("tau", 0x1D70F),
        ("upsilon", 0x1D710),
        ("phi", 0x1D711),
        ("chi", 0x1D712),
        ("psi", 0x1D713),
        ("omega", 0x1D714),
        ("Gamma", 0x393),
        ("Delta", 0x394),
        ("Theta", 0x398),
        ("Lambda", 0x39B),
        ("Xi", 0x39E),
        ("Pi", 0x3A0),
        ("Sigma", 0x3A3),
        ("Upsilon", 0x3A5),
        ("Phi", 0x3A6),
        ("Psi", 0x3A8),
        ("Omega", 0x3A9),
    ];
    greek
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, cp)| *cp)
        .or_else(|| {
            Some(match name {
                "infty" => 0x221E,
                "sum" => 0x2211,
                "prod" => 0x220F,
                "int" => 0x222B,
                "pm" => 0x00B1,
                "times" => 0x00D7,
                "otimes" => 0x2297,
                "cdot" => 0x22C5,
                "le" | "leq" => 0x2264,
                "ge" | "geq" => 0x2265,
                "neq" => 0x2260,
                "equiv" => 0x2261,
                "circ" => 0x2218,
                "approx" => 0x2248,
                "sim" => 0x223C,
                "in" => 0x2208,
                "notin" => 0x2209,
                "rightarrow" | "to" => 0x2192,
                "leftarrow" => 0x2190,
                "uparrow" => 0x2191,
                "downarrow" => 0x2193,
                "updownarrow" => 0x2195,
                "Uparrow" => 0x21d1,
                "Downarrow" => 0x21d3,
                "Updownarrow" => 0x21d5,
                "leftrightarrow" => 0x2194,
                "Leftarrow" => 0x21D0,
                "Rightarrow" => 0x21D2,
                "Leftrightarrow" => 0x21D4,
                "longleftarrow" => 0x27F5,
                "longrightarrow" => 0x27F6,
                "longleftrightarrow" => 0x27F7,
                "Longleftarrow" => 0x27F8,
                "Longrightarrow" => 0x27F9,
                "Longleftrightarrow" => 0x27FA,
                "longmapsto" => 0x27FC,
                "rightleftharpoons" => 0x21CC,
                "hookrightarrow" => 0x21AA,
                "hookleftarrow" => 0x21A9,
                "langle" => 0x27E8,
                "rangle" => 0x27E9,
                "vert" | "mid" => 0x2223,
                "partial" => 0x2202,
                "degree" => 0xB0,
                "ldots" => 0x2026,
                "cdots" => 0x22EF,
                "micro" => 0xB5,
                "ohm" => 0x2126,
                _ => return None,
            })
        })
}

// tex/source_runtime.go lookupMJSourceEntry ignores maps without a parser
// (remap/not_remap) and only admits backslash-prefixed DelimiterMap keys.
// The retained source table also contains raw delimiter keys; those entries
// belong to readDelimiter's namespace and do not register commands.
fn command_symbol(name: &str) -> Option<&'static source_symbols::Symbol> {
    let symbol = source_symbols::lookup(name)?;
    if matches!(symbol.map, "remap" | "not_remap")
        || (symbol.parser == source_symbols::Parser::Delimiter
            && matches!(name, "(" | ")" | "[" | "]" | "<" | ">" | "/" | "."))
    {
        return None;
    }
    Some(symbol)
}

fn resolve_named_color(color: String) -> String {
    named_colors::NAMED_COLORS
        .binary_search_by_key(&color.as_str(), |entry| entry.0)
        .ok()
        .map_or(color.clone(), |index| {
            named_colors::NAMED_COLORS[index].1.to_owned()
        })
}

fn parse(expression: &str) -> Option<Atom> {
    let mut result = parse_unresolved(expression)?;
    if cd::first_error(&result).is_none() && cd::first_nil_child(&result).is_none() {
        table::inherit_columns(&mut result, 'c');
        ams::containers(&mut result, "center");
        styles::resolve_sizes(&mut result, 1.0);
        styles::resolve(&mut result, true, 0, false);
        if let Err(message) = nonscript::filter(&mut result, None) {
            return Some(Atom::SourceError(message));
        }
        operator_dictionary::inherit(&mut result, operator_dictionary::Form::Infix);
    }
    Some(result)
}

// Inheritance selects mathchoice after the complete tree has been parsed.
// Nested cells and arrow labels must retain all four branches until then.
fn parse_unresolved(expression: &str) -> Option<Atom> {
    let shared = ams::Shared::default();
    let result = parse_unresolved_shared(expression, shared.clone())?;
    if cd::first_error(&result).is_none() && shared.borrow().current.tag.is_some() {
        let tag = ams::make_tag(&shared)?;
        return Some(Atom::TaggedTable {
            spacing: None,
            rows: vec![vec![result]],
            tags: vec![Some(tag)],
            environment: "equation-tag".into(),
            container_align: "center",
        });
    }
    Some(result)
}
fn parse_unresolved_shared(expression: &str, ams: ams::Shared) -> Option<Atom> {
    parse_with_stack(expression, ams, Default::default())
}
fn parse_with_stack(
    expression: &str,
    ams: ams::Shared,
    equation_seen: EquationStack,
) -> Option<Atom> {
    parse_with_root_stack(expression, ams, equation_seen, false, Default::default())
}
fn parse_with_root_stack(
    expression: &str,
    ams: ams::Shared,
    equation_seen: EquationStack,
    in_root: bool,
    root_moves: root_index::Shared,
) -> Option<Atom> {
    let mut parser = Parser {
        in_root,
        root_moves,
        physics_after: None,
        physics_auto: None,
        command_position: None,
        pending_cell: None,
        pending_infix: None,
        row_delimiter: None,
        active_color: None,
        equation_seen,
        array_cell: None,
        vector_alias: false,
        vector_font: None,
        ams,
        splice_inferred: false,
        chars: expression.chars().collect(),
        index: 0,
        active_font: None,
        normal_font: false,
        calligraphic_font: false,
        multi_letter_font: false,
        operator_letters: false,
        operator_pattern: false,
        no_auto_operator: false,
    };
    let result = parser.sequence(None)?;
    (parser.index == parser.chars.len()).then_some(result)
}

type EquationStack = std::rc::Rc<std::cell::Cell<bool>>;

struct Parser {
    in_root: bool,
    root_moves: root_index::Shared,
    physics_after: Option<physics::After>,
    physics_auto: Option<physics::AutoOpen>,
    command_position: Option<position::Position>,
    pending_cell: Option<array_body::PendingCell>,
    pending_infix: Option<general_fraction::PendingInfix>,
    row_delimiter: Option<braket::RowDelimiter>,
    active_color: Option<String>,
    equation_seen: EquationStack,
    array_cell: Option<array_body::CellState>,
    vector_alias: bool,
    vector_font: Option<bool>,
    ams: ams::Shared,
    splice_inferred: bool,
    chars: Vec<char>,
    index: usize,
    active_font: Option<&'static str>,
    normal_font: bool,
    calligraphic_font: bool,
    multi_letter_font: bool,
    operator_letters: bool,
    operator_pattern: bool,
    no_auto_operator: bool,
}

impl Parser {
    // parser.go ambientFontToken/applyFontScope: only the creating parser's
    // eligible tokens inherit a MathFont/SetFont environment. Named mathchar0mi
    // symbols and already resolved inner scopes retain their own variants.
    fn ambient_font(&self, mut atom: Atom) -> Atom {
        if let Some(font) = self.active_font {
            declaration_fonts::apply_wrapper(&mut atom, font);
        }
        self.vector_font_token(atom)
    }

    // parser.go applyFontScope's Go-only boldsymbol extension also selects
    // tokens without ambientFontSource. Resolve them in their creating parser
    // so an enclosing scope cannot overwrite an inner lexical font choice.
    fn nonambient_font(&self, atom: Atom) -> Atom {
        if self.active_font == Some("boldsymbol") {
            self.ambient_font(atom)
        } else {
            self.vector_font_token(atom)
        }
    }

    fn sequence(&mut self, closing: Option<char>) -> Option<Atom> {
        self.sequence_with_infix(closing, false)
    }

    fn sequence_with_infix(&mut self, closing: Option<char>, infix_pending: bool) -> Option<Atom> {
        self.sequence_with_prefix(closing, infix_pending, Vec::new())
    }

    fn sequence_with_prefix(
        &mut self,
        closing: Option<char>,
        infix_pending: bool,
        prefix: Vec<Atom>,
    ) -> Option<Atom> {
        let initial_font = self.active_font;
        let initial_color = self.active_color.clone();
        let result = self.sequence_with_dots(closing, infix_pending, false, prefix);
        self.active_font = initial_font;
        self.active_color = initial_color;
        result
    }

    fn style_sequence(&mut self, closing: Option<char>, infix_pending: bool) -> Option<Atom> {
        let initial_font = self.active_font;
        let result = self.sequence_with_dots(closing, infix_pending, true, Vec::new());
        self.active_font = initial_font;
        result
    }

    fn sequence_with_dots(
        &mut self,
        closing: Option<char>,
        infix_pending: bool,
        style_pending: bool,
        mut children: Vec<Atom>,
    ) -> Option<Atom> {
        let mut dots = pending_dots::Pending::default();
        let mut positions: Vec<position::Position> = Vec::new();
        let mut closed = false;
        while self.index < self.chars.len()
            || self.physics_after.is_some()
            || self.pending_cell.is_some()
            || self.pending_infix.is_some()
            || self.row_delimiter.is_some()
        {
            // Over's arguments are read once before StyleItems close; the
            // enclosing logical row then owns the complete numerator.
            if self.pending_infix.is_some() {
                if style_pending {
                    break;
                }
                dots.finish(&mut positions, &mut children);
                if let Some(position) = positions.last() {
                    return Some(position.missing_box());
                }
                let item = self.pending_infix.take()?;
                if infix_pending {
                    self.index = self.chars.len();
                    return Some(Atom::ErrorDisplay(format!(
                        "Ambiguous use of \\{}",
                        item.name
                    )));
                }
                self.active_font = item.font;
                self.active_color = item.color;
                let right = self.sequence_with_infix(closing, true)?;
                if cd::first_error(&right).is_some() {
                    return Some(right);
                }
                let content = self.finish_infix(Atom::Sequence(children), right, item.spec)?;
                if cd::first_error(&content).is_some() {
                    self.index = self.chars.len();
                }
                if self.pending_cell.is_some() {
                    children = vec![content];
                    continue;
                }
                return Some(Atom::Sequence(vec![content]));
            }
            // parser.go replays the same closing CellItem after Style/Over
            // have delivered their final MML. A real group owns the event;
            // it must not consume its close brace while styles unwind.
            if self.pending_cell.is_some() {
                if style_pending || infix_pending {
                    break;
                }
                dots.finish(&mut positions, &mut children);
                if let Some(position) = positions.last() {
                    return Some(position.missing_box());
                }
                let item = self.pending_cell.take()?;
                if !item.linebreak {
                    self.index = self.chars.len();
                    return Some(Atom::ErrorDisplay("Misplaced \\cr".into()));
                }
                self.active_font = item.font;
                self.active_color = item.color;
                children.push(Atom::Sequence(item.cell.spaces()));
                continue;
            }
            if let Some(after) = self.physics_after.take() {
                let tail = self.physics_complete_after(after)?;
                if cd::first_error(&tail).is_some() {
                    self.index = self.chars.len();
                    return Some(tail);
                }
                let Atom::Sequence(nodes) = tail else {
                    unreachable!()
                };
                for node in nodes {
                    let mut delivered = dots.deliver(&mut positions, node);
                    if let Some(last) = delivered.pop() {
                        children.extend(delivered);
                        let node = self.attach_scripts(last)?;
                        if children.last().is_some_and(physics::named_function)
                            && physics::accepts_application(&node)
                        {
                            children.push(Atom::ApplyFunction);
                        }
                        children.push(node);
                    }
                }
                continue;
            }
            if self.physics_auto.as_ref().is_some_and(|a| a.closed) {
                break;
            }
            // Right/Middle close every continuation before Left receives
            // the saved delimiter and its original lexical color.
            if self.row_delimiter.is_some() {
                break;
            }
            if closing.is_some_and(|value| self.chars[self.index] == value) {
                self.index += 1;
                closed = true;
                break;
            }
            if self.chars[self.index] == '}' && self.physics_auto.is_some() {
                return Some(physics::AutoOpen::stop_error());
            }
            if self.chars[self.index] == '}' {
                self.index = self.chars.len();
                return Some(Atom::ErrorDisplay(
                    "Extra close brace or missing open brace".to_owned(),
                ));
            }
            // Original parseRow handles syntax before raw character tokens.
            if matches!(self.chars[self.index], '&' | '#') {
                if self.chars[self.index] == '&' {
                    if let Some(position) = positions.last() {
                        self.index = self.chars.len();
                        return Some(position.missing_box());
                    }
                }
                if self.chars[self.index] == '&' && closing.is_none() {
                    if let Some(cell) = &mut self.array_cell {
                        cell.stop = Some(array_body::CellItem::entry());
                        self.index += 1;
                        break;
                    }
                }
                let message = if self.chars[self.index] == '&' {
                    "Misplaced &"
                } else {
                    "You can't use 'macro parameter character #' in math mode"
                };
                self.index = self.chars.len();
                return Some(Atom::ErrorDisplay(message.to_owned()));
            }
            if self.chars[self.index] == '%' {
                while self.index < self.chars.len() {
                    let c = self.chars[self.index];
                    self.index += 1;
                    if matches!(c, '\n' | '\r') {
                        break;
                    }
                }
                continue;
            }
            // Base's special map consumes ASCII space/tab/CR/LF and NBSP.
            // Other Unicode spaces are ordinary tokens and keep font metrics.
            if matches!(self.chars[self.index], ' ' | '\t' | '\r' | '\n' | '\u{a0}') {
                self.index += 1;
                continue;
            }
            if self.chars[self.index] == '\\' {
                let name: String = self.chars[self.index + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect();
                if self.paired_definition(&name).is_none() {
                    if !self.ams.borrow().macros.contains_key(&name) {
                        if name == "nonscript" {
                            self.index += name.len() + 1;
                            dots.finish(&mut positions, &mut children);
                            if let Some(last) = children.last_mut() {
                                pending_dots::finish_function(last);
                            }
                            dots = pending_dots::Pending::start_nonscript();
                            continue;
                        }
                        if name == "not" {
                            self.index += name.len() + 1;
                            dots.finish(&mut positions, &mut children);
                            if let Some(last) = children.last_mut() {
                                pending_dots::finish_function(last);
                            }
                            dots = pending_dots::Pending::start_not();
                            continue;
                        }
                        if name == "dots" {
                            self.index += name.len() + 1;
                            dots.finish(&mut positions, &mut children);
                            if let Some(last) = children.last_mut() {
                                pending_dots::finish_function(last);
                            }
                            dots = pending_dots::Pending::start(self);
                            continue;
                        }
                        // These stack items are non-MML successors. Left/Open
                        // retain Dots until their completed fenced/group node.
                        if matches!(
                            name.as_str(),
                            "not" | "nonscript" | "raise" | "lower" | "moveleft" | "moveright"
                        ) {
                            dots.finish(&mut positions, &mut children);
                        }
                    }
                    if matches!(name.as_str(), "hline" | "hdashline")
                        && !self.ams.borrow().macros.contains_key(&name)
                    {
                        let rules = self
                            .array_cell
                            .as_ref()
                            .filter(|cell| {
                                closing.is_none()
                                    && !dots.active()
                                    && cell.rule_allowed
                                    && cell.at_start
                                    && children.is_empty()
                            })
                            .and_then(|cell| cell.rules.clone());
                        let Some(rules) = rules else {
                            self.index = self.chars.len();
                            return Some(Atom::ErrorDisplay(format!("Misplaced \\{name}")));
                        };
                        if let Some(e) = self.array_cell.as_ref().and_then(|c| c.equation.as_ref())
                        {
                            rules.borrow_mut().rows = e.borrow().rows.len();
                        }
                        rules.borrow_mut().add(&name);
                        self.index += name.len() + 1;
                        continue;
                    }
                    if hfill::is_fill(&name) && !self.ams.borrow().macros.contains_key(&name) {
                        self.index += name.len() + 1;
                        let allowed = closing.is_none()
                            && !infix_pending
                            && self.physics_auto.is_none()
                            && positions.is_empty()
                            && !dots.active()
                            && !children.last().is_some_and(physics::named_function)
                            && self
                                .array_cell
                                .as_ref()
                                .is_some_and(|cell| !cell.fills.blocked);
                        if !allowed {
                            self.index = self.chars.len();
                            return Some(hfill::unsupported(&name));
                        }
                        self.array_cell.as_mut()?.fills.push(children.len());
                        continue;
                    }
                    if alignment::is_command(&name) {
                        self.index += name.len() + 1;
                        match self.equation_command(
                            &name,
                            &mut children,
                            closing.is_none() && !dots.active(),
                        ) {
                            Ok(true) => {}
                            Ok(false) => return None,
                            Err(error) => {
                                self.index = self.chars.len();
                                return Some(Atom::ErrorDisplay(error));
                            }
                        }
                        continue;
                    }
                    if matches!(name.as_str(), "shoveleft" | "shoveright") {
                        if dots.active() {
                            return Some(Atom::ErrorDisplay(format!(
                                "\\{name} only allowed in multline environment"
                            )));
                        }
                        if let Some(cell) = &mut self.array_cell {
                            if matches!(cell.environment.as_str(), "multline" | "multline*")
                                && cell.shove_allowed
                            {
                                if !cell.at_start || !children.is_empty() {
                                    return Some(Atom::ErrorDisplay(format!(
                                        "\\{name} must come at the beginning of the line"
                                    )));
                                }
                                cell.shove =
                                    Some(if name == "shoveleft" { "left" } else { "right" });
                                self.index += name.len() + 1;
                                continue;
                            }
                        }
                    }
                    if matches!(name.as_str(), "right" | "middle")
                        && !self.ams.borrow().macros.contains_key(&name)
                    {
                        self.index += name.len() + 1;
                        let delimiter = match self.native_delimiter(&name, false) {
                            Ok(code) => code,
                            Err(error) => return Some(Atom::ErrorDisplay(error)),
                        };
                        if closing != Some('\0') || self.physics_auto.is_some() {
                            return Some(Atom::ErrorDisplay(if name == "middle" {
                                "Extra \\middle".into()
                            } else {
                                "Missing \\left or extra \\right".into()
                            }));
                        }
                        self.row_delimiter = Some(braket::RowDelimiter {
                            middle: name == "middle",
                            delimiter,
                            color: self.active_color.clone(),
                        });
                        break;
                    }
                    if general_fraction::is_infix(&name)
                        && !self.ams.borrow().macros.contains_key(&name)
                    {
                        self.index += name.len() + 1;
                        // BaseMethods.Over reads both delimiters and the dimension
                        // before the pending OverItem checks ambiguity.
                        let spec = match self.read_infix_spec(&name) {
                            Ok(spec) => spec,
                            Err(error) => {
                                self.index = self.chars.len();
                                return Some(Atom::ErrorDisplay(error));
                            }
                        };
                        self.pending_infix = Some(general_fraction::PendingInfix {
                            name,
                            spec,
                            font: self.active_font,
                            color: self.active_color.clone(),
                        });
                        continue;
                    }
                    if matches!(name.as_str(), "uproot" | "leftroot")
                        && !self.ams.borrow().macros.contains_key(&name)
                    {
                        self.index += name.len() + 1;
                        if let Err(message) = self.move_root(&name) {
                            self.index = self.chars.len();
                            return Some(Atom::ErrorDisplay(message));
                        }
                        continue;
                    }
                    if declaration_fonts::declaration(&name)
                        && !self.ams.borrow().macros.contains_key(&name)
                    {
                        self.index += name.len() + 1;
                        if let Some(cell) = &mut self.array_cell {
                            cell.at_start &= children.is_empty();
                        }
                        let previous = self.active_font;
                        self.active_font = Some(declaration_fonts::font_name(&name)?);
                        if dots.active() {
                            continue;
                        }
                        if let Some(cell) = &mut self.array_cell {
                            cell.fills.offset += children.len();
                        }
                        // SetFont adds no stack item: its continuation keeps
                        // the enclosing StyleItem as the closing recipient.
                        let body = self.sequence_with_dots(
                            closing,
                            infix_pending,
                            style_pending,
                            Vec::new(),
                        );
                        if let Some(cell) = &mut self.array_cell {
                            cell.fills.offset -= children.len();
                        }
                        self.active_font = previous;
                        let body = body?;
                        if let Atom::Sequence(v) = body {
                            // SetFont continues the same row. A pending FnItem
                            // observes its first final child across this boundary.
                            if children.last().is_some_and(physics::named_function)
                                && v.first().is_some_and(physics::accepts_application)
                            {
                                children.push(Atom::ApplyFunction);
                            }
                            for atom in v {
                                children.extend(dots.deliver(&mut positions, atom));
                            }
                        } else {
                            children.extend(dots.deliver(&mut positions, body));
                        }
                        if let Some(position) = positions.last() {
                            return Some(position.missing_box());
                        }
                        if self.pending_cell.is_some() || self.pending_infix.is_some() {
                            continue;
                        }
                        return Some(Atom::Sequence(children));
                    }
                    if matches!(name.as_str(), "limits" | "nolimits") {
                        self.index += name.len() + 1;
                        if dots.active() {
                            self.index = self.chars.len();
                            return Some(Atom::ErrorDisplay(format!(
                                "\\{name} is allowed only on operators"
                            )));
                        }
                        match limits::set_limits(children.pop(), &name) {
                            Ok(base) => children.push(base),
                            Err(message) => {
                                self.index = self.chars.len();
                                return Some(Atom::ErrorDisplay(message));
                            }
                        }
                        continue;
                    }
                    if let Some(factor) = styles::size_declaration(&name) {
                        dots.finish(&mut positions, &mut children);
                        if let Some(cell) = &mut self.array_cell {
                            cell.shove_allowed = false;
                            cell.fills.blocked = true;
                            cell.rule_allowed = false;
                            if cell.cd_end.is_some() {
                                cell.cd_end = Some(true);
                            }
                        }
                        self.index += name.len() + 1;
                        let body = self.style_sequence(closing, infix_pending)?;
                        children.extend(dots.deliver(
                            &mut positions,
                            Atom::DeclaredSize {
                                body: Box::new(body),
                                factor,
                            },
                        ));
                        if self.pending_cell.is_some() || self.pending_infix.is_some() {
                            continue;
                        }
                        return Some(Atom::Sequence(children));
                    }
                    if let Some((display, level)) = styles::declaration(&name) {
                        dots.finish(&mut positions, &mut children);
                        if let Some(cell) = &mut self.array_cell {
                            cell.shove_allowed = false;
                            cell.fills.blocked = true;
                            cell.rule_allowed = false;
                            if cell.cd_end.is_some() {
                                cell.cd_end = Some(true);
                            }
                        }
                        self.index += name.len() + 1;
                        let body = self.style_sequence(closing, infix_pending)?;
                        children.extend(dots.deliver(
                            &mut positions,
                            Atom::DeclaredStyle {
                                body: Box::new(body),
                                display,
                                level,
                            },
                        ));
                        if self.pending_cell.is_some() || self.pending_infix.is_some() {
                            continue;
                        }
                        return Some(Atom::Sequence(children));
                    }
                }
            }
            let command: String = self.chars[self.index..].iter().take(7).collect();
            if command.starts_with(r"\color")
                && self.paired_definition("color").is_none()
                && self
                    .chars
                    .get(self.index + 6)
                    .is_none_or(|c| !c.is_ascii_alphabetic())
            {
                self.index += 6;
                let color = resolve_named_color(self.braced_text()?);
                if let Some(cell) = &mut self.array_cell {
                    cell.rule_allowed = false;
                    cell.fills.blocked = true;
                }
                dots.finish(&mut positions, &mut children);
                let previous_color = self.active_color.replace(color.clone());
                let body = self.style_sequence(closing, infix_pending);
                self.active_color = previous_color;
                let body = body?;
                children.extend(dots.deliver(
                    &mut positions,
                    Atom::Color {
                        body: Box::new(body),
                        color,
                    },
                ));
                if self.pending_cell.is_some() || self.pending_infix.is_some() {
                    continue;
                }
                return Some(Atom::Sequence(children));
            }
            if self.chars[self.index] == '\\' {
                let name: String = self.chars[self.index + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect();
                if matches!(name.as_str(), "ce" | "pu") && self.paired_definition(&name).is_none() {
                    self.index += name.len() + 1;
                    if let Err(message) = self.insert_chemistry(&name) {
                        self.index = self.chars.len();
                        return Some(Atom::ErrorDisplay(message));
                    }
                    continue;
                }
            }
            if matches!(self.chars[self.index], '\'' | '\u{2019}') {
                if dots.active() {
                    dots.finish(&mut positions, &mut children);
                    children.push(Atom::Identifier(Vec::new()));
                }
                let base = children
                    .pop()
                    .unwrap_or_else(|| Atom::Identifier(Vec::new()));
                let atom = self.attach_primes(base)?;
                if cd::first_error(&atom).is_some() {
                    self.index = self.chars.len();
                    return Some(atom);
                }
                children.push(atom);
                continue;
            }
            if matches!(self.chars[self.index], '_' | '^') {
                if dots.active() {
                    dots.finish(&mut positions, &mut children);
                    // Stack.Prev on DotsItem sees no stored MML. Scripts and
                    // primes receive a fresh empty mi, not the emitted dots.
                    children.push(Atom::Identifier(Vec::new()));
                }
                let base = children
                    .pop()
                    .unwrap_or_else(|| Atom::Identifier(Vec::new()));
                let atom = self.attach_scripts(base)?;
                if cd::first_error(&atom).is_some() {
                    self.index = self.chars.len();
                    return Some(atom);
                }
                children.push(atom);
                continue;
            }
            let autoclose = self
                .chars
                .get(self.index)
                .copied()
                .filter(|c| matches!(c, ')' | ']' | '|'));
            let position_close = !positions.is_empty() && self.position_linebreak();
            let atom = self.atom()?;
            if self
                .array_cell
                .as_ref()
                .is_some_and(|cell| cell.stop.is_some())
            {
                break;
            }
            // A source parser error aborts parsing before missing group closes
            // or later commands can replace the earliest diagnostic.
            if cd::first_error(&atom).is_some() {
                self.index = self.chars.len();
                return Some(atom);
            }
            if position_close {
                self.index = self.chars.len();
                return Some(positions.last()?.missing_box());
            }
            if self.pending_cell.is_some() {
                continue;
            }
            if let Some(position) = self.command_position.take() {
                positions.push(position);
                continue;
            }
            if std::mem::take(&mut self.splice_inferred) {
                if let Atom::Sequence(v) = atom {
                    for item in v {
                        for delivered in dots.deliver(&mut positions, item) {
                            if children.last().is_some_and(physics::named_function)
                                && physics::accepts_application(&delivered)
                            {
                                children.push(Atom::ApplyFunction);
                            }
                            children.push(delivered);
                        }
                    }
                    continue;
                }
            }
            let atom = if !positions.is_empty() && physics::named_function(&atom) {
                self.attach_scripts(atom)?
            } else {
                atom
            };
            let mut delivered = dots.deliver(&mut positions, atom);
            if let Some(auto) = &mut self.physics_auto {
                delivered.retain_mut(|item| {
                    if auto.closed {
                        auto.trailing.push(item.clone());
                        false
                    } else {
                        !auto.accept(item, autoclose)
                    }
                });
                if auto.closed {
                    children.extend(delivered);
                    break;
                }
            }
            let atom = self.attach_scripts(delivered.pop()?)?;
            children.extend(delivered);
            if cd::first_error(&atom).is_some() {
                self.index = self.chars.len();
                return Some(atom);
            }
            if children.last().is_some_and(physics::named_function)
                && physics::accepts_application(&atom)
            {
                children.push(Atom::ApplyFunction);
            }
            children.push(atom);
        }
        dots.finish(&mut positions, &mut children);
        // parseRow reports a missing terminator when the input ends first; a
        // nested group's consumed close brace does not close this row.
        if let Some(position) = positions.last() {
            return Some(position.missing_box());
        }
        if closing.is_some()
            && closing != Some('\0')
            && !closed
            && self.pending_cell.is_none()
            && self.pending_infix.is_none()
            && !self.physics_auto.as_ref().is_some_and(|a| a.closed)
        {
            return Some(Atom::ErrorDisplay(
                "Extra open brace or missing close brace".to_owned(),
            ));
        }
        if closing.is_none() {
            if let Some(cell) = &mut self.array_cell {
                if cell.stop.is_none() && cell.cd_end.take() == Some(true) {
                    children.push(Atom::Strut(850., 200., "mpadded"));
                }
            }
        }
        Some(Atom::Sequence(children))
    }

    fn insert_chemistry(&mut self, name: &str) -> Result<(), String> {
        let raw = self.padded_raw_argument(name)?;
        let expansion = mhchem::to_tex(&raw, name).map_err(|error| error.message)?;
        self.chars.splice(self.index..self.index, expansion.chars());
        Ok(())
    }

    // TexParser.GetNext uses ECMAScript whitespace, not Unicode White_Space.
    fn prime_space(c: &char) -> bool {
        matches!(*c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' |
            '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
            '\u{205f}' | '\u{3000}' | '\u{feff}')
    }

    // tex/pending_prime.go retains the original base until the next item.
    // An immediately following script consumes the pending prime token.
    fn attach_primes(&mut self, atom: Atom) -> Option<Atom> {
        self.index += 1;
        let mut count = 1;
        loop {
            while self.chars.get(self.index).is_some_and(Self::prime_space) {
                self.index += 1;
            }
            if !self
                .chars
                .get(self.index)
                .is_some_and(|c| matches!(c, '\'' | '\u{2019}'))
            {
                break;
            }
            self.index += 1;
            count += 1;
        }
        // HFill observes the pending PrimeItem before it is reduced to MML.
        if self.chars.get(self.index) == Some(&'\\') {
            let name: String = self.chars[self.index + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if hfill::is_fill(&name)
                && self.paired_definition(&name).is_none()
                && !self.ams.borrow().macros.contains_key(&name)
            {
                self.index = self.chars.len();
                return Some(hfill::unsupported(&name));
            }
        }
        // attachPrimes creates an ORD operator token. The named \prime
        // command is an identifier; preserve that distinct origin.
        let prime = Atom::OperatorToken(match count {
            1 => vec![Atom::Glyph(0x2032)],
            2 => vec![Atom::Glyph(0x2033)],
            3 => vec![Atom::Glyph(0x2034)],
            4 => vec![Atom::Glyph(0x2057)],
            _ => (0..count).map(|_| Atom::Glyph(0x2032)).collect(),
        });
        // Node properties do not hide the original script kind.
        let atom = match atom {
            Atom::ScriptPolicy { body, .. }
                if matches!(body.as_ref(), Atom::Scripts { limits: false, .. }) =>
            {
                *body
            }
            atom => atom,
        };
        if matches!(
            &atom,
            Atom::Scripts {
                limits: false,
                sup: Some(_),
                eager,
                sub,
                ..
            } if *eager || sub.is_some()
        ) {
            return Some(Atom::ErrorDisplay(
                "Prime causes double exponent: use braces to clarify".into(),
            ));
        }
        if self
            .chars
            .get(self.index)
            .is_some_and(|c| matches!(c, '^' | '_'))
        {
            let marker = self.chars[self.index];
            let mut result = self.attach_script_items(atom, true)?;
            if let Atom::Scripts { sup, .. } = &mut result {
                if marker == '^' {
                    if let Some(script) = sup.take() {
                        *sup = Some(Box::new(Atom::InferredRow(vec![prime, *script])));
                    }
                } else {
                    *sup = Some(Box::new(prime));
                }
            }
            return Some(result);
        }
        Some(match atom {
            Atom::Scripts {
                limits: false,
                base,
                sub,
                sup: None,
                leading_space,
                ..
            } => Atom::Scripts {
                limits: false,
                accent: None,
                base,
                sub,
                sup: Some(Box::new(prime)),
                leading_space,
                eager: true,
            },
            base => Atom::Scripts {
                limits: false,
                accent: None,
                eager: true,
                leading_space: 0.,
                base: Box::new(base),
                sub: None,
                sup: Some(Box::new(prime)),
            },
        })
    }

    fn attach_scripts(&mut self, atom: Atom) -> Option<Atom> {
        self.attach_script_items(atom, false)
    }

    fn attach_script_items(&mut self, atom: Atom, first_only: bool) -> Option<Atom> {
        if !self
            .chars
            .get(self.index)
            .is_some_and(|c| matches!(c, '_' | '^'))
        {
            return Some(atom);
        }
        let atom = match atom {
            Atom::ScriptPolicy {
                body,
                moves,
                movable,
                movable_property,
            } => {
                match *body {
                    Atom::Scripts {
                        base,
                        sub,
                        sup,
                        limits,
                        accent,
                        eager,
                        leading_space,
                    } if !limits || moves && (eager || sub.is_some()) => {
                        // setLimits has already selected the node family. Keep
                        // authored under accents when the existing owner is reused.
                        Atom::Scripts {
                            base,
                            eager: eager || sub.is_some(),
                            sub,
                            sup,
                            limits,
                            accent,
                            leading_space,
                        }
                    }
                    body @ Atom::Multiscripts {
                        prescript: true, ..
                    } if !moves => body,
                    body @ Atom::XArrow { .. } => body,
                    body => Atom::ScriptPolicy {
                        body: Box::new(body),
                        moves,
                        movable,
                        movable_property,
                    },
                }
            }
            atom => atom,
        };
        // script_attachment.go checks occupied authored under/over slots even
        // when movesupsub is absent and the next script needs a new owner.
        if let Atom::Scripts {
            sub: Some(_),
            sup,
            limits: true,
            eager: false,
            accent,
            ..
        } = &atom
            && !accent.is_some_and(|spec| spec.enabled)
        {
            let marker = self.chars[self.index];
            if marker == '_' || sup.is_some() {
                return Some(Atom::ErrorDisplay(
                    if marker == '_' {
                        "Double subscripts: use braces to clarify"
                    } else {
                        "Double exponent: use braces to clarify"
                    }
                    .into(),
                ));
            }
        }
        let (mut atom, mut sub, mut sup, limits, accent) = match atom {
            Atom::Multiscripts {
                base,
                sub,
                sup,
                post,
                prescript: true,
                mathsize,
            } => {
                let [post_sub, post_sup] = post.unwrap_or_else(multiscripts::empty_pair);
                let present =
                    |a: Box<Atom>| (!matches!(a.as_ref(), Atom::EmptyNode("none"))).then_some(a);
                (
                    Atom::Multiscripts {
                        base,
                        sub,
                        sup,
                        post: None,
                        prescript: true,
                        mathsize,
                    },
                    present(post_sub),
                    present(post_sup),
                    false,
                    None,
                )
            }
            Atom::Scripts {
                base,
                sub,
                sup,
                limits,
                accent,
                eager,
                ..
            } if eager || !limits && sub.is_some() && accent.is_none() => {
                (*base, sub, sup, limits, accent)
            }
            atom => {
                let limits = script_moves(&atom);
                (atom, None, None, limits, None)
            }
        };
        while self.index < self.chars.len() && matches!(self.chars[self.index], '_' | '^') {
            let marker = self.chars[self.index];
            self.index += 1;
            while self.chars.get(self.index).is_some_and(Self::prime_space) {
                self.index += 1;
            }
            // script_argument.go inserts a boundary after an immediate digit;
            // subsequent font/macro continuation retains the full number scanner.
            if self.chars.get(self.index).is_some_and(char::is_ascii_digit) {
                self.chars.insert(self.index + 1, ' ');
            }
            let duplicate = if marker == '_' {
                sub.is_some()
            } else {
                sup.is_some()
            };
            if duplicate {
                self.index = self.chars.len();
                return Some(Atom::ErrorDisplay(
                    if marker == '_' {
                        "Double subscripts: use braces to clarify"
                    } else {
                        "Double exponent: use braces to clarify"
                    }
                    .into(),
                ));
            }
            let script = loop {
                while self.chars.get(self.index).is_some_and(Self::prime_space) {
                    self.index += 1;
                }
                if self.index == self.chars.len() {
                    return Some(Atom::ErrorDisplay(
                        "Missing superscript or subscript argument".to_owned(),
                    ));
                }
                let braced = self.chars.get(self.index) == Some(&'{');
                if self.chars.get(self.index) == Some(&'\\') {
                    let start = self.index;
                    self.index += 1;
                    let name = self.capture_control_sequence();
                    self.index = start;
                    // script_argument.go: SetSize and SetStyle push stack items
                    // that a pending SubsupItem cannot accept without a group.
                    if self.paired_definition(&name).is_none()
                        && (matches!(name.as_str(), "dots" | "not" | "nonscript")
                            && !self.ams.borrow().macros.contains_key(&name)
                            || styles::size_declaration(&name).is_some()
                            || matches!(
                                name.as_str(),
                                "displaystyle" | "textstyle" | "scriptstyle" | "scriptscriptstyle"
                            ))
                    {
                        return Some(Atom::ErrorDisplay(format!(
                            "Missing open brace for {}script",
                            if marker == '_' { "sub" } else { "super" }
                        )));
                    }
                }
                let script = if self.chars.get(self.index) == Some(&'}') {
                    return Some(Atom::ErrorDisplay(
                        "Extra close brace or missing open brace".into(),
                    ));
                } else if self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| matches!(c, '^' | '_' | '&' | '\'' | '\u{2019}'))
                {
                    // script_argument.go: another pending stack item cannot fill
                    // SubsupItem. Check occupied slots before its missing-open error.
                    let next = self.chars[self.index];
                    let duplicate =
                        (next == '_' && sub.is_some()) || (next == '^' && sup.is_some());
                    return Some(Atom::ErrorDisplay(if duplicate {
                        if next == '_' {
                            "Double subscripts: use braces to clarify"
                        } else {
                            "Double exponent: use braces to clarify"
                        }
                        .into()
                    } else {
                        format!(
                            "Missing open brace for {}script",
                            if marker == '_' { "sub" } else { "super" }
                        )
                    }));
                } else {
                    self.atom_or_group()?
                };
                if self.command_position.take().is_some() {
                    return Some(Atom::ErrorDisplay(format!(
                        "Missing open brace for {}script",
                        if marker == '_' { "sub" } else { "super" }
                    )));
                }
                // Source parseScriptArgument retains a braced ORD TeXAtom,
                // unlike the ordinary parsed arguments of stackrel.
                let script = if braced {
                    Atom::Ord(Box::new(script))
                } else {
                    script
                };
                // Empty PushAll leaves SubsupItem pending, including the
                // no-node limits commands dispatched through commandEvent.
                if !matches!(&script, Atom::Sequence(items) if items.is_empty()) {
                    break script;
                }
            };
            if cd::first_error(&script).is_some() {
                self.index = self.chars.len();
                return Some(script);
            }
            if marker == '_' {
                sub = Some(Box::new(script));
            } else {
                sup = Some(Box::new(script));
            }
            if first_only {
                break;
            }
        }
        if sub.is_some() || sup.is_some() {
            if let Atom::Multiscripts {
                post,
                prescript: true,
                ..
            } = &mut atom
            {
                let [empty_sub, empty_sup] = multiscripts::empty_pair();
                *post = Some([sub.unwrap_or(empty_sub), sup.unwrap_or(empty_sup)]);
                return Some(atom);
            }
            atom = Atom::Scripts {
                eager: true,
                leading_space: 0.,
                accent,
                limits,
                base: Box::new(atom),
                sub,
                sup,
            };
        }
        Some(atom)
    }

    // Pinned base_ams_handlers.go amsOperatorName owns a genuine child
    // parser with normal font and the operator-letter identifier pattern.
    fn operator_name(&mut self) -> Option<Atom> {
        while self.chars.get(self.index).is_some_and(Self::prime_space) {
            self.index += 1;
        }
        let star = self.chars.get(self.index) == Some(&'*');
        if star {
            self.index += 1;
        }
        let raw = match self.padded_raw_argument("operatorname") {
            Ok(s) => s,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        // amsOperatorName reparses once in a copied normal-font environment.
        // Preserve the complete row so kerns, scripts and pending items keep
        // their actual recipient, instead of parsing each character separately.
        let terminal_space = raw.ends_with(' ');
        let mut raw = raw.trim_matches(|c| Self::prime_space(&c)).to_owned();
        if raw.ends_with('\\') && terminal_space {
            raw.push(' ');
        }
        let mut parser = self.padded_child(&raw);
        parser.active_font = Some("mathrm");
        parser.normal_font = false;
        parser.calligraphic_font = false;
        parser.multi_letter_font = false;
        parser.operator_letters = true;
        parser.operator_pattern = true;
        parser.vector_alias = false;
        let macro_count = std::mem::take(&mut self.ams.borrow_mut().paired_macro_count);
        let result = parser.sequence(None);
        self.ams.borrow_mut().paired_macro_count = macro_count;
        let result = result?;
        if cd::first_error(&result).is_some() {
            return Some(result);
        }
        let mut children = match result {
            Atom::Sequence(items) => items,
            atom => vec![atom],
        };
        // AMS writes movablelimits as a property, not an attribute. The
        // SVG script core reads attributes, so a newly owned core has no
        // movable-limits attribute; a wrapped child keeps its own attribute.
        let own_core =
            children.len() != 1 || children.first().is_some_and(operator_name_identifier);
        // amsOperatorName retains mi identity; every other result gets an OP TeXAtom.
        let body = if children.len() == 1 && children.first().is_some_and(operator_name_identifier)
        {
            children.pop().unwrap()
        } else {
            Atom::Class {
                class: 1,
                body: Box::new(Atom::Sequence(children)),
            }
        };
        let body = Atom::Operator(Box::new(body));
        if !star {
            while self.chars.get(self.index).is_some_and(Self::prime_space) {
                self.index += 1;
            }
            let rest: String = self.chars[self.index..].iter().take(7).collect();
            if rest == r"\limits"
                && self
                    .chars
                    .get(self.index + 7)
                    .is_none_or(|c| !c.is_ascii_alphabetic())
            {
                self.index += 7;
            }
        }
        Some(Atom::ScriptPolicy {
            body: Box::new(body),
            moves: star,
            movable: own_core.then_some(false),
            movable_property: true,
        })
    }

    fn atom_or_group(&mut self) -> Option<Atom> {
        let auto = self.physics_auto.take();
        let result = self.script_atom_or_group();
        self.physics_auto = auto;
        result
    }

    fn script_atom_or_group(&mut self) -> Option<Atom> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        if self.chars.get(self.index) == Some(&'{') {
            self.index += 1;
            self.sequence(Some('}'))
        } else {
            let previous = self.multi_letter_font;
            self.multi_letter_font = false;
            let result = self.atom();
            self.multi_letter_font = previous;
            result
        }
    }

    // Original tex/source_runtime.go lookupMJSourceSymbol for the
    // CharacterMap/DelimiterMap entries the native command handlers above do
    // not own. Entries whose attributes select an untranslated path return
    // None (explicit refusal) rather than an approximated token.
    fn source_symbol(&self, name: &str) -> Option<Atom> {
        use source_symbols::Parser as P;
        let symbol = command_symbol(name)?;
        let text = symbol.text?;
        let mut chars = text.chars();
        let character = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let code = character as u32;
        let attributes: Vec<(&str, &str)> = symbol.attributes.to_vec();
        match symbol.parser {
            P::Mathchar0mi | P::Mathchar7 => {
                if attributes.as_slice() == [("variantForm", "true")] {
                    if self.normal_font || self.calligraphic_font {
                        return None;
                    }
                    return Some(Atom::Identifier(vec![Atom::VariantGlyph(code)]));
                }
                let normal = match attributes.as_slice() {
                    [] => symbol.parser == P::Mathchar7,
                    [("mathvariant", "normal")] => true,
                    [("mathvariant", "italic")] => false,
                    _ => return None,
                };
                let glyph = if self.normal_font {
                    Atom::Glyph(code)
                } else if self.calligraphic_font {
                    Atom::CalligraphicGlyph(code)
                } else if normal {
                    Atom::Glyph(code)
                } else {
                    automatic_italic_atom(character)
                };
                let token = Atom::Identifier(vec![glyph]);
                Some(if symbol.parser == P::Mathchar7 {
                    self.ambient_font(token)
                } else {
                    self.nonambient_font(token)
                })
            }
            // Other registered CharacterMap parsers also create mo tokens;
            // command_symbol has already excluded the parserless remap maps.
            P::Mathchar0mo | P::Delimiter | P::Other => {
                let (mut big, mut texclass, mut variant) = (false, None::<i8>, false);
                match attributes.as_slice() {
                    [] | [("largeop", "false")] | [("accent", "false")] => {}
                    [("texClass", "1"), ("movesupsub", "true")]
                    | [
                        ("texClass", "1"),
                        ("movesupsub", "true"),
                        ("movablelimits", "true"),
                    ] => big = true,
                    // Integral-like OP entries do not move their scripts.
                    [("texClass", "1")] => {
                        return (is_large_operator(code)
                            && !self.normal_font
                            && !self.calligraphic_font)
                            .then(|| self.nonambient_font(Atom::Glyph(code)));
                    }
                    [("texClass", "6")] => texclass = Some(6),
                    [("variantForm", "true")] => variant = true,
                    [("mathvariant", "bold")] if symbol.parser == P::Mathchar0mo => {
                        // Physics vector mo: the bold variant of the character.
                        let mut body = Atom::Glyph(code);
                        declaration_fonts::apply_wrapper(&mut body, "mathbf");
                        if !matches!(body, Atom::Glyph(_) | Atom::FontGlyph { .. })
                            || self.normal_font
                            || self.calligraphic_font
                        {
                            return None;
                        }
                        let definition =
                            operator_dictionary::lookup(code, operator_dictionary::Form::Infix);
                        if definition.flag("largeop") {
                            return None;
                        }
                        return Some(Atom::RawOperator {
                            attributes: &[],
                            cleanup: false,
                            texclass: None,
                            code,
                            body: Box::new(body),
                            definition,
                        });
                    }
                    _ => return None,
                }
                if self.normal_font || self.calligraphic_font {
                    return None;
                }
                if big {
                    // Native large-operator path (sum/prod/int); other glyphs
                    // are not yet sized there.
                    return is_large_operator(code).then(|| {
                        Atom::MovesScripts(Box::new(self.nonambient_font(Atom::Glyph(code))))
                    });
                }
                let mut definition =
                    operator_dictionary::lookup(code, operator_dictionary::Form::Infix);
                if definition.flag("largeop") {
                    return None;
                }
                if let Some(class) = texclass {
                    definition.class = class;
                }
                let operator = Atom::RawOperator {
                    attributes: symbol.attributes,
                    cleanup: symbol.parser == P::Mathchar0mo,
                    texclass,
                    code,
                    body: Box::new(self.nonambient_font(if variant {
                        Atom::VariantGlyph(code)
                    } else {
                        Atom::Glyph(code)
                    })),
                    definition,
                };
                // tex/clean_stretchy.go also visits registered command mo
                // tokens. An ORD dictionary entry keeps its TeXAtom owner.
                Some(
                    if symbol.parser == P::Mathchar0mo && definition.class == 0 {
                        Atom::Ord(Box::new(operator))
                    } else {
                        operator
                    },
                )
            }
        }
    }

    fn atom(&mut self) -> Option<Atom> {
        let character = *self.chars.get(self.index)?;
        self.index += 1;
        if character == '{' {
            let previous = self.multi_letter_font;
            self.multi_letter_font = false;
            let array = self.array_cell.take();
            let auto = self.physics_auto.take();
            let group = self.sequence(Some('}'));
            self.physics_auto = auto;
            self.array_cell = array;
            self.multi_letter_font = previous;
            return group.map(|body| Atom::Ord(Box::new(body)));
        }
        if character == '\\' {
            let start = self.index;
            while self.index < self.chars.len() && self.chars[self.index].is_ascii_alphabetic() {
                self.index += 1;
            }
            if self.index == start {
                let paired_name = self
                    .chars
                    .get(self.index)
                    .copied()
                    .unwrap_or(' ')
                    .to_string();
                if let Some(definition) = self.paired_definition(&paired_name) {
                    if self.index < self.chars.len() {
                        self.index += 1;
                    }
                    return self.paired_delimiter(&paired_name, definition);
                }
                let Some(&delimiter) = self.chars.get(self.index) else {
                    return Some(self.internal_text(" ".to_owned()));
                };
                self.index += 1;
                // parser.go readControlSequence's non-dotall fallback.
                let delimiter = match delimiter {
                    '\n' | '\r' | '\u{2028}' | '\u{2029}' => ' ',
                    c => c,
                };
                return Some(Atom::DeclaredStyle {
                    display: false,
                    level: 0,
                    body: Box::new(Atom::Space(match delimiter {
                        '!' => -0.167,
                        ',' => 0.167,
                        ':' => 0.222,
                        ';' => 0.278,
                        // Original notchar map shadows the Spacer entry for this name.
                        '{' | '}' => {
                            return Some(Atom::RawOperator {
                                attributes: &[],
                                cleanup: false,
                                texclass: None,
                                code: delimiter as u32,
                                body: Box::new(Atom::Glyph(delimiter as u32)),
                                definition: operator_dictionary::lookup(
                                    delimiter as u32,
                                    operator_dictionary::Form::Infix,
                                ),
                            });
                        }
                        // tex/braket.go braketBar takes priority over the
                        // Base delimiter map and creates an ORD U+2225 mo.
                        '|' => {
                            return Some(Atom::RawOperator {
                                attributes: &[],
                                cleanup: false,
                                texclass: Some(0),
                                code: 0x2225,
                                body: Box::new(self.nonambient_font(Atom::Glyph(0x2225))),
                                definition: operator_dictionary::lookup(
                                    0x2225,
                                    operator_dictionary::Form::Infix,
                                ),
                            });
                        }
                        '>' => 0.222,
                        '\\' => return self.cell_command("\\"),
                        ' ' => return Some(self.internal_text(" ".to_owned())),
                        _ => {
                            let name = delimiter.to_string();
                            if let Some(atom) = self.source_symbol(&name) {
                                return Some(atom);
                            }
                            if command_registry::recognized(&name)
                                || command_symbol(&name).is_some()
                            {
                                return None;
                            }
                            return Some(Atom::ErrorDisplay(format!(
                                "Undefined control sequence \\{name}"
                            )));
                        }
                    })),
                });
            }
            let name: String = self.chars[start..self.index].iter().collect();
            if let Some(definition) = self.paired_definition(&name) {
                return self.paired_delimiter(&name, definition);
            }
            // Original tex/commands.go declareMathOperator: \cs expands to
            // \operatorname{body} (or \operatorname*) via the parse-state macros.
            if name == "DeclareMathOperator" {
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| c.is_whitespace())
                {
                    self.index += 1;
                }
                let star = self.chars.get(self.index) == Some(&'*');
                if star {
                    self.index += 1;
                }
                let cs = self.padded_raw_argument(&name).ok()?;
                let cs = cs.trim();
                let cs_name = cs.strip_prefix('\\')?;
                let single = cs_name.chars().all(|c| c.is_ascii_alphabetic())
                    || cs_name.chars().count() == 1;
                if cs_name.is_empty() || !single {
                    return None;
                }
                let body = self.padded_raw_argument(&name).ok()?;
                let command = if star {
                    "\\operatorname*"
                } else {
                    "\\operatorname"
                };
                self.ams
                    .borrow_mut()
                    .macros
                    .insert(cs_name.to_owned(), format!("{command}{{{body}}}"));
                self.splice_inferred = true;
                return Some(Atom::Sequence(Vec::new()));
            }
            let user_macro = self.ams.borrow().macros.get(&name).cloned();
            if let Some(expansion) = user_macro {
                if self.chars.get(self.index) == Some(&' ') {
                    self.index += 1;
                }
                self.chars.splice(self.index..self.index, expansion.chars());
                self.splice_inferred = true;
                return Some(Atom::Sequence(Vec::new()));
            }
            if name == "not" {
                return Some(pending_not::fallback());
            }
            if hfill::is_fill(&name) {
                return Some(hfill::unsupported(&name));
            }
            if matches!(name.as_str(), "uproot" | "leftroot") {
                return Some(match self.move_root(&name) {
                    Ok(()) => {
                        self.splice_inferred = true;
                        Atom::Sequence(Vec::new())
                    }
                    Err(error) => Atom::ErrorDisplay(error),
                });
            }
            // Original tex/symbols.go simpleMacros (installed as parse-state
            // macros, so command() expands them before any other handler):
            // the expansion is spliced in front of the unparsed input.
            if let Some(body) = source_macros::body(&name) {
                let expand = (|| -> Result<String, String> {
                    let count = body
                        .as_bytes()
                        .windows(2)
                        .filter(|pair| pair[0] == b'#' && (b'1'..=b'9').contains(&pair[1]))
                        .map(|pair| (pair[1] - b'0') as usize)
                        .max()
                        .unwrap_or(0);
                    let mut args = Vec::new();
                    if let Some(default) = source_macros::optional_default(&name) {
                        while self.chars.get(self.index).is_some_and(Self::prime_space) {
                            self.index += 1;
                        }
                        args.push(if self.chars.get(self.index) == Some(&'[') {
                            self.padded_brackets()?
                        } else {
                            default.into()
                        });
                    }
                    for _ in args.len()..count {
                        args.push(self.padded_raw_argument(&name)?);
                    }
                    let mut expansion = if count == 0 {
                        body.to_owned()
                    } else {
                        paired_delimiters::substitute(body, &args)?
                    };
                    paired_delimiters::add_args(
                        &mut expansion,
                        &self.chars[self.index..].iter().collect::<String>(),
                    )?;
                    Ok(expansion)
                })();
                let expansion = match expand {
                    Ok(expansion) => expansion,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                self.chars = expansion.chars().collect();
                self.index = 0;
                self.ams.borrow_mut().paired_macro_count += 1;
                if self.ams.borrow().paired_macro_count > 1000 {
                    return Some(Atom::ErrorDisplay("MathJax maximum macro substitution count exceeded; is here a recursive macro call?".into()));
                }
                self.splice_inferred = true;
                return Some(Atom::Sequence(Vec::new()));
            }
            if matches!(
                name.as_str(),
                "DeclarePairedDelimiter"
                    | "DeclarePairedDelimiters"
                    | "DeclarePairedDelimiterX"
                    | "DeclarePairedDelimitersX"
                    | "DeclarePairedDelimiterXPP"
                    | "DeclarePairedDelimitersXPP"
            ) {
                return self.declare_paired_delimiter(&name);
            }
            if matches!(
                name.as_str(),
                "newenvironment"
                    | "renewenvironment"
                    | "newcommand"
                    | "renewcommand"
                    | "providecommand"
                    | "def"
                    | "gdef"
                    | "edef"
                    | "xdef"
                    | "let"
            ) {
                return Some(Atom::ErrorDisplay(format!(
                    "Undefined control sequence \\{name}"
                )));
            }
            if matches!(name.as_str(), "cr" | "newline") {
                return self.cell_command(&name);
            }
            if matches!(name.as_str(), "shoveleft" | "shoveright") {
                return Some(Atom::ErrorDisplay(format!(
                    "\\{name} only allowed in multline environment"
                )));
            }
            if name == "dots" {
                let mut nodes = Vec::new();
                pending_dots::Pending::start(self).finish(&mut Vec::new(), &mut nodes);
                self.splice_inferred = true;
                return Some(Atom::Sequence(nodes));
            }
            if name == "sideset" {
                let pre = self.padded_argument(&name)?;
                if cd::first_error(&pre).is_some() {
                    return Some(pre);
                }
                let post = self.padded_argument(&name)?;
                if cd::first_error(&post).is_some() {
                    return Some(post);
                }
                let base = self.padded_argument(&name)?;
                if cd::first_error(&base).is_some() {
                    return Some(base);
                }
                let body = multiscripts::side_set(pre, post, base);
                // Source properties on the OP TeXAtom do not supply the
                // script core's movablelimits attribute (msubsup.go).
                return Some(Atom::ScriptPolicy {
                    body: Box::new(Atom::Class {
                        body: Box::new(body),
                        class: 1,
                    }),
                    moves: true,
                    movable: None,
                    movable_property: false,
                });
            }
            if name == "prescript" {
                let sup = self.prescript_argument("sup")?;
                if cd::first_error(&sup).is_some() {
                    return Some(sup);
                }
                let sub = self.prescript_argument("sub")?;
                if cd::first_error(&sub).is_some() {
                    return Some(sub);
                }
                let base = self.prescript_argument("arg")?;
                if cd::first_error(&base).is_some() {
                    return Some(base);
                }
                if matches!(sub, Atom::EmptyNode("none")) && matches!(sup, Atom::EmptyNode("none"))
                {
                    self.splice_inferred = true;
                    return Some(base);
                }
                return Some(Atom::Multiscripts {
                    base: Box::new(base),
                    sub: Box::new(sub),
                    sup: Box::new(sup),
                    post: None,
                    prescript: true,
                    mathsize: 1.0,
                });
            }
            if name == "mathtoolsset" {
                let raw = match self.padded_raw_argument(&name) {
                    Ok(v) => v,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                let result = self.ams.borrow_mut().set_options(&raw);
                if result.is_ok() {
                    self.splice_inferred = true;
                }
                return Some(match result {
                    Ok(()) => Atom::Sequence(Vec::new()),
                    Err(e) => Atom::ErrorDisplay(e),
                });
            }
            if matches!(
                name.as_str(),
                "centercolon" | "ordinarycolon" | "MTThinColon"
            ) {
                return Some(self.mathtools_colon(name != "ordinarycolon", name == "MTThinColon"));
            }
            let relation = match name.as_str() {
                "coloneqq" => Some((":=", "≔")),
                "Coloneqq" => Some(("::=", "⩴")),
                "coloneq" => Some((":-", "")),
                "Coloneq" => Some(("::-", "")),
                "eqqcolon" => Some(("=:", "≕")),
                "Eqqcolon" => Some(("=::", "")),
                "eqcolon" => Some(("-:", "∹")),
                "Eqcolon" => Some(("-::", "")),
                "colonapprox" => Some((r":\approx", "")),
                "Colonapprox" => Some((r"::\approx", "")),
                "colonsim" => Some((r":\sim", "")),
                "Colonsim" => Some((r"::\sim", "")),
                "dblcolon" => Some((r"::", "∷")),
                _ => None,
            };
            if let Some((expansion, unicode)) = relation {
                if !unicode.is_empty() && self.ams.borrow().option("use-unicode") == "true" {
                    return Some(Atom::Class {
                        class: 3,
                        body: Box::new(Atom::Glyph(unicode.chars().next().unwrap() as u32)),
                    });
                }
                let expansion = expansion
                    .replace(':', "\\MTThinColon")
                    .replace('-', "\\mathrel{-}");
                return parse_unresolved_shared(
                    &format!("\\mathrel{{{expansion}}}"),
                    self.ams.clone(),
                );
            }
            if !command_registry::recognized(&name) {
                return Some(Atom::ErrorDisplay(format!(
                    "Undefined control sequence \\{name}"
                )));
            }
            // Original tex/commands.go spacer fixes scriptlevel at zero.
            let explicit_space = match name.as_str() {
                "quad" => Some(1.0),
                "qquad" => Some(2.0),
                "enspace" | "enskip" => Some(0.5),
                "thinspace" => Some(0.167),
                "negthinspace" => Some(-0.167),
                "negmedspace" => Some(-0.222),
                "negthickspace" => Some(-0.278),
                _ => None,
            };
            if let Some(width) = explicit_space {
                return Some(Atom::DeclaredStyle {
                    display: false,
                    level: 0,
                    body: Box::new(Atom::Space(width)),
                });
            }
            if let Some((display, level)) = styles::declaration(&name) {
                return Some(Atom::DeclaredStyle {
                    body: Box::new(self.sequence(None)?),
                    display,
                    level,
                });
            }
            if let Some(parameters) = xarrow::parameters(&name) {
                return self.xarrow_argument(&name, parameters);
            }
            if name == "mathchoice" {
                let mut branches = Vec::new();
                for _ in 0..4 {
                    let a = self.padded_argument(&name)?;
                    if cd::first_error(&a).is_some() {
                        return Some(a);
                    }
                    // parseArgumentString returns an inferred mrow for a
                    // multi-item branch. It survives MathChoice replacement.
                    let a = match a {
                        Atom::Sequence(items) if items.len() != 1 => Atom::ExplicitRow(items),
                        a => a,
                    };
                    branches.push(Box::new(a));
                }
                return Some(Atom::Choice(branches.try_into().ok()?));
            }
            if name == "mmlToken" {
                return self.mml_token(&name);
            }
            if matches!(name.as_str(), "splitfrac" | "splitdfrac") {
                let numerator = self.padded_argument(&name)?;
                if cd::first_error(&numerator).is_some() {
                    return Some(numerator);
                }
                let denominator = self.padded_argument(&name)?;
                if cd::first_error(&denominator).is_some() {
                    return Some(denominator);
                }
                let unwrap = |a| {
                    if let Atom::Sequence(v) = a {
                        v
                    } else {
                        vec![a]
                    }
                };
                let mut num = unwrap(numerator);
                num.extend([Atom::Identifier(Vec::new()), Atom::Space(1.)]);
                let mut den = vec![Atom::Space(1.), Atom::Identifier(Vec::new())];
                den.extend(unwrap(denominator));
                let style = |v| {
                    Box::new(Atom::DeclaredStyle {
                        body: Box::new(Atom::Sequence(v)),
                        display: false,
                        level: 0,
                    })
                };
                return Some(Atom::DeclaredStyle {
                    body: Box::new(Atom::SplitFraction(style(num), style(den), 1.)),
                    display: name == "splitdfrac",
                    level: 0,
                });
            }
            if matches!(name.as_str(), "binom" | "dbinom" | "tbinom") {
                return self.binomial_command(&name);
            }
            if matches!(name.as_str(), "dfrac" | "tfrac") {
                return self.styled_fraction(&name);
            }
            if matches!(
                name.as_str(),
                "bar"
                    | "hat"
                    | "widehat"
                    | "tilde"
                    | "widetilde"
                    | "vec"
                    | "dot"
                    | "ddot"
                    | "dddot"
                    | "ddddot"
                    | "acute"
                    | "grave"
                    | "breve"
                    | "check"
                    | "mathring"
            ) {
                let previous_alias = self.vector_alias;
                self.vector_alias |= matches!(name.as_str(), "hat" | "vec");
                let base = self.padded_argument(&name);
                self.vector_alias = previous_alias;
                let base = base?;
                if cd::first_error(&base).is_some() {
                    return Some(base);
                }
                let code = match name.as_str() {
                    "bar" => 0xaf,
                    "hat" | "widehat" => 0x5e,
                    "tilde" | "widetilde" => 0x7e,
                    "vec" => 0x20d7,
                    "dot" => 0x2d9,
                    "ddot" => 0xa8,
                    "dddot" => 0x20db,
                    "ddddot" => 0x20dc,
                    "acute" => 0xb4,
                    "grave" => 0x60,
                    "breve" => 0x2d8,
                    "check" => 0x2c7,
                    _ => 0x2da,
                };
                let body = if matches!(code, 0x20db | 0x20dc) {
                    Atom::OperatorToken(
                        (0..if code == 0x20db { 3 } else { 4 })
                            .map(|_| Atom::Glyph(0x2e))
                            .collect(),
                    )
                } else {
                    // inheritance.go checkOperatorPrimes precedes the accent
                    // character remap, including command-created grave mo.
                    Atom::Glyph(if code == 0x60 { 0x2035 } else { code })
                };
                let mark = Atom::HorizontalMark {
                    body: Box::new(self.ambient_font(body)),
                    code,
                    stretchy: name.starts_with("wide"),
                    mathaccent: true,
                };
                return Some(Atom::Ord(Box::new(Atom::Scripts {
                    base: Box::new(base),
                    sub: None,
                    sup: Some(Box::new(mark)),
                    limits: true,
                    eager: false,
                    leading_space: 0.,
                    accent: Some(accents::Spec {
                        enabled: true,
                        under: false,
                        mathsize: 1.,
                    }),
                })));
            }
            if matches!(
                name.as_str(),
                "overline"
                    | "underline"
                    | "overbrace"
                    | "underbrace"
                    | "overparen"
                    | "underparen"
                    | "overrightarrow"
                    | "overleftarrow"
                    | "underrightarrow"
                    | "underleftarrow"
                    | "overleftrightarrow"
                    | "underleftrightarrow"
            ) {
                let mut base = self.padded_argument(&name)?;
                if cd::first_error(&base).is_some() {
                    return Some(base);
                }
                check_movable_limits(&mut base);
                let under = name.starts_with("under");
                let code = match name.as_str() {
                    "overline" | "underline" => 0x2015,
                    "overbrace" => 0x23de,
                    "underbrace" => 0x23df,
                    "overparen" => 0x23dc,
                    "underparen" => 0x23dd,
                    "overleftrightarrow" | "underleftrightarrow" => 0x2194,
                    "overrightarrow" | "underrightarrow" => 0x2192,
                    "overleftarrow" | "underleftarrow" => 0x2190,
                    _ => 0xaf,
                };
                let mark = Box::new(Atom::HorizontalMark {
                    body: Box::new(Atom::Glyph(code)),
                    code,
                    stretchy: name != "bar",
                    mathaccent: code == 0x2192,
                });
                let stack = Atom::Scripts {
                    base: Box::new(base),
                    sub: if under { Some(mark.clone()) } else { None },
                    sup: if under { None } else { Some(mark) },
                    limits: true,
                    eager: false,
                    leading_space: 0.,
                    accent: Some(accents::Spec {
                        enabled: true,
                        under,
                        mathsize: 1.,
                    }),
                };
                return Some(if matches!(name.as_str(), "overbrace" | "underbrace") {
                    // UnderOver(stack=true) writes movesupsub on the OP
                    // TeXAtom, but no movablelimits attribute on its core.
                    Atom::ScriptPolicy {
                        body: Box::new(Atom::Class {
                            class: 1,
                            body: Box::new(stack),
                        }),
                        moves: true,
                        movable: None,
                        movable_property: false,
                    }
                } else {
                    stack
                });
            }
            if name == "stackrel" || name == "stackbin" {
                // Pinned tex/symbols.go expands these before direct handlers.
                let expansion = (|| -> Result<String, String> {
                    let script = self.padded_raw_argument(&name)?;
                    let base = self.padded_raw_argument(&name)?;
                    let class = if name == "stackrel" {
                        "mathrel"
                    } else {
                        "mathbin"
                    };
                    Ok([
                        "\\",
                        class,
                        "{\\mathop{",
                        &base,
                        "}\\limits^{",
                        &script,
                        "}}",
                    ]
                    .concat())
                })();
                return match expansion {
                    Ok(expansion) => {
                        self.chars.splice(self.index..self.index, expansion.chars());
                        self.atom()
                    }
                    Err(error) => Some(Atom::ErrorDisplay(error)),
                };
            }
            if matches!(name.as_str(), "overset" | "underset" | "overunderset") {
                return self.stack_annotation(&name);
            }
            if name == "buildrel" {
                return self.build_relation();
            }
            if name == "adjustlimits" {
                return self.adjust_limits(&name);
            }
            if matches!(name.as_str(), "shoveleft" | "shoveright") {
                return Some(Atom::ErrorDisplay(format!(
                    "\\{name} only allowed in multline environment"
                )));
            }
            if matches!(
                name.as_str(),
                "Aboxed"
                    | "ArrowBetweenLines"
                    | "MTFlushSpaceAbove"
                    | "MTFlushSpaceBelow"
                    | "vdotswithin"
                    | "shortvdotswithin"
            ) {
                return Some(Atom::ErrorDisplay(format!(
                    "\\{name} can only be used in aligment environments"
                )));
            }
            if matches!(name.as_str(), "overbracket" | "underbracket") {
                return self.bracket_command(&name);
            }
            if name == "cfrac" {
                return self.continued_fraction();
            }
            if name == "genfrac" {
                return self.general_fraction_command(&name);
            }
            if name == "frac" {
                let numerator = self.padded_argument(&name)?;
                if cd::first_error(&numerator).is_some() {
                    return Some(numerator);
                }
                let denominator = self.padded_argument(&name)?;
                if cd::first_error(&denominator).is_some() {
                    return Some(denominator);
                }
                return Some(Atom::Fraction(Box::new(numerator), Box::new(denominator)));
            }
            if matches!(name.as_str(), "sqrt" | "root") {
                return self.root_command(&name);
            }
            if matches!(name.as_str(), "minCDarrowwidth" | "minCDarrowheight") {
                let raw = match self.read_dimension(&name) {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                let value = length::dimension(&raw);
                let mut state = self.ams.borrow_mut();
                let minimums = state
                    .cd_minimums
                    .get_or_insert((Dimension::relative(2.75), Dimension::relative(1.75)));
                if name == "minCDarrowwidth" {
                    minimums.0 = value;
                } else {
                    minimums.1 = value;
                }
                // Original amscdCommand returns no nodes.
                self.splice_inferred = true;
                return Some(Atom::Sequence(Vec::new()));
            }
            if matches!(
                name.as_str(),
                "kern" | "mkern" | "hskip" | "mskip" | "hspace" | "mspace"
            ) {
                let raw = match self.read_dimension(&name) {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                return Some(Atom::Kern(length::dimension(&raw)));
            }
            if name == "strut" {
                // commands.go Strut and base_ams_handlers.go CFrac use the
                // same explicit empty mrow and fixed 8.6pt/3pt mpadded box.
                return Some(Atom::FractionStrut(1.));
            }
            if matches!(
                name.as_str(),
                "matrix" | "array" | "pmatrix" | "cases" | "eqalign" | "eqalignno" | "leqalignno"
            ) {
                // matrix_command.go startMatrixBody checks the opening
                // argument before installing its ArrayItem. Preserve that
                // diagnostic even while command-form table layout is outside
                // the translated surface (environment tables are separate).
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| *c == '\u{feff}' || (*c != '\u{85}' && c.is_whitespace()))
                {
                    self.index += 1;
                }
                if self.index == self.chars.len() {
                    return Some(Atom::ErrorDisplay(format!("Missing argument for \\{name}")));
                }
                return None;
            }
            if name == "rule" {
                return self.rule();
            }
            if matches!(name.as_str(), "Rule" | "Space") {
                return self.rule_space(&name);
            }
            if matches!(name.as_str(), "raise" | "lower" | "moveleft" | "moveright") {
                return self.position_command(&name);
            }
            if matches!(
                name.as_str(),
                "evaluated"
                    | "eval"
                    | "outerproduct"
                    | "dyad"
                    | "ketbra"
                    | "op"
                    | "matrixelement"
                    | "matrixel"
                    | "mel"
                    | "expectationvalue"
                    | "expval"
                    | "ev"
                    | "commutator"
                    | "comm"
                    | "anticommutator"
                    | "acomm"
                    | "poissonbracket"
                    | "pb"
                    | "mqty"
                    | "matrixquantity"
                    | "pmqty"
                    | "Pmqty"
                    | "bmqty"
                    | "vmqty"
                    | "smqty"
                    | "smallmatrixquantity"
                    | "spmqty"
                    | "sPmqty"
                    | "sbmqty"
                    | "svmqty"
            ) {
                return self.physics_macro(&name);
            }
            if matches!(
                name.as_str(),
                "gradient"
                    | "grad"
                    | "div"
                    | "divergence"
                    | "curl"
                    | "Res"
                    | "pv"
                    | "principalvalue"
                    | "PV"
                    | "Re"
                    | "Im"
            ) {
                return self.physics_operator_application(&name);
            }
            if name == "vnabla" {
                return self.physics_nabla();
            }
            // Physics ordinary-symbol map wraps the bold mo in one ORD atom;
            // vnabla above is a separate macro with two ORD owners.
            if name == "gradientnabla" {
                return Some(Atom::Ord(Box::new(self.source_symbol(&name)?)));
            }
            // commands.go vectorAccent is a continuing StarMacro. Let the
            // registered hat/vec handler own its accent and caller suffix.
            if matches!(name.as_str(), "vectorunit" | "vu") {
                while self.chars.get(self.index).is_some_and(Self::prime_space) {
                    self.index += 1;
                }
                let star = self.chars.get(self.index) == Some(&'*');
                if star {
                    self.index += 1;
                }
                let raw = match self.padded_raw_argument(&name) {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                let recipe = if star {
                    r"\hat{\vb*{#1}}"
                } else {
                    r"\hat{\vb{#1}}"
                };
                let expansion = paired_delimiters::substitute(recipe, &[raw]);
                let mut expansion = match expansion {
                    Ok(value) => value,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                if let Err(message) = paired_delimiters::add_args(
                    &mut expansion,
                    &self.chars[self.index..].iter().collect::<String>(),
                ) {
                    return Some(Atom::ErrorDisplay(message));
                }
                self.chars = expansion.chars().collect();
                self.index = 0;
                self.ams.borrow_mut().paired_macro_count += 1;
                if self.ams.borrow().paired_macro_count > 1000 {
                    return Some(Atom::ErrorDisplay("MathJax maximum macro substitution count exceeded; is here a recursive macro call?".into()));
                }
                return self.atom();
            }
            if name == "diffd" {
                // tex/commands.go physicsDifferential("d"): TeXAtom(ORD, mi d normal).
                return Some(Atom::Ord(Box::new(Atom::Identifier(vec![Atom::Glyph(
                    0x64,
                )]))));
            }
            if let Some(text) = match name.as_str() {
                "qcc" => Some("c.c."),
                "qif" => Some("if"),
                "qthen" => Some("then"),
                "qelse" => Some("else"),
                "qotherwise" => Some("otherwise"),
                "qunless" => Some("unless"),
                "qgiven" => Some("given"),
                "qusing" => Some("using"),
                "qassume" => Some("assume"),
                "qsince" => Some("since"),
                "qlet" => Some("let"),
                "qfor" => Some("for"),
                "qall" => Some("all"),
                "qeven" => Some("even"),
                "qodd" => Some("odd"),
                "qinteger" => Some("integer"),
                "qand" => Some("and"),
                "qor" => Some("or"),
                "qas" => Some("as"),
                "qin" => Some("in"),
                "qqtext" | "qq" => Some(""),
                _ => None,
            } {
                // commands.go quickQuadText inserts an uncharged program in
                // this parser, retaining late-bound text/quad and pending items.
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| internal_text::space(*c))
                {
                    self.index += 1;
                }
                let star = self.chars.get(self.index) == Some(&'*');
                if star {
                    self.index += 1;
                }
                let mut overrun = false;
                let text = if text.is_empty() {
                    while self
                        .chars
                        .get(self.index)
                        .is_some_and(|c| internal_text::space(*c))
                    {
                        self.index += 1;
                    }
                    overrun = self.index + 1 == self.chars.len()
                        && self.chars.get(self.index) == Some(&'\\');
                    match self.padded_raw_argument(&name) {
                        Ok(raw) => raw,
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    }
                } else {
                    text.to_owned()
                };
                let expansion = format!(
                    "{}\\text{{{text}}}\\quad ",
                    if star { "" } else { "\\quad" }
                );
                self.chars.splice(self.index..self.index, expansion.chars());
                if overrun {
                    self.index += 1;
                }
                self.splice_inferred = true;
                return Some(Atom::Sequence(Vec::new()));
            }
            if matches!(name.as_str(), "vb" | "vectorbold") {
                return self.physics_vector_bold(&name);
            }
            if declaration_fonts::wrapper(&name) {
                let body = self.bounded_font_argument(&name)?;
                return Some(Atom::Ord(Box::new(body)));
            }
            if name == "displaylines" || name == "substack" {
                let raw = match self.padded_raw_argument(&name) {
                    Ok(v) => v,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                let (body, environment, column_spec) = if name == "displaylines" {
                    (raw, "displaylines", String::new())
                } else {
                    (raw, "subarray", "c".to_owned())
                };
                let mut options = None;
                let rows = table::parse_rows_count_options(
                    &body,
                    environment,
                    self.ams.clone(),
                    self.equation_seen.clone(),
                    None,
                    &mut options,
                )?;
                return Some(Atom::Matrix {
                    spacing: None,
                    options,
                    rows,
                    environment: environment.to_owned(),
                    column_spec,
                });
            }
            if name == "label" {
                let raw = match self.padded_raw_argument(&name) {
                    Ok(v) => v,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                let result = self.ams.borrow_mut().label(raw);
                if let Err(e) = result {
                    return Some(Atom::ErrorDisplay(e));
                }
                self.splice_inferred = true;
                return Some(Atom::Sequence(vec![]));
            }
            if matches!(name.as_str(), "ref" | "refeq" | "eqref") {
                let raw = match self.padded_raw_argument(&name) {
                    Ok(v) => v,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                let label = self
                    .ams
                    .borrow()
                    .labels
                    .get(&raw)
                    .cloned()
                    .unwrap_or_else(|| ams::Label {
                        text: "???".into(),
                        id: String::new(),
                    });
                return Some(Atom::Reference {
                    body: Box::new(self.internal_math(
                        &if name == "eqref" {
                            format!("({})", label.text)
                        } else {
                            label.text
                        },
                        None,
                        false,
                    )?),
                    href: format!("#{}", ams::uri(&label.id)),
                });
            }
            if name == "tag" {
                if matches!(
                    self.ams.borrow().current.environment.as_str(),
                    "split" | "aligned"
                ) {
                    return Some(Atom::ErrorDisplay(format!(
                        "\\tag not allowed in {} environment",
                        self.ams.borrow().current.environment
                    )));
                }
                if self
                    .ams
                    .borrow()
                    .current
                    .tag
                    .as_ref()
                    .is_some_and(|t| !t.raw.is_empty())
                {
                    return Some(Atom::ErrorDisplay("Multiple \\tag".into()));
                }
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| c.is_whitespace())
                {
                    self.index += 1;
                }
                let star = self.chars.get(self.index) == Some(&'*');
                if star {
                    self.index += 1;
                }
                let raw = match self.padded_raw_argument(&name) {
                    Ok(v) => v,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                self.ams.borrow_mut().set_tag(raw, star);
                self.splice_inferred = true;
                return Some(Atom::Sequence(vec![]));
            }
            if matches!(name.as_str(), "notag" | "nonumber") {
                self.ams.borrow_mut().notag();
                self.splice_inferred = true;
                return Some(Atom::Sequence(vec![]));
            }
            if name == "fbox" {
                let raw = match self.padded_raw_argument(&name) {
                    Ok(raw) => raw,
                    Err(message) => return Some(Atom::ErrorDisplay(message)),
                };
                return Some(Atom::Boxed(Box::new(
                    self.internal_math(&raw, None, false)?,
                )));
            }
            if name == "framebox" {
                return self.frame_box();
            }
            if name == "begin" {
                let environment = self.braced_text()?;
                return self.environment(&environment);
            }
            if styles::size_declaration(&name).is_some() {
                return Some(Atom::ErrorDisplay(format!(
                    "Undefined control sequence \\{name}"
                )));
            }
            if matches!(
                name.as_str(),
                "phantom"
                    | "hphantom"
                    | "vphantom"
                    | "smash"
                    | "llap"
                    | "rlap"
                    | "clap"
                    | "mathllap"
                    | "mathrlap"
                    | "mathclap"
                    | "crampedllap"
                    | "crampedrlap"
                    | "crampedclap"
            ) {
                return self.padded_command(&name);
            }
            if let Some(class) = [
                "mathord",
                "mathop",
                "mathbin",
                "mathrel",
                "mathopen",
                "mathclose",
                "mathpunct",
                "mathinner",
                "vcenter",
            ]
            .iter()
            .position(|command| *command == name)
            {
                let atom = Atom::Class {
                    body: Box::new(self.padded_argument(&name)?),
                    class,
                };
                // commands.go mathClass returns a pending FnItem only for
                // this command, not for all generated OP TeXAtoms.
                return Some(if class == 1 {
                    Atom::NamedFunction(Box::new(atom))
                } else {
                    atom
                });
            }
            if name == "ce" || name == "pu" {
                // parseOneToken calls the command in the owning parser. The
                // inserted text belongs to its subsequent row, while this
                // one-token argument receives an empty inferred row.
                return Some(match self.insert_chemistry(&name) {
                    Ok(()) => Atom::Sequence(Vec::new()),
                    Err(message) => Atom::ErrorDisplay(message),
                });
            }
            if name == "celsius" || name == "perthousand" {
                return Some(Atom::Unit(if name == "celsius" { 0x2103 } else { 0x2030 }));
            }
            if matches!(
                name.as_str(),
                "text"
                    | "textnormal"
                    | "textrm"
                    | "textup"
                    | "textbf"
                    | "textit"
                    | "texttt"
                    | "textsf"
                    | "mbox"
                    | "hbox"
            ) {
                return self.hbox_command(&name);
            }
            if matches!(
                name.as_str(),
                "bra"
                    | "ket"
                    | "braket"
                    | "innerproduct"
                    | "ip"
                    | "Bra"
                    | "Ket"
                    | "Braket"
                    | "Set"
                    | "set"
            ) {
                return self.native_braket(&name);
            }
            if matches!(
                name.as_str(),
                "mathbb" | "mathbf" | "mathfrak" | "mathit" | "mathclap" | "substack"
            ) {
                return self.atom_or_group();
            }
            if name == "operatorname" {
                return self.operator_name();
            }
            if name == "colon" {
                // Base mathchar0mo: mo ":" with an explicit texClass PUNCT.
                let mut definition =
                    operator_dictionary::lookup(0x3A, operator_dictionary::Form::Infix);
                definition.class = 6;
                return Some(Atom::RawOperator {
                    attributes: &[],
                    cleanup: false,
                    texclass: Some(6),
                    code: 0x3A,
                    body: Box::new(Atom::Glyph(0x3A)),
                    definition,
                });
            }
            if matches!(name.as_str(), "limits" | "nolimits") {
                return Some(Atom::Sequence(Vec::new()));
            }
            if name == "textcolor" {
                let color = resolve_named_color(self.braced_text()?);
                if let Some(cell) = &mut self.array_cell {
                    cell.rule_allowed = false;
                }
                let previous_color = self.active_color.replace(color.clone());
                let body = self.atom_or_group();
                self.active_color = previous_color;
                return Some(Atom::Color {
                    body: Box::new(body?),
                    color,
                });
            }
            if name == "color" {
                let color = resolve_named_color(self.braced_text()?);
                if let Some(cell) = &mut self.array_cell {
                    cell.rule_allowed = false;
                }
                return Some(Atom::Color {
                    body: Box::new(Atom::Sequence(Vec::new())),
                    color,
                });
            }
            if name == "colorbox" {
                let background = match self.padded_raw_argument(&name) {
                    Ok(raw) => resolve_named_color(raw),
                    Err(error) => return Some(Atom::ErrorDisplay(error)),
                };
                let text = match self.padded_raw_argument(&name) {
                    Ok(raw) => raw,
                    Err(error) => return Some(Atom::ErrorDisplay(error)),
                };
                return Some(Atom::ColorBox {
                    // commands.go colorBox uses internalMath, whose literal
                    // text retains tildes and inherits the creating font.
                    body: Box::new(self.internal_math(&text, None, false)?),
                    background,
                    mathsize: 1.,
                });
            }
            if matches!(name.as_str(), "cancel" | "bcancel" | "xcancel" | "cancelto") {
                return self.enclose_command(&name);
            }
            if matches!(
                name.as_str(),
                "qty"
                    | "quantity"
                    | "pqty"
                    | "bqty"
                    | "Bqty"
                    | "vqty"
                    | "abs"
                    | "absolutevalue"
                    | "norm"
                    | "order"
            ) {
                return self.physics_quantity(&name);
            }
            if matches!(
                name.as_str(),
                "dv" | "derivative"
                    | "pdv"
                    | "partialderivative"
                    | "pderivative"
                    | "fdv"
                    | "fderivative"
                    | "functionalderivative"
                    | "dd"
                    | "differential"
                    | "var"
                    | "variation"
            ) {
                return self.physics_derivative(&name);
            }
            if matches!(
                name.as_str(),
                "vert" | "lvert" | "rvert" | "Vert" | "lVert" | "rVert"
            ) {
                let code = if name.contains("Vert") { 0x2016 } else { 0x7c };
                // The source DelimiterMap entry's explicit texClass (e.g. \vert
                // ORD, \lvert OPEN) survives operator inheritance.
                let texclass = source_symbols::lookup(&name).and_then(|symbol| {
                    symbol
                        .attributes
                        .iter()
                        .find(|(key, _)| *key == "texClass")
                        .and_then(|(_, value)| value.parse::<i8>().ok())
                });
                let mut definition =
                    operator_dictionary::lookup(code, operator_dictionary::Form::Infix);
                if let Some(class) = texclass {
                    definition.class = class;
                }
                return Some(Atom::RawOperator {
                    attributes: &[],
                    cleanup: false,
                    texclass,
                    code,
                    body: Box::new(Atom::Glyph(code)),
                    definition,
                });
            }
            if physics::is_function(&name) {
                return self.physics_expression(&name);
            }
            if name == "left" {
                return self.left_right();
            }
            if name == "right" {
                return Some(Atom::ErrorDisplay(
                    self.native_delimiter("right", false)
                        .err()
                        .unwrap_or_else(|| "Missing \\left or extra \\right".into()),
                ));
            }
            if name == "middle" {
                return self.middle();
            }
            if let Some(size) = braket::fixed_size(&name) {
                let code = match self.native_delimiter(&name, true) {
                    Ok(c) => c,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                };
                let class = if name.ends_with('l') {
                    4
                } else if name.ends_with('r') {
                    5
                } else if name.ends_with('m') {
                    3
                } else {
                    0
                };
                return Some(Atom::Class {
                    body: Box::new(Atom::FixedDelimiter { code, size }),
                    class,
                });
            }
            if matches!(name.as_str(), "degree" | "micro" | "ohm") {
                return command_codepoint(&name).map(Atom::Unit);
            }
            if matches!(name.as_str(), "sum" | "prod") {
                return Some(Atom::MovesScripts(Box::new(
                    self.nonambient_font(Atom::Glyph(command_codepoint(&name)?)),
                )));
            }
            if matches!(name.as_str(), "space" | "nobreakspace") {
                return Some(Atom::Text("\u{00a0}".to_owned()));
            }
            if name == "rightleftharpoons" {
                return Some(Atom::VariantGlyph(0x21CC));
            }
            if name == "hbar" {
                return Some(Atom::VariantGlyph(0x210F));
            }
            if name == "prime" {
                return Some(Atom::VariantGlyph(0x2032));
            }
            if name == "backprime" {
                return Some(Atom::VariantGlyph(0x2035));
            }
            if let Some((code, normal)) = source_identifier(&name) {
                let glyph = if self.normal_font {
                    Atom::Glyph(code)
                } else if self.calligraphic_font {
                    Atom::CalligraphicGlyph(code)
                } else if normal {
                    Atom::Glyph(code)
                } else {
                    automatic_italic_atom(char::from_u32(code).unwrap())
                };
                // These original map entries create mi tokens, including
                // non-letter identifiers such as partial and nabla.
                let token = Atom::Identifier(vec![glyph]);
                return Some(
                    if source_symbols::lookup(&name)
                        .is_some_and(|s| s.parser == source_symbols::Parser::Mathchar7)
                    {
                        self.ambient_font(token)
                    } else {
                        self.nonambient_font(token)
                    },
                );
            }
            if let Some(codepoint) = command_codepoint(&name) {
                if command_symbol(&name)
                    .is_some_and(|s| s.attributes.contains(&("accent", "false")))
                {
                    return self.source_symbol(&name);
                }
                if source_symbols::lookup(&name)
                    .is_some_and(|s| s.parser == source_symbols::Parser::Mathchar7)
                {
                    return self.source_symbol(&name);
                }
                let token = self.nonambient_font(Atom::Glyph(
                    if self.normal_font && (0x1D6FC..=0x1D714).contains(&codepoint) {
                        match name.as_str() {
                            // Base character map retains these variant Greek identities.
                            "epsilon" => 0x3F5,
                            "phi" => 0x3D5,
                            _ => 0x3B1 + codepoint - 0x1D6FC,
                        }
                    } else {
                        codepoint
                    },
                ));
                return Some(
                    if source_symbols::lookup(&name).is_some_and(|s| {
                        s.parser == source_symbols::Parser::Mathchar0mo
                            && operator_dictionary::lookup(
                                codepoint,
                                operator_dictionary::Form::Infix,
                            )
                            .class
                                == 0
                    }) {
                        Atom::Ord(Box::new(Atom::RawOperator {
                            attributes: source_symbols::lookup(&name)?.attributes,
                            cleanup: true,
                            code: codepoint,
                            body: Box::new(token),
                            definition: operator_dictionary::lookup(
                                codepoint,
                                operator_dictionary::Form::Infix,
                            ),
                            texclass: None,
                        }))
                    } else {
                        token
                    },
                );
            }
            if let Some(text) = physics::function_name(&name) {
                let moving = matches!(
                    name.as_str(),
                    "lim" | "min" | "max" | "sup" | "inf" | "gcd" | "liminf" | "limsup"
                );
                let token = text.chars().map(|c| Atom::Glyph(c as u32)).collect();
                let body = if moving {
                    Atom::OperatorToken(token)
                } else {
                    Atom::Identifier(token)
                };
                let operator = Atom::Operator(Box::new(self.nonambient_font(body)));
                // Original commands.go: non-limit function names set
                // commandNamedFunction, so parseRow's pending FnItem may add
                // an ApplyFunction before the next item.
                return Some(if moving {
                    Atom::MovesScripts(Box::new(operator))
                } else {
                    Atom::NamedFunction(Box::new(operator))
                });
            }
            if let Some(atom) = self.source_symbol(&name) {
                return Some(atom);
            }
            // A valid source command with an untranslated native handler stays
            // on the visible implementation fallback, never an undefined error.
            return None;
        }
        if character == '}' {
            return None;
        }
        if character == '~' {
            return Some(Atom::Text("\u{a0}".to_owned()));
        }
        // parser.go otherNode consults the option only for math tokens;
        // InternalText retains literal colons and subsequent option changes.
        if character == ':' {
            return Some(
                self.mathtools_colon(self.ams.borrow().option("centercolon") == "true", false),
            );
        }
        if self.operator_letters
            && (character.is_ascii_alphabetic() || matches!(character, '-' | '*'))
        {
            let mut letters = vec![character];
            if self.operator_pattern {
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '-' | '*'))
                {
                    letters.push(self.chars[self.index]);
                    self.index += 1;
                }
            } else if matches!(character, '-' | '*') && self.multi_letter_font {
                return Some(Atom::SourceError(format!(
                    "identifier pattern does not match {:?}",
                    character.to_string()
                )));
            } else if self.multi_letter_font {
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    letters.push(self.chars[self.index]);
                    self.index += 1;
                }
            }
            let token = self.ambient_font(Atom::Identifier(
                letters
                    .into_iter()
                    .map(|c| {
                        if self.active_font == Some("") {
                            automatic_italic_atom(c)
                        } else {
                            Atom::Glyph(c as u32)
                        }
                    })
                    .collect(),
            ));
            return Some(
                if self.operator_pattern
                    && !self.no_auto_operator
                    && matches!(&token, Atom::Identifier(v) if automatic_operator(v))
                {
                    Atom::AutoOperator(Box::new(token))
                } else {
                    token
                },
            );
        }
        // parser.go ordinaryNumberPattern: literal commas are separate mo
        // tokens; only braced thousands separators belong to one number.
        if character.is_ascii_digit() || matches!(character, '.' | ',') {
            static NUMBER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let pattern = NUMBER.get_or_init(|| {
                regex::Regex::new(r"^(?:[0-9]+(?:\{,\}[0-9]{3})*(?:\.[0-9]*)?|\.[0-9]+)").unwrap()
            });
            let rest: String = self.chars[self.index - 1..].iter().collect();
            if let Some(number) = pattern.find(&rest) {
                self.index += number.as_str().chars().count() - 1;
                return Some(
                    self.ambient_font(Atom::Number(
                        number
                            .as_str()
                            .chars()
                            .filter(|c| !matches!(c, '{' | '}'))
                            .map(|c| Atom::Glyph(c as u32))
                            .collect(),
                    )),
                );
            }
            // The digit scanner does not register fixStretchy on unmatched
            // dots/commas, unlike BaseConfiguration.Other.
            return Some(self.ambient_font(Atom::RawOperator {
                attributes: &[],
                cleanup: false,
                texclass: None,
                code: character as u32,
                body: Box::new(Atom::Glyph(character as u32)),
                definition: operator_dictionary::lookup(
                    character as u32,
                    operator_dictionary::Form::Infix,
                ),
            }));
        }
        if self.multi_letter_font && character.is_ascii_alphabetic() {
            let mut letters = vec![character];
            while self
                .chars
                .get(self.index)
                .is_some_and(|c| c.is_ascii_alphabetic())
            {
                letters.push(self.chars[self.index]);
                self.index += 1;
            }
            return Some(
                self.ambient_font(Atom::Identifier(
                    letters
                        .into_iter()
                        .map(|c| {
                            if self.calligraphic_font {
                                Atom::CalligraphicGlyph(c as u32)
                            } else {
                                Atom::Glyph(c as u32)
                            }
                        })
                        .collect(),
                )),
            );
        }
        // Source applyMathVariant changes the variant on the original token,
        // never rewrites a physical SMP/Planck character into an ASCII letter.
        if matches!(character as u32, 0x1D400..=0x1D7FF | 0x210E) {
            let code = character as u32;
            // Font SMP remapping owns known physical glyphs. A hole in that
            // table remains a one-character mi with the default italic variant.
            let glyph = if uses_normal_text_fallback(code) {
                self.ambient_font(Atom::ItalicGlyph(code))
            } else {
                self.nonambient_font(Atom::LiteralGlyph(code))
            };
            return Some(Atom::Identifier(vec![glyph]));
        }
        let range = operator_dictionary::range(character as u32);
        // parser.go parseCharacterChecked: Other without a pinned Unicode
        // range is a conversion error, rather than a rendered TeX diagnostic.
        if range.is_none() && !number_unicode::is_digit(character) {
            return Some(Atom::SourceError(format!(
                "no Unicode range for character U+{:04X}",
                character as u32
            )));
        }
        let identifier = character.is_ascii_alphabetic() || range.is_some_and(|r| r.kind == "mi");
        if identifier && range.is_some_and(|r| r.variant == Some("normal")) {
            // Other's authored range variant overrides the ambient font.
            return Some(
                self.nonambient_font(Atom::Identifier(vec![Atom::Glyph(character as u32)])),
            );
        }
        if !identifier {
            let code = match character {
                '-' => 0x2212,
                '*' => 0x2217,
                '`' => 0x2018,
                _ => character as u32,
            };
            // tex/inheritance.go moPrimeRemap changes the rendered text,
            // retaining the authored operator for dictionary classification.
            let paint = match code {
                0x22 | 0x201D | 0x201E => 0x2033,
                0x27 | 0x2019 | 0x201A => 0x2032,
                0x60 | 0x2018 | 0x201B => 0x2035,
                0x201C | 0x201F => 0x2036,
                _ => code,
            };
            let body = if matches!(character, ')' | ']' | '|') {
                // parser.go Physics AutoClose creates its own non-stretchy
                // mo, without Other's ambient-font eligibility.
                self.nonambient_font(Atom::Glyph(paint))
            } else if self.calligraphic_font {
                Atom::CalligraphicGlyph(paint)
            } else {
                self.ambient_font(Atom::Glyph(paint))
            };
            let definition = operator_dictionary::lookup(code, operator_dictionary::Form::Infix);
            let operator = Atom::RawOperator {
                attributes: &[],
                cleanup: !matches!(character, ')' | ']' | '|'),
                texclass: None,
                code,
                body: Box::new(body),
                definition,
            };
            // tex/clean_stretchy.go wraps registered raw ORD operators in a
            // TeXAtom after inheritance; command-produced tokens stay distinct.
            return Some(
                if definition.class == 0 && !matches!(character, ')' | ']' | '|') {
                    Atom::Ord(Box::new(operator))
                } else {
                    operator
                },
            );
        }
        if self.calligraphic_font {
            return Some(Atom::CalligraphicGlyph(character as u32));
        }
        // Original base remap table converts a math hyphen to U+2212 even
        // under a normal-font TeXAtom; literal text remains a hyphen.
        if !self.normal_font && identifier {
            return Some(
                self.ambient_font(Atom::Identifier(vec![automatic_italic_atom(character)])),
            );
        }
        Some(Atom::Glyph(if character == '-' {
            0x2212
        } else {
            character as u32
        }))
    }

    fn braced_text(&mut self) -> Option<String> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        if self.chars.get(self.index) != Some(&'{') {
            return None;
        }
        self.index += 1;
        let start = self.index;
        let mut depth = 1;
        while self.index < self.chars.len() {
            match self.chars[self.index] {
                '\\' => {
                    self.index += 1;
                }
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let text = self.chars[start..self.index].iter().collect();
                        self.index += 1;
                        return Some(text);
                    }
                }
                _ => {}
            }
            self.index += 1;
        }
        None
    }

    // tex/dimension.go GetDimen owns its own whitespace and argument scan.
    fn read_dimension(&mut self, name: &str) -> Result<String, String> {
        let missing = || format!("Missing dimension or its units for \\{name}");
        static GRAMMAR: std::sync::OnceLock<(regex::Regex, regex::Regex)> =
            std::sync::OnceLock::new();
        let (full, prefix) = GRAMMAR.get_or_init(|| {
            let space = r"[\t-\r \x{00a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";
            let value = format!(r"{space}*([-+]?(?:[.,][0-9]+|[0-9]+(?:[.,][0-9]*)?)){space}*(pt|em|ex|mu|px|mm|cm|in|pc)");
            (regex::Regex::new(&format!(r"^{value}{space}*$")).unwrap(),
             regex::Regex::new(&format!(r"^{value} ?")).unwrap())
        });
        while self.chars.get(self.index).is_some_and(|c| {
            matches!(
                c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' |
                '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
            )
        }) {
            self.index += 1;
        }
        let braced = self.chars.get(self.index) == Some(&'{');
        let raw = if braced {
            self.braced_text()
                .ok_or_else(|| "Missing close brace".to_owned())?
        } else {
            self.chars[self.index..].iter().collect()
        };
        let captures = if braced { full } else { prefix }
            .captures(&raw)
            .ok_or_else(missing)?;
        if !braced {
            self.index += captures[0].chars().count();
        }
        let numeric = captures[1].replace(',', ".");
        if &captures[2] == "mu" {
            let em = numeric.parse::<f64>().unwrap_or(0.0) / 18.0;
            if em.abs() < 0.0006 {
                return Ok("0em".into());
            }
            return Ok(format!(
                "{}em",
                number(em.signum() * alignment::round_thousandth(em.abs()))
            ));
        }
        Ok(format!("{numeric}{}", &captures[2]))
    }

    fn mathtools_colon(&self, center: bool, thin: bool) -> Atom {
        let colon = Atom::RawOperator {
            attributes: &[],
            cleanup: false,
            texclass: None,
            code: 0x3a,
            body: Box::new(Atom::Glyph(0x3a)),
            definition: operator_dictionary::lookup(0x3a, operator_dictionary::Form::Infix),
        };
        if !center {
            return colon;
        }
        let dy = length::dimension(&self.ams.borrow().option("centercolon-offset")).em(0.0, 1.0);
        let mut padding = padded::Padding {
            height_add: dy,
            depth_add: -dy,
            voffset_em: dy,
            ..Default::default()
        };
        if thin {
            let width = self.ams.borrow().option("thincolon-dw");
            let value = length::dimension(&width).em(0.0, 1.0);
            if width.starts_with(['+', '-']) {
                padding.width_add = value;
            } else {
                padding.width_absolute = Some(value);
            }
            padding.lspace_em =
                length::dimension(&self.ams.borrow().option("thincolon-dx")).em(0.0, 1.0);
        }
        Atom::Padded {
            body: Box::new(colon),
            padding,
        }
    }

    fn environment(&mut self, environment: &str) -> Option<Atom> {
        let in_root = self.in_root;
        if root_index::array_environment(environment) {
            self.in_root = false;
        }
        let result = self.environment_inner(environment);
        self.in_root = in_root;
        result
    }
    fn environment_inner(&mut self, environment: &str) -> Option<Atom> {
        // BeginEnd dispatches the literal name and rejects it before reading
        // a body (pinned tex/environment_count.go and environments.go).
        if environment.contains('\\') {
            return Some(Atom::ErrorDisplay(format!(
                "Invalid environment name '{environment}'"
            )));
        }
        if !known_environment(environment) {
            return Some(Atom::ErrorDisplay(format!(
                "Unknown environment '{environment}'"
            )));
        }
        if environment == "spreadlines" {
            let spread = match self.read_dimension("begin") {
                Ok(v) => v,
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            let body = match self.capture_environment(environment) {
                Ok(v) => v,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            let mut atom = parse_with_root_stack(
                &body,
                self.ams.clone(),
                self.equation_seen.clone(),
                self.in_root,
                self.root_moves.clone(),
            )?;
            table::spread(&mut atom, length::dimension(&spread).em(0.0, 1.0));
            self.splice_inferred = true;
            return Some(atom);
        }
        if matches!(environment, "numcases" | "subnumcases") {
            if let Err(error) = self.padded_raw_argument(&format!("begin{{{environment}}}")) {
                return Some(Atom::ErrorDisplay(error));
            }
            let body = match self.capture_environment(environment) {
                Ok(v) => v,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            for cells in table::raw_rows(&body) {
                let atom = parse_unresolved_shared(cells[0].trim(), self.ams.clone())?;
                if cd::first_error(&atom).is_some() {
                    return Some(atom);
                }
                if cells.len() > 2 {
                    return Some(Atom::ErrorDisplay(
                        "Extra alignment tab in text for numcase environment".into(),
                    ));
                }
            }
            // Cases emits this dependency after row parsing, but D2 does not
            // activate EmpheqConfiguration in its default compiler.
            return Some(Atom::ErrorDisplay(
                "Undefined control sequence \\empheqlbrace".into(),
            ));
        }
        let mut multline = matches!(environment, "multline" | "multline*" | "multlined")
            .then(table::MultlineOptions::default);
        if environment == "multlined" {
            let present =
                self.chars[self.index..].iter().find(|c| !c.is_whitespace()) == Some(&'[');
            let first = match self.padded_brackets() {
                Ok(v) => {
                    if !present {
                        self.ams.borrow().option("multlined-pos")
                    } else {
                        v
                    }
                }
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            };
            let second = if !first.is_empty() {
                match self.padded_brackets() {
                    Ok(v) => v,
                    Err(e) => return Some(Atom::ErrorDisplay(e)),
                }
            } else {
                String::new()
            };
            let (position, width) = if present && !"cbt".contains(first.as_str()) {
                (second, first)
            } else {
                (first, second)
            };
            let options = multline.as_mut().unwrap();
            options.position = position;
            options.width = width;
        }
        let column_spec = if matches!(environment, "array" | "subarray" | "crampedsubarray") {
            match self.padded_raw_argument(&format!("begin{{{environment}}}")) {
                Ok(v) => v,
                Err(e) => return Some(Atom::ErrorDisplay(e)),
            }
        } else if matches!(environment, "aligned" | "alignedat") {
            match self.padded_brackets() {
                Ok(value) => match value.trim() {
                    "t" => "baseline 1".into(),
                    "b" => "baseline -1".into(),
                    "c" => "axis".into(),
                    value => value.into(),
                },
                Err(message) => {
                    let message = if message == "Could not find closing ']' for argument" {
                        format!("{message} to \\begin")
                    } else {
                        message
                    };
                    return Some(Atom::ErrorDisplay(message));
                }
            }
        } else if environment.ends_with('*') && environment.contains("matrix") {
            let present =
                self.chars[self.index..].iter().find(|c| !c.is_whitespace()) == Some(&'[');
            match self.padded_brackets() {
                Ok(value) => {
                    if !present && environment.contains("smallmatrix") {
                        self.ams.borrow().option("smallmatrix-align")
                    } else {
                        value
                    }
                }
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            }
        } else {
            String::new()
        };
        // Source beginEnvironment consumes the pair-count argument before
        // captureEnvironment. Its contents are not part of the table cells.
        let mut pair_count = None;
        if environment.contains("alignat") || environment == "alignedat" {
            let count = match self.padded_raw_argument("begin") {
                Ok(count) => count,
                Err(message) => return Some(Atom::ErrorDisplay(message)),
            };
            if !count.chars().all(|c| c.is_ascii_digit()) {
                return Some(Atom::ErrorDisplay(format!(
                    "Argument to \\begin{{{environment}}} must me a positive integer"
                )));
            }
            pair_count = Some(count);
        }
        // parser.go checkEquationEnvironment: Stack.global survives End and
        // row/cell continuations; a real child parser starts a fresh stack.
        if matches!(
            environment,
            "equation"
                | "equation*"
                | "align"
                | "align*"
                | "alignat"
                | "alignat*"
                | "flalign"
                | "flalign*"
                | "xalignat"
                | "xalignat*"
                | "xxalignat"
                | "gather"
                | "gather*"
                | "eqnarray"
                | "eqnarray*"
                | "multline"
                | "multline*"
        ) && self.equation_seen.replace(true)
        {
            return Some(Atom::ErrorDisplay(
                "Erroneous nesting of equation structures".into(),
            ));
        }
        let content = match self.capture_environment(environment) {
            Ok(content) => content,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        if !known_environment(environment) {
            return Some(Atom::ErrorDisplay(format!(
                "Unknown environment '{environment}'"
            )));
        }
        // The starred mathtools cases forms typeset their second column as
        // text (mathtools_handlers_environments.go); that path is not
        // translated, so refuse instead of laying the column out as math.
        if environment.ends_with("cases*") {
            return None;
        }
        if let Some(mut options) = multline {
            let rows = table::parse_multline(
                &content,
                environment,
                self.ams.clone(),
                self.equation_seen.clone(),
                &mut options,
            )?;
            return Some(Atom::Matrix {
                spacing: None,
                options: Some(options),
                rows,
                environment: environment.into(),
                column_spec,
            });
        }
        if environment == "CD" {
            let body = cd::parse_rows(&content, self.ams.clone(), self.equation_seen.clone())?;
            let options = Some(table::MultlineOptions {
                row_spacing: body.spacing,
                align: body.align,
                rules: body.rules,
                ..Default::default()
            });
            return Some(Atom::Matrix {
                spacing: None,
                options,
                rows: body.rows,
                environment: environment.to_owned(),
                column_spec,
            });
        }
        // Only equation/equation* own this AMS tag scope. The legacy
        // math/displaymath handlers are unreachable: neither name is in
        // the pinned default environment maps.
        if matches!(environment, "equation" | "equation*") {
            self.ams.borrow_mut().start("equation");
            let result = parse_with_root_stack(
                &content,
                self.ams.clone(),
                self.equation_seen.clone(),
                self.in_root,
                self.root_moves.clone(),
            );
            let tag = ams::get_tag(&self.ams)?;
            self.ams.borrow_mut().end();
            if let Some(tag) = tag {
                return Some(Atom::TaggedTable {
                    spacing: None,
                    rows: vec![vec![result?]],
                    tags: vec![Some(tag)],
                    environment: "equation-tag".into(),
                    container_align: "center",
                });
            }
            self.splice_inferred = true;
            return result;
        }
        let scoped = matches!(environment, "split" | "align" | "align*" | "aligned");
        if scoped {
            self.ams.borrow_mut().start(environment);
        }
        let mut options = None;
        let rows = table::parse_rows_count_options(
            &content,
            environment,
            self.ams.clone(),
            self.equation_seen.clone(),
            pair_count.as_deref(),
            &mut options,
        );
        let tags = self.ams.borrow().current.row_tags.clone();
        if scoped {
            self.ams.borrow_mut().end();
        }
        if scoped && tags.iter().any(Option::is_some) {
            return Some(Atom::TaggedTable {
                spacing: None,
                rows: rows?,
                tags,
                environment: environment.into(),
                container_align: "center",
            });
        }
        if environment == "alignedat" {
            let options = options.get_or_insert_with(Default::default);
            options.position = column_spec.clone();
            if pair_count
                .as_ref()
                .is_some_and(|count| count.trim_start_matches('0').is_empty())
            {
                options.align = rows
                    .as_ref()?
                    .iter()
                    .map(|r| vec!["center"; r.len()])
                    .collect();
                options.column_gap = Some(1.0);
            }
        }
        Some(Atom::Matrix {
            spacing: None,
            options,
            rows: rows?,
            environment: environment.to_owned(),
            column_spec,
        })
    }

    // Original tex/parser.go readControlSequence/readArgument and
    // environments.go captureEnvironment. Escaped control characters do not
    // start environments; only matching trimmed names affect nesting.
    fn capture_environment(&mut self, environment: &str) -> Result<String, String> {
        let start = self.index;
        let mut depth = 1usize;
        let mut braces = 0usize;
        while self.index < self.chars.len() {
            if matches!(environment, "numcases" | "subnumcases") {
                match self.chars[self.index] {
                    '{' => braces += 1,
                    '}' => braces = braces.saturating_sub(1),
                    '&' if braces == 0 => {
                        // Do not raise a text error while capturing ahead
                        // of a formula that still needs to execute.
                        if let Ok(end) = cases::text_end(&self.chars, self.index + 1) {
                            self.index = end;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            if self.chars[self.index] != '\\' {
                self.index += 1;
                continue;
            }
            let command_start = self.index;
            self.index += 1;
            let name = self.capture_control_sequence();
            if name != "begin" && name != "end" {
                continue;
            }
            while self
                .chars
                .get(self.index)
                .is_some_and(|c| c.is_whitespace())
            {
                self.index += 1;
            }
            let raw = match self.chars.get(self.index) {
                None => return Err(format!("Missing argument for \\{name}")),
                Some('}') => return Err("Extra close brace or missing open brace".to_owned()),
                Some('{') => self
                    .braced_text()
                    .ok_or_else(|| "Missing close brace".to_owned())?,
                Some('\\') => {
                    let argument_start = self.index;
                    self.index += 1;
                    self.capture_control_sequence();
                    self.chars[argument_start..self.index].iter().collect()
                }
                Some(_) => {
                    let value = self.chars[self.index].to_string();
                    self.index += 1;
                    value
                }
            };
            if raw.trim() != environment {
                continue;
            }
            if name == "begin" {
                depth += 1;
            } else {
                depth -= 1;
                if depth == 0 {
                    return Ok(self.chars[start..command_start].iter().collect());
                }
            }
        }
        Err(format!("Missing \\end{{{environment}}}"))
    }

    fn capture_control_sequence(&mut self) -> String {
        let start = self.index;
        if self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            while self
                .chars
                .get(self.index)
                .is_some_and(|c| c.is_ascii_alphabetic())
            {
                self.index += 1;
            }
            let name = self.chars[start..self.index].iter().collect();
            if self.chars.get(self.index) == Some(&' ') {
                self.index += 1;
            }
            name
        } else if let Some(c) = self.chars.get(self.index).copied() {
            self.index += 1;
            c.to_string()
        } else {
            " ".to_owned()
        }
    }
}

// D2's default compiler has no augmented empheq/newcommand environment map.
// Registered names from source_tables_{base,ams,mathtools,amscd_braket_cases,
// physics}_maps.go. Legacy handler switch cases in environments.go do not
// register names (notably math and displaymath).
fn known_environment(name: &str) -> bool {
    matches!(
        name,
        "equation"
            | "equation*"
            | "split"
            | "matrix"
            | "matrix*"
            | "smallmatrix"
            | "smallmatrix*"
            | "array"
            | "subarray"
            | "crampedsubarray"
            | "pmatrix"
            | "pmatrix*"
            | "bmatrix"
            | "bmatrix*"
            | "Bmatrix"
            | "Bmatrix*"
            | "vmatrix"
            | "vmatrix*"
            | "Vmatrix"
            | "Vmatrix*"
            | "psmallmatrix"
            | "psmallmatrix*"
            | "bsmallmatrix"
            | "bsmallmatrix*"
            | "Bsmallmatrix"
            | "Bsmallmatrix*"
            | "vsmallmatrix"
            | "vsmallmatrix*"
            | "Vsmallmatrix"
            | "Vsmallmatrix*"
            | "cases"
            | "dcases"
            | "rcases"
            | "drcases"
            | "cases*"
            | "dcases*"
            | "rcases*"
            | "drcases*"
            | "numcases"
            | "subnumcases"
            | "align"
            | "align*"
            | "eqnarray"
            | "eqnarray*"
            | "alignat"
            | "alignat*"
            | "xalignat"
            | "xalignat*"
            | "xxalignat"
            | "aligned"
            | "alignedat"
            | "gather"
            | "gather*"
            | "gathered"
            | "multline"
            | "multline*"
            | "multlined"
            | "lgathered"
            | "rgathered"
            | "spreadlines"
            | "flalign"
            | "flalign*"
            | "CD"
    )
}

fn layout(atom: &Atom) -> Option<Layout> {
    let mut result = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    };
    result.width = layout_at(atom, 0.0, 0.0, 1.0, &mut result)?;
    if let Some(width) = result.width_em {
        result.width = width * 1000.0;
    }
    retain_negative_glyph_depth(&mut result);
    ams::mark(&mut result, true);
    table::resolve(&mut result);
    Some(result)
}

fn retain_negative_glyph_depth(bounds: &mut Layout) {
    if bounds.rules.is_empty()
        && bounds.groups.iter().all(|g| {
            matches!(
                g.kind,
                "TeXAtom"
                    | "mi"
                    | "mo"
                    | "msup"
                    | "msub"
                    | "msubsup"
                    | "mpadded"
                    | "mphantom"
                    | "mstyle"
                    | "mtext"
                    | "mover"
                    | "munder"
                    | "munderover"
                    | "mtable"
                    | "mspace"
            )
        })
        && (!bounds.glyphs.is_empty() || !bounds.groups.is_empty())
    {
        // Source BBox.Empty starts H/D at -infinity. Preserve signed natural
        // glyph bounds on both sides of the baseline before viewport clamping.
        if bounds.depth == 0.0 {
            bounds.depth = bounds
                .glyphs
                .iter()
                .filter_map(|g| {
                    glyph_for(*g).map(|data| data.depth * 1000.0 * g.scale - g.baseline)
                })
                .chain(
                    bounds
                        .groups
                        .iter()
                        .map(|g| styles::group_natural_bounds(g).1 - g.baseline),
                )
                .fold(f64::NEG_INFINITY, f64::max);
        }
        if bounds.ascent == 0.0 {
            bounds.ascent = bounds
                .glyphs
                .iter()
                .filter_map(|g| {
                    glyph_for(*g).map(|data| data.height * 1000.0 * g.scale + g.baseline)
                })
                .chain(
                    bounds
                        .groups
                        .iter()
                        .map(|g| styles::group_natural_bounds(g).0 + g.baseline),
                )
                .fold(f64::NEG_INFINITY, f64::max);
        }
    }
}

fn layout_at(atom: &Atom, x: f64, baseline: f64, scale: f64, out: &mut Layout) -> Option<f64> {
    layout_at_with_style(atom, x, baseline, scale, out, true, false)
}

fn layout_at_with_style(
    atom: &Atom,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display_style: bool,
    prime_style: bool,
) -> Option<f64> {
    out.intrinsic_width_em = None;
    let width =
        layout_at_with_style_inner(atom, x, baseline, scale, out, display_style, prime_style);
    if matches!(atom, Atom::Sequence(_)) && style_classes::has_declaration(atom)
        || !matches!(
            atom,
            Atom::Glyph(_)
                | Atom::Sequence(_)
                | Atom::Identifier(_)
                | Atom::Number(_)
                | Atom::OperatorToken(_)
                | Atom::StyledText(_)
                | Atom::Scripts { .. }
        )
    {
        out.intrinsic_width_em = None;
    }
    width
}

fn layout_at_with_style_inner(
    atom: &Atom,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display_style: bool,
    prime_style: bool,
) -> Option<f64> {
    match atom {
        Atom::NonscriptSpace { .. } => None, // consumed by filterNonscript
        Atom::FontGlyph {
            original,
            paint,
            bold,
            font_variant,
        } => double_struck::layout(
            *original,
            *paint,
            *bold,
            font_variant,
            x,
            baseline,
            scale,
            out,
            display_style,
        ),
        Atom::FixedDelimiter { code, size } => {
            braket::layout_fixed(*code, *size, x, baseline, scale, out)
        }
        Atom::FenceDelimiter { code, class, color } => {
            let width =
                braket::layout_token(*code, *class, x, baseline, scale, out, display_style)?;
            if let Some(color) = braket::explicit_color(color.as_deref()) {
                out.groups.last_mut()?.color = Some(color);
            }
            Some(width)
        }
        Atom::Fenced {
            open,
            body,
            close,
            close_color,
            stretchy,
            ..
        } => braket::layout(
            *open,
            body,
            *close,
            close_color.as_deref(),
            *stretchy,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::NamedFunction(body) => {
            layout_at_with_style(body, x, baseline, scale, out, display_style, prime_style)
        }
        Atom::ApplyFunction => layout_at_with_style(
            &Atom::OperatorToken(vec![Atom::Glyph(0x2061)]),
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::ExplicitRow(children) => {
            let mut local = general_fraction::blank();
            local.width = layout_at_with_style(
                &Atom::Sequence(children.clone()),
                0.,
                0.,
                scale,
                &mut local,
                display_style,
                prime_style,
            )?;
            retain_negative_glyph_depth(&mut local);
            let width = local.width;
            out.ascent = out.ascent.max(baseline + local.ascent);
            out.depth = out.depth.max(local.depth - baseline);
            out.width_em = local.width_em;
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind: "mrow",
                delimiter: None,
                arrow: None,
                body: Box::new(local),
                lines: Vec::new(),
            });
            Some(width)
        }
        Atom::InferredRow(children) => physics::layout_inferred(
            children,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::SourceError(_) | Atom::ErrorDisplay(_) | Atom::Choice(_) => None,
        Atom::RawOperator {
            body, definition, ..
        } => operator_dictionary::layout(body, *definition, x, baseline, scale, out, display_style),
        Atom::DeclaredStyle {
            body,
            display,
            level,
        } => styles::layout_decl(body, *display, *level, x, baseline, out),
        Atom::DeclaredSize { body, factor } => styles::layout_size(
            body,
            *factor,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::XArrow { code, over, under } => xarrow::layout(
            *code,
            over,
            under.as_deref(),
            x,
            baseline,
            scale,
            out,
            prime_style,
        ),
        Atom::Strut(height, depth, kind) => {
            out.ascent = out.ascent.max(baseline + height * scale);
            out.depth = out.depth.max(depth * scale - baseline);
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind,
                body: Box::new(Layout {
                    bounds_em: None,
                    width_em: None,
                    intrinsic_width_em: None,
                    width: 0.0,
                    ascent: 0.0,
                    depth: 0.0,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                    groups: Vec::new(),
                }),
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
            });
            Some(0.0)
        }
        Atom::CdArrow {
            code,
            over,
            under,
            minimums,
            ..
        } => cd::layout_arrow(
            *code,
            over.as_deref(),
            under.as_deref(),
            *minimums,
            x,
            baseline,
            scale,
            out,
        ),
        Atom::Unit(codepoint) => {
            let width = layout_at_with_style(
                &Atom::Glyph(*codepoint),
                x,
                baseline,
                scale,
                out,
                display_style,
                prime_style,
            )?;
            out.glyphs.last_mut()?.unit = true;
            Some(width)
        }
        Atom::Glyph(codepoint)
        | Atom::LiteralGlyph(codepoint)
        | Atom::VariantGlyph(codepoint)
        | Atom::CalligraphicGlyph(codepoint)
        | Atom::ItalicGlyph(codepoint) => {
            let calligraphic =
                matches!(atom, Atom::CalligraphicGlyph(_)) && !is_large_operator(*codepoint);
            let italic = matches!(atom, Atom::ItalicGlyph(_)) && !is_large_operator(*codepoint);
            let variant = matches!(atom, Atom::VariantGlyph(_));
            let literal = matches!(atom, Atom::LiteralGlyph(_));
            let small_operator = !literal && !display_style && is_large_operator(*codepoint);
            let large = !literal && display_style && is_large_operator(*codepoint);
            let glyph = if large {
                glyph_for(PositionedGlyph {
                    codepoint: *codepoint,
                    x: 0.0,
                    baseline: 0.0,
                    scale: 1.0,
                    font_original: None,
                    font_variant: "normal",
                    bold: false,
                    small_operator: false,
                    variant: false,
                    unit: false,
                    calligraphic: false,
                    italic: false,
                    large: true,
                    stretch: false,
                    radical_variant: None,
                    assembly_height: None,
                })?
            } else if small_operator {
                smallop_glyph(*codepoint)?
            } else if variant {
                variant_glyph(*codepoint)?
            } else if calligraphic {
                calligraphic_glyph(*codepoint)?
            } else if italic {
                italic_glyph(*codepoint)?
            } else {
                glyph(*codepoint)?
            };
            let axis_shift = if !literal && is_large_operator(*codepoint) {
                ((glyph.height + glyph.depth) / 2.0 + 0.25 - glyph.height) * 1000.0 * scale
            } else {
                0.0
            };
            out.glyphs.push(PositionedGlyph {
                font_original: None,
                font_variant: "normal",
                bold: false,
                small_operator,
                variant,
                unit: false,
                calligraphic,
                italic,
                codepoint: *codepoint,
                x,
                baseline: baseline + axis_shift,
                scale,
                large,
                stretch: false,
                radical_variant: None,
                assembly_height: None,
            });
            out.ascent = out
                .ascent
                .max(baseline + axis_shift + glyph.height * 1000.0 * scale);
            out.depth = out
                .depth
                .max(-(baseline + axis_shift - glyph.depth * 1000.0 * scale));
            let corrections = if small_operator {
                smallop_glyphs::ITALIC_CORRECTIONS
            } else if variant {
                tex_variant_glyphs::ITALIC_CORRECTIONS
            } else if !literal && is_large_operator(*codepoint) {
                large_glyphs::ITALIC_CORRECTIONS
            } else if calligraphic {
                calligraphic_glyphs::ITALIC_CORRECTIONS
            } else if italic {
                italic_glyphs::ITALIC_CORRECTIONS
            } else {
                glyphs::ITALIC_CORRECTIONS
            };
            let correction = if calligraphic {
                calligraphic_correction(*codepoint)
            } else if italic {
                italic_correction(*codepoint)
            } else {
                corrections
                    .binary_search_by_key(codepoint, |entry| entry.0)
                    .ok()
                    .map_or_else(
                        || {
                            if compound(*codepoint).is_some() {
                                glyphs::ITALIC_CORRECTIONS
                                    .binary_search_by_key(codepoint, |e| e.0)
                                    .ok()
                                    .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1)
                            } else {
                                0.0
                            }
                        },
                        |index| corrections[index].1,
                    )
            };
            out.width_em = Some((glyph.width + correction) * scale);
            out.intrinsic_width_em = Some(glyph.width + correction);
            Some((glyph.width + correction) * 1000.0 * scale)
        }
        Atom::Space(_) | Atom::Kern(_) | Atom::EmptyNode(_) | Atom::RuleSpace { .. } => {
            let (height, depth, background) = if let Atom::RuleSpace {
                dimensions,
                background,
                mathsize,
            } = atom
            {
                (
                    dimensions[1].em(0., scale * mathsize) * 1000. * scale,
                    dimensions[2].em(0., scale * mathsize) * 1000. * scale,
                    background.clone(),
                )
            } else {
                (0., 0., None)
            };
            let width = match atom {
                Atom::Space(width) => width * 1000.0 * scale,
                Atom::Kern(dimension) => dimension.em(0.0, scale) * 1000.0 * scale,
                Atom::EmptyNode(_) => 0.0,
                Atom::RuleSpace {
                    dimensions,
                    mathsize,
                    ..
                } => dimensions[0].em(0., scale * mathsize) * 1000. * scale,
                _ => unreachable!(),
            };
            // Explicit TeX glue is an mspace node even though it paints no
            // primitive. Preserve that node independently of its advance.
            let local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width,
                ascent: height,
                depth,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: background
                    .filter(|c| c != "transparent")
                    .map(|c| (c, width, height, depth)),
                kind: if let Atom::EmptyNode(kind) = atom {
                    kind
                } else {
                    "mspace"
                },
                body: Box::new(local),
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
            });
            out.ascent = out.ascent.max(baseline + height);
            out.depth = out.depth.max(depth - baseline);
            Some(width)
        }
        Atom::Sequence(children) => {
            if style_classes::has_declaration(atom) {
                return style_classes::layout(
                    atom,
                    x,
                    baseline,
                    scale,
                    out,
                    display_style,
                    prime_style,
                );
            }
            let classes = sequence_tex_classes(children);
            let stretch = braket::row_dimensions(children, scale, display_style, prime_style)?;
            // Source wrapper.computeChildrenBBox/BBox.Append composes em widths
            // before scaling the row at its parent boundary.
            let mut cursor_em = 0.0;
            let mut intrinsic_cursor_em = 0.0;
            let mut previous = None;
            for (index, child) in children.iter().enumerate() {
                let mut boundary_space = 0.0;
                if !operator_dictionary::spacelike(child) {
                    if let Some(previous_index) = previous {
                        let script = scale < 1.0
                            || !style_classes::adjusts_current_level(child)
                                && style_classes::returned_script_level(
                                    &children[previous_index],
                                    false,
                                );
                        boundary_space = tex_class_spacing(
                            return_tex_class(&children[previous_index], classes[previous_index]),
                            classes::adjustment_class(child, classes[index]).or(Some(0)),
                            script,
                        ) * scale;
                    }
                    previous = Some(index);
                }
                if table::replaces_leading_space(child) || matches!(child, Atom::InferredRow(_)) {
                    boundary_space = 0.0;
                }
                let (left_space, right_space) = mml_spacing::row_spaces(children, index, scale);
                if mml_spacing::core(child).is_some() {
                    boundary_space = left_space * 1000.0 * scale;
                }
                cursor_em += boundary_space / (1000.0 * scale);
                intrinsic_cursor_em += boundary_space / (1000.0 * scale);
                let cursor = x + (cursor_em * scale) * 1000.0;
                let group_start = out.groups.len();
                out.width_em = None;
                out.intrinsic_width_em = None;
                let advance =
                    if let (Some((height, depth)), Atom::FenceDelimiter { code, class, color }) =
                        (stretch, child)
                        && table::can_stretch(*code)
                    {
                        braket::layout_stretched_token(
                            *code,
                            *class,
                            color.as_deref(),
                            height,
                            depth,
                            cursor,
                            baseline,
                            scale,
                            out,
                        )?
                    } else {
                        layout_at_with_style(
                            child,
                            cursor,
                            baseline,
                            scale,
                            out,
                            display_style,
                            prime_style,
                        )?
                    };
                // Preserve the source glyph/space's intrinsic em metrics instead
                // of recovering them from an already scaled/rounded unit value.
                let em = match child {
                    Atom::Glyph(code) if !is_large_operator(*code) => glyph(*code).map(|g| {
                        g.width
                            + glyphs::ITALIC_CORRECTIONS
                                .binary_search_by_key(code, |v| v.0)
                                .ok()
                                .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1)
                    }),
                    Atom::Identifier(items) | Atom::Number(items) | Atom::OperatorToken(items) => {
                        let mut width = 0.0;
                        let mut last_ic = 0.0;
                        let mut complete = true;
                        for item in items {
                            if let Atom::Glyph(code) = item {
                                if let Some(g) = glyph(*code).filter(|_| !is_large_operator(*code))
                                {
                                    width += g.width;
                                    last_ic = glyphs::ITALIC_CORRECTIONS
                                        .binary_search_by_key(code, |v| v.0)
                                        .ok()
                                        .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1);
                                } else {
                                    complete = false;
                                    break;
                                }
                            } else {
                                complete = false;
                                break;
                            }
                        }
                        complete.then_some(
                            width
                                + if matches!(child, Atom::Number(_)) {
                                    0.0
                                } else {
                                    last_ic
                                },
                        )
                    }
                    Atom::Space(width) => Some(*width),
                    Atom::Kern(dimension) => Some(dimension.em(0.0, scale)),
                    _ => None,
                };
                intrinsic_cursor_em += em
                    .or(out.intrinsic_width_em)
                    .or(out.width_em.map(|w| w / scale))
                    .unwrap_or(advance / (1000.0 * scale));
                cursor_em += em
                    .or(out.width_em.map(|w| w / scale))
                    .unwrap_or(advance / (1000.0 * scale));
                cursor_em += right_space;
                intrinsic_cursor_em += right_space;
                if atom_tex_class(child) == Some(2) && classes[index] == Some(0) {
                    classes::demote_group(&mut out.groups[group_start..]);
                }
            }
            out.intrinsic_width_em = Some(intrinsic_cursor_em);
            out.width_em = Some(cursor_em * scale);
            Some(cursor_em * 1000.0 * scale)
        }
        Atom::Identifier(children)
        | Atom::Number(children)
        | Atom::OperatorToken(children)
        | Atom::StyledText(children) => {
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            let mut cursor = 0.0;
            let mut width_em = 0.0;
            let mut intrinsic_width_em = 0.0;
            for (i, child) in children.iter().enumerate() {
                local.width_em = None;
                local.intrinsic_width_em = None;
                let advance = layout_at_with_style(
                    child,
                    cursor,
                    0.0,
                    scale,
                    &mut local,
                    display_style,
                    prime_style,
                )?;
                cursor += advance;
                width_em += local.width_em.unwrap_or(advance / 1000.0);
                intrinsic_width_em += local
                    .intrinsic_width_em
                    .unwrap_or_else(|| local.width_em.unwrap_or(advance / 1000.0) / scale);
                // mtext advances by glyph widths alone (no italic correction).
                if matches!(atom, Atom::Number(_) | Atom::StyledText(_)) || i + 1 < children.len() {
                    let correction = classes::italic_correction(child, display_style);
                    cursor -= correction * 1000.0 * scale;
                    width_em -= correction * scale;
                    intrinsic_width_em -= correction;
                }
            }
            local.width = cursor;
            retain_negative_glyph_depth(&mut local);
            out.ascent = out.ascent.max(baseline + local.ascent);
            out.depth = out.depth.max(local.depth - baseline);
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind: if matches!(atom, Atom::Number(_)) {
                    "mn"
                } else if matches!(atom, Atom::StyledText(_)) {
                    "mtext"
                } else if matches!(atom, Atom::OperatorToken(_)) {
                    "mo"
                } else {
                    "mi"
                },
                delimiter: None,
                arrow: None,
                body: Box::new(local),
                lines: Vec::new(),
            });
            out.width_em = Some(width_em);
            out.intrinsic_width_em = Some(intrinsic_width_em);
            Some(cursor)
        }
        Atom::MathMLSpace { body, .. }
        | Atom::AutoOperator(body)
        | Atom::Operator(body)
        | Atom::MovesScripts(body)
        | Atom::ScriptPolicy { body, .. } => {
            layout_at_with_style(body, x, baseline, scale, out, display_style, prime_style)
        }
        Atom::Phantom(body) => padded::layout_node(
            body,
            None,
            true,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::Padded { body, padding } => padded::layout_node(
            body,
            Some(padding),
            false,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::CrampedStyle(body, _) => {
            let data_cramped = match atom {
                Atom::CrampedStyle(_, cramped) => Some(*cramped),
                _ => None,
            };
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            local.width = layout_at_with_style(
                body,
                0.0,
                0.0,
                scale,
                &mut local,
                display_style,
                prime_style,
            )?;
            retain_negative_glyph_depth(&mut local);
            let width = local.width;
            out.width_em = local.width_em;
            out.ascent = out.ascent.max(baseline + local.ascent);
            out.depth = out.depth.max(local.depth - baseline);
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind: "mstyle",
                body: Box::new(local),
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
            });
            Some(width)
        }
        Atom::Ord(body) | Atom::Class { body, .. } => {
            // Original texatom.go/standardSVG: a TeXAtom owns a local frame.
            // Keep child placements local until SVG serialization so parent and
            // child translations receive the original independent rounding.
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            local.width = layout_at_with_style(
                body,
                0.0,
                0.0,
                scale,
                &mut local,
                display_style,
                prime_style,
            )?;
            retain_negative_glyph_depth(&mut local);
            let width = local.width;
            out.width_em = local.width_em;
            let (h, d) = if matches!(atom, Atom::Class { class: 8, .. }) {
                let (h, d) = styles::em_bounds(&local);
                let delta = (h + d) / 2. + 0.25 * scale - h;
                ((h + delta) * 1000., (d - delta) * 1000.)
            } else {
                (local.ascent, local.depth)
            };
            out.ascent = out.ascent.max(baseline + h);
            out.depth = out.depth.max(d - baseline);
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: if let Atom::Class { class, .. } = atom {
                    *class
                } else {
                    0
                },
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind: "TeXAtom",
                body: Box::new(local),
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
            });
            Some(width)
        }
        Atom::Mrow(body) | Atom::Reference { body, .. } => {
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: vec![],
                rules: vec![],
                groups: vec![],
            };
            local.width = layout_at_with_style(
                body,
                0.0,
                0.0,
                scale,
                &mut local,
                display_style,
                prime_style,
            )?;
            let width = local.width;
            out.ascent = out.ascent.max(baseline + local.ascent);
            out.depth = out.depth.max(local.depth - baseline);
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: match atom {
                    Atom::Reference { href, .. } => Some(href.clone()),
                    _ => None,
                },
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind: "mrow",
                body: Box::new(local),
                delimiter: None,
                arrow: None,
                lines: vec![],
            });
            Some(width)
        }
        Atom::TaggedTable {
            rows,
            tags,
            environment,
            container_align,
            spacing,
        } => ams::layout_tagged(
            rows,
            tags,
            environment,
            container_align,
            *spacing,
            x,
            baseline,
            scale,
            out,
        ),
        Atom::Boxed(body) => {
            enclose::layout_box(body, x, baseline, scale, out, display_style, prime_style)
        }
        Atom::NotToken { first, kind, .. } => pending_not::layout_token(
            first,
            kind,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::Text(text) => {
            // Original mtext appends glyph widths without mi italic correction.
            // Retain its local frame so zero-width laps use the actual text width.
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            let mut cursor = 0.0;
            for character in text.chars() {
                let code = character as u32;
                cursor += layout_at_with_style(
                    &Atom::LiteralGlyph(code),
                    cursor,
                    0.0,
                    scale,
                    &mut local,
                    display_style,
                    prime_style,
                )?;
                cursor -= glyphs::ITALIC_CORRECTIONS
                    .binary_search_by_key(&code, |e| e.0)
                    .ok()
                    .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1)
                    * 1000.0
                    * scale;
            }
            local.width = cursor;
            retain_negative_glyph_depth(&mut local);
            out.ascent = out.ascent.max(baseline + local.ascent);
            out.depth = out.depth.max(local.depth - baseline);
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color: None,
                background: None,
                kind: "mtext",
                body: Box::new(local),
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
            });
            Some(cursor)
        }
        Atom::Cancel {
            body,
            up,
            down,
            arrow,
            options,
        } => enclose::layout_node(
            body,
            *up,
            *down,
            *arrow,
            options,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::Color { body, .. } | Atom::ColorBox { body, .. } => {
            // Original menclose.go/menclose_notation.go/.shapes.go and mstyle:
            // retain scope, dynamic child bounds, padding, and diagonal strokes.
            let pad = if let Atom::ColorBox { mathsize, .. } = atom {
                // Original colorBox uses PaddingProperties(5px), Ex8/XHeight.442.
                length::dimension("5px").em(0., scale * mathsize) * 1000. * scale
            } else {
                0.0
            };
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            // tex/inheritance.go cleanMathMLAttributes drops an explicit
            // mathcolor equal to the inherited one, so a nested mstyle of the
            // same color emits no fill/stroke.
            let explicit_color = if let Atom::Color { color, .. } = atom {
                let inherited = INHERITED_COLOR.with(|c| c.borrow().last().cloned());
                INHERITED_COLOR.with(|c| c.borrow_mut().push(color.clone()));
                (inherited.as_deref() != Some(color.as_str())).then(|| color.clone())
            } else {
                None
            };
            let laid_out = layout_at_with_style(
                body,
                0.0,
                0.0,
                scale,
                &mut local,
                display_style,
                prime_style,
            );
            if matches!(atom, Atom::Color { .. }) {
                INHERITED_COLOR.with(|c| c.borrow_mut().pop());
            }
            local.width = laid_out? + 2.0 * pad;
            if pad != 0.0 {
                // Original svg/mpadded.go paddedToSVG: the lspace offset is a
                // separate content g placed inside the mpadded node.
                let content = std::mem::replace(
                    &mut local,
                    Layout {
                        bounds_em: None,
                        width_em: None,
                        intrinsic_width_em: None,
                        width: 0.0,
                        ascent: 0.0,
                        depth: 0.0,
                        glyphs: Vec::new(),
                        rules: Vec::new(),
                        groups: Vec::new(),
                    },
                );
                local.width = content.width;
                local.ascent = content.ascent;
                local.depth = content.depth;
                local.width_em = content.width_em;
                local.groups.push(LayoutGroup {
                    x_frame: None,
                    data_cramped: None,
                    own_translate: None,
                    accent_offset: None,
                    box_border: None,
                    reference: None,
                    tagged: None,
                    percentage: None,
                    table_line: None,
                    mathsize: None,
                    tex_class: 0,
                    before_glyph: 0,
                    scale,
                    x: pad,
                    baseline: 0.0,
                    color: None,
                    background: None,
                    kind: "",
                    delimiter: None,
                    arrow: None,
                    body: Box::new(content),
                    lines: Vec::new(),
                });
            }
            let (ascent, depth) = if matches!(atom, Atom::ColorBox { .. }) {
                // Original mpadded adds dimensions in em before SVG *1000;
                // adding converted pixel values changes exact binary ties.
                (
                    (local.ascent / 1000.0 + pad / 1000.0) * 1000.0,
                    (local.depth / 1000.0 + pad / 1000.0) * 1000.0,
                )
            } else {
                (local.ascent + pad, local.depth + pad)
            };
            let width = local.width;
            let lines = Vec::new();
            let color = explicit_color;
            let background = if let Atom::ColorBox { background, .. } = atom {
                Some((background.clone(), width, ascent, depth))
            } else {
                None
            };
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x,
                baseline,
                color,
                background,
                kind: if matches!(atom, Atom::ColorBox { .. }) {
                    "mpadded"
                } else {
                    "mstyle"
                },
                body: Box::new(local),
                delimiter: None,
                arrow: None,
                lines,
            });
            out.ascent = out.ascent.max(baseline + ascent);
            out.depth = out.depth.max(depth - baseline);
            Some(width)
        }
        Atom::SplitFraction(numerator, denominator, mathsize) => split_fraction::layout(
            numerator,
            denominator,
            *mathsize,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::BracketMark {
            body,
            thickness,
            height,
            under,
            mathsize,
        } => brackets::layout(
            body,
            *thickness,
            height,
            *under,
            *mathsize,
            x,
            baseline,
            scale,
            out,
            prime_style,
        ),
        Atom::HorizontalMark { body, .. } => {
            // Under/over marks are mo text tokens, including fixed line marks.
            // Their role is distinct from the same character used as a name.
            if matches!(body.as_ref(), Atom::OperatorToken(_)) {
                layout_at_with_style(body, x, baseline, scale, out, display_style, prime_style)
            } else {
                layout_at_with_style(
                    &Atom::OperatorToken(vec![(**body).clone()]),
                    x,
                    baseline,
                    scale,
                    out,
                    display_style,
                    prime_style,
                )
            }
        }
        Atom::FractionStrut(mathsize) => general_fraction::strut(
            *mathsize,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::GeneralFraction(numerator, denominator, spec) => general_fraction::layout(
            numerator,
            denominator,
            *spec,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::Fraction(numerator, denominator) => general_fraction::layout(
            numerator,
            denominator,
            general_fraction::Spec::new("medium", false),
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::Sqrt(body) | Atom::SizedSqrt { body, .. } | Atom::Root { body, .. } => {
            let mathsize = if let Atom::SizedSqrt { mathsize, .. } = atom {
                *mathsize
            } else {
                1.0
            };
            let root_index = match atom {
                // Sized mroot layout is not translated.
                Atom::Root { mathsize, .. } if *mathsize != 1.0 => return None,
                Atom::Root { index, .. } => Some(index.as_ref()),
                _ => None,
            };
            let parent_x = x;
            let parent_baseline = baseline;
            let parent = out;
            let mut root = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            let out = &mut root;
            let x = 0.0;
            let baseline = 0.0;
            // Original internal/svg/msqrt.go and mo.go: exact delimiter selection,
            // rootPQ clearance, child prime style and quantized assembled surds.
            let mut body_box = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            body_box.width =
                layout_at_with_style(body, 0.0, 0.0, scale, &mut body_box, display_style, true)?;
            retain_negative_glyph_depth(&mut body_box);
            let rule = 60.0 * scale;
            let clearance = if display_style { 442.0 * scale } else { rule };
            let requested = body_box.ascent + body_box.depth + 2.0 * rule + clearance / 4.0;
            let variant = [1.0, 1.2, 1.8, 2.4, 3.0]
                .iter()
                .position(|size| size * 1000.0 * scale >= requested);
            let assembly_height = if variant.is_none() {
                let total =
                    2.3 + ((requested / (1000.0 * scale) - 2.3).max(0.0) / 0.65).ceil() * 0.65;
                // Original mo.go stretchBaseline: preserve exact requested H/D,
                // otherwise distribute quantized size using delimiter HDW .85/.35.
                let depth = if requested / (1000.0 * scale) == total {
                    body_box.depth / (1000.0 * scale)
                } else {
                    0.35 * total / (0.85 + 0.35)
                };
                Some((total, depth))
            } else {
                None
            };
            let surd = if let Some(index) = variant {
                radical_glyphs::RADICAL_GLYPHS[index]
            } else {
                let (total, depth) = assembly_height?;
                GlyphData {
                    height: total - depth,
                    depth,
                    width: 1.056,
                    path: "",
                }
            };
            // Original computes root bounds in em before the outer SVG scale.
            // Retain that operation order when padded nodes expose H/D alone.
            let body_h_em = body_box.ascent / (1000.0 * scale);
            let body_d_em = body_box.depth / (1000.0 * scale);
            let clearance_em = if display_style { 0.442 } else { 0.06 };
            let requested_em = body_h_em + body_d_em + 2.0 * 0.06 + clearance_em / 4.0;
            let q_em = if surd.height + surd.depth > requested_em {
                ((surd.height + surd.depth) - (requested_em - 2.0 * 0.06 - clearance_em / 2.0))
                    / 2.0
            } else {
                0.06 + clearance_em / 4.0
            };
            let height_em = body_h_em + q_em + 0.06;
            // All five pinned radical font variants have italic correction .02.
            let surd_width =
                (surd.width + if variant.is_some() { 0.02 } else { 0.0 }) * 1000.0 * scale;
            // Source rootToSVG scales the paint rule before adding it to H+q.
            // Bounds use unscaled Rule independently in computeRootBBox.
            let paint_height_em = body_h_em + q_em + 0.06 * scale * mathsize;
            let surd_baseline = baseline + (paint_height_em - surd.height) * 1000.0 * scale;
            // rootToSVG retains a translated baseContainer independently of
            // each child placement. Flattening those translations changes
            // downstream affine arithmetic even when serialized sums agree.
            let body_width = body_box.width;
            let body_width_em = body_box.width_em.unwrap_or(body_width / 1000.0) / scale;
            let width_em = ((surd.width + if variant.is_some() { 0.02 } else { 0.0 })
                + body_width_em)
                .max(surd.width + if variant.is_some() { 0.02 } else { 0.0 })
                * scale;
            parent.width_em = Some(width_em);
            // Original svg/msqrt.go rootDimensions: the index is laid out at
            // scriptlevel+2 and shifts the surd and base right by x when it
            // is wider than the surd's offset. wrapper.go:getScale retains
            // the squared multiplier (0.4999999999999999): the default 8px
            // minimum is only 0.442em at pxPerEm = Ex / XHeight.
            let mut root_shift = 0.0;
            let mut root_group = None;
            let mut root_glyph: Option<PositionedGlyph> = None;
            if let Some(index) = root_index {
                let index_scale = styles::SCRIPT_MULTIPLIER * styles::SCRIPT_MULTIPLIER;
                let mut index_box = Layout {
                    bounds_em: None,
                    width_em: None,
                    intrinsic_width_em: None,
                    width: 0.0,
                    ascent: 0.0,
                    depth: 0.0,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                    groups: Vec::new(),
                };
                index_box.width = layout_at_with_style(
                    index,
                    0.0,
                    0.0,
                    index_scale,
                    &mut index_box,
                    false,
                    prime_style,
                )?;
                retain_negative_glyph_depth(&mut index_box);
                let rscale = index_scale / scale;
                let root_w = index_box.width_em.unwrap_or(index_box.width / 1000.0) / index_scale;
                let root_h = index_box.ascent / (1000.0 * index_scale);
                let root_d = index_box.depth / (1000.0 * index_scale);
                let surd_w = surd.width + if variant.is_some() { 0.02 } else { 0.0 };
                let offset = if variant.is_some() { 0.6 } else { 0.5 } * surd_w;
                let width = root_w.max(offset / rscale);
                let dx = (width - root_w).max(0.0);
                let total = surd.height + surd.depth;
                let b = if variant.is_some() { 0.55 * total } else { 1.9 };
                let h = b - (total - paint_height_em) + (root_d * rscale).max(0.0);
                // computeRootBBox calls rootDimensions with unscaled Rule;
                // rootToSVG independently uses Rule * bbox.Scale for paint.
                let bbox_h = b - (total - height_em) + (root_d * rscale).max(0.0);
                let x_root = width * rscale - offset;
                root_shift = x_root * 1000.0 * scale;
                // computeRootBBox: Combine(root, 0, h), then the base at
                // x + surd.W, in em before the parent scale.
                parent.width_em = Some(
                    ((x_root + surd_w) + body_width_em)
                        .max(surd_w)
                        .max(root_w * rscale)
                        * scale,
                );
                out.ascent = out
                    .ascent
                    .max(baseline + (bbox_h + root_h * rscale) * 1000.0 * scale);
                out.depth = out
                    .depth
                    .max((root_d * rscale - bbox_h) * 1000.0 * scale - baseline);
                let mut group = LayoutGroup {
                    x_frame: None,
                    data_cramped: None,
                    own_translate: None,
                    accent_offset: None,
                    box_border: None,
                    reference: None,
                    tagged: None,
                    percentage: None,
                    table_line: None,
                    mathsize: None,
                    tex_class: 0,
                    before_glyph: out.glyphs.len(),
                    scale: index_scale,
                    x: dx * rscale * 1000.0 * scale,
                    baseline: baseline + h * 1000.0 * scale,
                    color: None,
                    background: None,
                    kind: "",
                    body: Box::new(index_box),
                    delimiter: None,
                    arrow: None,
                    lines: Vec::new(),
                };
                // The index is the mroot's own child node (no extra frame).
                if group.body.glyphs.is_empty()
                    && group.body.rules.is_empty()
                    && group.body.groups.len() == 1
                {
                    let mut child = group.body.groups.pop()?;
                    child.x += group.x;
                    child.x_frame = None;
                    child.baseline += group.baseline;
                    child.before_glyph = out.glyphs.len();
                    group = child;
                } else if group.body.groups.is_empty()
                    && group.body.rules.is_empty()
                    && group.body.glyphs.len() == 1
                {
                    let mut glyph = group.body.glyphs.pop()?;
                    glyph.x += group.x;
                    glyph.baseline += group.baseline;
                    root_glyph = Some(glyph);
                } else {
                    group.kind = "mrow";
                }
                root_group = Some((group, root_w * rscale * 1000.0 * scale));
            }
            out.ascent = out.ascent.max(baseline + body_box.ascent);
            out.depth = out.depth.max(body_box.depth - baseline);
            let body_box = if root_index.is_some() {
                // mroot's base is its own (non-inferred) child: wrap it in an
                // untransformed container and place the base node itself.
                let shift = root_shift + surd_width;
                let mut container = Layout {
                    bounds_em: None,
                    width_em: body_box.width_em,
                    intrinsic_width_em: None,
                    width: body_box.width,
                    ascent: body_box.ascent,
                    depth: body_box.depth,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                    groups: Vec::new(),
                };
                let mut base = body_box;
                if base.glyphs.is_empty() && base.rules.is_empty() && base.groups.len() == 1 {
                    let mut child = base.groups.pop()?;
                    child.x += shift;
                    child.x_frame = None;
                    child.before_glyph = 0;
                    container.groups.push(child);
                } else if base.groups.is_empty() && base.rules.is_empty() && base.glyphs.len() == 1
                {
                    let mut glyph = base.glyphs.pop()?;
                    glyph.x += shift;
                    container.glyphs.push(glyph);
                } else {
                    container.groups.push(LayoutGroup {
                        x_frame: None,
                        data_cramped: None,
                        own_translate: None,
                        accent_offset: None,
                        box_border: None,
                        reference: None,
                        tagged: None,
                        percentage: None,
                        table_line: None,
                        mathsize: None,
                        tex_class: 0,
                        before_glyph: 0,
                        scale,
                        x: shift,
                        baseline: 0.0,
                        color: None,
                        background: None,
                        kind: "mrow",
                        body: Box::new(base),
                        delimiter: None,
                        arrow: None,
                        lines: Vec::new(),
                    });
                }
                container
            } else {
                body_box
            };
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                scale,
                x: if root_index.is_some() {
                    x
                } else {
                    x + surd_width
                },
                baseline,
                color: None,
                background: None,
                kind: "",
                body: Box::new(body_box),
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
            });
            let mut root_width = 0.0;
            if let Some((group, width)) = root_group {
                root_width = width;
                if let Some(glyph) = root_glyph {
                    out.glyphs.push(glyph);
                } else {
                    out.groups.push(group);
                }
            }
            out.glyphs.push(PositionedGlyph {
                font_original: None,
                font_variant: "normal",
                bold: false,
                small_operator: false,
                variant: false,
                unit: false,
                calligraphic: false,
                italic: false,
                codepoint: 0x221A,
                x: x + root_shift,
                baseline: surd_baseline,
                scale,
                large: false,
                stretch: false,
                radical_variant: variant,
                assembly_height,
            });
            // Source rootToSVG applies bbox.Scale before the script parent scale.
            let painted_rule = rule * scale * mathsize;
            out.rules.push((
                x + root_shift + surd_width,
                baseline + (paint_height_em - 0.06 * scale * mathsize) * 1000.0 * scale,
                body_width,
                painted_rule,
            ));
            // BBox.Combine adds the scaled surd height to its already computed
            // placement offset; retain that subtraction before addition.
            out.ascent = out.ascent.max(
                baseline
                    + (height_em + 0.06).max(surd.height + (height_em - surd.height))
                        * 1000.0
                        * scale,
            );
            out.depth = out
                .depth
                .max((surd.depth - (height_em - surd.height)) * 1000.0 * scale - baseline);
            let width = (root_shift + surd_width + body_width)
                .max(surd_width)
                .max(root_width);
            out.width = width;
            parent.ascent = parent.ascent.max(parent_baseline + out.ascent);
            parent.depth = parent.depth.max(out.depth - parent_baseline);
            parent.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: parent.glyphs.len(),
                // wrapper.place adds the mroot's own dx (rootToSVG sets it to
                // the index shift) to the parent's placement.
                x: parent_x + root_shift,
                baseline: parent_baseline,
                scale,
                color: None,
                background: None,
                kind: if root_index.is_some() {
                    "mroot"
                } else {
                    "msqrt"
                },
                delimiter: None,
                arrow: None,
                body: Box::new(root),
                lines: Vec::new(),
            });
            Some(width)
        }
        Atom::Matrix {
            rows,
            environment,
            column_spec,
            options,
            spacing,
        } => table::layout(
            rows,
            environment,
            column_spec,
            options.as_ref(),
            *spacing,
            x,
            baseline,
            scale,
            out,
            prime_style,
        ),
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            prescript,
            mathsize,
        } => multiscripts::layout(
            base,
            sub,
            sup,
            post.as_ref(),
            *prescript,
            *mathsize,
            x,
            baseline,
            scale,
            out,
            display_style,
            prime_style,
        ),
        Atom::Scripts {
            base,
            sub,
            sup,
            limits,
            leading_space,
            accent,
            ..
        } => {
            // Pinned tex/move_limits_filter.go converts an inline stack when
            // its direct base owns the movablelimits property. A mathop
            // TeXAtom owns that property independently of its deeper core.
            let operator_limits = *limits && (display_style || !mathop_movable_property(base));
            if operator_limits && !classes::movable_limits(base, display_style) {
                return classes::layout_limits(
                    base,
                    sub.as_deref(),
                    sup.as_deref(),
                    *leading_space,
                    *accent,
                    x,
                    baseline,
                    scale,
                    out,
                    display_style,
                    prime_style,
                );
            }
            // Original internal/svg/msubsup.go: actual child bounds, base italic
            // correction, and TeX sub/sup shift constraints determine the box.
            let ordinary_probe = |atom: &Atom, child_scale, display, prime| {
                let mut bounds = Layout {
                    bounds_em: None,
                    width_em: None,
                    intrinsic_width_em: None,
                    width: 0.0,
                    ascent: 0.0,
                    depth: 0.0,
                    glyphs: Vec::new(),
                    rules: Vec::new(),
                    groups: Vec::new(),
                };
                bounds.width =
                    layout_at_with_style(atom, 0.0, 0.0, child_scale, &mut bounds, display, prime)?;
                retain_negative_glyph_depth(&mut bounds);
                Some(bounds)
            };
            let base_box = ordinary_probe(base, scale, display_style, prime_style)?;
            // wrapper.go caps scriptlevel at two before getScale. The
            // level-two product stays below 0.5 at the default 8px minimum.
            let script_scale = (scale * styles::SCRIPT_MULTIPLIER)
                .max(styles::SCRIPT_MULTIPLIER * styles::SCRIPT_MULTIPLIER);
            // Source constructs/stretches under-over children before selecting
            // movable-limit side placement. The core mark keeps that sizing.
            let mut requested_em = base_box.width_em.unwrap_or(base_box.width / 1000.);
            if operator_limits {
                for a in [sub, sup].into_iter().flatten() {
                    if !matches!(a.as_ref(), Atom::HorizontalMark { stretchy: true, .. }) {
                        let bounds = ordinary_probe(a, script_scale, false, prime_style)?;
                        requested_em =
                            requested_em.max(bounds.width_em.unwrap_or(bounds.width / 1000.));
                    }
                }
            }
            let probe = |atom: &Atom, child_scale, display, prime| {
                if operator_limits && matches!(atom, Atom::HorizontalMark { .. }) {
                    accents::probe_mark(atom, requested_em, child_scale, prime, display)
                } else {
                    ordinary_probe(atom, child_scale, display, prime)
                }
            };
            let sub_box = if let Some(a) = sub {
                Some(probe(a, script_scale, false, true)?)
            } else {
                None
            };
            let sup_box = if let Some(a) = sup {
                Some(probe(a, script_scale, false, prime_style)?)
            } else {
                None
            };
            let (core, core_scale, core_display, core_prime) =
                styles::script_core(base, scale, display_style, prime_style);
            let core_box = probe(core, core_scale, core_display, core_prime)?;
            let (character, ic) = if let Atom::RawOperator {
                body, definition, ..
            } = core
            {
                (
                    !definition.flag("largeop"),
                    operator_dictionary::body_correction(body, *definition, core_display)
                        * 1000.0
                        * core_scale,
                )
            } else if let Atom::Glyph(codepoint)
            | Atom::LiteralGlyph(codepoint)
            | Atom::VariantGlyph(codepoint)
            | Atom::Unit(codepoint)
            | Atom::CalligraphicGlyph(codepoint)
            | Atom::ItalicGlyph(codepoint) = core
            {
                let corrections = if !core_display && is_large_operator(*codepoint) {
                    smallop_glyphs::ITALIC_CORRECTIONS
                } else if matches!(core, Atom::VariantGlyph(_)) {
                    tex_variant_glyphs::ITALIC_CORRECTIONS
                } else if is_large_operator(*codepoint) {
                    large_glyphs::ITALIC_CORRECTIONS
                } else if matches!(core, Atom::CalligraphicGlyph(_)) {
                    calligraphic_glyphs::ITALIC_CORRECTIONS
                } else {
                    glyphs::ITALIC_CORRECTIONS
                };
                let ic = (if matches!(core, Atom::CalligraphicGlyph(_))
                    && !is_large_operator(*codepoint)
                {
                    calligraphic_correction(*codepoint)
                } else if matches!(core, Atom::ItalicGlyph(_)) && !is_large_operator(*codepoint) {
                    italic_correction(*codepoint)
                } else {
                    corrections
                        .binary_search_by_key(codepoint, |e| e.0)
                        .ok()
                        .map_or(0.0, |i| corrections[i].1)
                }) * 1000.0
                    * core_scale;
                (!is_large_operator(*codepoint), ic)
            } else if let Atom::FontGlyph { paint, .. } = core {
                (
                    !is_large_operator(*paint),
                    classes::italic_correction(core, core_display) * 1000.0 * core_scale,
                )
            } else if let Atom::Identifier(children) = core {
                let ic = children
                    .last()
                    .map_or(0., |a| classes::italic_correction(a, core_display))
                    * 1000.0
                    * core_scale;
                (children.len() == 1, ic)
            } else if let Atom::Number(children) = core {
                (children.len() == 1, 0.0)
            } else {
                (false, 0.0)
            };
            // Source baseWidth/adjustedIC remain in em until place() scales
            // the combined offset. Scaling their terms first changes ties.
            let unit = 1000.0 * scale;
            let base_em = base_box
                .width_em
                .map(|v| v / scale)
                .unwrap_or(base_box.width / unit);
            let ic_em = ic / unit;
            let base_width_em = base_em - if sub.is_some() { ic_em } else { 0.0 } + 0.033;
            let adjusted_em = if ic == 0.0 {
                0.0
            } else {
                (1.05 * (ic / (1000.0 * core_scale)) + 0.05) * (core_scale / scale)
            };
            let base_width = base_width_em * unit;
            // CommonMsubsup.scriptChild remains child 1 (the subscript).
            // supShift therefore starts from that child's depth when both
            // scripts exist, before enforcing their mutual clearance.
            // svg/msubsup.go uses the script wrapper's RScale in drop
            // terms; a mathsize or scriptlevel declaration belongs to that
            // wrapper, independently of the inherited script scale.
            let sub_scale = sub
                .as_deref()
                .map_or(script_scale, |a| accents::own_scale(a, script_scale));
            let first_script_scale = sub
                .as_deref()
                .or(sup.as_deref())
                .map_or(script_scale, |a| accents::own_scale(a, script_scale));
            let mut u = sub_box.as_ref().or(sup_box.as_ref()).map_or(0.0, |bounds| {
                let minimum = (if prime_style {
                    289.0
                } else if display_style {
                    413.0
                } else {
                    363.0
                }) * scale;
                let drop = if character {
                    0.0
                } else {
                    core_box.ascent - 386.0 * first_script_scale
                };
                drop.max(minimum).max(bounds.depth + 110.5 * scale)
            });
            let mut v = sub_box.as_ref().map_or(0.0, |bounds| {
                let drop = if character {
                    0.0
                } else {
                    core_box.depth + 50.0 * sub_scale
                };
                if sup.is_some() {
                    drop.max(247.0 * scale)
                } else {
                    drop.max(150.0 * scale).max(
                        (bounds.ascent / (1000.0 * script_scale) * (script_scale / scale)
                            - 0.8 * 0.442)
                            * 1000.0
                            * scale,
                    )
                }
            });
            if let (Some(sub_bounds), Some(sup_bounds)) = (&sub_box, &sup_box) {
                let space = (u - sup_bounds.depth) - (sub_bounds.ascent - v);
                if space < 180.0 * scale {
                    v += 180.0 * scale - space;
                    let p = 353.6 * scale - (u - sup_bounds.depth);
                    if p > 0.0 {
                        u += p;
                        v -= p;
                    }
                }
            }
            let mut local = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: 0.0,
                ascent: 0.0,
                depth: 0.0,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            // Source msubsup places child wrappers in its own frame. Keep
            // composite child frames (and independent RScale rounding).
            let mut place = |atom: &Atom,
                             dx: f64,
                             dy: f64,
                             child_scale: f64,
                             display: bool,
                             prime: bool|
             -> Option<()> {
                let mut child = probe(atom, child_scale, display, prime)?;
                if child.glyphs.is_empty()
                    && child.rules.is_empty()
                    && child.groups.len() == 1
                    && child.groups[0].x == 0.0
                    && child.groups[0].baseline == 0.0
                    && (child.groups[0].scale == child_scale || child.groups[0].kind == "mstyle")
                {
                    let mut group = child.groups.pop()?;
                    group.x = dx;
                    group.x_frame = None;
                    group.baseline = dy;
                    group.before_glyph = local.glyphs.len();
                    local.groups.push(group);
                } else if matches!(
                    atom,
                    Atom::Glyph(_)
                        | Atom::FontGlyph { .. }
                        | Atom::LiteralGlyph(_)
                        | Atom::VariantGlyph(_)
                        | Atom::CalligraphicGlyph(_)
                        | Atom::ItalicGlyph(_)
                        | Atom::Unit(_)
                ) {
                    layout_at_with_style(atom, dx, dy, child_scale, &mut local, display, prime)?;
                } else if child.groups.is_empty()
                    && child.rules.is_empty()
                    && child.glyphs.len() == 1
                {
                    // Transparent parser policies (for example MovesScripts)
                    // do not introduce MathML rows. Source msubsup places the
                    // existing child wrapper; retain its intrinsic glyph offset.
                    classes::place(child, dx, dy, child_scale, &mut local);
                } else {
                    local.groups.push(LayoutGroup {
                        x_frame: None,
                        data_cramped: None,
                        own_translate: None,
                        accent_offset: None,
                        box_border: None,
                        reference: None,
                        tagged: None,
                        percentage: None,
                        table_line: None,
                        mathsize: None,
                        tex_class: 0,
                        before_glyph: local.glyphs.len(),
                        x: dx,
                        baseline: dy,
                        scale: child_scale,
                        kind: "mrow",
                        color: None,
                        background: None,
                        delimiter: None,
                        arrow: None,
                        lines: Vec::new(),
                        body: Box::new(child),
                    });
                }
                Some(())
            };
            place(base, 0.0, 0.0, scale, display_style, prime_style)?;
            let mut ascent = base_box.ascent;
            let mut depth = base_box.depth;
            if let (Some(atom), Some(bounds)) = (sup, &sup_box) {
                let offset = (base_width_em
                    + if sub.is_some() {
                        adjusted_em
                    } else {
                        adjusted_em - ic_em
                    })
                    * unit;
                place(atom, offset, u, script_scale, false, prime_style)?;
                ascent = ascent.max(u + bounds.ascent);
                depth = depth.max(bounds.depth - u);
            }
            if let (Some(atom), Some(bounds)) = (sub, &sub_box) {
                place(atom, base_width, -v, script_scale, false, true)?;
                ascent = ascent.max(bounds.ascent - v);
                depth = depth.max(bounds.depth + v);
            }
            // Source computes script BBox.Combine widths in em, then applies
            // parent scaling. Adding scaled font-unit values first moves ties.

            let mut width_em = base_em + leading_space / unit;
            if let Some(bounds) = &sup_box {
                let offset = base_width_em
                    + if sub.is_some() {
                        adjusted_em
                    } else {
                        adjusted_em - ic_em
                    };
                width_em =
                    width_em.max(offset + bounds.width_em.unwrap_or(bounds.width / 1000.0) / scale);
            }
            if let Some(bounds) = &sub_box {
                width_em = width_em
                    .max(base_width_em + bounds.width_em.unwrap_or(bounds.width / 1000.0) / scale);
            }
            let width = (width_em
                + if sub.is_some() || sup.is_some() {
                    0.05
                } else {
                    0.0
                })
                * unit;
            local.width = width;
            local.ascent = ascent;
            local.depth = depth;
            let script_width_em = (width_em
                + if sub.is_some() || sup.is_some() {
                    0.05
                } else {
                    0.0
                })
                * scale;
            local.width_em = Some(script_width_em);
            out.width_em = Some(script_width_em);
            out.intrinsic_width_em = Some(
                width_em
                    + if sub.is_some() || sup.is_some() {
                        0.05
                    } else {
                        0.0
                    },
            );
            out.groups.push(LayoutGroup {
                x_frame: None,
                data_cramped: None,
                own_translate: None,
                accent_offset: None,
                box_border: None,
                reference: None,
                tagged: None,
                percentage: None,
                table_line: None,
                mathsize: None,
                tex_class: 0,
                before_glyph: out.glyphs.len(),
                x,
                baseline,
                scale,
                kind: if operator_limits {
                    if sub.is_some() {
                        if sup.is_some() {
                            "munderover"
                        } else {
                            "munder"
                        }
                    } else {
                        "mover"
                    }
                } else if sub.is_some() {
                    if sup.is_some() { "msubsup" } else { "msub" }
                } else {
                    "msup"
                },
                color: None,
                background: None,
                delimiter: None,
                arrow: None,
                lines: Vec::new(),
                body: Box::new(local),
            });
            out.ascent = out.ascent.max(baseline + ascent);
            out.depth = out.depth.max(depth - baseline);
            Some(width)
        }
    }
}

// AMS operatorname keeps a single mi result; other tokens acquire a
// TeXAtom owner. This distinguishes core attributes from owner attributes.
fn operator_name_identifier(atom: &Atom) -> bool {
    match atom {
        Atom::Identifier(_) | Atom::AutoOperator(_) => true,
        Atom::Operator(a) | Atom::ScriptPolicy { body: a, .. } => operator_name_identifier(a),
        Atom::Glyph(c)
        | Atom::LiteralGlyph(c)
        | Atom::ItalicGlyph(c)
        | Atom::VariantGlyph(c)
        | Atom::CalligraphicGlyph(c) => char::from_u32(*c).is_some_and(letter_unicode::is_letter),
        _ => false,
    }
}

fn mathop_movable_property(base: &Atom) -> bool {
    match base {
        Atom::ScriptPolicy {
            movable_property: true,
            ..
        } => true,
        Atom::Class { class: 1, .. } => true,
        Atom::NamedFunction(body) => mathop_movable_property(body),
        _ => false,
    }
}

// commands.go checkMovableLimits inspects only the direct parsed argument,
// preserving policies below authored grouped and scripted bases.
fn check_movable_limits(base: &mut Atom) {
    if let Atom::Sequence(children) = base
        && children.len() == 1
    {
        check_movable_limits(&mut children[0]);
        return;
    }
    if matches!(base, Atom::MovesScripts(_))
        || mathop_movable_property(base)
        || matches!(base, Atom::RawOperator { definition, .. }
            if definition.flag("movablelimits"))
    {
        let moves = script_moves(base);
        // checkMovableLimits writes on the direct node. A mathop TeXAtom
        // keeps the independent attribute of its deeper operator core.
        let own_core = !mathop_movable_property(base);
        let body = std::mem::replace(base, Atom::Sequence(Vec::new()));
        *base = Atom::ScriptPolicy {
            body: Box::new(body),
            moves,
            movable: own_core.then_some(false),
            movable_property: false,
        };
    }
}

// Source movesupsub is independent of the retained script node kind.
fn script_moves(atom: &Atom) -> bool {
    match atom {
        Atom::ScriptPolicy { moves, .. } => *moves,
        Atom::NamedFunction(body) => script_moves(body),
        Atom::Class { class: 1, .. } | Atom::MovesScripts(_) => true,
        _ => false,
    }
}

// Original internal/svg/texclass.go and internal/mml/node.go. Fraction/root
// fractions retain null classes; nodeFactory initializes radical wrappers to ORD.
fn atom_tex_class(atom: &Atom) -> Option<usize> {
    match atom {
        Atom::NonscriptSpace { body, .. } => atom_tex_class(body),
        // svg/texclass.go MmlMi.operatorName: normal multi-character ASCII
        // identifiers acquire OP spacing without becoming function items.
        Atom::NotToken { class, .. } => Some(*class),
        Atom::AutoOperator(_) => Some(1),
        Atom::CdArrow { texclass, .. } => *texclass,
        Atom::MathMLSpace { .. } => Some(0),
        Atom::FontGlyph { original, .. } => atom_tex_class(&Atom::Glyph(*original)),
        Atom::RawOperator { definition, .. } => {
            (definition.class >= 0).then_some(definition.class as usize)
        }
        Atom::Glyph(code)
        | Atom::LiteralGlyph(code)
        | Atom::VariantGlyph(code)
        | Atom::CalligraphicGlyph(code)
        | Atom::ItalicGlyph(code) => Some(compound(*code).map_or_else(
            || {
                match *code {
                    0x220F | 0x2211 | 0x222B => 1,
                    c if is_large_operator(c) => 1,
                    0x2B | 0x2D | 0x2212 | 0xB1 | 0xD7 | 0x22C5 | 0x2297 | 0x2218 => 2,
                    0x3D
                    | 0x2248
                    | 0x223C
                    | 0x2260
                    | 0x2261
                    | 0x2264
                    | 0x2265
                    | 0x3C
                    | 0x3E
                    | 0x2190..=0x2195
                    | 0x21A9
                    | 0x21AA
                    | 0x21CC
                    | 0x21D0..=0x21D5
                    | 0x27F5..=0x27FA
                    | 0x27FC => 3,
                    0x28 | 0x5B | 0x7B => 4,
                    0x29 | 0x5D | 0x7D => 5,
                    0x2C | 0x3B => 6,
                    // Other mo tokens take their class from the original
                    // operator dictionary (tex/operator_dictionary_generated.go).
                    c if mathml_node_kind(c) == "mo" => {
                        operator_dictionary::lookup(c, operator_dictionary::Form::Infix)
                            .class
                            .max(0) as usize
                    }
                    _ => 0,
                }
            },
            |g| g.class,
        )),
        Atom::NamedFunction(body) => {
            if let Atom::Sequence(items) = body.as_ref() {
                items
                    .iter()
                    .rev()
                    .find(|item| !operator_dictionary::spacelike(item))
                    .and_then(atom_tex_class)
            } else {
                atom_tex_class(body)
            }
        }
        Atom::ApplyFunction => None,
        Atom::InferredRow(children) | Atom::ExplicitRow(children) => {
            children.first().and_then(atom_tex_class)
        }
        Atom::FenceDelimiter { code, class: 0, .. } => Some(
            operator_dictionary::lookup(*code, operator_dictionary::Form::Infix)
                .class
                .max(0) as usize,
        ),
        Atom::FenceDelimiter { class, .. } => Some(*class),
        Atom::Fenced { inner, .. } => Some(if *inner { 7 } else { 4 }),
        Atom::XArrow { .. } => Some(3),
        Atom::Operator(..) => Some(1),
        Atom::MovesScripts(body) | Atom::ScriptPolicy { body, .. } => atom_tex_class(body),
        Atom::Class { class, .. } => Some(*class),
        Atom::Boxed(..)
        | Atom::Sqrt(..)
        | Atom::Root { .. }
        | Atom::SizedSqrt { .. }
        | Atom::TaggedTable { .. }
        | Atom::Mrow(..)
        | Atom::Reference { .. }
        | Atom::Text(..)
        | Atom::Identifier(..)
        | Atom::StyledText(..)
        | Atom::Number(..)
        | Atom::OperatorToken(..)
        | Atom::Unit(..)
        | Atom::Ord(..)
        | Atom::ColorBox { .. } => Some(0),
        Atom::Color { body, .. }
        | Atom::Phantom(body)
        | Atom::CrampedStyle(body, _)
        | Atom::DeclaredStyle { body, .. }
        | Atom::DeclaredSize { body, .. }
        | Atom::Padded { body, .. } => atom_tex_class(body),
        Atom::Cancel { .. } => Some(0),
        Atom::Scripts { base, .. } => physics::script_class(base),
        Atom::Multiscripts { base, .. } => Some(physics::script_class(base).unwrap_or(0)),
        Atom::Matrix { environment, .. } => Some(if table::fences(environment).is_some() {
            7
        } else {
            0
        }),
        Atom::Sequence(children) => children.last().and_then(atom_tex_class),
        Atom::HorizontalMark { .. } => Some(0),
        Atom::BracketMark { .. } => None,
        Atom::FractionStrut(_)
        | Atom::FixedDelimiter { .. }
        | Atom::Choice(..)
        | Atom::Fraction(..)
        | Atom::SplitFraction(..)
        | Atom::GeneralFraction(..)
        | Atom::Space(_)
        | Atom::RuleSpace { .. }
        | Atom::Kern(_)
        | Atom::EmptyNode(_)
        | Atom::Strut(..)
        | Atom::ErrorDisplay(..)
        | Atom::SourceError(..) => None,
    }
}

fn automatic_operator(children: &[Atom]) -> bool {
    children.len() >= 2
        && children.iter().enumerate().all(|(index, child)| {
            matches!(child, Atom::Glyph(code) if char::from_u32(*code).is_some_and(|c|
            c.is_ascii_alphabetic() || index > 0 && c.is_ascii_digit()))
        })
}
// setRowTeXClass returns a left/right row's closing delimiter while the
// row wrapper retains INNER for its own leading space.
fn return_tex_class(atom: &Atom, prepared: Option<usize>) -> Option<usize> {
    if mml_spacing::resets_tex_class(atom) {
        return None;
    }
    match atom {
        // svg/texclass.go effectiveTeXClass: an unset fraction class is
        // ORD when it becomes a predecessor, unlike an explicit NONE.
        Atom::Fraction(..) | Atom::GeneralFraction(..) | Atom::SplitFraction(..) => Some(0),
        Atom::InferredRow(children) | Atom::ExplicitRow(children) => {
            children.last().and_then(atom_tex_class)
        }
        Atom::Scripts { base, .. } if classes::wrapped_core_class(base).is_some() => {
            classes::adjustment_class(atom, prepared)
        }
        Atom::NamedFunction(body) => {
            if let Atom::Sequence(items) = body.as_ref() {
                items
                    .iter()
                    .rev()
                    .find(|item| !operator_dictionary::spacelike(item))
                    .and_then(|item| return_tex_class(item, atom_tex_class(item)))
            } else {
                return_tex_class(body, prepared)
            }
        }
        Atom::Fenced { .. } => Some(5),
        Atom::Matrix { environment, .. } if table::fences(environment).is_some() => Some(5),
        Atom::Color { body, .. }
        | Atom::Phantom(body)
        | Atom::CrampedStyle(body, _)
        | Atom::DeclaredStyle { body, .. }
        | Atom::DeclaredSize { body, .. }
        | Atom::Padded { body, .. }
        | Atom::ScriptPolicy { body, .. } => return_tex_class(body, prepared),
        Atom::Sequence(children) => {
            // setRowTeXClass returns the final child's prepared class. In
            // particular, isLastTeXAtom can demote a trailing BIN inside a
            // declaration before the following sibling reads PrevClass.
            let classes = sequence_tex_classes(children);
            children
                .iter()
                .enumerate()
                .rev()
                .find(|(_, child)| !operator_dictionary::spacelike(child))
                .and_then(|(index, child)| return_tex_class(child, classes[index]))
        }
        _ => prepared,
    }
}
fn sequence_tex_classes(children: &[Atom]) -> Vec<Option<usize>> {
    let mut classes: Vec<_> = children.iter().map(atom_tex_class).collect();
    let mut previous = None;
    for (index, child) in children.iter().enumerate() {
        if operator_dictionary::spacelike(child) {
            continue;
        }
        // adjustTeXClass demotes an automatically classified mi before a
        // binary or relation item, before deciding that item's own class.
        if matches!(
            classes::adjustment_class(child, classes[index]),
            Some(2 | 3)
        ) {
            if let Some(i) = previous {
                if matches!(&children[i], Atom::AutoOperator(_)) {
                    classes[i] = Some(0);
                }
            }
        }
        let previous_class =
            previous.and_then(|i: usize| return_tex_class(&children[i], classes[i]));
        if classes::adjustment_class(child, classes[index]) == Some(2)
            && (matches!(previous_class, None | Some(1 | 2 | 3 | 4 | 6))
                // texclass.go isLastTeXAtom tests the physical final child.
                // Empty styles and spaces still keep a preceding BIN internal.
                || index + 1 == children.len())
        {
            classes[index] = Some(0);
        } else if matches!(
            classes::adjustment_class(child, classes[index]),
            Some(3 | 5 | 6)
        ) && previous_class == Some(2)
        {
            classes[previous.unwrap()] = Some(0);
        }
        previous = Some(index);
    }
    classes
}
fn tex_class_spacing(left: Option<usize>, right: Option<usize>, script: bool) -> f64 {
    const TABLE: [[i8; 8]; 8] = [
        [0, -1, 2, 3, 0, 0, 0, 1],
        [-1, -1, 0, 3, 0, 0, 0, 1],
        [2, 2, 0, 0, 2, 0, 0, 2],
        [3, 3, 0, 0, 3, 0, 0, 3],
        [0, 0, 0, 0, 0, 0, 0, 0],
        [0, -1, 2, 3, 0, 0, 0, 1],
        [1, 1, 0, 1, 1, 1, 1, 1],
        [1, -1, 2, 3, 1, 0, 1, 1],
    ];
    let (Some(left), Some(right)) = (left, right) else {
        return 0.0;
    };
    if left > 7 || right > 7 {
        return 0.0;
    }
    let space = TABLE[left][right];
    if script && space >= 0 {
        return 0.0;
    }
    match space.abs() {
        1 => 1000.0 / 6.0,
        2 => 2000.0 / 9.0,
        3 => 5000.0 / 18.0,
        _ => 0.0,
    }
}

// Original internal/svg/stretch.go: top/bottom parts and clipped extender.
fn render_radical_assembly(svg: &mut String, item: PositionedGlyph, total: f64, depth: f64) {
    let height = total - depth;
    let [top, extender, bottom] = radical_glyphs::ASSEMBLY_PARTS;
    let width = 1.056;
    // Original svg/stretch.go stretchVertical: wrapper.place on the mo,
    // standardSVG's scale only for RScale != 1, then placeChar paths whose
    // own place() transform follows their d attribute.
    let (placed_x, placed_y) = (root_number(item.x), root_number(item.baseline));
    let mut transform = if placed_x == "0" && placed_y == "0" {
        String::new()
    } else {
        format!("translate({placed_x},{placed_y})")
    };
    if item.scale != 1.0 && number(item.scale) != "1" {
        transform = format!("{transform} scale({})", number(item.scale))
            .trim()
            .to_owned();
    }
    if transform.is_empty() {
        svg.push_str("<g data-mml-node=\"mo\">");
    } else {
        svg.push_str(&format!(
            "<g data-mml-node=\"mo\" transform=\"{transform}\">"
        ));
    }
    for (code, glyph, y) in [
        (0xE001, top, height - top.height),
        (0x23B7, bottom, bottom.depth - depth),
    ] {
        let (x, y) = (
            root_number((width - glyph.width) * 500.0),
            root_number(y * 1000.0),
        );
        let place = if x == "0" && y == "0" {
            String::new()
        } else {
            format!(" transform=\"translate({x},{y})\"")
        };
        svg.push_str(&format!(
            "<path data-c=\"{:X}\" d=\"M{}Z\"{place}></path>",
            code, glyph.path
        ));
    }
    let top_extent = (top.height + top.depth - 0.1).max(0.0);
    let bottom_extent = (bottom.height + bottom.depth - 0.1).max(0.0);
    let extent = total - top_extent - bottom_extent;
    if extent > 0.0 && extender.height + extender.depth != 0.0 {
        let stretch = 1.5 * extent / (extender.height + extender.depth);
        let y = (stretch * (extender.height - extender.depth) - extent) / 2.0;
        svg.push_str(&format!("<svg width=\"{}\" height=\"{}\" y=\"{}\" x=\"{}\" viewBox=\"0 {} {} {}\"><path data-c=\"E000\" d=\"M{}Z\" transform=\"scale(1,{})\"></path></svg>",
            root_number(extender.width*1000.0), root_number(extent*1000.0),
            root_number((bottom_extent-depth)*1000.0), root_number((width-extender.width)*500.0),
            root_number(y*1000.0), root_number(extender.width*1000.0), root_number(extent*1000.0),
            extender.path, number(stretch)));
    }
    svg.push_str("</g>");
}

// Original wrapper.place/standardSVG retain local coordinates and RScale.
// Returning to the parent's frame before fixed() preserves Source rounding;
// multiplying rounded world coordinates is observably different for scripts.
fn layout_in_parent_frame(layout: &Layout, scale: f64) -> Layout {
    Layout {
        bounds_em: layout.bounds_em.map(|(h, d)| (h / scale, d / scale)),
        width_em: layout.width_em.map(|w| w / scale),
        intrinsic_width_em: None,
        width: layout.width / scale,
        ascent: layout.ascent / scale,
        depth: layout.depth / scale,
        glyphs: layout
            .glyphs
            .iter()
            .map(|glyph| PositionedGlyph {
                x: glyph.x / scale,
                baseline: glyph.baseline / scale,
                scale: glyph.scale / scale,
                ..*glyph
            })
            .collect(),
        rules: layout
            .rules
            .iter()
            .map(|(x, y, w, h)| (x / scale, y / scale, w / scale, h / scale))
            .collect(),
        groups: layout
            .groups
            .iter()
            .map(|group| LayoutGroup {
                x_frame: group.x_frame.map(|(x, frame)| (x, frame / scale)),
                data_cramped: group.data_cramped,
                own_translate: group.own_translate,
                accent_offset: group.accent_offset,
                box_border: group.box_border.map(|v| v / scale),
                reference: group.reference.clone(),
                tagged: group.tagged.as_ref().map(|t| ams::copy_tagged(t, scale)),
                percentage: group.percentage.clone(),
                table_line: group.table_line,
                mathsize: group.mathsize,
                tex_class: group.tex_class,
                before_glyph: group.before_glyph,
                scale: group.scale / scale,
                x: group
                    .x_frame
                    .map_or(group.x / scale, |(x, frame)| x * (frame / scale)),
                baseline: group.baseline / scale,
                color: group.color.clone(),
                kind: group.kind,
                delimiter: group.delimiter,
                arrow: group.arrow,
                background: group
                    .background
                    .as_ref()
                    .map(|(c, w, h, d)| (c.clone(), w / scale, h / scale, d / scale)),
                body: Box::new(layout_in_parent_frame(&group.body, scale)),
                lines: group
                    .lines
                    .iter()
                    .map(|(x1, y1, x2, y2, w)| {
                        (x1 / scale, y1 / scale, x2 / scale, y2 / scale, w / scale)
                    })
                    .collect(),
            })
            .collect(),
    }
}

// Original svg/wrapper.go placeChar: preserve a compound group, use the same
// variant for children, and advance by their returned widths; parent width is
// the source compound entry, not the measured painted run.
fn render_compound_character(mut item: PositionedGlyph, code: u32, x: f64, y: f64) -> String {
    if compound(item.codepoint).is_some_and(|g| g.large.is_some()) {
        item.calligraphic = false;
        item.italic = false;
        item.variant = false;
    }

    let (placed_x, placed_y) = (root_number(x * 1000.0), root_number(y * 1000.0));
    let place = if placed_x == "0" && placed_y == "0" {
        String::new()
    } else {
        format!(" transform=\"translate({placed_x},{placed_y})\"")
    };
    let content = if let Some(content) = declaration_fonts::content(code, item.font_variant) {
        Some(content)
    } else if item.bold {
        double_struck::bold(code)
            .map(|r| r.2)
            .filter(|s| !s.is_empty())
    } else {
        compound(code).map(|g| g.content)
    };
    if let Some(content) = content {
        let mut svg = format!("<g data-c=\"{code:X}\"{place}>");
        let mut offset = 0.0;
        for ch in content.chars() {
            let (paint, bold) = if item.bold {
                double_struck::bold_child(ch as u32)
            } else {
                (ch as u32, false)
            };
            let child = PositionedGlyph {
                codepoint: paint,
                bold,
                ..item
            };
            svg.push_str(&render_compound_character(child, paint, offset, y));
            offset += glyph_for(child).expect("source compound child").width;
        }
        svg.push_str("</g>");
        return svg;
    }
    let child = PositionedGlyph {
        codepoint: code,
        ..item
    };
    let g = glyph_for(child).expect("source character");
    let path = if g.path.is_empty() {
        String::new()
    } else {
        format!("M{}Z", g.path)
    };
    format!("<path data-c=\"{code:X}\" d=\"{path}\"{place}></path>")
}

fn painted_code(item: PositionedGlyph) -> u32 {
    if (item.italic || item.calligraphic)
        && (!item.calligraphic
            || calligraphic_glyphs::CALLIGRAPHIC_GLYPHS
                .binary_search_by_key(&item.codepoint, |g| g.codepoint)
                .is_err())
        && italic_glyphs::ITALIC_GLYPHS
            .binary_search_by_key(&item.codepoint, |g| g.codepoint)
            .is_err()
    {
        italic_font_code(item.codepoint)
    } else {
        item.codepoint
    }
}

fn render_layout_group(svg: &mut String, group: &LayoutGroup, parent_kind: &str) {
    if group.tagged.is_some() {
        ams::render_tagged(svg, group);
        return;
    }
    if group.kind == "#array-frame-lines" {
        for (x1, y1, x2, y2, width) in &group.lines {
            svg.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-width=\"{}\"></line>",
                root_number(*x1 / group.scale),
                root_number(*y1 / group.scale),
                root_number(*x2 / group.scale),
                root_number(*y2 / group.scale),
                root_number(*width / group.scale)
            ));
        }
        return;
    }
    if let Some(line) = group.table_line {
        let style = if line.dashed { "dashed" } else { "solid" };
        if line.frame {
            svg.push_str(&format!("<rect data-frame=\"true\" class=\"mjx-{style}\" width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\"></rect>", root_number(line.x2.unwrap() - line.x), root_number(line.y2 - line.y1), root_number(line.x), root_number(line.y1)));
        } else {
            svg.push_str(&format!("<line data-line=\"{}\" class=\"mjx-{style}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"></line>", if line.x2.is_some() { "h" } else { "v" }, root_number(line.x), root_number(line.y1), root_number(line.x2.unwrap_or(line.x)), root_number(line.y2)));
        }
        return;
    }
    if group.kind == "#inferred-place" {
        return;
    }
    // Accent/stretch character plans can retain an identity frame solely
    // for their selected delimiter glyph. It is not another SVG element.
    if group.kind.is_empty()
        && parent_kind == "mo"
        && group.x == 0.0
        && group.baseline == 0.0
        && group.scale == 1.0
    {
        if let Some(delimiter) = group.delimiter {
            cd::render_delimiter(svg, delimiter);
            return;
        }
    }
    let mut local =
        layout_in_parent_frame(&group.body, group.scale / group.mathsize.unwrap_or(1.0));
    let offsets: Vec<_> = local
        .groups
        .iter()
        .filter(|g| g.kind == "#inferred-place")
        .map(|g| (g.x, g.baseline))
        .collect();
    local.groups.retain(|g| g.kind != "#inferred-place");
    let inherited = offsets
        .iter()
        .rev()
        .map(|(x, y)| format!(" translate({},{})", root_number(*x), root_number(*y)))
        .collect::<String>();
    let color = group
        .color
        .as_ref()
        .map(|c| format!(" fill=\"{}\" stroke=\"{}\"", escape(c), escape(c)))
        .unwrap_or_default();
    // moToSVG SetAttr replaces standardSVG's scale when mathaccent installs
    // its own centering translation; place() then prepends its translation.
    let vcenter = group.kind == "TeXAtom" && group.tex_class == 8;
    // moToSVG installs the fixed variant's centering transform on its own
    // token element, replacing standardSVG's scale when the offset is nonzero.
    let fixed_fence_offset = group
        .delimiter
        .filter(|d| d.fixed && d.font_variant.is_some())
        .map(|d| d.height);
    // standardSVG emits scale only for RScale != 1 (see glyph placement).
    let scaling = if group.scale == 1.0
        || number(group.scale) == "1"
        || group.own_translate.is_some()
        || group.accent_offset.is_some()
        || vcenter
        || fixed_fence_offset.is_some_and(|v| v != 0.0)
    {
        String::new()
    } else {
        format!(" scale({})", number(group.scale))
    };
    let tex_class = if group.kind == "TeXAtom" {
        format!(
            " data-mjx-texclass=\"{}\"",
            [
                "ORD", "OP", "BIN", "REL", "OPEN", "CLOSE", "PUNCT", "INNER", "VCENTER"
            ][group.tex_class]
        )
    } else {
        String::new()
    };
    let bracket = matches!(group.kind, "#bracket-over" | "#bracket-under");
    let placed_x = group.x
        + if bracket {
            (group.box_border.unwrap() / group.scale).max(0.)
        } else {
            0.
        };
    // mtable explicitly calls wrapper.place on rows and cells. The source
    // tests numeric truthiness before fixed(), retaining tiny translations.
    let (placed_x, placed_y) = (root_number(placed_x), root_number(group.baseline));
    let enclose_block =
        group.kind.is_empty() && matches!(parent_kind, "menclose" | "#array-menclose-zero");
    let position = if enclose_block {
        // menclose creates its anonymous block directly, with this exact
        // separator and a positive-left test rather than wrapper.place.
        if group.x > 0.0 {
            format!("translate({placed_x}, {placed_y})")
        } else {
            String::new()
        }
    } else if placed_x == "0"
        && placed_y == "0"
        && !(matches!(group.kind, "mtr" | "mtd") && (group.x != 0.0 || group.baseline != 0.0))
    {
        String::new()
    } else {
        format!("translate({placed_x},{placed_y})")
    };
    let mut placement = format!("{position}{inherited}{scaling}").trim().to_owned();
    if let Some(offset) = fixed_fence_offset.filter(|v| *v != 0.0) {
        let own = format!("translate(0 {})", root_number(offset));
        placement = if placement.is_empty() {
            own
        } else {
            format!("{placement} {own}")
        };
    }
    if let Some(offset) = group.own_translate {
        let own = format!("translate(0 {})", root_number(offset));
        placement = if placement.is_empty() {
            own
        } else {
            format!("{placement} {own}")
        };
    }
    if let Some(offset) = group.accent_offset {
        let own = format!("translate({} 0)", root_number(offset));
        placement = if placement.is_empty() {
            own
        } else {
            format!("{placement} {own}")
        };
    }
    if vcenter {
        let delta = (local.ascent + local.depth) / 2. + 250. - local.ascent;
        let own = format!("translate(0 {})", root_number(delta));
        placement = if placement.is_empty() {
            own
        } else {
            format!("{placement} {own}")
        };
    }
    let transform = if placement.is_empty() {
        String::new()
    } else {
        format!(" transform=\"{}\"", placement)
    };
    if let Some(href) = &group.reference {
        svg.push_str(&format!("<a href=\"{}\">", escape(href)));
    }
    // amsHandleReference attaches MathJax_ref; SVG addElementClass retains
    // the initial space when installing the first class on the wrapper.
    let reference_class = if group.reference.is_some() {
        " class=\" MathJax_ref\""
    } else {
        ""
    };
    if group.kind.is_empty() {
        svg.push_str(&format!("<g{}{}>", color, transform));
    } else {
        let border_style = if bracket {
            brackets::border_style(
                group.box_border.unwrap() / group.scale,
                group.kind == "#bracket-under",
            )
        } else {
            String::new()
        };
        // standardSVG installs relative scale before color and attributes;
        // parent place updates that transform in its original attribute slot.
        let table_attr = if group.kind == "#array-menclose-zero" {
            " data-padding=\"0\""
        } else if group.kind == "#padded-left" {
            " data-align=\"left\""
        } else if group.kind == "#padded-right" {
            " data-align=\"right\""
        } else if group.kind == "#padded-center" {
            " data-align=\"center\""
        } else if group
            .percentage
            .as_ref()
            .is_some_and(|t| t.width_includes_label)
        {
            " data-width-includes-label=\"true\""
        } else {
            ""
        };
        let attrs = if !table_attr.is_empty() && group.scale != 1.0 {
            format!("{transform}{color}{table_attr}")
        } else if !table_attr.is_empty() {
            format!("{color}{table_attr}{transform}")
        } else if let Some(cramped) = group.data_cramped {
            // standardSVG installs scale before handleAttributes copies the
            // explicit data-cramped; a later place() transform follows it.
            let cramped = format!(" data-cramped=\"{cramped}\"");
            if scaling.is_empty() {
                format!("{color}{cramped}{transform}")
            } else {
                format!("{transform}{color}{cramped}")
            }
        } else if !scaling.is_empty() {
            format!("{border_style}{transform}{color}{reference_class}{tex_class}")
        } else {
            format!("{reference_class}{color}{border_style}{tex_class}{transform}")
        };
        svg.push_str(&format!(
            "<g data-mml-node=\"{}\"{attrs}>",
            if bracket || group.kind.starts_with("#padded-") {
                "mpadded"
            } else if group.kind == "#array-menclose-zero" {
                "menclose"
            } else {
                group.kind
            }
        ));
    }
    if group.reference.is_some() {
        svg.push_str(&format!("<rect data-hitbox=\"true\" fill=\"none\" stroke=\"none\" pointer-events=\"all\" width=\"{}\" height=\"{}\" y=\"{}\"></rect>",root_number(local.width),root_number(local.ascent+local.depth),root_number(-local.depth)));
    }
    if let Some((color, width, ascent, depth)) = &group.background {
        svg.push_str(&format!("<rect fill=\"{}\" x=\"0\" y=\"{}\" width=\"{}\" height=\"{}\" data-bgcolor=\"true\"></rect>",
            escape(color), root_number(-depth/group.scale), root_number(*width/group.scale), root_number((ascent+depth)/group.scale)));
    }
    // Render the retained child scope using the same native primitive renderer;
    // its outer viewport is omitted because mstyle/menclose are not SVG viewports.
    if let Some(delimiter) = group.delimiter {
        cd::render_delimiter(svg, delimiter);
    }
    if bracket {
        brackets::render(
            svg,
            &local,
            group.box_border.unwrap() / group.scale,
            group.kind == "#bracket-under",
        );
    }
    // Token wrappers own the MathML node. Their children are character
    // primitives, as wrapper.textToSVG/placeChar emit directly into that node.
    let child_kind =
        if group.kind.is_empty() && matches!(parent_kind, "mi" | "mn" | "mo" | "mtext" | "text") {
            parent_kind
        } else {
            group.kind
        };
    let child = render_layout_in_parent(&local, child_kind);
    let marker = "<g data-mml-node=\"math\">";
    let start = child.find(marker).expect("native math root") + marker.len();
    let end = child.len() - "</g></g></svg>".len();
    svg.push_str(&child[start..end]);
    for (x1, y1, x2, y2, width) in &group.lines {
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-width=\"{}\"></line>",
            root_number(*x1 / group.scale),
            root_number(*y1 / group.scale),
            root_number(*x2 / group.scale),
            root_number(*y2 / group.scale),
            root_number(*width / group.scale)
        ));
    }
    if let Some(t) = group.box_border.filter(|_| !bracket) {
        let t = t / group.scale;
        svg.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke-width=\"{}\"></rect>",root_number(t/2.0),root_number(t/2.0-local.depth),root_number(local.width-t),root_number(local.ascent+local.depth-t),root_number(t)));
    }
    if let Some(arrow) = group.arrow {
        enclose::render_arrow(svg, arrow);
    }
    svg.push_str("</g>");
    if group.reference.is_some() {
        svg.push_str("</a>");
    }
}

fn render_layout(layout: &Layout) -> String {
    render_layout_in_parent(layout, "math")
}

fn render_layout_in_parent(layout: &Layout, parent_kind: &str) -> String {
    let character_parent = matches!(parent_kind, "mi" | "mn" | "mo" | "mtext" | "text");
    // Original wrapper.computeChildrenBBox uses BBox.Append: signed logical
    // advances determine width, including negative kerns and deferred paint.
    let width = layout.width;
    let ascent = layout.ascent;
    let depth = layout.depth;
    // Preserve the emitted intrinsic coordinate system independently of measurement.
    // Original Typesetter.Typeset clamps only the final viewport to
    // options.Em/1000 em (default Em=16), preserving the logical baseline.
    let (ascent_em, depth_em) = if styles::has_em_bounds(layout) {
        styles::viewport_bounds(layout)
    } else {
        (ascent / 1000., depth / 1000.)
    };
    let (view_width, view_height, x_scale, y_scale) = (
        width.max(16.0),
        ((ascent_em + depth_em).max(0.016)) * 1000.0,
        1.0,
        1.0,
    );
    // MathJax serializes its TeX em units using a 442-unit ex conversion.
    let width_ex = view_width / 442.0;
    let height_ex = (ascent_em + depth_em).max(0.016) / 0.442;
    // svg/typesetter.go serializes the retained em BBox in SVG units once.
    // Reusing paint-frame bounds can round declaration scales a second time.
    let scaled_depth = depth_em * 1000.;
    let scaled_ascent = ascent_em * 1000.;
    let mut svg = format!(
        "<svg style=\"vertical-align: {};\" xmlns=\"http://www.w3.org/2000/svg\" width=\"{}ex\" height=\"{}ex\" role=\"img\" focusable=\"false\" viewBox=\"0 {} {} {}\"><g stroke=\"currentColor\" fill=\"currentColor\" stroke-width=\"0\" transform=\"scale(1,-1)\"><g data-mml-node=\"math\">",
        if scaled_depth == 0.0 {
            "0".to_owned()
        } else {
            format!("{}ex", number(-scaled_depth / 442.0))
        },
        number(width_ex),
        number(height_ex),
        root_number(-scaled_ascent),
        root_number(view_width),
        root_number(view_height)
    );
    if ams::responsive(layout) {
        svg = format!(
            "<svg style=\"vertical-align: {}; min-width: {}ex;\" xmlns=\"http://www.w3.org/2000/svg\" width=\"100%\" height=\"{}ex\" role=\"img\" focusable=\"false\"><g stroke=\"currentColor\" fill=\"currentColor\" stroke-width=\"0\" transform=\"scale(0.0181,-0.0181) translate(0, {})\"><g data-mml-node=\"math\">",
            if depth == 0.0 {
                "0".into()
            } else {
                format!("{}ex", number(-depth / 442.0))
            },
            number(math_minwidth::minimum(layout).unwrap_or(view_width) / 442.0),
            number(height_ex),
            root_number(-ascent)
        );
    }
    if x_scale != 1.0 || y_scale != 1.0 {
        svg.push_str(&format!(
            "<g transform=\"scale({} {})\">",
            number(x_scale),
            number(y_scale)
        ));
    }
    let inferred: Vec<_> = layout
        .groups
        .iter()
        .filter(|g| g.kind == "#inferred-place")
        .collect();
    for group in inferred.iter().rev() {
        svg.push_str(&format!(
            "<g transform=\"translate({},{})\">",
            root_number(group.x),
            root_number(group.baseline)
        ));
    }
    {
        let mut glyph_index = 0;
        while glyph_index < layout.glyphs.len() {
            for group in layout
                .groups
                .iter()
                .filter(|g| g.before_glyph == glyph_index)
            {
                render_layout_group(&mut svg, group, parent_kind);
            }
            let item = layout.glyphs[glyph_index];
            if let Some((total, depth)) = item.assembly_height {
                render_radical_assembly(&mut svg, item, total, depth);
                glyph_index += 1;
                continue;
            }
            // Script elements are emitted by their owning LayoutGroup. Glyph
            // coordinates alone do not introduce a second MathML script node.
            let item = layout.glyphs[glyph_index];
            let Some(glyph) = glyph_for(item) else {
                glyph_index += 1;
                continue;
            };
            let path = if glyph.path.is_empty() {
                String::new()
            } else {
                format!("M{}Z", glyph.path)
            };
            let node = mathml_node_kind(item.codepoint);
            // Original svg/mo.go moToSVG: a symmetric/largeop mo installs its
            // own "translate(0 u)" (u = centerOffset of its children bbox,
            // replacing standardSVG's scale), and wrapper.place prepends the
            // parent placement, which excludes u.
            let axis_u = if (item.large || item.small_operator) && !character_parent {
                Some((glyph.height + glyph.depth) / 2.0 + 0.25 - glyph.height).filter(|u| *u != 0.0)
            } else {
                None
            };
            let placed_baseline = item.baseline - axis_u.map_or(0.0, |u| u * 1000.0 * item.scale);
            let (placed_x, placed_y) = (root_number(item.x), root_number(placed_baseline));
            let position = if placed_x == "0" && placed_y == "0" {
                String::new()
            } else {
                format!("translate({placed_x},{placed_y})")
            };
            // standardSVG emits scale only for RScale != 1; source script
            // scales are exact powers clamped to 8px, so a ratio that
            // serializes as 1 is the source's exact 1.
            let own_scale = if let Some(u) = axis_u {
                format!("translate(0 {})", root_number(u * 1000.0))
            } else if item.scale == 1.0 || number(item.scale) == "1" {
                String::new()
            } else {
                format!("scale({})", number(item.scale))
            };
            let transform = format!("{position} {own_scale}").trim().to_owned();
            let transform_attribute = if transform.is_empty() {
                String::new()
            } else {
                format!(" transform=\"{transform}\"")
            };
            // Known spacing glyphs also have no path. They remain blank;
            // only failed font lookup uses the unknown-text fallback.
            let text_fallback = if item.calligraphic {
                calligraphic_glyphs::CALLIGRAPHIC_GLYPHS
                    .binary_search_by_key(&item.codepoint, |g| g.codepoint)
                    .is_err()
                    && italic_glyphs::ITALIC_GLYPHS
                        .binary_search_by_key(&item.codepoint, |g| g.codepoint)
                        .is_err()
                    && uses_normal_text_fallback(italic_font_code(item.codepoint))
            } else if item.italic {
                italic_glyphs::ITALIC_GLYPHS
                    .binary_search_by_key(&item.codepoint, |g| g.codepoint)
                    .is_err()
                    && uses_normal_text_fallback(italic_font_code(item.codepoint))
            } else {
                uses_normal_text_fallback(item.codepoint)
            };
            // A Unit is an explicit node, not a character child of a token.
            let character_child = character_parent && !item.unit;
            let content = if declaration_fonts::content(item.codepoint, item.font_variant).is_some()
                || compound(item.codepoint).is_some()
                || item.bold && double_struck::bold(item.codepoint).is_some_and(|r| !r.2.is_empty())
            {
                render_compound_character(
                    item,
                    item.codepoint,
                    if character_child {
                        item.x / 1000.0
                    } else {
                        0.0
                    },
                    if character_child {
                        item.baseline / 1000.0
                    } else {
                        0.0
                    },
                )
            } else if glyph.path.is_empty() && text_fallback {
                let text_transform = if character_child && !transform_attribute.is_empty() {
                    format!("{transform} scale(1,-1)")
                } else {
                    "scale(1,-1)".into()
                };
                let code = painted_code(item);
                let encoded_style = (0x1D400..=0x1D7FF).contains(&code);
                format!(
                    "<text data-variant=\"{}\" transform=\"{text_transform}\" font-size=\"884px\"{}{}>{}</text>",
                    if item.large {
                        "-largeop"
                    } else if item.small_operator {
                        "-smallop"
                    } else if item.calligraphic {
                        "-tex-calligraphic"
                    } else if item.italic {
                        "italic"
                    } else if item.font_original.is_some() {
                        item.font_variant
                    } else {
                        "normal"
                    },
                    if encoded_style {
                        String::new()
                    } else {
                        format!(
                            " font-family=\"{}\"",
                            if matches!(item.font_variant, "monospace" | "sans-serif") {
                                item.font_variant
                            } else if matches!(
                                item.font_variant,
                                "bold-sans-serif" | "sans-serif-italic" | "sans-serif-bold-italic"
                            ) {
                                "sans-serif"
                            } else if item.calligraphic
                                || matches!(
                                    item.font_variant,
                                    "script" | "bold-script" | "-tex-bold-calligraphic"
                                )
                            {
                                "cursive"
                            } else {
                                "serif"
                            }
                        )
                    },
                    if encoded_style {
                        ""
                    } else {
                        match (
                            item.italic
                                || item.calligraphic
                                || matches!(
                                    item.font_variant,
                                    "bold-italic"
                                        | "-tex-mathit"
                                        | "sans-serif-italic"
                                        | "sans-serif-bold-italic"
                                        | "-tex-bold-calligraphic"
                                ),
                            item.font_original.is_some()
                                && matches!(
                                    item.font_variant,
                                    "bold"
                                        | "bold-italic"
                                        | "double-struck"
                                        | "bold-fraktur"
                                        | "bold-script"
                                        | "bold-sans-serif"
                                        | "sans-serif-bold-italic"
                                        | "-tex-bold-calligraphic"
                                ),
                        ) {
                            (true, true) => " font-style=\"italic\" font-weight=\"bold\"",
                            (true, false) => " font-style=\"italic\"",
                            (false, true) => " font-weight=\"bold\"",
                            _ => "",
                        }
                    },
                    escape_text(&char::from_u32(code).unwrap().to_string())
                )
            } else {
                let code = painted_code(item);
                format!(
                    "<path data-c=\"{:X}\" d=\"{}\"{}></path>",
                    code,
                    path,
                    if character_child {
                        transform_attribute.as_str()
                    } else {
                        ""
                    }
                )
            };
            if character_child {
                svg.push_str(&content);
            } else {
                svg.push_str(&format!(
                    "<g data-mml-node=\"{}\"{}{}>{}</g>",
                    node,
                    if item.unit {
                        " class=\" MathML-Unit\""
                    } else {
                        ""
                    },
                    transform_attribute,
                    content
                ));
            }
            glyph_index += 1;
        }
    }
    for group in layout
        .groups
        .iter()
        .filter(|g| g.before_glyph == layout.glyphs.len())
    {
        render_layout_group(&mut svg, group, parent_kind);
    }
    for (x, y, rule_width, rule_height) in &layout.rules {
        svg.push_str(&format!(
            "<rect width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\"></rect>",
            root_number(*rule_width),
            root_number(*rule_height),
            root_number(*x),
            root_number(*y)
        ));
    }
    for _ in &inferred {
        svg.push_str("</g>");
    }
    if x_scale != 1.0 || y_scale != 1.0 {
        svg.push_str("</g>");
    }
    svg.push_str("</g></g></svg>");
    svg
}

fn mathml_node_kind(codepoint: u32) -> &'static str {
    if (b'0' as u32..=b'9' as u32).contains(&codepoint) {
        "mn"
    } else if matches!(codepoint,0x2190..=0x2195|0x21A9|0x21AA|0x21CC|0x21D0..=0x21D5|0x27F5..=0x27FA|0x27FC)
        || [
            '+' as u32, '-' as u32, '=' as u32, '<' as u32, '>' as u32, '|' as u32, '(' as u32,
            ')' as u32, 0x27E8, 0x27E9, 0x2223, 0x2212, 0x221A, 0x220F, 0x2211, 0x222B,
            // Remaining command_codepoint targets registered by the original
            // base mathchar0mo map (tex/source_tables_base_maps.go), whose
            // ParseMethods.mathchar0mo creates an mo token.
            0xB1, 0xD7, 0x2297, 0x22C5, 0x2264, 0x2265, 0x2260, 0x2261, 0x2218, 0x2248, 0x223C,
            0x2208, 0x2209, 0x2026, 0x22EF,
        ]
        .contains(&codepoint)
        || is_large_operator(codepoint)
    {
        "mo"
    } else {
        "mi"
    }
}

fn validate(expression: &str) -> Result<(), MathError> {
    if expression.trim().is_empty() {
        return Err(MathError::Empty);
    }
    let mut stack = Vec::new();
    let mut escaped = false;
    for (byte, rune) in expression.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match rune {
            '\\' => escaped = true,
            '{' => stack.push(byte),
            '}' if stack.pop().is_none() => {
                return Err(MathError::Unbalanced {
                    delimiter: '}',
                    byte,
                });
            }
            _ => {}
        }
    }
    if let Some(byte) = stack.pop() {
        return Err(MathError::Unbalanced {
            delimiter: '{',
            byte,
        });
    }
    Ok(())
}

// Original svg/dom.go uses distinct character-data and attribute escaping.
fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}

// Original root viewBox uses CommonOutputJax.fixed with one decimal.
fn root_number(value: f64) -> String {
    // jscompat.ToFixed rounds the exact binary number, resolving ties upward.
    if value.abs() < 0.0006 {
        return "0".to_owned();
    }
    let magnitude = value.abs();
    let bits = magnitude.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let mantissa = ((bits & ((1_u64 << 52) - 1)) | (1_u64 << 52)) as u128;
    let numerator = mantissa * 10;
    let rounded = if exponent >= 0 {
        numerator.checked_shl(exponent as u32)
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            Some(0)
        } else {
            let quotient = numerator >> shift;
            let remainder = numerator & ((1_u128 << shift) - 1);
            Some(quotient + u128::from(remainder >= 1_u128 << (shift - 1)))
        }
    };
    let Some(rounded) = rounded else {
        return format!("{value:.1}");
    };
    let sign = if value < 0.0 { "-" } else { "" };
    if rounded % 10 == 0 {
        format!("{sign}{}", rounded / 10)
    } else {
        format!("{sign}{}.{}", rounded / 10, rounded % 10)
    }
}

fn number(value: f64) -> String {
    let mut output = format!("{value:.3}");
    while output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{measure, render};
    use d2_rs_core::Size;

    #[test]
    fn linebreak_mspace_keeps_operator_spacing_on_the_following_token() {
        // Pinned mathjax-go 94d80186bfd7 svg/texclass.go setTeXClass returns
        // the previous node for mspace; wrapper.go addChildren places the
        // zero-width break before the following token's leading space.
        // These transforms are captured from the pinned Go D2 SVG renderer.
        for (expression, spaces, last_identifier) in [
            (r"= \\ b", vec!["translate(778,0)"], "translate(1055.8,0)"),
            (
                r"a = \\ b",
                vec!["translate(1584.8,0)"],
                "translate(1862.6,0)",
            ),
            (
                r"a + \\ b",
                vec!["translate(1529.2,0)"],
                "translate(1751.4,0)",
            ),
            (
                r"a < \\ b",
                vec!["translate(1584.8,0)"],
                "translate(1862.6,0)",
            ),
            (
                r"a = \\b",
                vec!["translate(1584.8,0)"],
                "translate(1862.6,0)",
            ),
            (r"a, \\ b", vec!["translate(807,0)"], "translate(973.7,0)"),
            (r"a; \\ b", vec!["translate(807,0)"], "translate(973.7,0)"),
            (
                r"a = \newline b",
                vec!["translate(1584.8,0)"],
                "translate(1862.6,0)",
            ),
            (
                r"a = \\[2pt] b",
                vec!["translate(1584.8,0)", "translate(1584.8,0)"],
                "translate(1862.6,0)",
            ),
            (r"a \\ b", vec!["translate(529,0)"], "translate(529,0)"),
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let actual = document
                .descendants()
                .filter(|node| node.attribute("data-mml-node") == Some("mspace"))
                .map(|node| node.attribute("transform").unwrap_or(""))
                .collect::<Vec<_>>();
            assert_eq!(actual, spaces, "break placement for {expression:?}");
            let identifier = document
                .descendants()
                .rfind(|node| node.attribute("data-mml-node") == Some("mi"))
                .unwrap();
            assert_eq!(
                identifier.attribute("transform"),
                Some(last_identifier),
                "following token placement for {expression:?}"
            );
        }
        let svg = super::render_exact(r"a + \\").unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let spacer = document
            .descendants()
            .find(|node| node.attribute("data-mml-node") == Some("mspace"))
            .unwrap();
        assert_eq!(spacer.attribute("transform"), Some("translate(1529.2,0)"));

        // Pinned svg/wrapper.go addChildren accumulates em advances before
        // place converts to SVG units. A size declaration closed by a break
        // must preserve that order in fenced rows.
        for (expression, kind, placement) in [
            (r"\left(\small a \\ b \right)", "mo", "translate(1267.6,0)"),
            (
                r"\left(\small a \newline b \right)",
                "mo",
                "translate(1267.6,0)",
            ),
            // texclass.go setRowTeXClass returns the prepared trailing BIN
            // (demoted by isLastTeXAtom) through the closed declaration.
            (r"\left(\small a + \\ b\right)", "mi", "translate(1499.9,0)"),
            (
                r"\left(\scriptstyle a + \\ b\right)",
                "mi",
                "translate(1313.2,0)",
            ),
            (
                r"\left(\scriptscriptstyle a + \\ b\right)",
                "mi",
                "translate(1042.5,0)",
            ),
            (
                r"\left(\displaystyle a + \\ b\right)",
                "mi",
                "translate(1696,0)",
            ),
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let last = document
                .descendants()
                .rfind(|node| node.attribute("data-mml-node") == Some(kind))
                .unwrap();
            assert_eq!(last.attribute("transform"), Some(placement), "{expression}");
        }

        // parser.go pendingCell closes StyleItems before an open owner emits
        // spaces. The lexical environment survives, but mstyle does not.
        // wrapper.go addChildren retains local em positions inside scripts;
        // scaling and then dividing 449.65 changes fixed()'s rounding tie.
        for expression in [r"x^{\small a \\ b}", r"x^{\small a \newline b}"] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let spacer = document
                .descendants()
                .find(|node| node.attribute("data-mml-node") == Some("mspace"))
                .unwrap();
            assert_eq!(
                spacer.attribute("transform"),
                Some("translate(449.6,0)"),
                "{expression}"
            );
            let identifier = document
                .descendants()
                .rfind(|node| node.attribute("data-mml-node") == Some("mi"))
                .unwrap();
            assert_eq!(
                identifier.attribute("transform"),
                Some("translate(449.6,0)"),
                "{expression}"
            );
        }
        for (expression, spacer_x, token_x) in [
            (r"\displaystyle a = \\ b", "1584.8", "1862.6"),
            (r"\scriptstyle a = \\ b", "924.2", "924.2"),
            (r"\displaystyle a \\ b", "529", "529"),
            (r"\scriptstyle a \\ b", "374.1", "374.1"),
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let math = document
                .descendants()
                .find(|node| node.attribute("data-mml-node") == Some("math"))
                .unwrap();
            let children: Vec<_> = math.children().filter(|node| node.is_element()).collect();
            assert_eq!(
                children
                    .iter()
                    .map(|node| node.attribute("data-mml-node").unwrap())
                    .collect::<Vec<_>>(),
                ["mstyle", "mspace", "mi"],
                "style closure for {expression:?}"
            );
            assert_eq!(
                children[1].attribute("transform").unwrap(),
                format!("translate({spacer_x},0)")
            );
            assert_eq!(
                children[2].attribute("transform").unwrap(),
                format!("translate({token_x},0)")
            );
        }
        // wrapper.go getScale and BBox.Append scale the intrinsic em width.
        // Reconstructing it from paint units changes mfrac.go alignX's tie.
        for expression in [
            r"\frac{\scriptscriptstyle a + \\ b}{c}",
            r"\frac{\scriptscriptstyle a + \newline b}{c}",
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let denominator = document
                .descendants()
                .find(|node| node.attribute("data-mml-node") == Some("mfrac"))
                .unwrap()
                .children()
                .filter(|node| node.is_element())
                .nth(1)
                .unwrap();
            assert_eq!(
                denominator.attribute("transform"),
                Some("translate(544.7,-686)"),
                "{expression}"
            );
        }
        let svg = super::render_exact(r"a + \color{red}").unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        let operator = document
            .descendants()
            .find(|node| node.attribute("data-mml-node") == Some("mo"))
            .unwrap();
        assert_eq!(operator.attribute("transform"), Some("translate(751.2,0)"));

        // svg/texclass.go setPreviousClass reads the node returned from a
        // transparent wrapper. A raised style returns its final script-level
        // token, so the following identifier retains that level for spacing.
        // adjustTeXClass on a following mo instead uses its own inherited
        // level. Keep the operator controls at their Go positions as well.
        for position in [r"\raise1em", r"\lower1em"] {
            for (style, identifier_x, plus_x, after_operator_x) in [
                (r"\scriptstyle", "924.2", "1202", "1980"),
                (r"\scriptscriptstyle", "653.5", "931.3", "1709.3"),
                (r"\displaystyle", "1862.6", "1862.6", "2640.6"),
            ] {
                for (tail, token_x) in [
                    ("b", identifier_x),
                    ("+ b", after_operator_x),
                    ("= b", after_operator_x),
                ] {
                    let expression = format!(r"{position}{style} a = \\ {tail}");
                    let svg = super::render_exact(&expression).unwrap();
                    let document = roxmltree::Document::parse(&svg).unwrap();
                    let identifier = document
                        .descendants()
                        .rfind(|node| node.attribute("data-mml-node") == Some("mi"))
                        .unwrap();
                    assert_eq!(
                        identifier.attribute("transform").unwrap(),
                        format!("translate({token_x},0)"),
                        "{expression}"
                    );
                    if tail == "+ b" {
                        let operator = document
                            .descendants()
                            .rfind(|node| node.attribute("data-mml-node") == Some("mo"))
                            .unwrap();
                        assert_eq!(
                            operator.attribute("transform").unwrap(),
                            format!("translate({plus_x},0)"),
                            "{expression}"
                        );
                    }
                }
            }
        }

        // tex/parser.go captures the RightItem's lexical color before
        // Style/Over/Cell continuations restore the enclosing environment.
        for expression in [
            r"\left(\color{red}x\right)",
            r"\left(\color{red}x\right.",
            r"\left(\color{red}x\over y\right)",
            r"\left(\color{red} a \over b \\ c\right)",
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let close = document
                .descendants()
                .rfind(|node| node.attribute("data-mml-node") == Some("mo"))
                .unwrap();
            assert_eq!(close.attribute("fill"), Some("red"), "{expression}");
            assert_eq!(close.attribute("stroke"), Some("red"), "{expression}");
        }
        // LeftItem retains the middle's prefix, empty CLOSE/OPEN atoms and
        // lexical color. cleanMathMLAttributes removes inherited duplicates.
        for (expression, colors, middles) in [
            (
                r"\left(\color{red}x\middle|y\right)",
                vec![None, Some("red"), None],
                1,
            ),
            (
                r"\left(\color{red}\bf x\middle|y\over z\right)",
                vec![None, Some("red"), None],
                1,
            ),
            (
                r"\color{green}\left(\color{red}x\middle|y\right)+z",
                vec![None, Some("red"), None, None],
                1,
            ),
            (
                r"\left(\color{red}\left[x\right]\right)",
                vec![None, None, None, Some("red")],
                0,
            ),
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(
                document
                    .descendants()
                    .filter(|node| node.attribute("data-mml-node") == Some("mo"))
                    .map(|node| node.attribute("fill"))
                    .collect::<Vec<_>>(),
                colors,
                "{expression}",
            );
            for class in ["OPEN", "CLOSE"] {
                assert_eq!(
                    document
                        .descendants()
                        .filter(|node| node.attribute("data-mjx-texclass") == Some(class))
                        .count(),
                    middles,
                    "{class} in {expression}",
                );
            }
        }

        // svg/wrapper.go:112 and mo.go:351 stretch delimiters in every row,
        // including rows captured by OverItems. These pinned Go viewBoxes
        // cover local fraction siblings, styles, colors and an empty middle.
        for (expression, view_box) in [
            (
                r"\left(\color{red}x\over y\middle|z\over w\right)",
                "0 -1939.9 3832.5 3379.8",
            ),
            (
                r"\left(x\over y\middle|\frac{z}{w}\right)",
                "0 -1449.5 3973 2399",
            ),
            (
                r"\left(\frac{x}{y}\middle|z\over w\right)",
                "0 -1939.9 3832.5 3379.8",
            ),
            (
                r"\scriptstyle\left(x\over y\middle|z\over w\right)",
                "0 -1040.4 2257 1711.8",
            ),
            (
                r"\left(x\over \frac{y}{z}\middle\Vert w\right)",
                "0 -1749.5 4082.5 2999",
            ),
            (
                r"\left(x\over y\middle[\frac{z}{w}\right)",
                "0 -1449.5 4168 2399",
            ),
            (
                r"\left(x\over y\middle.\frac{z}{w}\right)",
                "0 -1449.5 3640 2399",
            ),
            (r"\left(x\middle|y\right)", "0 -750 2118 1000"),
            (r"\left(x\middle\Vert y\right)", "0 -750 2618 1000"),
            (
                r"\scriptstyle\left(x\middle\Vert y\right)",
                "0 -530.3 1851.2 707.1",
            ),
            (
                r"\color{red}\left(x\middle\Vert y\right)",
                "0 -750 2618 1000",
            ),
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(
                document.root_element().attribute("viewBox"),
                Some(view_box),
                "middle stretching for {expression}",
            );
        }
        // tex/left_right.go leaves the middle's class and symmetry to the
        // dictionary. mo.go stretchBaseline retains the requested H/D for
        // asymmetric assembly instead of centering it on the math axis.
        for (expression, placement, extent) in [
            (
                r"\left(x\middle\Vert\frac{y}{z}\over w\right)",
                "translate(572,0)",
                Some("1150.3"),
            ),
            (
                r"\scriptstyle \left(x\middle\Vert\frac{y}{z}\over w\right)",
                "translate(572,0)",
                Some("1343"),
            ),
            (
                r"\left(x\over y\middle\Vert\frac{z}{w}\right)",
                "translate(1748,0)",
                Some("2009"),
            ),
            (r"\left(\middle\Vert\right)", "translate(389,0)", None),
            (
                r"\left(x\middle\uparrow\frac{y}{z}\over w\right)",
                "translate(849.8,0)",
                Some("363.3"),
            ),
            (r"\left(\middle\uparrow\right)", "translate(666.8,0)", None),
        ] {
            let svg = super::render_exact(expression).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            let middle = document
                .descendants()
                .filter(|node| node.attribute("data-mml-node") == Some("mo"))
                .nth(1)
                .unwrap();
            assert_eq!(
                middle.attribute("transform"),
                Some(placement),
                "{expression}"
            );
            assert_eq!(
                middle
                    .children()
                    .find(|node| node.has_tag_name("svg"))
                    .and_then(|node| node.attribute("height")),
                extent,
                "{expression}",
            );
        }
    }

    #[test]
    fn measures_scripts_deterministically() {
        let base = measure("a+b").unwrap();
        let scripted = measure("a^2+b_1").unwrap();
        assert!(scripted.height > base.height);
        assert_eq!(measure("a^2+b_1"), Ok(scripted));
    }

    #[test]
    fn source_metacharacter_errors_and_fraction_validation() {
        assert!(measure(r#"\frac{1}{2}"#).unwrap().width > 0.0);
        let svg = render(r#"x < y & z"#).unwrap();
        assert!(svg.contains("data-mjx-error=\"Misplaced &amp;\""));
        assert_eq!(
            measure(r#"\frac{1}{"#),
            Ok(Size {
                width: 207.0,
                height: 18.0
            })
        );
    }

    #[test]
    fn original_empty_expressions_keep_the_clamped_em_viewport() {
        // Actual original Go Render/Measure controls: empty and whitespace
        // have the same positive 16-unit viewport and one-pixel measure.
        for expression in ["", "  ", "\t", "\n"] {
            assert_eq!(
                measure(expression),
                Ok(Size {
                    width: 1.0,
                    height: 1.0
                })
            );
        }
    }

    #[test]
    fn renders_original_recoverable_fraction_argument_error() {
        // Actual Go v0.9.0 renders this parser diagnostic as merror.
        assert!(
            render(r"\frac{")
                .unwrap()
                .contains("data-mjx-error=\"Missing close brace\"")
        );
        let size = measure(r"\frac{").unwrap();
        assert_eq!((size.width, size.height), (207.0, 18.0));
    }

    #[test]
    fn renders_source_derived_glyphs_for_common_tex_constructs() {
        let fraction = render(r#"\frac{1}{2}"#).unwrap();
        assert!(fraction.contains("data-c=\"31\""));
        assert!(fraction.contains("data-c=\"32\""));
        assert!(fraction.contains("<rect "));

        let radical = render(r#"\sqrt{x+1}"#).unwrap();
        assert!(radical.contains("data-c=\"221A\""));
        assert!(radical.contains("data-c=\"1D465\""));

        let greek = render(r#"\alpha+\Omega"#).unwrap();
        assert!(greek.contains("data-c=\"1D6FC\""));
        assert!(greek.contains("data-c=\"3A9\""));

        let matrix = render(r#"\begin{pmatrix}a&b\\c&d\end{pmatrix}"#).unwrap();
        assert!(matrix.contains("data-c=\"28\""));
        assert!(matrix.contains("data-c=\"29\""));
        assert!(matrix.contains("data-c=\"1D44E\""));

        let aligned = render(r#"\begin{aligned}a&=b+c\\d&=e\end{aligned}"#).unwrap();
        assert!(aligned.contains("data-c=\"3D\""));
        assert!(aligned.contains("data-c=\"1D44E\""));
    }

    #[test]
    fn matches_frozen_v090_mathjax_dimensions() {
        let cases = [
            (r#"a + b = c"#, (72.0, 15.0)),
            (r#"\frac{1}{2}"#, (18.0, 37.0)),
            (r#"x_i^2+\sum_{n=0}^{\infty}n^{-2}"#, (99.0, 51.0)),
            (
                r#"\left\langle \frac{x}{y}\middle|z\right\rangle"#,
                (60.0, 44.0),
            ),
            (r#"\begin{pmatrix}a&b\\c&d\end{pmatrix}"#, (64.0, 44.0)),
            (r#"\begin{aligned}a&=b+c\\d&=e\end{aligned}"#, (72.0, 42.0)),
            (
                r#"\min_{\mathclap{\substack{x\in\mathbb{R}^n\\x\geq0\\Ax\leq b}}}c^Tx"#,
                (62.0, 55.0),
            ),
            (
                r#"\begin{CD}A@>f>>B\\@VgVV@VVhV\\C@>>k>D\end{CD}"#,
                (106.0, 112.0),
            ),
            (r#"\bra{\psi}A\ket{\phi}+\braket{x|y}"#, (154.0, 19.0)),
            (
                r#"\cancel{x}+\bcancel{y}+\xcancel{z}+\cancelto{0}{w}"#,
                (155.0, 26.0),
            ),
            (
                r#"f(x)=\begin{cases}x^2&x\geq0\\-x&x<0\end{cases}"#,
                (159.0, 44.0),
            ),
            (r#"\color{red}{x}+\colorbox{yellow}{y}"#, (53.0, 22.0)),
            (r#"90\degree+20\celsius+5\micro\ohm"#, (134.0, 18.0)),
            (r#"\ce{2H2 + O2 -> 2H2O}"#, (165.0, 16.0)),
            (r#"\dv{x}f(x)+\qty(\frac{a}{b})"#, (125.0, 38.0)),
            (
                r#"\Huge{\frac{\alpha g^2}{\omega^5} e^{[ -0.74\bigl\{\frac{\omega U_\omega 19.5}{g}\bigr\}^{\!-4}\,]}}"#,
                (382.0, 101.0),
            ),
            (r#"gibberish\; math:\sum_{i=0}^\infty i^2"#, (179.0, 51.0)),
            (
                r#"\min_{ \mathclap{\substack{ x \in \mathbb{R}^n \ x \geq 0 \ Ax \leq b }}} c^T x"#,
                (62.0, 32.0),
            ),
            (
                r#"\begin{equation} \label{eq1}
\begin{split}
A & = \frac{\\pi r^2}{2} \\
 & = \frac{1}{2} \pi r^2
\end{split}
\end{equation}"#,
                (82.0, 82.0),
            ),
            (
                r#"\lim_{h \rightarrow 0 } \frac{f(x+h)-f(x)}{h}"#,
                (162.0, 41.0),
            ),
            (r#"f(x) = x^2 + 2x + 1"#, (150.0, 21.0)),
            (
                r#"\var{F[g(x)]}
\dd(\cos\theta)"#,
                (128.0, 19.0),
            ),
            (
                r#"\displaylines{x = a + b \\ y = b + c}
\sum_{k=1}^{n} h_{k} \int_{0}^{1} \bigl(\partial_{k} f(x_{k-1}+t h_{k} e_{k}) -\partial_{k} f(a)\bigr) \,dt"#,
                (404.0, 52.0),
            ),
            (r#"1 + 1"#, (41.0, 14.0)),
        ];
        for (expression, (width, height)) in cases {
            assert_eq!(
                measure(expression).unwrap(),
                Size { width, height },
                "MathJax fixture dimensions for {expression:?}"
            );
        }
    }
}

#[cfg(test)]
mod original_general_fraction_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_fraction_boxes_and_serialized_dimensions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/fractions-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 13);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            assert_eq!(
                measure(expression).unwrap(),
                Size {
                    width: row["width"].as_f64().unwrap(),
                    height: row["height"].as_f64().unwrap()
                },
                "{expression}"
            );
            let native = render(expression).unwrap();
            let native = roxmltree::Document::parse(&native).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["width", "height", "viewBox", "style"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression} {key}"
                );
            }
        }
    }
    #[test]
    fn source_binary_ties_use_ecmascript_root_serialization() {
        for (value, expected) in [
            (2.25, "2.3"),
            (2.55, "2.5"),
            (-2.25, "-2.3"),
            (0.0005, "0"),
            (1233.5533905932737, "1233.6"),
        ] {
            assert_eq!(root_number(value), expected);
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_radical_contract {
    use super::{measure, render};
    use d2_rs_core::Size;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_varied_radical_boxes_and_serialized_dimensions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/radicals-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 16);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            assert_eq!(
                measure(expression).unwrap(),
                Size {
                    width: row["width"].as_f64().unwrap(),
                    height: row["height"].as_f64().unwrap(),
                },
                "{expression}"
            );
            let native = render(expression).unwrap();
            let native = roxmltree::Document::parse(&native).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["width", "height", "viewBox", "style"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression} {key}"
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_cancel_color_contract {
    use super::{measure, render};
    use d2_rs_core::Size;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_cancel_color_boxes_and_serialized_dimensions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cancel-color-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 25);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            assert_eq!(
                measure(expression).unwrap(),
                Size {
                    width: row["width"].as_f64().unwrap(),
                    height: row["height"].as_f64().unwrap()
                },
                "{expression}"
            );
            let native = render(expression).unwrap();
            let native = roxmltree::Document::parse(&native).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["width", "height", "viewBox", "style"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression} {key}"
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_gensymb_contract {
    use super::{measure, render};
    use d2_rs_core::Size;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_gensymb_boxes_and_serialized_dimensions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/gensymb-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 22);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            assert_eq!(
                measure(expression).unwrap(),
                Size {
                    width: row["width"].as_f64().unwrap(),
                    height: row["height"].as_f64().unwrap()
                },
                "{expression}"
            );
            let native = render(expression).unwrap();
            let native = roxmltree::Document::parse(&native).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["width", "height", "viewBox", "style"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression} {key}"
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_texatom_scope_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn upright_scopes_preserve_source_boxes_negative_depth_and_local_placements() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/texatom-scope-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 28);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let expected = row["svg"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(expected).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            if expression == r"a+\mathrm{x+y}+b" {
                let source_scope = source
                    .descendants()
                    .find(|n| n.attribute("data-mml-node") == Some("TeXAtom"))
                    .unwrap();
                let native_scope = native
                    .descendants()
                    .find(|n| n.attribute("data-mml-node") == Some("TeXAtom"))
                    .unwrap();
                for key in ["transform", "data-mjx-texclass"] {
                    assert_eq!(native_scope.attribute(key), source_scope.attribute(key));
                }
                let placements = |n: roxmltree::Node<'_, '_>| {
                    n.descendants()
                        .filter(|n| n.has_tag_name("g"))
                        .filter_map(|n| n.attribute("transform").map(str::to_owned))
                        .collect::<Vec<_>>()
                };
                assert_eq!(placements(native_scope), placements(source_scope));
            }
        }
        // Math remapping is independent of normal-font selection; literal
        // text keeps the ASCII hyphen from the original text path.
        assert!(render(r"\mathrm{u-v}").unwrap().contains("data-c=\"2212\""));
        assert!(
            render(r"\colorbox{red}{u-v}")
                .unwrap()
                .contains("data-c=\"2D\"")
        );
    }
}

fn italic_glyph(code: u32) -> Option<GlyphData> {
    if let Ok(i) = italic_glyphs::ITALIC_GLYPHS.binary_search_by_key(&code, |g| g.codepoint) {
        let g = italic_glyphs::ITALIC_GLYPHS[i];
        return Some(GlyphData {
            height: g.height,
            depth: g.depth,
            width: g.width,
            path: g.path,
        });
    }
    glyph(italic_font_code(code))
}
fn italic_correction(code: u32) -> f64 {
    if italic_glyphs::ITALIC_GLYPHS
        .binary_search_by_key(&code, |g| g.codepoint)
        .is_ok()
    {
        return italic_glyphs::ITALIC_CORRECTIONS
            .binary_search_by_key(&code, |g| g.0)
            .ok()
            .map_or(0.0, |i| italic_glyphs::ITALIC_CORRECTIONS[i].1);
    }
    let code = italic_font_code(code);
    glyphs::ITALIC_CORRECTIONS
        .binary_search_by_key(&code, |g| g.0)
        .ok()
        .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1)
}

fn calligraphic_glyph(codepoint: u32) -> Option<GlyphData> {
    if let Ok(index) =
        calligraphic_glyphs::CALLIGRAPHIC_GLYPHS.binary_search_by_key(&codepoint, |g| g.codepoint)
    {
        let data = calligraphic_glyphs::CALLIGRAPHIC_GLYPHS[index];
        return Some(GlyphData {
            height: data.height,
            depth: data.depth,
            width: data.width,
            path: data.path,
        });
    }
    if let Ok(index) =
        italic_glyphs::ITALIC_GLYPHS.binary_search_by_key(&codepoint, |g| g.codepoint)
    {
        let data = italic_glyphs::ITALIC_GLYPHS[index];
        return Some(GlyphData {
            height: data.height,
            depth: data.depth,
            width: data.width,
            path: data.path,
        });
    }
    glyph(italic_font_code(codepoint))
}

fn italic_font_code(code: u32) -> u32 {
    match code {
        0x41..=0x5A => 0x1D434 + code - 0x41,
        0x61..=0x7A => {
            if code == 0x68 {
                0x210E
            } else {
                0x1D44E + code - 0x61
            }
        }
        0x391..=0x3A9 if code != 0x3A2 => 0x1D6E2 + code - 0x391,
        0x3F4 => 0x1D6F3,
        0x3B1..=0x3C9 => 0x1D6FC + code - 0x3B1,
        0x2207 => 0x1D6FB,
        0x2202 => 0x1D715,
        0x3F5 => 0x1D716,
        0x3D1 => 0x1D717,
        0x3F0 => 0x1D718,
        0x3D5 => 0x1D719,
        0x3F1 => 0x1D71A,
        0x3D6 => 0x1D71B,
        _ => code,
    }
}

fn calligraphic_correction(codepoint: u32) -> f64 {
    if calligraphic_glyphs::CALLIGRAPHIC_GLYPHS
        .binary_search_by_key(&codepoint, |g| g.codepoint)
        .is_ok()
    {
        return calligraphic_glyphs::ITALIC_CORRECTIONS
            .binary_search_by_key(&codepoint, |e| e.0)
            .ok()
            .map_or(0.0, |i| calligraphic_glyphs::ITALIC_CORRECTIONS[i].1);
    }
    if italic_glyphs::ITALIC_GLYPHS
        .binary_search_by_key(&codepoint, |g| g.codepoint)
        .is_ok()
    {
        return italic_glyphs::ITALIC_CORRECTIONS
            .binary_search_by_key(&codepoint, |e| e.0)
            .ok()
            .map_or(0.0, |i| italic_glyphs::ITALIC_CORRECTIONS[i].1);
    }
    let smp = italic_font_code(codepoint);
    glyphs::ITALIC_CORRECTIONS
        .binary_search_by_key(&smp, |e| e.0)
        .ok()
        .map_or(0.0, |i| glyphs::ITALIC_CORRECTIONS[i].1)
}

fn original_font_code(code: u32) -> u32 {
    match code {
        0x1D434..=0x1D44D => code - 0x1D434 + 'A' as u32,
        0x1D44E..=0x1D467 => code - 0x1D44E + 'a' as u32,
        0x210E => 'h' as u32,
        0x1D6E2..=0x1D6FA => code - 0x1D6E2 + 0x391,
        0x1D6FC..=0x1D714 => code - 0x1D6FC + 0x3B1,
        0x1D6FB => 0x2207,
        0x1D715 => 0x2202,
        0x1D716 => 0x3F5,
        0x1D717 => 0x3D1,
        0x1D718 => 0x3F0,
        0x1D719 => 0x3D5,
        0x1D71A => 0x3F1,
        0x1D71B => 0x3D6,
        _ => code,
    }
}

fn apply_math_font(atom: &mut Atom, calligraphic: bool) {
    match atom {
        Atom::FontGlyph { original, .. } => {
            let code = *original;
            *atom = if calligraphic {
                Atom::CalligraphicGlyph(code)
            } else {
                Atom::Glyph(code)
            };
        }
        Atom::Glyph(code) | Atom::CalligraphicGlyph(code) | Atom::ItalicGlyph(code) => {
            let original = original_font_code(*code);
            *atom = if calligraphic {
                Atom::CalligraphicGlyph(original)
            } else {
                Atom::Glyph(original)
            };
        }
        Atom::Choice(v) => {
            for a in v {
                apply_math_font(a, calligraphic);
            }
        }
        Atom::Fenced { body: v, .. }
        | Atom::InferredRow(v)
        | Atom::ExplicitRow(v)
        | Atom::Sequence(v)
        | Atom::Identifier(v)
        | Atom::Number(v)
        | Atom::OperatorToken(v) => {
            for a in v {
                apply_math_font(a, calligraphic);
            }
        }
        Atom::Fraction(a, b)
        | Atom::SplitFraction(a, b, _)
        | Atom::GeneralFraction(a, b, _)
        | Atom::Root {
            body: a, index: b, ..
        } => {
            apply_math_font(a, calligraphic);
            apply_math_font(b, calligraphic);
        }
        Atom::BracketMark { body: a, .. }
        | Atom::HorizontalMark { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Sqrt(a)
        | Atom::SizedSqrt { body: a, .. }
        | Atom::Operator(a)
        | Atom::AutoOperator(a)
        | Atom::MathMLSpace { body: a, .. }
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::RawOperator { body: a, .. }
        | Atom::Reference { body: a, .. }
        | Atom::Boxed(a)
        | Atom::Ord(a)
        | Atom::Class { body: a, .. }
        | Atom::Phantom(a)
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::ColorBox { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Cancel { body: a, .. } => apply_math_font(a, calligraphic),
        Atom::Matrix { rows, .. } | Atom::TaggedTable { rows, .. } => {
            for row in rows {
                for a in row {
                    apply_math_font(a, calligraphic);
                }
            }
        }
        Atom::Multiscripts {
            base,
            sub,
            sup,
            post,
            ..
        } => {
            for a in [base, sub, sup] {
                apply_math_font(a, calligraphic);
            }
            if let Some(v) = post {
                for a in v {
                    apply_math_font(a, calligraphic);
                }
            }
        }
        Atom::Scripts { base, sub, sup, .. } => {
            apply_math_font(base, calligraphic);
            for a in [sub, sup].into_iter().flatten() {
                apply_math_font(a, calligraphic);
            }
        }
        Atom::XArrow { over, under, .. } => {
            apply_math_font(over, calligraphic);
            if let Some(a) = under {
                apply_math_font(a, calligraphic);
            }
        }
        Atom::CdArrow { over, under, .. } => {
            for a in [over, under].into_iter().flatten() {
                apply_math_font(a, calligraphic);
            }
        }
        _ => {}
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_amscd_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_source_cd_font_text_dimension_and_error_boxes() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/amscd-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 32);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            if source
                .descendants()
                .any(|node| node.attribute("data-mml-node") == Some("merror"))
            {
                assert_eq!(
                    import_scene_svg(&actual).unwrap_err(),
                    "element <g> has unsupported style property \"font-family\""
                );
            }
        }
        // A whitespace variant exercises the same algorithm as the former
        // literal witness; no complete formula geometry is dispatched.
        let canonical = render(r"\begin{CD}A@>f>>B\\@VgVV@VVhV\\C@>>k>D\end{CD}").unwrap();
        let spaced = render(r"\begin{CD} A@>f>>B\\@VgVV@VVhV\\C@>>k>D\end{CD}").unwrap();
        assert_eq!(canonical, spaced);
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_cd_paint_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_cached_equality_bounds_and_local_script_label_frames() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cd-paint-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 12);
        for (index, row) in rows.iter().enumerate() {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let measured = measure(expression).unwrap();
            assert_eq!(
                (measured.width, measured.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            if index == 0 {
                let nested = |doc: &roxmltree::Document<'_>| {
                    doc.descendants()
                        .find(|n| n.has_tag_name("svg") && n.parent_element().is_some())
                        .unwrap()
                        .attribute("width")
                        .unwrap()
                        .to_owned()
                };
                assert_eq!(nested(&source), "2750");
                assert_eq!(nested(&native), nested(&source));
            }
            if index == 8 {
                let placement = |doc: &roxmltree::Document<'_>| {
                    doc.descendants()
                        .find(|n| n.attribute("data-c") == Some("32"))
                        .unwrap()
                        .ancestors()
                        .find_map(|n| n.attribute("transform"))
                        .unwrap()
                        .to_owned()
                };
                assert_eq!(placement(&source), "translate(609,363) scale(0.707)");
                assert_eq!(placement(&native), placement(&source));
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_cd_capture_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_row_annotations_environment_nesting_and_earliest_errors() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cd-capture-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 11);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let measured = measure(expression).unwrap();
            assert_eq!(
                (measured.width, measured.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let error = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .find_map(|node| node.attribute("data-mjx-error"))
                    .map(str::to_owned)
            };
            assert_eq!(error(&native), error(&source), "{expression}");
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_numeric_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_numeric_runs_script_core_font_scopes_and_unknown_metrics() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/numeric-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 32);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let measured = measure(expression).unwrap();
            assert_eq!(
                (measured.width, measured.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
        }
        // Root dimensions alone do not distinguish a known blank glyph from
        // missing-font text. Preserve the original scene acceptance contract.
        let prior: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/amscd-source-v090.json"
        ))
        .unwrap();
        let spacing = prior
            .iter()
            .find(|row| row["expression"] == r"\text{two words}")
            .unwrap();
        let source_svg = spacing["svg"].as_str().unwrap();
        let native_svg = render(spacing["expression"].as_str().unwrap()).unwrap();
        assert!(import_scene_svg(source_svg).is_ok());
        assert!(import_scene_svg(&native_svg).is_ok());
        let source = roxmltree::Document::parse(source_svg).unwrap();
        let native = roxmltree::Document::parse(&native_svg).unwrap();
        let blanks = |doc: &roxmltree::Document<'_>| {
            doc.descendants()
                .filter(|n| n.has_tag_name("path") && n.attribute("d") == Some(""))
                .count()
        };
        assert!(blanks(&source) > 0);
        assert_eq!(blanks(&native), blanks(&source));
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_length_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_relative_absolute_signed_spacing_and_root_viewports() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/length-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 36);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let measured = measure(expression).unwrap();
            assert_eq!(
                (measured.width, measured.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let error = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .find_map(|n| n.attribute("data-mjx-error"))
                    .map(str::to_owned)
            };
            assert_eq!(error(&native), error(&source), "{expression}");
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_class_contract {
    use super::*;
    #[test]
    fn source_classes_arrows_limits_variants_and_inherited_script_scales() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/classes-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 275);
        for (index, row) in rows.into_iter().enumerate() {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("path"))
                    .map(|n| {
                        (
                            n.attribute("data-c").unwrap_or("").to_owned(),
                            n.attribute("d").unwrap_or("").to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paths(&native),
                paths(&source),
                "{expression}: complete glyph inventory"
            );
            let error = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .find_map(|n| n.attribute("data-mjx-error"))
                    .map(str::to_owned)
            };
            assert_eq!(error(&native), error(&source), "{expression}");
            if index < 128 && index % 4 == 0 {
                let classes = |doc: &roxmltree::Document<'_>| {
                    doc.descendants()
                        .filter_map(|n| n.attribute("data-mjx-texclass"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    classes(&native),
                    classes(&source),
                    "{expression}: effective class"
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_style_contract {
    use super::*;
    #[test]
    fn source_style_declarations_choices_boundaries_and_inherited_levels() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/styles-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 259);
        for row in rows.into_iter() {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("path"))
                    .map(|n| {
                        (
                            n.attribute("data-c").unwrap_or("").to_owned(),
                            n.attribute("d").unwrap_or("").to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paths(&native),
                paths(&source),
                "{expression}: complete glyph inventory"
            );
            let error = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .find_map(|n| n.attribute("data-mjx-error"))
                    .map(str::to_owned)
            };
            assert_eq!(error(&native), error(&source), "{expression}");
            // Preserve the source's independently rounded parent/style frames
            // and radical paint Rule scaling, which equal roots cannot check.
            if [
                r"x_{\displaystyle x+y}",
                r"x^{\displaystyle x+y}",
                r"x^{\displaystyle \sqrt{x}}",
                r"x_{\displaystyle \sqrt{x}}",
                r"x_{\textstyle x+y}",
                r"\scriptstyle \sqrt{x}",
                r"\scriptscriptstyle \sqrt{x}",
                r"x_{\scriptscriptstyle \sqrt{x}}",
                r"x^{\scriptscriptstyle \sqrt{x}}",
            ]
            .contains(&expression)
            {
                let transforms = |doc: &roxmltree::Document<'_>| {
                    doc.descendants()
                        .filter_map(|n| n.attribute("transform"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    transforms(&native),
                    transforms(&source),
                    "{expression}: retained paint frames"
                );
                let rectangles = |doc: &roxmltree::Document<'_>| {
                    doc.descendants()
                        .filter(|n| n.has_tag_name("rect"))
                        .map(|n| {
                            ["x", "y", "width", "height"]
                                .map(|key| n.attribute(key).unwrap_or("").to_owned())
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    rectangles(&native),
                    rectangles(&source),
                    "{expression}: scaled paint rule"
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_xarrow_contract {
    use super::*;
    #[test]
    fn source_labeled_arrow_families_padding_styles_and_script_errors() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/xarrows-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 286);
        for row in rows.into_iter() {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("path"))
                    .map(|n| {
                        (
                            n.attribute("data-c").unwrap_or("").to_owned(),
                            n.attribute("d").unwrap_or("").to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paths(&native),
                paths(&source),
                "{expression}: complete glyph inventory"
            );
            let error = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .find_map(|n| n.attribute("data-mjx-error"))
                    .map(str::to_owned)
            };
            assert_eq!(error(&native), error(&source), "{expression}");
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_host_primitive_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    fn compare(source: &str, count: usize) {
        let rows: Vec<serde_json::Value> = serde_json::from_str(source).unwrap();
        assert_eq!(rows.len(), count);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let svg = render(expression).unwrap();
            let native = roxmltree::Document::parse(&svg).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("path"))
                    .map(|n| {
                        (
                            n.attribute("data-c").unwrap_or("").to_owned(),
                            n.attribute("d").unwrap_or("").to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paths(&native),
                paths(&source),
                "{expression}: glyph inventory"
            );
            let errors = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter_map(|n| n.attribute("data-mjx-error"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                errors(&native),
                errors(&source),
                "{expression}: diagnostics"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_equivalence_circle_and_nbsp_commands_in_context() {
        compare(
            d2_rs_testdata::oracle_fixture!("tests/fixtures/host-primitives-source-v090.json"),
            161,
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_undefined_command_namespace_and_error_precedence() {
        compare(
            d2_rs_testdata::oracle_fixture!("tests/fixtures/undefined-commands-source-v090.json"),
            758,
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_default_command_recognition_preserves_unimplemented_names() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/commands-source-v090.json"
        ))
        .unwrap();
        assert!(
            !rows.is_empty(),
            "the source command inventory must be complete"
        );
        let mut recognized_names = Vec::new();
        for row in &rows {
            let name = row["command"].as_str().unwrap();
            // Go recognizes a name unless the only diagnostic of its render is
            // "Undefined control sequence \<name>" (scripts/generate-math-command-registry.py).
            let svg = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let errors: Vec<&str> = svg
                .descendants()
                .filter_map(|n| n.attribute("data-mjx-error"))
                .collect();
            let recognized = errors != [format!("Undefined control sequence \\{name}")];
            if recognized {
                recognized_names.push(name);
            }
            assert_eq!(command_registry::recognized(name), recognized, "{name}");
        }
        assert_eq!(command_registry::COMMANDS, recognized_names);
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_mhchem_host_contract {
    use super::*;
    #[test]
    fn source_raw_chemistry_units_nested_arguments_and_error_precedence() {
        // Full1039-case differential reports retain123 outstanding host cases.
        // This permanent fixture contains the 916 completely qualified controls.
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/mhchem-host-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 916);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            if let Some(expected) = row["render_error"].as_str() {
                assert_eq!(
                    render(expression).unwrap_err().to_string(),
                    expected,
                    "{expression}"
                );
                assert_eq!(
                    measure(expression).unwrap_err().to_string(),
                    row["measure_error"].as_str().unwrap(),
                    "{expression}"
                );
                assert!(
                    row["svg"].is_null(),
                    "{expression}: source conversion failed"
                );
                continue;
            }
            let actual = render(expression).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("path"))
                    .map(|n| {
                        (
                            n.attribute("data-c").unwrap_or("").to_owned(),
                            n.attribute("d").unwrap_or("").to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paths(&native),
                paths(&source),
                "{expression}: full glyph inventory"
            );
            let errors = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter_map(|n| n.attribute("data-mjx-error"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                errors(&native),
                errors(&source),
                "{expression}: diagnostics"
            );
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_literal_font_contract {
    use super::*;
    #[test]
    fn source_automatic_letter_variants_and_explicit_font_contexts() {
        // Complete1062-case source inventory includes literals, named variants and nested fonts.
        // Root/glyph/text/error checks are separate from the complete raster gate.
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/literal-font-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 1062);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let content = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| {
                        n.has_tag_name("path")
                            || n.has_tag_name("text")
                            || n.attribute("data-mjx-error").is_some()
                    })
                    .map(|n| {
                        if n.has_tag_name("path") {
                            vec![
                                "path".to_owned(),
                                n.attribute("data-c").unwrap_or("").to_owned(),
                                n.attribute("d").unwrap_or("").to_owned(),
                            ]
                        } else if n.has_tag_name("text") {
                            let mut a = n
                                .attributes()
                                .map(|a| (a.name(), a.value()))
                                .collect::<Vec<_>>();
                            a.sort_unstable();
                            vec![
                                "text".to_owned(),
                                format!("{a:?}"),
                                n.text().unwrap_or("").to_owned(),
                            ]
                        } else {
                            vec![
                                "error".to_owned(),
                                n.attribute("data-mjx-error").unwrap().to_owned(),
                            ]
                        }
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                content(&native),
                content(&source),
                "{expression}: glyph/text/diagnostic inventory"
            );
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_compound_font_contract {
    use super::*;
    #[test]
    fn source_compound_metrics_children_groups_and_font_contexts() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/compound-font-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 160);
        assert_eq!(compound_glyphs::COMPOUNDS.len(), 20);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let actual = render(expression).unwrap();
            let native = roxmltree::Document::parse(&actual).unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            for key in ["style", "width", "height", "viewBox"] {
                assert_eq!(
                    native.root_element().attribute(key),
                    source.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("path"))
                    .map(|n| {
                        (
                            n.attribute("data-c").map(str::to_owned),
                            n.attribute("d").map(str::to_owned),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paths(&native),
                paths(&source),
                "{expression}: glyph inventory"
            );
            let groups = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.has_tag_name("g") && n.attribute("data-c").is_some())
                    .map(|n| {
                        (
                            n.attribute("data-c").map(str::to_owned),
                            n.children()
                                .filter(|c| c.is_element())
                                .map(|c| {
                                    (
                                        c.attribute("data-c").map(str::to_owned),
                                        c.attribute("transform").map(str::to_owned),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                groups(&native),
                groups(&source),
                "{expression}: compound child identity/placement"
            );
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod original_matrix_table_contract {
    use super::*;
    #[test]
    fn original_generic_matrix_geometry_glyphs_row_cell_frames_and_errors() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/matrix-table-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 293);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let svg = render(expression).unwrap();
            let a = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let n = roxmltree::Document::parse(&svg).unwrap();
            for key in ["style", "width", "height", "viewBox", "role", "focusable"] {
                assert_eq!(
                    n.root_element().attribute(key),
                    a.root_element().attribute(key),
                    "{expression}: {key}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
            let paths = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|e| e.has_tag_name("path"))
                    .map(|e| {
                        (
                            e.attribute("data-c").unwrap_or("").to_owned(),
                            e.attribute("d").unwrap_or("").to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(paths(&n), paths(&a), "{expression}: paths");
            let frames = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|e| matches!(e.attribute("data-mml-node"), Some("mtr" | "mtd")))
                    .map(|e| {
                        (
                            e.attribute("data-mml-node").unwrap().to_owned(),
                            local_matrix(e.attribute("transform").unwrap_or("")),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                frames(&n),
                frames(&a),
                "{expression}: actual row/cell frames"
            );
            // Script/style scopes may place the table on an ancestor or on
            // mtable itself. Compare its full affine placement, preserving
            // every serialized coordinate rather than demanding wrapper bytes.
            fn multiply(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
                [
                    a[0] * b[0] + a[2] * b[1],
                    a[1] * b[0] + a[3] * b[1],
                    a[0] * b[2] + a[2] * b[3],
                    a[1] * b[2] + a[3] * b[3],
                    a[0] * b[4] + a[2] * b[5] + a[4],
                    a[1] * b[4] + a[3] * b[5] + a[5],
                ]
            }
            fn local_matrix(transform: &str) -> [f64; 6] {
                let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                for operation in transform.split(')') {
                    let Some((name, values)) = operation.trim().split_once('(') else {
                        continue;
                    };
                    let v = values
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|x| !x.is_empty())
                        .map(|x| x.parse::<f64>().unwrap())
                        .collect::<Vec<_>>();
                    let own = match name.trim() {
                        "translate" => [1.0, 0.0, 0.0, 1.0, v[0], v.get(1).copied().unwrap_or(0.0)],
                        "scale" => [v[0], 0.0, 0.0, v.get(1).copied().unwrap_or(v[0]), 0.0, 0.0],
                        "matrix" => v.try_into().unwrap(),
                        _ => panic!("unsupported source transform: {name}"),
                    };
                    matrix = multiply(matrix, own);
                }
                matrix
            }
            fn world(node: roxmltree::Node<'_, '_>) -> [f64; 6] {
                node.ancestors().collect::<Vec<_>>().into_iter().rev().fold(
                    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    |matrix, ancestor| {
                        multiply(
                            matrix,
                            local_matrix(ancestor.attribute("transform").unwrap_or("")),
                        )
                    },
                )
            }
            let tables = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|e| e.attribute("data-mml-node") == Some("mtable"))
                    .map(world)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                tables(&n),
                tables(&a),
                "{expression}: complete table affine placement"
            );
            let errors = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter_map(|e| e.attribute("data-mjx-error"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(errors(&n), errors(&a), "{expression}: errors");
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod prime_contract_tests;

#[cfg(all(test, feature = "oracle-fixtures"))]
mod ams_contract_tests;

#[doc(hidden)]
pub mod svg_image_static;

pub mod png_first_frame;
#[doc(hidden)]
pub mod svg_image_config;
pub mod webp_first_frame;

#[doc(hidden)]
pub mod svg_document_import;

#[doc(hidden)]
pub mod svg_asset_cache;

#[cfg(test)]
mod mathjax_exact_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod source_nil_child_tests;

/// Shared imageasset SVG acquisition helpers.
pub mod image_svg;
