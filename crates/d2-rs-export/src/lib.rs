// SPDX-License-Identifier: MPL-2.0
//! Diagram export for d2-rs: PNG, PDF, PPTX, GIF and ASCII output, ported from
//! upstream D2's exporters and the `lib/png`, `lib/pdf`, `lib/pptx` and
//! `lib/xgif` packages, byte-identical with Go D2.
//!
//! SVG construction is delegated to `d2-rs-render`. The `diagnostic-traces`
//! feature enables renderer traces; `oracle-fixtures` compiles tests that read
//! regenerated Go-oracle artifacts.

// Keep source-shaped export helpers close to the upstream raster/package
// algorithms.  Their argument lists and fixed-size byte loops are deliberate:
// changing them casually can change floating-point or encoding behavior.
#![allow(
    clippy::chunks_exact_to_as_chunks,
    clippy::collapsible_if,
    clippy::identity_op,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]

use d2_rs_core::{Graph, NodeId, Point, Rect, theme};
use d2_rs_language::compiler::SourceNodeStyle;
use d2_rs_render::RenderOptions;
use d2_rs_tala::LayoutResult;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub mod ascii_target;
mod document_links;
mod gif_encode;
mod gif_quantize;
pub use gif_quantize::palette as gif_median_cut_palette;
mod go_png;
mod go_zip;
#[cfg(all(test, feature = "oracle-fixtures"))]
use d2_rs_math::image_svg::go_xml_names;
mod paged_admission;
mod pdf_document;
mod pdf_font;
mod png_image;
mod pptx;
use d2_rs_math::image_svg::svg_asset_codec;
#[cfg(all(test, feature = "oracle-fixtures"))]
use d2_rs_math::image_svg::svg_asset_xml;
pub use document_links::page_fill as document_page_fill;
pub use paged_admission::{GifBoard, GifFontResources};
pub use pptx::PptxDocumentProperties;
mod svg_import;
#[cfg(all(test, feature = "oracle-fixtures"))]
use d2_rs_math::image_svg::svg_import_envelope;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod svg_import_geometry;

/// Admit an acquired image before retaining it in a raster resolver cache.
/// PNG container/configuration checks mirror imageasset.probeRaster before
/// retention; pixel decoding remains at the caller's raster boundary. SVG
/// envelopes and declared compressed SVG use the existing bounded codec.
pub fn prepare_image_asset_bytes(
    source: &str,
    mime_hint: Option<&str>,
    bytes: &[u8],
) -> Result<Vec<u8>, String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        // internal/rasterimage.Config validates every PNG/APNG chunk before
        // image.DecodeConfig, even when SVG bundling never decodes pixels.
        d2_rs_math::png_first_frame::inspect_png_container(bytes)
            .and_then(|_| d2_rs_math::svg_image_config::png_decode_config(bytes, None))
            .map_err(|error| format!("malformed PNG image: {error}"))?;
        return Ok(bytes.to_vec());
    }
    if !raster_asset_validation::signature(bytes).is_empty() {
        return Ok(bytes.to_vec());
    }
    let declared = svg_asset_codec::declared_svg(source)
        || mime_hint.is_some_and(|hint| {
            hint.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("image/svg+xml")
        });
    let looks_xml = bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(bytes)
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        == Some(&b'<');
    if !declared && !looks_xml {
        return Err("unsupported or malformed image".into());
    }
    svg_asset_codec::prepare(bytes, declared, svg_asset_codec::CLI_SVG_LIMIT)
        .map(|bytes| bytes.into_owned())
}

/// Formats understood by the native exporter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    /// Semantic SVG output.
    Svg,
    /// Deterministic terminal-friendly text output.
    Ascii,
    /// PNG raster output.
    Png,
    /// GIF output. The current single-layout API emits one deterministic frame.
    Gif,
    /// PDF output. The current single-layout API emits a raster page.
    Pdf,
    /// PowerPoint output as an OOXML package containing the rendered slide.
    Pptx,
}

impl ExportFormat {
    /// Parses a D2 format spelling.
    pub fn parse(value: &str) -> Result<Self, ExportError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "svg" => Ok(Self::Svg),
            "ascii" | "txt" | "text" => Ok(Self::Ascii),
            "png" => Ok(Self::Png),
            "gif" => Ok(Self::Gif),
            "pdf" => Ok(Self::Pdf),
            "pptx" | "ppt" => Ok(Self::Pptx),
            other => Err(ExportError::UnknownFormat(other.to_owned())),
        }
    }

    /// Canonical lower-case extension for this format.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Ascii => "txt",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Pdf => "pdf",
            Self::Pptx => "pptx",
        }
    }
}

/// Export failure with a stable, user-facing diagnostic.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExportError {
    /// The requested format name is not known.
    #[error("unknown output format {0:?}")]
    UnknownFormat(String),
    /// The format is part of the D2 surface but has no native encoder yet.
    #[error("output format {format:?} is not implemented in d2-rs yet")]
    Unsupported { format: &'static str },
    /// Native rasterization or PNG encoding failed.
    #[error("could not encode PNG: {0}")]
    Png(String),
    /// Native GIF encoding failed.
    #[error("could not encode GIF: {0}")]
    Gif(String),
    /// Native PDF encoding failed.
    #[error("could not encode PDF: {0}")]
    Pdf(String),
    /// Native PowerPoint package construction failed.
    #[error("could not encode PPTX: {0}")]
    Pptx(String),
    /// The layout contains no drawable entities for a text export.
    #[error("cannot export an empty layout as ASCII")]
    EmptyLayout,
    /// The source renderer rejected unsupported raw HTML in a Markdown label.
    #[error("{0}")]
    Render(String),
}

/// A source hyperlink and its rectangle in the rendered output canvas.
/// Coordinates are in the SVG/raster logical coordinate system, before the
/// page or slide packager applies centering and document units.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentLinkRegion {
    /// Destination URI or fragment.
    pub href: String,
    /// Top-left output-canvas coordinate and dimensions.
    pub rect: Rect,
}

/// Destination provenance and explicit tooltip retained by the scene builder.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentLinkMetadata {
    pub region: DocumentLinkRegion,
    pub tooltip: String,
    pub board_target: bool,
}
impl std::ops::Deref for DocumentLinkMetadata {
    type Target = DocumentLinkRegion;
    fn deref(&self) -> &Self::Target {
        &self.region
    }
}
impl std::ops::DerefMut for DocumentLinkMetadata {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.region
    }
}
impl From<DocumentLinkRegion> for DocumentLinkMetadata {
    fn from(region: DocumentLinkRegion) -> Self {
        Self {
            region,
            tooltip: String::new(),
            board_target: false,
        }
    }
}

/// Exports a laid-out graph in a selected format.
pub fn export(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    format: ExportFormat,
) -> Result<Vec<u8>, ExportError> {
    export_with_options(
        graph,
        result,
        labels,
        styles,
        format,
        RenderOptions::default(),
    )
}

/// Exports a graph while applying output-canvas options to raster and SVG
/// formats. Layout geometry is computed before these options are applied.
pub fn export_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    format: ExportFormat,
    options: RenderOptions,
) -> Result<Vec<u8>, ExportError> {
    export_with_sketch(graph, result, labels, styles, format, options, false)
}

/// Exports a graph while optionally applying D2's deterministic sketch
/// overlay to SVG-backed formats.
pub fn export_with_sketch(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    format: ExportFormat,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, ExportError> {
    match format {
        ExportFormat::Svg => {
            d2_rs_render::render_d2svg(graph, result, labels, styles, &options, sketch)
                .map_err(ExportError::Render)
        }
        ExportFormat::Ascii => {
            render_ascii_with_options(graph, result, labels, styles, &options, sketch)
        }
        ExportFormat::Png => {
            render_png_with_options(graph, result, labels, styles, options, sketch, 2)
        }
        ExportFormat::Gif => {
            render_gif_with_options(graph, result, labels, styles, options, sketch)
        }
        ExportFormat::Pdf => {
            render_pdf_with_options(graph, result, labels, styles, options, sketch)
        }
        ExportFormat::Pptx => {
            render_pptx_with_options(graph, result, labels, styles, options, sketch)
        }
    }
}

/// Add D2's Make/Model eXIf chunk after IHDR in fresh PNG encoder output.
/// This consumes a trusted, metadata-free encoded PNG, not an arbitrary asset.
/// Internal document images do not use this standalone CLI metadata step.
pub fn add_png_exif_to_encoder_output(png: Vec<u8>, version: &str) -> Result<Vec<u8>, ExportError> {
    go_png::with_exif(png, version).map_err(ExportError::Png)
}

/// Renders the graph's typed scene to an opaque PNG, as d2cli's PNG export
/// does (raster_png_output.go renderPNGToWriter).
pub fn render_png(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Vec<u8>, ExportError> {
    render_png_at_scale(
        graph,
        result,
        labels,
        styles,
        RenderOptions::default(),
        false,
        2,
    )
}

/// Encodes a PNG at an explicit pixel density relative to the semantic SVG.
///
/// D2's standalone PNG path uses a 2x density, while GIF frames are rendered
/// at 1x before they are assembled into an animation.  Keeping this choice
/// explicit prevents the multiboard packaging path from accidentally using
/// PNG density for GIF output.
pub fn render_png_at_scale(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
) -> Result<Vec<u8>, ExportError> {
    if pixel_scale == 0 {
        return Err(ExportError::Png("pixel scale must be positive".into()));
    }
    render_png_with_options(graph, result, labels, styles, options, sketch, pixel_scale)
}

/// Renders one board into a transparent RGBA PNG for document packaging.
///
/// D2's paged PDF path rasterizes its scene with a transparent frame, then
/// stores the image alpha as a PDF soft mask over the page fill. This variant
/// preserves that boundary; ordinary PNG output remains opaque RGB.
pub fn render_png_rgba_at_scale(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
) -> Result<Vec<u8>, ExportError> {
    render_paged_png_rgba_at_scale(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        &mut PagedFrameBudget::default(),
    )
}

/// Complete-document pixel allowance and board count used to divide scene
/// and frame work. Source: d2cli/paged.go pagedFrameOptions.
pub struct PagedFrameBudget {
    remaining_pixels: u64,
    total_boards: usize,
    resources: Option<paged_admission::Resources>,
}

impl Default for PagedFrameBudget {
    fn default() -> Self {
        Self {
            remaining_pixels: 256 * 1024 * 1024,
            total_boards: 1,
            resources: None,
        }
    }
}

impl PagedFrameBudget {
    /// Divide per-board scene and raster work across the complete document,
    /// as d2cli/paged.go does before rendering its first board.
    pub fn new(total_boards: usize) -> Self {
        Self {
            total_boards,
            ..Self::default()
        }
    }
}

/// Renders a paged board, admitting its frame before pixel allocation and
/// charging the complete-document allowance only after successful encoding.
pub fn render_paged_png_rgba_at_scale(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<Vec<u8>, ExportError> {
    render_paged_png_with_links(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        budget,
    )
    .map(|(png, _)| png)
}

/// `render_paged_png_rgba_at_scale` with the board's links, mapped from the
/// same scene into the frame (d2cli/paged.go pagedRenderer.render).
pub fn render_paged_png_with_links(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<(Vec<u8>, Vec<DocumentLinkMetadata>), ExportError> {
    let frame = render_paged_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        budget,
    )?;
    let png =
        go_png::encode(frame.width, frame.height, &frame.rgba, 4).map_err(ExportError::Png)?;
    Ok((png, frame.links))
}

/// A paged signature shared by `render_paged_png_with_links` and
/// `render_paged_pdf_page_with_links`: one board's image and its links.
pub type PagedFrameRenderer<T> = fn(
    &Graph,
    &LayoutResult,
    &BTreeMap<NodeId, String>,
    &BTreeMap<NodeId, SourceNodeStyle>,
    RenderOptions,
    bool,
    u32,
    &mut PagedFrameBudget,
) -> Result<(T, Vec<DocumentLinkMetadata>), ExportError>;

/// One rendered PDF page image, ready for
/// `package_pdf_page_rasters_with_navigation_metadata_for_theme`.
#[derive(Clone, Debug)]
pub struct PdfPage(PdfRasterPage);

/// `render_paged_png_with_links` for a PDF page. FPDF parses the page PNG
/// back into color and alpha planes; PNG is lossless, so those planes are the
/// frame's own unpremultiplied bytes. Only an opaque page, whose RGB PNG
/// image data FPDF embeds as is, still needs the PNG.
pub fn render_paged_pdf_page_with_links(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<(PdfPage, Vec<DocumentLinkMetadata>), ExportError> {
    let frame = render_paged_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        pixel_scale,
        budget,
    )?;
    let page = pdf_page_from_rgba(frame.width, frame.height, frame.rgba)?.compacted()?;
    Ok((PdfPage(page), frame.links))
}

/// The `decode_pdf_png` result for `go_png::encode(width, height, rgba, 4)`,
/// without decoding: the encoder writes an RGB PNG exactly when every pixel
/// is opaque (then FPDF keeps its non-interlaced image data), and an RGBA PNG
/// otherwise, whose decoded samples are `rgba` itself.
fn pdf_page_from_rgba(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) -> Result<PdfRasterPage, ExportError> {
    let opaque = rgba
        .chunks(1 << 14)
        .all(|block| block.chunks_exact(4).fold(255, |alpha, p| alpha & p[3]) == 255);
    let pixels = rgba.len() / 4;
    let mut rgb = Vec::with_capacity(pixels * 3);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    if opaque {
        let png = go_png::encode(width, height, &rgba, 4).map_err(ExportError::Png)?;
        drop(rgba);
        return Ok(PdfRasterPage {
            width,
            height,
            rgb,
            alpha: None,
            rgb_png_idat: Some(pdf_rgb_png_idat(&png)?),
            streams: None,
        });
    }
    // Validate dimensions exactly as encoding the PNG would.
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len()) {
        return Err(ExportError::Png(
            "invalid PNG image dimensions or pixel buffer".into(),
        ));
    }
    let alpha = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
    Ok(PdfRasterPage {
        width,
        height,
        rgb,
        alpha: Some(alpha),
        rgb_png_idat: None,
        streams: None,
    })
}

#[cfg(test)]
mod pdf_page_from_rgba_tests {
    /// PDF page planes built from the frame equal parsing its page PNG.
    #[test]
    fn frame_planes_match_decoded_page_png() {
        let (width, height) = (37_u32, 23_u32);
        let frame = |alpha: fn(usize) -> u8| {
            (0..(width * height) as usize)
                .flat_map(|index| {
                    [
                        (index * 7) as u8,
                        (index * 13 + 5) as u8,
                        (index / 3) as u8,
                        alpha(index),
                    ]
                })
                .collect::<Vec<_>>()
        };
        for rgba in [
            frame(|_| 255),
            frame(|index| (index * 31) as u8),
            frame(|_| 0),
        ] {
            let png = super::go_png::encode(width, height, &rgba, 4).unwrap();
            let decoded = super::decode_pdf_png(&png).unwrap();
            let direct = super::pdf_page_from_rgba(width, height, rgba).unwrap();
            assert_eq!(
                (
                    direct.width,
                    direct.height,
                    &direct.rgb,
                    &direct.alpha,
                    &direct.rgb_png_idat
                ),
                (
                    decoded.width,
                    decoded.height,
                    &decoded.rgb,
                    &decoded.alpha,
                    &decoded.rgb_png_idat
                )
            );
            assert_eq!(direct.image_streams(), decoded.image_streams());
        }
    }
}

/// The frame of a paged board, unpremultiplied RGBA over a transparent page,
/// with its links; charges the document's pixel allowance.
fn render_paged_frame(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
    budget: &mut PagedFrameBudget,
) -> Result<paged_admission::PagedFrame, ExportError> {
    if budget.total_boards == 0 || budget.remaining_pixels == 0 {
        return Err(ExportError::Render(
            "paged export exhausted its operation budget".into(),
        ));
    }
    if pixel_scale == 0 {
        return Err(ExportError::Png("pixel scale must be positive".into()));
    }
    let frame = paged_admission::render_frame(
        graph,
        result,
        labels,
        styles,
        &options,
        sketch,
        pixel_scale,
        budget,
    )?;
    budget.remaining_pixels -= u64::from(frame.width) * u64::from(frame.height);
    Ok(frame)
}

fn render_png_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    pixel_scale: u32,
) -> Result<Vec<u8>, ExportError> {
    paged_admission::render_png(graph, result, labels, styles, &options, sketch, pixel_scale)
}

/// Encodes a laid-out graph as a deterministic single-frame GIF.
///
/// D2's full GIF surface can contain animation frames from boards and steps.
/// This exporter currently receives one `Graph`/`LayoutResult` pair, so it
/// emits that layout as one frame without a repeat extension. The CLI uses
/// the interval-aware sampled API for both static and animated boards.
pub fn render_gif(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Vec<u8>, ExportError> {
    render_gif_with_options(
        graph,
        result,
        labels,
        styles,
        RenderOptions::default(),
        false,
    )
}

/// Encodes one laid-out graph while sampling D2's 30fps animated-dash
/// schedule. Static graphs should continue to use [`render_gif`], which
/// emits one frame just like the v0.9.0 CLI does for a non-animated board.
pub fn render_animated_gif(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    render_animated_gif_with_options(
        graph,
        result,
        labels,
        styles,
        RenderOptions::default(),
        false,
        interval_ms,
    )
}

/// Returns whether a graph contains a D2 `style.animated: true` track.
pub fn has_animated_content(graph: &Graph, styles: &BTreeMap<NodeId, SourceNodeStyle>) -> bool {
    graph.edges().any(|(id, _)| {
        graph
            .edge_style(id)
            .animated
            .as_deref()
            .is_some_and(parse_animation_bool)
    }) || graph
        .legend
        .as_ref()
        .is_some_and(|legend| legend.connections.iter().any(|c| c.animated))
        || has_animated_shapes(styles)
}

/// Returns whether any shape declares D2's `style.animated: true` effect.
fn has_animated_shapes(styles: &BTreeMap<NodeId, SourceNodeStyle>) -> bool {
    styles
        .values()
        .any(|style| style.animated.as_deref().is_some_and(parse_animation_bool))
}

/// Encodes an animated graph with explicit canvas and sketch options.
pub fn render_animated_gif_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    // raster.go renderGIFWithSession for a one-board schedule.
    let frames_per_board = gif_frames_per_board(interval_ms)?;
    if frames_per_board > 1_800 {
        return Err(ExportError::Render(format!(
            "GIF requires {frames_per_board} frames per board, exceeding the 1800-frame limit"
        )));
    }
    let board = plan_gif_board(
        graph,
        result,
        labels,
        styles,
        &options,
        sketch,
        1,
        frames_per_board,
    )?;
    let (width, height) = board.size();
    validate_gif_normalized_footprint(width, height, frames_per_board)?;
    let animated = has_animated_content(graph, styles);
    encode_planned_gif_board(board, animated, interval_ms)
}

/// Plans one GIF board of a `total_boards`-board, `total_frames`-frame
/// schedule (raster.go renderGIFWithSession): builds its scene with the
/// board-divided budgets and admits its logical canvas.
#[allow(clippy::too_many_arguments)]
pub fn plan_gif_board(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    total_boards: usize,
    total_frames: usize,
) -> Result<GifBoard, ExportError> {
    GifBoard::plan(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        total_boards,
        total_frames,
    )
}

/// Plan a GIF board using the fallback state shared by its export schedule.
#[allow(clippy::too_many_arguments)]
pub fn plan_gif_board_with_fonts(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
    total_boards: usize,
    total_frames: usize,
    fonts: &GifFontResources,
) -> Result<GifBoard, ExportError> {
    GifBoard::plan_with_fonts(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        total_boards,
        total_frames,
        Some(fonts),
    )
}

/// raster.go validateGIFNormalizedFootprint: every frame is normalized to
/// the running max(width) x max(height) canvas, charged against
/// gifMaxNormalizedPixels across the whole schedule.
pub fn validate_gif_normalized_footprint(
    width: u32,
    height: u32,
    total_frames: usize,
) -> Result<(), ExportError> {
    const MAX_NORMALIZED_PIXELS: i64 = 160 * 1024 * 1024;
    let total_frames = i64::try_from(total_frames).unwrap_or(i64::MAX);
    if width == 0 || height == 0 || total_frames <= 0 {
        return Err(ExportError::Render(
            "GIF normalized frame footprint requires positive dimensions and frame count".into(),
        ));
    }
    let max_pixels_per_frame = MAX_NORMALIZED_PIXELS / total_frames;
    if i64::from(width) > max_pixels_per_frame / i64::from(height) {
        return Err(ExportError::Render(format!(
            "GIF normalized frame footprint exceeds the {MAX_NORMALIZED_PIXELS} cumulative-pixel limit"
        )));
    }
    Ok(())
}

/// Renders a planned GIF board: one static image (repeated on the schedule
/// by the packager), or every 30-fps sample of an animated board. The first
/// sample's d2raster preflight applies the frame-divided ceilings first.
#[allow(clippy::too_many_arguments)]
pub fn render_planned_gif_board(
    board: GifBoard,
    graph: &Graph,
    _result: &LayoutResult,
    _labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    _options: RenderOptions,
    _sketch: bool,
    interval_ms: u64,
) -> Result<Vec<Vec<u8>>, ExportError> {
    board.preflight()?;
    if !has_animated_content(graph, styles) {
        let mut board = board;
        let (width, height, rgba) = board.render(0, 0)?;
        let mut stream = go_png::RowStream::new(width, height, 3);
        stream.push_rows(&rgba, 4);
        return Ok(vec![stream.finish()]);
    }
    render_gif_board_samples(board, interval_ms)
}

/// Encodes a single planned board as its samples are rendered. Like
/// d2cli/raster.go's single-board OpaquePalettedAnimationEncoder path, this
/// retains the scene and one quantized frame, without PNG intermediates.
pub fn encode_planned_gif_board(
    mut board: GifBoard,
    animated: bool,
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    if interval_ms == 0 {
        return Err(ExportError::Gif(
            "GIF animation interval must be positive".into(),
        ));
    }
    board.preflight()?;
    let frame_count = gif_frames_per_board(interval_ms)?;
    let delays = gif_sampled_frame_delays(frame_count, interval_ms)?;
    let mut encoder = gif_encode::Encoder::new(frame_count, true);
    let mut quantized = None;
    for (frame_index, delay) in delays.into_iter().enumerate() {
        if animated || quantized.is_none() {
            let timestamp = gif_animation_frame_time_ns(frame_index)?;
            let (width, height, rgba) = board.render(frame_index, timestamp)?;
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var_os("D2_RS_GIF_TRACE_FRAMES").is_some() {
                let mut stream = go_png::RowStream::new(width, height, 3);
                stream.push_rows(&rgba, 4);
                write_gif_trace_frame(frame_index, &stream.finish())?;
            }
            quantized = Some(gif_quantize::frame(
                &rgba,
                width as u16,
                height as u16,
                width as u16,
                height as u16,
                delay,
            ));
        }
        let frame = quantized.as_mut().expect("quantized GIF frame");
        frame.delay = delay;
        encoder.write_frame(frame);
    }
    Ok(encoder.finish())
}

/// Renders one static GIF board, admitting its logical frame before allocation.
pub fn render_gif_png_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, ExportError> {
    let (width, height, rgba) =
        render_static_gif_frame(graph, result, labels, styles, &options, sketch)?;
    let mut stream = go_png::RowStream::new(width, height, 3);
    stream.push_rows(&rgba, 4);
    Ok(stream.finish())
}

/// A one-board, one-frame GIF schedule's frame (raster.go
/// renderGIFWithSession): opaque RGBA rows of the scene at 1x.
fn render_static_gif_frame(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<(u32, u32, Vec<u8>), ExportError> {
    let mut board = GifBoard::plan(graph, result, labels, styles, options, sketch, 1, 1)?;
    board.preflight()?;
    board.render(0, 0)
}

/// Renders the 30-fps PNG samples used by an animated GIF export.
///
/// Keeping sampling separate lets multiboard callers assemble one D2 GIF
/// schedule while retaining each board's animated scene state.
pub fn render_animated_png_frames_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
    interval_ms: u64,
) -> Result<Vec<Vec<u8>>, ExportError> {
    if interval_ms == 0 {
        return Err(ExportError::Gif(
            "GIF animation interval must be positive".into(),
        ));
    }
    let frame_count = gif_frames_per_board(interval_ms)?;
    let scene_board = GifBoard::plan(
        graph,
        result,
        labels,
        styles,
        &options,
        sketch,
        1,
        frame_count,
    )?;
    render_gif_board_samples(scene_board, interval_ms)
}

/// Samples the prepared scene at 30 fps, including edge dash offsets and
/// animated shape effects, as d2cli/raster.go renderGIFBoardFrames does.
fn render_gif_board_samples(
    mut board: GifBoard,
    interval_ms: u64,
) -> Result<Vec<Vec<u8>>, ExportError> {
    if interval_ms == 0 {
        return Err(ExportError::Gif(
            "GIF animation interval must be positive".into(),
        ));
    }
    let frame_count = gif_frames_per_board(interval_ms)?;
    let mut frames = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let timestamp = gif_animation_frame_time_ns(frame_index)?;
        let (width, height, rgba) = board.render(frame_index, timestamp)?;
        let mut stream = go_png::RowStream::new(width, height, 3);
        stream.push_rows(&rgba, 4);
        let frame = stream.finish();
        #[cfg(feature = "diagnostic-traces")]
        write_gif_trace_frame(frame_index, &frame)?;
        frames.push(frame);
    }
    Ok(frames)
}

#[cfg(feature = "diagnostic-traces")]
fn write_gif_trace_frame(frame_index: usize, frame: &[u8]) -> Result<(), ExportError> {
    if let Some(dump_dir) = std::env::var_os("D2_RS_GIF_TRACE_FRAMES") {
        let dump_dir = std::path::PathBuf::from(dump_dir);
        std::fs::create_dir_all(&dump_dir).map_err(|error| {
            ExportError::Gif(format!("could not create GIF trace directory: {error}"))
        })?;
        let path = dump_dir.join(format!("{frame_index:06}.png"));
        std::fs::write(&path, frame).map_err(|error| {
            ExportError::Gif(format!(
                "could not write GIF trace frame {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

fn parse_animation_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

/// Packages rendered PNG boards as one deterministic animated GIF.
///
/// GIF has one logical screen size, so boards with different raster sizes are
/// centered on a white canvas sized to the largest board. Each
/// board becomes a full-canvas frame with a fixed 1000 ms delay and infinite
/// looping, matching D2's document-animation defaults without introducing
/// timing or browser dependencies.
pub fn package_gif_frames(png_frames: &[Vec<u8>]) -> Result<Vec<u8>, ExportError> {
    package_gif_frames_with_interval(png_frames, 1000)
}

/// Packages rendered PNG boards using an explicit per-board interval in
/// milliseconds. GIF stores delays in centiseconds, so values are rounded up
/// to avoid shortening the requested interval.
pub fn package_gif_frames_with_interval(
    png_frames: &[Vec<u8>],
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    if png_frames.is_empty() {
        return Err(ExportError::Gif("cannot create an empty GIF".into()));
    }
    if interval_ms == 0 {
        return Err(ExportError::Gif(
            "GIF animation interval must be positive".into(),
        ));
    }
    // This entry point receives one PNG per board. Animated scene sampling is
    // exposed separately because the same frame count is otherwise
    // ambiguous with a multi-board document.
    let delays = vec![gif_delay_centiseconds(interval_ms)?; png_frames.len()];
    package_gif_frames_with_delays(png_frames, delays)
}

/// Packages PNGs that were sampled from one animated board at 30 fps.
///
/// The caller supplies every sampled frame; this function only applies D2's
/// centisecond delay distribution and GIF canvas/quantization rules.
pub fn package_gif_samples_with_interval(
    png_frames: &[Vec<u8>],
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    if interval_ms == 0 {
        return Err(ExportError::Gif(
            "GIF animation interval must be positive".into(),
        ));
    }
    let delays = gif_sampled_frame_delays(png_frames.len(), interval_ms)?;
    // d2cli/raster.go renderGIF streams a single board's samples through
    // xgif.NewOpaquePalettedAnimationEncoder, which shares the first frame's
    // palette as the global color table (gifOpaqueGlobalColorTable).
    package_gif_frames_encoded(png_frames, delays, true)
}

/// Packages one rendered PNG per board using D2's multiboard GIF schedule.
///
/// The v0.9.0 raster path samples each board at 30 fps for the requested
/// interval, repeats the board image for those samples, and centers every
/// frame on the largest board canvas. This API accepts already-rendered board
/// images; callers that have animated scene content should supply a sampled
/// frame for each board sample instead.
pub fn package_gif_boards_with_interval(
    png_boards: &[Vec<u8>],
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    let sampled_boards = png_boards
        .iter()
        .map(|frame| vec![frame.clone()])
        .collect::<Vec<_>>();
    package_gif_sampled_boards_with_interval(&sampled_boards, interval_ms)
}

/// Packages sampled frames for each board using D2's multiboard GIF schedule.
///
/// A board may provide one static PNG, which is repeated for the board's full
/// sample interval, or exactly the 30-fps sample count returned by the pinned
/// v0.9.0 schedule. Every frame is centered on the largest board canvas.
pub fn package_gif_sampled_boards_with_interval(
    board_frames: &[Vec<Vec<u8>>],
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    if board_frames.is_empty() {
        return Err(ExportError::Gif("cannot create an empty GIF".into()));
    }
    if interval_ms == 0 {
        return Err(ExportError::Gif(
            "GIF animation interval must be positive".into(),
        ));
    }
    let frames_per_board = gif_frames_per_board(interval_ms)?;
    let delays = gif_sampled_frame_delays(frames_per_board, interval_ms)?;
    let mut frames = Vec::with_capacity(board_frames.len() * frames_per_board);
    for (board_index, samples) in board_frames.iter().enumerate() {
        if samples.is_empty() {
            return Err(ExportError::Gif(format!(
                "GIF board {board_index} has no rendered frames"
            )));
        }
        if samples.len() == 1 {
            frames.extend(std::iter::repeat_n(samples[0].as_slice(), frames_per_board));
        } else if samples.len() == frames_per_board {
            frames.extend(samples.iter().map(Vec::as_slice));
        } else {
            return Err(ExportError::Gif(format!(
                "GIF board {board_index} has {} frames; expected one or {frames_per_board}",
                samples.len()
            )));
        }
    }
    let frame_delays = (0..frames.len())
        .map(|index| delays[index % frames_per_board])
        .collect();
    package_gif_frames_encoded(&frames, frame_delays, true)
}

/// Packages rendered board SVGs as D2's multiboard animated SVG.
///
/// The v0.9.0 CLI keeps each board's semantic SVG body, combines the board
/// styles into one document, and switches board visibility with CSS
/// keyframes.  This stays a source-level SVG package rather than rasterizing
/// the boards, so fonts, links, masks, and embedded assets remain available
/// to a browser.
pub fn package_svg_boards_with_animation(
    svg_boards: &[Vec<u8>],
    interval_ms: u64,
) -> Result<Vec<u8>, ExportError> {
    package_svg_boards_with_animation_hash(svg_boards, interval_ms, None)
}

/// Packages animated boards with an explicit recursive `Diagram.HashID`.
/// Native source callers can provide the target byte stream assembled from
/// the root and child boards; synthetic SVG callers keep the legacy class
/// extraction fallback through [`package_svg_boards_with_animation`].
pub fn package_svg_boards_with_animation_hash(
    svg_boards: &[Vec<u8>],
    interval_ms: u64,
    animation_hash: Option<&str>,
) -> Result<Vec<u8>, ExportError> {
    package_svg_boards_with_animation_canvas(svg_boards, interval_ms, animation_hash, None)
}

/// Target-space Diagram.NestedBoundingBox and source wrapper options. Painted
/// SVG canvases may include root borders/legends that d2animate.Wrap excludes.
#[derive(Clone, Copy, Debug)]
pub struct SvgAnimationCanvas {
    pub bounds: [f64; 4],
    pub pad: f64,
    pub scale: Option<f64>,
}

pub fn package_svg_boards_with_animation_canvas(
    svg_boards: &[Vec<u8>],
    interval_ms: u64,
    animation_hash: Option<&str>,
    canvas: Option<SvgAnimationCanvas>,
) -> Result<Vec<u8>, ExportError> {
    if svg_boards.is_empty() {
        return Err(ExportError::Render(
            "cannot create an empty animated SVG".into(),
        ));
    }
    if interval_ms == 0 {
        return Err(ExportError::Render(
            "SVG animation interval must be positive".into(),
        ));
    }
    let board_count = svg_boards.len();
    let total_duration = interval_ms
        .checked_mul(u64::try_from(board_count).unwrap_or(u64::MAX))
        .ok_or_else(|| ExportError::Render("SVG animation interval is too large".into()))?;
    let mut boards = Vec::with_capacity(board_count);
    let mut max_width = 0.0_f64;
    let mut max_height = 0.0_f64;
    let mut inner_min_x = f64::INFINITY;
    let mut inner_min_y = f64::INFINITY;
    let mut inner_max_x = f64::NEG_INFINITY;
    let mut inner_max_y = f64::NEG_INFINITY;
    let mut root_class = None;
    let mut styles = String::new();
    let mut seen_styles = BTreeSet::new();

    for (index, bytes) in svg_boards.iter().enumerate() {
        let source = std::str::from_utf8(bytes).map_err(|error| {
            ExportError::Render(format!("animated SVG board {index} was not UTF-8: {error}"))
        })?;
        let root_start = source.find("<svg").ok_or_else(|| {
            ExportError::Render(format!("animated SVG board {index} has no root element"))
        })?;
        let root_end = source[root_start..]
            .find('>')
            .map(|offset| root_start + offset)
            .ok_or_else(|| {
                ExportError::Render(format!("animated SVG board {index} has an incomplete root"))
            })?;
        let root_open = &source[root_start..=root_end];
        if root_class.is_none() {
            root_class = animated_svg_attribute(root_open, "class");
        }
        let root_close = source.rfind("</svg>").ok_or_else(|| {
            ExportError::Render(format!("animated SVG board {index} has no closing root"))
        })?;
        if root_close <= root_end {
            return Err(ExportError::Render(format!(
                "animated SVG board {index} has an invalid root range"
            )));
        }
        let root_view_box = animated_svg_attribute(root_open, "viewBox")
            .and_then(parse_svg_view_box)
            .ok_or_else(|| {
                ExportError::Render(format!("animated SVG board {index} has no numeric viewBox"))
            })?;
        let inner_source = &source[root_end + 1..root_close];
        let inner_start = inner_source.find("<svg").ok_or_else(|| {
            ExportError::Render(format!("animated SVG board {index} has no inner target"))
        })?;
        let inner_end = inner_source[inner_start..]
            .find('>')
            .map(|offset| inner_start + offset)
            .ok_or_else(|| {
                ExportError::Render(format!(
                    "animated SVG board {index} has an incomplete target"
                ))
            })?;
        let inner_close = inner_source.rfind("</svg>").ok_or_else(|| {
            ExportError::Render(format!("animated SVG board {index} has no closing target"))
        })?;
        if inner_close <= inner_end {
            return Err(ExportError::Render(format!(
                "animated SVG board {index} has an invalid target range"
            )));
        }
        let inner_open = &inner_source[inner_start..=inner_end];
        let inner_view_box = animated_svg_attribute(inner_open, "viewBox")
            .and_then(parse_svg_view_box)
            .unwrap_or(root_view_box);
        max_width = max_width.max(root_view_box.2);
        max_height = max_height.max(root_view_box.3);
        inner_min_x = inner_min_x.min(inner_view_box.0);
        inner_min_y = inner_min_y.min(inner_view_box.1);
        inner_max_x = inner_max_x.max(inner_view_box.0 + inner_view_box.2);
        inner_max_y = inner_max_y.max(inner_view_box.1 + inner_view_box.3);
        // Styles are emitted before the animated groups. They intentionally
        // remain per-board: generated masks and font-face selectors can have
        // board-specific IDs even when the theme is shared.
        for style in extract_svg_styles(&inner_source[..inner_start]) {
            if seen_styles.insert(style.clone()) {
                styles.push_str(&style);
            }
        }
        boards.push((
            inner_open[4..inner_open.len() - 1].trim().to_owned(),
            inner_source[inner_end + 1..inner_close].to_owned(),
            inner_view_box,
        ));
    }

    let animation_hash = animation_hash
        .map(ToOwned::to_owned)
        .or_else(|| extract_animation_hash(svg_boards))
        .unwrap_or_else(|| "d2-rs".to_owned());
    let animation_id = format!("d2Transition-{animation_hash}");
    if let Some(canvas) = canvas {
        let [left, top, right, bottom] = canvas.bounds;
        inner_min_x = left - canvas.pad;
        inner_min_y = top - canvas.pad;
        max_width = right - left + 2.0 * canvas.pad;
        max_height = bottom - top + 2.0 * canvas.pad;
        inner_max_x = inner_min_x + max_width;
        inner_max_y = inner_min_y + max_height;
    }
    let mut output = String::new();
    output.push_str(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
    );
    if let Some(class) = root_class {
        output.push_str(" class=\"");
        output.push_str(&escape_svg_attribute(&class));
        output.push('"');
    }
    if let Some(scale) = canvas.and_then(|c| c.scale) {
        output.push_str(&format!(
            " width=\"{}\" height=\"{}\"",
            format_svg_number((max_width * scale).ceil()),
            format_svg_number((max_height * scale).ceil())
        ));
    }
    output.push_str(&format!(
        " preserveAspectRatio=\"xMinYMin meet\" viewBox=\"0 0 {} {}\"><svg class=\"d2-svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">",
        format_svg_number(max_width),
        format_svg_number(max_height),
        format_svg_number(max_width),
        format_svg_number(max_height),
        format_svg_number(inner_min_x),
        format_svg_number(inner_min_y),
        format_svg_number(inner_max_x - inner_min_x),
        format_svg_number(inner_max_y - inner_min_y),
    ));
    output.push_str(&styles);
    output.push_str("<style type=\"text/css\"><![CDATA[");
    for index in 0..board_count {
        let total = total_duration as f64;
        let delay = index as f64 * interval_ms as f64;
        let duration = interval_ms as f64;
        let before = ((delay - 1.0).max(0.0) / total) * 100.0;
        let start = (delay / total) * 100.0;
        let active_end = ((delay + duration - 1.0) / total) * 100.0;
        if active_end.ceil() as i32 == 100 {
            output.push_str(&format!(
                "@keyframes {animation_id}-{index} {{\n\t\t0%, {before:.6}% {{\n\t\t\t\topacity: 0;\n\t\t}}\n\t\t{start:.6}%, {:.0}% {{\n\t\t\t\topacity: 1;\n\t\t}}\n}}",
                active_end.ceil()
            ));
        } else {
            let after = ((delay + duration) / total) * 100.0;
            output.push_str(&format!(
                "@keyframes {animation_id}-{index} {{\n\t\t0%, {before:.6}% {{\n\t\t\t\topacity: 0;\n\t\t}}\n\t\t{start:.6}%, {active_end:.6}% {{\n\t\t\t\topacity: 1;\n\t\t}}\n\t\t{after:.6}%, 100% {{\n\t\t\t\topacity: 0;\n\t\t}}\n}}"
            ));
        }
    }
    output.push_str("]]></style>");
    for (index, (attributes, body, _view_box)) in boards.into_iter().enumerate() {
        output.push_str(&format!(
            "<g style=\"animation: {animation_id}-{index} {total_duration}ms infinite\" {attributes}>",
        ));
        output.push_str(&body);
        output.push_str("</g>");
    }
    output.push_str("</svg></svg>");
    Ok(output.into_bytes())
}

fn extract_animation_hash(svg_boards: &[Vec<u8>]) -> Option<String> {
    for board in svg_boards {
        let source = std::str::from_utf8(board).ok()?;
        let mut cursor = 0;
        while let Some(relative) = source[cursor..].find("class=\"") {
            let start = cursor + relative + "class=\"".len();
            let end = start + source[start..].find('"')?;
            if let Some(class) = source[start..end].split_ascii_whitespace().find(|class| {
                let Some(digits) = class.strip_prefix("d2-") else {
                    return false;
                };
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                return Some(class.to_owned());
            }
            cursor = end + 1;
        }
    }
    None
}

fn animated_svg_attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn parse_svg_view_box(value: String) -> Option<(f64, f64, f64, f64)> {
    let values = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|value| !value.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (values.len() == 4
        && values.iter().all(|value| value.is_finite())
        && values[2] > 0.0
        && values[3] > 0.0)
        .then(|| (values[0], values[1], values[2], values[3]))
}

fn extract_svg_styles(source: &str) -> Vec<String> {
    let mut styles = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = source[cursor..].find("<style") {
        let start = cursor + relative_start;
        let Some(relative_end) = source[start..].find("</style>") else {
            break;
        };
        let end = start + relative_end + "</style>".len();
        styles.push(source[start..end].to_owned());
        cursor = end;
    }
    styles
}

fn escape_svg_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn format_svg_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn package_gif_frames_with_delays(
    png_frames: &[Vec<u8>],
    delays: Vec<u16>,
) -> Result<Vec<u8>, ExportError> {
    package_gif_frames_encoded(png_frames, delays, false)
}

fn package_gif_frames_encoded<P: AsRef<[u8]>>(
    png_frames: &[P],
    delays: Vec<u16>,
    opaque: bool,
) -> Result<Vec<u8>, ExportError> {
    if png_frames.is_empty() {
        return Err(ExportError::Gif("cannot create an empty GIF".into()));
    }
    if delays.len() != png_frames.len() {
        return Err(ExportError::Gif(
            "GIF frame delay count does not match frame count".into(),
        ));
    }
    // A static board repeats one image for its whole sample schedule, and
    // quantization is a pure function of the pixels: decode, quantize and
    // compress each run of identical frames once (xgif output is unchanged).
    let mut sources = Vec::with_capacity(png_frames.len());
    let mut pixmaps = Vec::new();
    for (index, png) in png_frames.iter().enumerate() {
        let png = png.as_ref();
        if index > 0 && png == png_frames[index - 1].as_ref() {
            sources.push(pixmaps.len() - 1);
            continue;
        }
        pixmaps.push(
            DecodedFrame::decode_png(png).map_err(|error| {
                ExportError::Gif(format!("could not decode PNG frame: {error}"))
            })?,
        );
        sources.push(pixmaps.len() - 1);
    }
    let width = pixmaps
        .iter()
        .map(|pixmap| pixmap.width())
        .max()
        .unwrap_or(0);
    let height = pixmaps
        .iter()
        .map(|pixmap| pixmap.height())
        .max()
        .unwrap_or(0);
    let width = u16::try_from(width)
        .map_err(|_| ExportError::Gif("GIF canvas width exceeds 65535 pixels".into()))?;
    let height = u16::try_from(height)
        .map_err(|_| ExportError::Gif("GIF canvas height exceeds 65535 pixels".into()))?;
    // The CLI's raster frames are opaque. General callers can still supply
    // transparency, which uses xgif's local-table encoding contract.
    let opaque = opaque
        && pixmaps
            .iter()
            .all(|pixmap| pixmap.data().chunks_exact(4).all(|pixel| pixel[3] == 255));
    let mut encoder = gif_encode::Encoder::new(png_frames.len(), opaque);
    let mut quantized: Option<(usize, gif::Frame<'static>)> = None;
    for (frame_index, &source) in sources.iter().enumerate() {
        match quantized.as_mut() {
            Some((cached, frame)) if *cached == source => frame.delay = delays[frame_index],
            _ => {
                let pixmap = &pixmaps[source];
                quantized = Some((
                    source,
                    gif_quantize::frame(
                        pixmap.data(),
                        pixmap.width() as u16,
                        pixmap.height() as u16,
                        width,
                        height,
                        delays[frame_index],
                    ),
                ));
            }
        }
        encoder.write_frame(&quantized.as_ref().expect("quantized frame").1);
    }
    Ok(encoder.finish())
}

/// A GIF frame's PNG decoded to premultiplied RGBA8, the pixels the frame
/// quantizer has always received (tiny-skia 0.12 Pixmap::decode_png).
struct DecodedFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl DecodedFrame {
    fn decode_png(png: &[u8]) -> Result<Self, String> {
        let mut decoder = png::Decoder::new(Cursor::new(png));
        decoder.set_transformations(
            png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
        );
        let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| "PNG frame is too large".to_owned())?;
        let mut buffer = vec![0_u8; size];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|error| error.to_string())?;
        let mut data = apng_rgba8(&buffer[..info.buffer_size()], info.color_type)?;
        for pixel in data.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                // tiny-skia color.rs premultiply_u8.
                let product = u32::from(*channel) * alpha + 128;
                *channel = ((product + (product >> 8)) >> 8) as u8;
            }
        }
        Ok(Self {
            width: info.width,
            height: info.height,
            data,
        })
    }
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Returns the centisecond delay for one unsampled board frame.
fn gif_delay_centiseconds(interval_ms: u64) -> Result<u16, ExportError> {
    interval_ms
        .saturating_add(9)
        .checked_div(10)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| ExportError::Gif("GIF animation interval is too large".into()))
}

/// Returns the centisecond delays for already sampled 30fps animation frames.
/// This is separate from [`package_gif_frames_with_interval`], whose input is
/// one frame per board and therefore keeps a full interval on each frame.
fn gif_sampled_frame_delays(frame_count: usize, interval_ms: u64) -> Result<Vec<u16>, ExportError> {
    if frame_count == 0 {
        return Err(ExportError::Gif("cannot create an empty GIF".into()));
    }
    let frames_per_board = gif_frames_per_board(interval_ms)?;
    let schedule_frames = if frame_count.is_multiple_of(frames_per_board) {
        frames_per_board
    } else {
        frame_count
    };
    let total_centiseconds = usize::from(gif_delay_centiseconds(interval_ms)?);
    let base = total_centiseconds / schedule_frames;
    let extra = total_centiseconds % schedule_frames;
    let base = u16::try_from(base)
        .map_err(|_| ExportError::Gif("GIF animation delay is too large".into()))?;
    let extra_delay = base
        .checked_add(1)
        .ok_or_else(|| ExportError::Gif("GIF animation delay is too large".into()))?;
    let mut delays = Vec::with_capacity(frame_count);
    let mut error_term = 0usize;
    for frame_index in 0..frame_count {
        let within_board = frame_index % schedule_frames;
        let delay = if extra != 0 && error_term >= schedule_frames - extra {
            error_term -= schedule_frames - extra;
            extra_delay
        } else {
            error_term += extra;
            base
        };
        delays.push(delay);
        if within_board + 1 == schedule_frames {
            error_term = 0;
        }
    }
    Ok(delays)
}

/// D2's 30-fps sample count for one board shown for `interval_ms`.
pub fn gif_frames_per_board(interval_ms: u64) -> Result<usize, ExportError> {
    interval_ms
        .checked_mul(30)
        .and_then(|value| value.checked_add(999))
        .map(|value| (value / 1000).max(1))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| ExportError::Gif("GIF animation frame count is too large".into()))
}

/// Returns the exact integer timestamp used by D2 v0.9.0's 30-fps GIF sampler.
fn gif_animation_frame_time_ns(frame_index: usize) -> Result<i64, ExportError> {
    let frame_index = i64::try_from(frame_index).map_err(|_| {
        ExportError::Gif("GIF animation frame time exceeds the duration domain".into())
    })?;
    let whole_seconds = frame_index / 30;
    let remainder = frame_index % 30;
    whole_seconds
        .checked_mul(1_000_000_000)
        .and_then(|whole| whole.checked_add(remainder * 1_000_000_000 / 30))
        .ok_or_else(|| {
            ExportError::Gif("GIF animation frame time exceeds the duration domain".into())
        })
}

fn render_gif_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, ExportError> {
    let (width, height, rgba) =
        render_static_gif_frame(graph, result, labels, styles, &options, sketch)?;
    let width_u16 = u16::try_from(width)
        .map_err(|_| ExportError::Gif("SVG width exceeds GIF's 65535-pixel limit".into()))?;
    let height_u16 = u16::try_from(height)
        .map_err(|_| ExportError::Gif("SVG height exceeds GIF's 65535-pixel limit".into()))?;
    let mut output = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut output, width_u16, height_u16, &[])
            .map_err(|error| ExportError::Gif(error.to_string()))?;
        let frame = gif_quantize::frame(&rgba, width_u16, height_u16, width_u16, height_u16, 0);
        encoder
            .write_frame(&frame)
            .map_err(|error| ExportError::Gif(error.to_string()))?;
    }
    Ok(output)
}

/// Encodes a laid-out graph as a deterministic single-page PDF.
///
/// The page embeds the same rasterized semantic target used by PNG and GIF,
/// with D2's FPDF document resources and hyperlink annotations.
pub fn render_pdf(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Vec<u8>, ExportError> {
    render_pdf_with_options(
        graph,
        result,
        labels,
        styles,
        RenderOptions::default(),
        false,
    )
}

/// Packages already-rendered PNG boards into one deterministic PDF document.
///
/// The document command uses this boundary for multi-board exports so a PDF
/// remains a paged document instead of becoming a directory of unrelated
/// single-page files.
pub fn package_pdf_pages(pages: &[Vec<u8>]) -> Result<Vec<u8>, ExportError> {
    if pages.is_empty() {
        return Err(ExportError::Pdf("cannot create an empty PDF".into()));
    }
    let decoded = pages
        .iter()
        .map(|page| decode_pdf_png(page))
        .collect::<Result<Vec<_>, _>>()?;
    encode_pdf_pages(&decoded, true)
}

/// Packages rendered boards into a PDF while preserving each board's source
/// hyperlinks as page annotations. Link rectangles use the logical output
/// canvas coordinates returned by [`document_link_regions`].
pub fn package_pdf_pages_with_link_regions(
    pages: &[Vec<u8>],
    link_regions: &[Vec<DocumentLinkRegion>],
) -> Result<Vec<u8>, ExportError> {
    if pages.is_empty() {
        return Err(ExportError::Pdf("cannot create an empty PDF".into()));
    }
    if link_regions.len() != pages.len() {
        return Err(ExportError::Pdf(
            "PDF link-region count must match page count".into(),
        ));
    }
    if link_regions.iter().all(Vec::is_empty) {
        return package_pdf_pages(pages);
    }
    let decoded = pages
        .iter()
        .map(|page| decode_pdf_png(page))
        .collect::<Result<Vec<_>, _>>()?;
    encode_pdf_pages_with_links(&decoded, link_regions, true)
}

/// Packages rendered boards with the v0.9.0 paged-export title header. The
/// final title is bold; ancestor titles receive internal page destinations.
/// Source links that name another board ID are also converted to internal
/// destinations when that board is present in `board_paths`.
pub fn package_pdf_pages_with_navigation(
    pages: &[Vec<u8>],
    link_regions: &[Vec<DocumentLinkRegion>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    include_nav: bool,
) -> Result<Vec<u8>, ExportError> {
    package_pdf_pages_with_navigation_for_theme(
        pages,
        link_regions,
        board_paths,
        title_paths,
        include_nav,
        None,
    )
}

/// Packages rendered boards with the PDF separator color selected from the
/// D2 page's default theme fill.
pub fn package_pdf_pages_with_navigation_for_theme(
    pages: &[Vec<u8>],
    link_regions: &[Vec<DocumentLinkRegion>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    include_nav: bool,
    theme_id: Option<i64>,
) -> Result<Vec<u8>, ExportError> {
    let metadata = link_regions
        .iter()
        .map(|row| row.iter().cloned().map(Into::into).collect())
        .collect::<Vec<_>>();
    package_pdf_pages_with_navigation_metadata_for_theme(
        pages,
        &metadata,
        board_paths,
        title_paths,
        include_nav,
        theme_id,
        &[],
    )
}

/// Packages typed scene links, rejecting internal destinations with no page.
pub fn package_pdf_pages_with_navigation_metadata_for_theme(
    pages: &[Vec<u8>],
    link_regions: &[Vec<DocumentLinkMetadata>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    include_nav: bool,
    theme_id: Option<i64>,
    page_fills: &[[u8; 3]],
) -> Result<Vec<u8>, ExportError> {
    validate_pdf_page_metadata(pages.len(), link_regions, board_paths, title_paths)?;
    let decoded = pages
        .iter()
        .map(|page| decode_pdf_png(page))
        .collect::<Result<Vec<_>, _>>()?;
    package_pdf_decoded_pages(
        &decoded,
        link_regions,
        board_paths,
        title_paths,
        include_nav,
        theme_id,
        page_fills,
    )
}

/// `package_pdf_pages_with_navigation_metadata_for_theme` for pages rendered
/// by `render_paged_pdf_page_at_scale`, whose images need no PNG decoding.
pub fn package_pdf_page_rasters_with_navigation_metadata_for_theme(
    pages: Vec<PdfPage>,
    link_regions: &[Vec<DocumentLinkMetadata>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    include_nav: bool,
    theme_id: Option<i64>,
    page_fills: &[[u8; 3]],
) -> Result<Vec<u8>, ExportError> {
    validate_pdf_page_metadata(pages.len(), link_regions, board_paths, title_paths)?;
    let decoded = pages.into_iter().map(|page| page.0).collect::<Vec<_>>();
    package_pdf_decoded_pages(
        &decoded,
        link_regions,
        board_paths,
        title_paths,
        include_nav,
        theme_id,
        page_fills,
    )
}

fn validate_pdf_page_metadata(
    pages: usize,
    link_regions: &[Vec<DocumentLinkMetadata>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
) -> Result<(), ExportError> {
    if pages == 0 {
        return Err(ExportError::Pdf("cannot create an empty PDF".into()));
    }
    if link_regions.len() != pages || board_paths.len() != pages || title_paths.len() != pages {
        return Err(ExportError::Pdf(
            "PDF page metadata count must match page count".into(),
        ));
    }
    Ok(())
}

fn package_pdf_decoded_pages(
    decoded: &[PdfRasterPage],
    link_regions: &[Vec<DocumentLinkMetadata>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    include_nav: bool,
    theme_id: Option<i64>,
    page_fills: &[[u8; 3]],
) -> Result<Vec<u8>, ExportError> {
    let header_rule_is_light = pdf_default_fill_is_light(theme_id);
    let page_map = board_paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| path.last().map(|id| (id.as_str(), index)))
        .collect::<BTreeMap<_, _>>();
    let mut annotations = vec![Vec::new(); decoded.len()];
    for (index, regions) in link_regions.iter().enumerate() {
        for (link_index, region) in regions.iter().enumerate() {
            if region.board_target && !page_map.contains_key(region.href.as_str()) {
                return Err(ExportError::Pdf(format!(
                    "PDF link {link_index} target {:?} has no renderable page",
                    region.href
                )));
            }
            if region.href.is_empty() {
                continue;
            }
            let destination = page_map
                .get(region.href.as_str())
                .copied()
                .map(PdfAnnotationDestination::Page)
                .unwrap_or_else(|| PdfAnnotationDestination::Uri(region.href.clone()));
            annotations[index].push(PdfAnnotation {
                rect: region.rect,
                destination,
                header: false,
            });
        }
        if include_nav {
            let names = &title_paths[index];
            let mut left = 28.0;
            for (title_index, name) in names.iter().enumerate() {
                if title_index + 1 == names.len() {
                    break;
                }
                let width = pdf_text_width(name, false);
                let target = board_paths[index]
                    .get(title_index)
                    .and_then(|id| page_map.get(id.as_str()).copied());
                if let Some(target) = target {
                    annotations[index].push(PdfAnnotation {
                        rect: Rect {
                            // fpdf.CellFormat uses its 2.835pt cell inset and
                            // centers a 14pt text link inside the 72pt cell.
                            origin: Point {
                                x: left + 2.835,
                                y: 29.0,
                            },
                            size: d2_rs_core::Size {
                                width,
                                height: 14.0,
                            },
                        },
                        destination: PdfAnnotationDestination::Page(target),
                        header: true,
                    });
                }
                left += width + pdf_text_width("  /  ", false);
            }
        }
    }
    for regions in &mut annotations {
        regions.sort_by_key(|annotation| !annotation.header);
    }
    encode_pdf_pages_with_navigation(
        decoded,
        &annotations,
        title_paths,
        include_nav,
        header_rule_is_light,
        page_fills,
    )
}

#[derive(Clone, Debug)]
struct PdfAnnotation {
    rect: Rect,
    destination: PdfAnnotationDestination,
    header: bool,
}

#[derive(Clone, Debug)]
enum PdfAnnotationDestination {
    Uri(String),
    Page(usize),
}

#[derive(Clone, Debug)]
struct PdfRasterPage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
    // FPDF preserves non-interlaced RGB PNG data, including its original
    // filtering/compression. Decoded pixels still validate the page buffer.
    rgb_png_idat: Option<Vec<u8>>,
    // A compacted page's image streams, encoded from its validated planes,
    // which it no longer keeps (`compacted`).
    streams: Option<(Vec<u8>, Option<Vec<u8>>)>,
}

impl PdfRasterPage {
    fn image_streams(&self) -> (Vec<u8>, Option<Vec<u8>>) {
        if let Some(streams) = &self.streams {
            streams.clone()
        } else if let Some(idat) = &self.rgb_png_idat {
            (idat.clone(), None)
        } else {
            go_png::pdf_streams(self.width, &self.rgb, self.alpha.as_deref())
        }
    }

    /// The page with its image streams encoded now and its planes released.
    /// A document keeps every page until it is packaged; like FPDF, which
    /// keeps each page image's compressed streams, it then holds no pixels.
    fn compacted(mut self) -> Result<Self, ExportError> {
        validate_pdf_raster(&self)?;
        let streams = self.image_streams();
        self.rgb = Vec::new();
        if let Some(alpha) = &mut self.alpha {
            *alpha = Vec::new();
        }
        self.rgb_png_idat = None;
        self.streams = Some(streams);
        Ok(self)
    }
}

fn pdf_font_width_units(advance: u16, units_per_em: u16) -> u16 {
    (u32::from(advance) * 1000 + u32::from(units_per_em) / 2)
        .checked_div(u32::from(units_per_em))
        .unwrap_or_default()
        .min(u32::from(u16::MAX)) as u16
}

fn validate_pdf_raster(page: &PdfRasterPage) -> Result<(), ExportError> {
    if page.streams.is_some() {
        // Validated when it was compacted.
        return Ok(());
    }
    let pixels = (page.width as usize).saturating_mul(page.height as usize);
    if page.rgb.len() != pixels.saturating_mul(3)
        || page
            .alpha
            .as_ref()
            .is_some_and(|alpha| alpha.len() != pixels)
    {
        return Err(ExportError::Pdf(
            "raster dimensions do not match the pixel buffer".into(),
        ));
    }
    Ok(())
}

fn pdf_text_width(value: &str, bold: bool) -> f64 {
    let bytes = if bold {
        d2_rs_text::measure::bundled_source_sans_pro_bold_bytes()
    } else {
        d2_rs_text::measure::bundled_source_sans_pro_regular_bytes()
    };
    let Ok(face) = ttf_parser::Face::parse(bytes, 0) else {
        return value.chars().count() as f64 * 7.0;
    };
    let units_per_em = face.units_per_em();
    if units_per_em == 0 {
        return value.chars().count() as f64 * 7.0;
    }
    let width_units = value
        .chars()
        .filter_map(|character| face.glyph_index(character))
        .filter_map(|glyph| face.glyph_hor_advance(glyph))
        .map(|advance| u32::from(pdf_font_width_units(advance, units_per_em)))
        .sum::<u32>();
    f64::from(width_units) * 14.0 / 1000.0
}

fn pdf_default_fill_is_light(theme_id: Option<i64>) -> bool {
    let palette = theme::palette(theme_id.unwrap_or(0))
        .or_else(|| theme::palette(0))
        .expect("the default D2 theme palette exists");
    let hex = palette.neutrals.n7.trim_start_matches('#');
    let Ok(rgb) = u32::from_str_radix(hex, 16) else {
        return true;
    };
    let red = f64::from((rgb >> 16) & 0xff);
    let green = f64::from((rgb >> 8) & 0xff);
    let blue = f64::from(rgb & 0xff);
    (0.299 * red * red + 0.587 * green * green + 0.114 * blue * blue).sqrt() > 130.0
}

fn pdf_timestamp_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    pdf_timestamp_at_utc(seconds)
}

fn pdf_timestamp_at_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;

    // Convert days since 1970-01-01 to the proleptic Gregorian calendar.
    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);

    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;
    format!("D:{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}+00'00'")
}

fn encode_pdf_pages_with_navigation(
    pages: &[PdfRasterPage],
    annotations: &[Vec<PdfAnnotation>],
    title_paths: &[Vec<String>],
    include_nav: bool,
    header_rule_is_light: bool,
    page_fills: &[[u8; 3]],
) -> Result<Vec<u8>, ExportError> {
    pdf_document::encode(
        pages,
        annotations,
        title_paths,
        include_nav,
        header_rule_is_light,
        page_fills,
        &pdf_document::timestamp()?,
    )
}

fn decode_pdf_png(png_bytes: &[u8]) -> Result<PdfRasterPage, ExportError> {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| ExportError::Pdf(format!("could not decode PNG page: {error}")))?;
    let mut buffer = vec![
        0;
        reader.output_buffer_size().ok_or_else(|| {
            ExportError::Pdf("PNG decoder did not report an output buffer size".into())
        })?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| ExportError::Pdf(format!("could not decode PNG page: {error}")))?;
    if info.bit_depth != png::BitDepth::Eight
        || !matches!(info.color_type, png::ColorType::Rgb | png::ColorType::Rgba)
    {
        return Err(ExportError::Pdf(
            "PDF pages require 8-bit RGB or RGBA PNGs".into(),
        ));
    }
    let pixel_bytes = &buffer[..info.buffer_size()];
    let (rgb, alpha) = match info.color_type {
        png::ColorType::Rgb => (pixel_bytes.to_vec(), None),
        png::ColorType::Rgba => {
            let mut rgb = Vec::with_capacity(pixel_bytes.len() / 4 * 3);
            let mut alpha = Vec::with_capacity(pixel_bytes.len() / 4);
            for pixel in pixel_bytes.chunks_exact(4) {
                rgb.extend_from_slice(&pixel[..3]);
                alpha.push(pixel[3]);
            }
            (rgb, Some(alpha))
        }
        _ => unreachable!("color type was checked above"),
    };
    let preserve_idat = info.color_type == png::ColorType::Rgb && !reader.info().interlaced;
    reader
        .finish()
        .map_err(|error| ExportError::Pdf(format!("could not decode PNG page: {error}")))?;
    let rgb_png_idat = preserve_idat
        .then(|| pdf_rgb_png_idat(png_bytes))
        .transpose()?;
    Ok(PdfRasterPage {
        width: info.width,
        height: info.height,
        rgb,
        alpha,
        rgb_png_idat,
        streams: None,
    })
}

// FPDF png.go:120–124,247 concatenates every IDAT payload. Call only after
// the PNG decoder has validated the complete non-interlaced RGB input.
fn pdf_rgb_png_idat(png: &[u8]) -> Result<Vec<u8>, ExportError> {
    let invalid = || ExportError::Pdf("could not decode PNG page: invalid chunk envelope".into());
    let mut chunks = png.get(8..).ok_or_else(invalid)?;
    let mut idat = Vec::new();
    loop {
        let header = chunks.get(..8).ok_or_else(invalid)?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = length.checked_add(12).ok_or_else(invalid)?;
        let chunk = chunks.get(..end).ok_or_else(invalid)?;
        match &header[4..] {
            b"IDAT" => idat.extend_from_slice(&chunk[8..end - 4]),
            b"IEND" => return Ok(idat),
            _ => {}
        }
        chunks = &chunks[end..];
    }
}

// All PDF entry points share D2 lib/pdf's FPDF document writer, including
// font resources, PNG streams, page links and the explicit clock control.
fn encode_pdf_pages(
    pages: &[PdfRasterPage],
    header_rule_is_light: bool,
) -> Result<Vec<u8>, ExportError> {
    encode_pdf_pages_with_links(pages, &vec![Vec::new(); pages.len()], header_rule_is_light)
}

fn encode_pdf_pages_with_links(
    pages: &[PdfRasterPage],
    link_regions: &[Vec<DocumentLinkRegion>],
    header_rule_is_light: bool,
) -> Result<Vec<u8>, ExportError> {
    if pages.is_empty() || pages.len() != link_regions.len() {
        return Err(ExportError::Pdf(
            "PDF page and link-region counts must match".into(),
        ));
    }
    let annotations = link_regions
        .iter()
        .map(|regions| {
            regions
                .iter()
                .filter(|region| !region.href.is_empty())
                .map(|region| PdfAnnotation {
                    rect: region.rect,
                    destination: PdfAnnotationDestination::Uri(region.href.clone()),
                    header: false,
                })
                .collect()
        })
        .collect::<Vec<_>>();
    encode_pdf_pages_with_navigation(
        pages,
        &annotations,
        &vec![Vec::new(); pages.len()],
        false,
        header_rule_is_light,
        &[],
    )
}

fn render_pdf_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, ExportError> {
    let header_rule_is_light = pdf_default_fill_is_light(options.theme_id);
    let frame = render_paged_frame(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        2,
        &mut PagedFrameBudget::default(),
    )
    .map_err(|error| match error {
        error @ ExportError::Render(_) => error,
        ExportError::Png(message) => ExportError::Pdf(message),
        other => ExportError::Pdf(other.to_string()),
    })?;
    let link_regions = linked_regions(frame.links);
    let page = pdf_page_from_rgba(frame.width, frame.height, frame.rgba)?;
    encode_pdf_pages_with_links(&[page], &[link_regions], header_rule_is_light)
}

/// Computes source hyperlink rectangles in the rendered output coordinate
/// system for a graph that is about to be packaged as a document page.
pub fn document_link_regions(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<DocumentLinkRegion>, ExportError> {
    document_link_metadata(graph, result, labels, styles, options, sketch).map(linked_regions)
}

/// The regions of links with a destination (pdfLinkShapes skips the rest).
fn linked_regions(links: Vec<DocumentLinkMetadata>) -> Vec<DocumentLinkRegion> {
    links
        .into_iter()
        .filter(|m| !m.href.is_empty())
        .map(|m| m.region)
        .collect()
}

/// Source scene links with URL/board provenance and tooltip text intact,
/// mapped as d2cli/paged.go maps a board's links into its frame. Paged
/// renders get the same links with their pixels from
/// `render_paged_png_with_links` and `render_paged_pdf_page_with_links`.
pub fn document_link_metadata(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<DocumentLinkMetadata>, ExportError> {
    paged_admission::document_links(graph, result, labels, styles, &options, sketch)
}

fn pdf_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('(');
    for byte in value.bytes() {
        match byte {
            b'(' | b')' | b'\\' => {
                escaped.push('\\');
                escaped.push(byte as char);
            }
            0x20..=0x7e => escaped.push(byte as char),
            _ => escaped.push('?'),
        }
    }
    escaped.push(')');
    escaped
}

/// Encodes one laid-out graph as a deterministic OOXML PowerPoint package.
///
/// D2's v0.9.0 PPTX exporter places a rendered PNG on each slide.  This
/// single-layout entry point follows that same package shape and leaves
/// multiboard navigation to the document exporter.
pub fn render_pptx(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
) -> Result<Vec<u8>, ExportError> {
    render_pptx_with_options(
        graph,
        result,
        labels,
        styles,
        RenderOptions::default(),
        false,
    )
}

/// Packages already-rendered PNG boards into one deterministic PPTX deck.
/// This is used by the document command when a source contains multiple
/// boards; each board is still laid out and rasterized independently.
pub fn package_pptx_slides(slides: &[Vec<u8>]) -> Result<Vec<u8>, ExportError> {
    pptx::package_slides(slides).map_err(ExportError::Pptx)
}

/// Packages rendered boards into a PPTX while preserving their stable D2
/// board IDs in the picture metadata. The IDs are also used by the v0.9.0
/// exporter as the semantic description of each slide image.
pub fn package_pptx_slides_with_board_ids(
    slides: &[Vec<u8>],
    board_ids: &[String],
) -> Result<Vec<u8>, ExportError> {
    pptx::package_slides_with_board_ids(slides, board_ids).map_err(ExportError::Pptx)
}

/// Packages rendered boards while retaining each board's ancestor path for
/// PPTX navigation relationships. Paths contain stable `root.*` board IDs in
/// document order.
pub fn package_pptx_slides_with_board_paths(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
) -> Result<Vec<u8>, ExportError> {
    pptx::package_slides_with_board_paths(slides, board_paths).map_err(ExportError::Pptx)
}

/// Packages rendered boards into a PPTX while preserving source hyperlinks
/// on each slide. Regions use the logical output-canvas coordinates returned
/// by [`document_link_regions`].
pub fn package_pptx_slides_with_board_paths_and_link_regions(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    link_regions: &[Vec<DocumentLinkRegion>],
) -> Result<Vec<u8>, ExportError> {
    if slides.len() != link_regions.len() {
        return Err(ExportError::Pptx(
            "PPTX link-region count must match slide count".into(),
        ));
    }
    let mapped = slides
        .iter()
        .zip(link_regions.iter())
        .map(|(png, regions)| pptx_link_regions(png, regions.clone(), false))
        .collect::<Vec<_>>();
    pptx::package_slides_with_board_paths_and_links(slides, board_paths, &mapped)
        .map_err(ExportError::Pptx)
}

/// Packages rendered boards with the v0.9.0 title-path header and navigation
/// metadata. `title_paths` contains display names in the same order as the
/// corresponding `board_paths`; the final name is bold and ancestor names are
/// linked to their board slides.
pub fn package_pptx_slides_with_navigation(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    link_regions: &[Vec<DocumentLinkRegion>],
    include_nav: bool,
) -> Result<Vec<u8>, ExportError> {
    let metadata = link_regions
        .iter()
        .map(|row| row.iter().cloned().map(Into::into).collect())
        .collect::<Vec<_>>();
    package_pptx_slides_with_navigation_metadata(
        slides,
        board_paths,
        title_paths,
        &metadata,
        include_nav,
    )
}

/// Packages typed links with explicit tooltips and board destinations.
pub fn package_pptx_slides_with_navigation_metadata(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    link_regions: &[Vec<DocumentLinkMetadata>],
    include_nav: bool,
) -> Result<Vec<u8>, ExportError> {
    package_pptx_slides_with_document_properties(
        slides,
        board_paths,
        title_paths,
        link_regions,
        include_nav,
        &PptxDocumentProperties::default(),
    )
}

/// Packages typed links and explicit presentation properties without global state.
pub fn package_pptx_slides_with_document_properties(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    title_paths: &[Vec<String>],
    link_regions: &[Vec<DocumentLinkMetadata>],
    include_nav: bool,
    properties: &PptxDocumentProperties,
) -> Result<Vec<u8>, ExportError> {
    if slides.len() != link_regions.len() {
        return Err(ExportError::Pptx(
            "PPTX link-region count must match slide count".into(),
        ));
    }
    if slides.len() != title_paths.len() {
        return Err(ExportError::Pptx(
            "PPTX title-path count must match slide count".into(),
        ));
    }
    let mapped = slides
        .iter()
        .zip(link_regions.iter())
        .map(|(png, regions)| pptx_link_metadata(png, regions.clone(), include_nav))
        .collect::<Vec<_>>();
    let page_map = board_paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| path.last().map(|id| (id.as_str(), index + 1)))
        .collect::<BTreeMap<_, _>>();
    for regions in link_regions {
        for (index, region) in regions.iter().enumerate() {
            if region.board_target && !page_map.contains_key(region.href.as_str()) {
                return Err(ExportError::Pptx(format!(
                    "PPTX link {index} target {:?} has no renderable slide",
                    region.href
                )));
            }
        }
    }
    let mut mapped = mapped;
    for (regions, mapped_regions) in link_regions.iter().zip(mapped.iter_mut()) {
        let mut source_regions = regions.iter().filter(|region| !region.href.is_empty());
        for mapped_region in mapped_regions {
            let Some(source_region) = source_regions.next() else {
                break;
            };
            mapped_region.target_slide = page_map.get(source_region.href.as_str()).copied();
        }
    }
    let titles = title_paths
        .iter()
        .map(|names| pptx::BoardTitlePath {
            names: names.clone(),
        })
        .collect::<Vec<_>>();
    pptx::package_slides_with_properties(
        slides,
        board_paths,
        &titles,
        &mapped,
        include_nav,
        properties,
    )
    .map_err(ExportError::Pptx)
}

fn render_pptx_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, ExportError> {
    let include_nav = options
        .target_root_label
        .as_deref()
        .is_some_and(|s| !s.is_empty());
    let (png, semantic_links) = render_paged_png_with_links(
        graph,
        result,
        labels,
        styles,
        options,
        sketch,
        2,
        &mut PagedFrameBudget::default(),
    )?;
    package_pptx_slides_with_navigation_metadata(
        &[png],
        &[vec!["root".into()]],
        &[vec!["root".into()]],
        &[semantic_links],
        include_nav,
    )
}

fn pptx_link_regions(
    png: &[u8],
    regions: Vec<DocumentLinkRegion>,
    include_nav: bool,
) -> Vec<pptx::LinkRegion> {
    pptx_link_metadata(
        png,
        regions.into_iter().map(Into::into).collect(),
        include_nav,
    )
}

fn pptx_link_metadata(
    png: &[u8],
    regions: Vec<DocumentLinkMetadata>,
    include_nav: bool,
) -> Vec<pptx::LinkRegion> {
    if png.len() < 24 {
        return Vec::new();
    }
    let source_width = u32::from_be_bytes(png[16..20].try_into().unwrap_or_default());
    let source_height = u32::from_be_bytes(png[20..24].try_into().unwrap_or_default());
    if source_width == 0 || source_height == 0 {
        return Vec::new();
    }
    let (image_width, _image_height, image_top) =
        pptx::fit_image_for_slide(source_width, source_height, include_nav);
    let image_left = (pptx::SLIDE_WIDTH - image_width) / 2;
    let factor = f64::from(image_width) / f64::from(source_width);
    regions
        .into_iter()
        .filter(|region| !region.href.is_empty())
        .map(|region| {
            // paged.go rounds outward in source pixels. lib/pptx deliberately
            // truncates its scale factor for extents, but multiplies before
            // truncation for positions; do not replace this with nearest rounding.
            let left = (region.rect.origin.x * 2.0).floor();
            let top = (region.rect.origin.y * 2.0).floor();
            let right = ((region.rect.origin.x + region.rect.size.width) * 2.0).ceil();
            let bottom = ((region.rect.origin.y + region.rect.size.height) * 2.0).ceil();
            pptx::LinkRegion {
                left: i64::from(image_left) + (left * factor) as i64,
                top: i64::from(image_top) + (top * factor) as i64,
                width: (right - left) as i64 * factor as i64,
                height: (bottom - top) as i64 * factor as i64,
                href: region.region.href,
                tooltip: region.tooltip,
                target_slide: None,
            }
        })
        .collect()
}

fn push_pdf_object_at(pdf: &mut Vec<u8>, offsets: &mut [usize], number: usize, body: &[u8]) {
    offsets[number] = pdf.len();
    pdf.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    pdf.extend_from_slice(body);
    pdf.extend_from_slice(b"\nendobj\n");
}

// Source: d2cli/raster.go gifDocumentFrameBounds. These are logical GIF
// pixels; ordinary raster and streaming PNG callers have different ceilings.
fn validate_gif_frame_dimensions(width: f64, height: f64) -> Result<(), ExportError> {
    const MAX_DIMENSION: f64 = 32_768.0;
    const MAX_PIXELS: u64 = 2 * 1024 * 1024;
    let error = if !width.is_finite() || width <= 0.0 || !height.is_finite() || height <= 0.0 {
        Some("GIF scene has invalid logical dimensions".to_owned())
    } else if width.ceil() > MAX_DIMENSION || height.ceil() > MAX_DIMENSION {
        Some(format!(
            "GIF scene dimensions {:.0}x{:.0} exceed the 32768-pixel dimension limit",
            width.ceil(),
            height.ceil(),
        ))
    } else if width.ceil() as u64 > MAX_PIXELS / height.ceil() as u64 {
        Some(format!(
            "GIF frame pixels exceed the {MAX_PIXELS}-pixel limit"
        ))
    } else {
        None
    };
    match error {
        Some(message) => Err(ExportError::Render(message)),
        None => Ok(()),
    }
}

fn apng_rgba8(data: &[u8], color_type: png::ColorType) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity(
        data.len()
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_div(color_type.samples() as usize))
            .ok_or_else(|| "APNG pixel buffer is too large".to_owned())?,
    );
    match color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => {
            for pixel in data.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &value in data {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in data.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err("APNG decoder retained indexed pixels".into());
        }
    }
    Ok(rgba)
}

/// Native source JPEG decoding preserves sixteen-bit colors through EXIF
/// orientation; PNG16 carries those values into the shared source sampler.
fn jpeg_frame_png(
    data: &[u8],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(i64, i64, Vec<u8>), String> {
    let poll = || {
        cancel
            .and_then(d2_rs_core::CancellationToken::error)
            .map_or(Ok(()), Err)
    };
    let mut input = d2_rs_core::jpeg_first::Bytes { data, offset: 0 };
    let decoded = d2_rs_core::jpeg_first::decode(data, &mut input, &poll)?;
    let (width, height) = (decoded.width, decoded.height);
    let mut bytes = Vec::with_capacity(width * height * 8);
    for pixel in decoded.rgba {
        for channel in pixel {
            bytes.extend_from_slice(&channel.to_be_bytes());
        }
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    drop(writer);
    Ok((width as i64, height as i64, output))
}

#[cfg(test)]
fn oriented_source_point(
    orientation: u8,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> (usize, usize) {
    match orientation {
        2 => (width - 1 - x, y),
        3 => (width - 1 - x, height - 1 - y),
        4 => (x, height - 1 - y),
        5 => (y, x),
        6 => (y, height - 1 - x),
        7 => (width - 1 - y, height - 1 - x),
        8 => (width - 1 - y, x),
        _ => (x, y),
    }
}

/// Reads the optional EXIF Orientation value without making malformed
/// metadata fatal.  This follows D2's rasterimage behavior: invalid or
/// unsupported EXIF leaves the JPEG in its encoded orientation.
#[cfg(test)]
fn jpeg_exif_orientation(data: &[u8]) -> u8 {
    d2_rs_core::raster_orientation::jpeg(data, None).expect("uncancelled orientation read")
}

/// Renders a deterministic ASCII projection of a layout.
///
/// The coordinate conversion and drawing order follow D2 v0.9.0's
/// `d2ascii` boundary: geometry is calibrated with the bundled font cell,
/// shapes are drawn as borders, routes are rasterized after shapes, and
/// arrowheads occupy the last route cell before the destination boundary.
/// The target remains intentionally terminal-oriented; SVG is the lossless
/// semantic target for richer styling and assets.
pub fn render_ascii(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
) -> Result<Vec<u8>, ExportError> {
    render_ascii_with_options(
        graph,
        result,
        labels,
        &BTreeMap::new(),
        &RenderOptions::default(),
        false,
    )
}

/// Export measured native graph records once, then use the shared target artist.
/// No geometry is recomputed, and no authored text is rewritten as a charset.
pub fn render_ascii_with_options(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, ExportError> {
    let primary_mono = options
        .theme_id
        .and_then(theme::palette)
        .is_some_and(|p| p.special_rules.mono);
    let diagram = serde_json::json!({
        "shapes": d2_rs_render::target_shapes_metadata(graph,result,labels,styles).map_err(|e|ExportError::Render(e.to_string()))?,
        "connections": d2_rs_render::target_connections_metadata(graph,result,styles).map_err(|e|ExportError::Render(e.to_string()))?,
        "root": d2_rs_render::target_root_metadata(graph,options).map_err(|e|ExportError::Render(e.to_string()))?,
        "fontFamily": if primary_mono {"SourceCodePro"} else if sketch {"HandDrawn"} else {"SourceSansPro"},
        "monoFontFamily":"SourceCodePro"
    });
    let diagram: d2_rs_render::target::TargetDiagram =
        serde_json::from_value(diagram).map_err(|e| ExportError::Render(e.to_string()))?;
    ascii_target::render(
        &diagram,
        options.ascii_mode.as_deref().unwrap_or("extended"),
        options.scale,
    )
    .map(String::into_bytes)
    .map_err(ExportError::Render)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn native_raster_keeps_embedded_raster_images() {
        // Keep a real PNG asset here so bundled assets cannot silently drop
        // from PNG/PDF/PPTX output.
        let mut asset = Vec::new();
        let mut encoder = png::Encoder::new(&mut asset, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[
                220, 20, 30, 255, 220, 20, 30, 255, 220, 20, 30, 255, 220, 20, 30, 255,
            ])
            .unwrap();
        drop(writer);

        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node {
            shape: d2_rs_core::ShapeKind::Image,
            size: d2_rs_core::Size {
                width: 128.0,
                height: 128.0,
            },
            ..d2_rs_core::Node::new("asset", d2_rs_core::Size::default())
        });
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut styles = BTreeMap::new();
        styles.insert(
            node,
            SourceNodeStyle {
                icon: Some("asset.png".into()),
                ..SourceNodeStyle::default()
            },
        );
        let output = render_png_at_scale(
            &graph,
            &result,
            &BTreeMap::new(),
            &styles,
            RenderOptions {
                asset_data: BTreeMap::from([("asset.png".into(), asset)]),
                ..RenderOptions::default()
            },
            false,
            1,
        )
        .unwrap();
        let decoder = png::Decoder::new(Cursor::new(output));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert!(
            pixels
                .chunks_exact(3)
                .any(|pixel| pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80)
        );
    }

    #[test]
    fn acquired_pngs_validate_the_container_before_retention() {
        use base64::Engine as _;
        // registry-sweep.py's icon-png witness. Go imageasset.probeRaster
        // rejects its IDAT checksum before dimensions or pixel decoding.
        let malformed = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAKklEQVR4nGNgGAWjYBSMglEwCkbBKBgFo2AUjIJRMApGwSgYBaNgFIyCUQAAuAQAAXhm0nEAAAAASUVORK5CYII=")
            .unwrap();
        assert_eq!(
            prepare_image_asset_bytes("./icon.png", None, &malformed).unwrap_err(),
            "malformed PNG image: invalid PNG IDAT checksum"
        );
        let mut valid = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut valid, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[20, 40, 60, 255])
                .unwrap();
        }
        // Go ignores bytes after IEND and retains the original encoding.
        valid.extend_from_slice(b"trailing data");
        assert_eq!(
            prepare_image_asset_bytes("./icon.png", None, &valid).unwrap(),
            valid
        );
        assert_eq!(
            prepare_image_asset_bytes("./icon.png", None, &valid[..33]).unwrap_err(),
            "malformed PNG image: truncated PNG chunk"
        );
    }

    #[test]
    fn parses_formats_and_extensions() {
        assert_eq!(ExportFormat::parse("SVG").unwrap(), ExportFormat::Svg);
        assert_eq!(ExportFormat::parse("text").unwrap().extension(), "txt");
        assert_eq!(ExportFormat::parse("ppt").unwrap(), ExportFormat::Pptx);
        assert_eq!(
            ExportFormat::parse("jpeg"),
            Err(ExportError::UnknownFormat("jpeg".into()))
        );
    }

    #[test]
    fn animated_canvas_uses_target_bounds_scale_and_preserves_nested_svg() {
        let board=br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 150"><svg viewBox="-50 -60 100 150"><g><svg viewBox="0 0 1 1"><path d="M0 0L1 1"/></svg></g></svg></svg>"#;
        let output = package_svg_boards_with_animation_canvas(
            &[board.to_vec()],
            500,
            Some("d2-123"),
            Some(SvgAnimationCanvas {
                bounds: [-1.0, -2.0, 19.0, 28.0],
                pad: 20.0,
                scale: Some(1.25),
            }),
        )
        .unwrap();
        let svg = std::str::from_utf8(&output).unwrap();
        let doc = roxmltree::Document::parse(svg).unwrap();
        let root = doc.root_element();
        assert_eq!(root.attribute("width"), Some("75"));
        assert_eq!(root.attribute("height"), Some("88"));
        assert_eq!(root.attribute("viewBox"), Some("0 0 60 70"));
        let inner = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "svg")
            .unwrap();
        assert_eq!(inner.attribute("viewBox"), Some("-21 -22 60 70"));
        assert_eq!(
            doc.descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "svg")
                .count(),
            3
        );
        assert_eq!(
            doc.descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "path")
                .count(),
            1
        );
    }

    #[test]
    fn animated_svg_preserves_board_targets_and_interval() {
        let first = br#"<svg data-d2-version="v0.9.0" class="d2-rs-theme" viewBox="0 0 20 30"><style>.first{fill:red}</style><svg class="d2-a d2-svg" width="20" height="30" viewBox="-1 -2 20 30"><rect class="first" width="5" height="6"/></svg></svg>"#;
        let second = br#"<svg data-d2-version="v0.9.0" class="d2-rs-theme" viewBox="0 0 40 10"><style>.second{fill:blue}</style><svg class="d2-b d2-svg" width="40" height="10" viewBox="0 0 40 10"><rect class="second" width="7" height="8"/></svg></svg>"#;
        let output = String::from_utf8(
            package_svg_boards_with_animation(&[first.to_vec(), second.to_vec()], 125).unwrap(),
        )
        .unwrap();
        assert!(output.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(output.contains("viewBox=\"0 0 40 30\""));
        assert!(output.contains("@keyframes d2Transition-d2-rs-0"));
        assert!(output.contains("animation: d2Transition-d2-rs-0 250ms infinite"));
        assert!(output.contains("animation: d2Transition-d2-rs-1 250ms infinite"));
        assert!(output.contains("class=\"first\""));
        assert!(output.contains("class=\"second\""));
        assert_eq!(output.matches("</g>").count(), 2);
    }

    #[test]
    fn ascii_matches_v090_calibrated_single_edge() {
        let mut graph = Graph::default();
        let a = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        let b = graph.add_node(d2_rs_core::Node::new(
            "b",
            d2_rs_core::Size {
                width: 53.0,
                height: 66.0,
            },
        ));
        // The source target exporter retains a nil label-position pointer;
        // model actual measured input explicitly instead of relying on the
        // retired ASCII painter's invented centering for an unset pointer.
        for id in [a, b] {
            graph.node_mut(id).unwrap().label_position =
                d2_rs_core::LabelPosition::InsideMiddleCenter;
            graph.node_mut(id).unwrap().label_size = Some(d2_rs_core::Size {
                width: 9.0,
                height: 21.0,
            });
        }
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
        let result = d2_rs_core::LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: [
                (
                    a,
                    d2_rs_core::Rect {
                        origin: d2_rs_core::Point { x: 0.0, y: 0.0 },
                        size: d2_rs_core::Size {
                            width: 53.0,
                            height: 66.0,
                        },
                    },
                ),
                (
                    b,
                    d2_rs_core::Rect {
                        origin: d2_rs_core::Point { x: 0.0, y: 132.0 },
                        size: d2_rs_core::Size {
                            width: 53.0,
                            height: 66.0,
                        },
                    },
                ),
            ]
            .into_iter()
            .collect(),
            routes: [(
                edge,
                vec![
                    d2_rs_core::Point { x: 26.0, y: 66.0 },
                    d2_rs_core::Point { x: 26.0, y: 132.0 },
                ],
            )]
            .into_iter()
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
        let labels = [(a, "a".to_owned()), (b, "b".to_owned())]
            .into_iter()
            .collect();
        let output = render_ascii(&graph, &result, &labels).unwrap();
        assert_eq!(
            output,
            "┌────┐\n│ a  │\n│    │\n└────┘\n   │  \n   ▼  \n┌────┐\n│ b  │\n│    │\n└────┘\n"
                .as_bytes()
        );
    }

    #[test]
    fn gif_encoder_returns_a_single_frame_stream() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let bytes = render_gif(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");
        assert_eq!(&bytes[bytes.len() - 1..], b";");
    }

    #[test]
    fn gif_delays_match_v090_sample_schedule() {
        // 250ms is eight 30fps samples. The interval is 25 centiseconds, so
        // eight frames receive [3, 3, 3, 3, 3, 3, 3, 4] centiseconds.
        assert_eq!(
            gif_sampled_frame_delays(8, 250).unwrap(),
            vec![3, 3, 3, 3, 3, 3, 3, 4]
        );

        // A caller supplying one frame per board still gets the complete
        // requested board interval when it is not a sampled animation.
        assert_eq!(gif_sampled_frame_delays(3, 250).unwrap(), vec![8, 8, 9]);
    }

    #[test]
    fn png_encoder_returns_a_native_png_stream() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let bytes = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        // Match D2's opaque RGB PNG contract rather than leaking the
        // intermediate SVG raster's alpha channel into the package.
        assert_eq!(bytes[24], 8, "PNG must use 8 bits per channel");
        assert_eq!(bytes[25], 2, "PNG must use RGB color type");
        let logical = render_png_at_scale(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
            1,
        )
        .unwrap();
        let dimensions = |png: &[u8]| {
            (
                u32::from_be_bytes(png[16..20].try_into().unwrap()),
                u32::from_be_bytes(png[20..24].try_into().unwrap()),
            )
        };
        let (width, height) = dimensions(&bytes);
        let (logical_width, logical_height) = dimensions(&logical);
        assert_eq!((width, height), (logical_width * 2, logical_height * 2));
    }

    #[test]
    fn sequence_png_keeps_blank_canvas_white_around_masked_routes() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let a = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 80.0,
                height: 50.0,
            },
        ));
        let b = graph.add_node(d2_rs_core::Node::new(
            "b",
            d2_rs_core::Size {
                width: 80.0,
                height: 50.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge {
            source: a,
            target: b,
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
                    a,
                    d2_rs_core::Rect {
                        origin: d2_rs_core::Point { x: 0.0, y: 0.0 },
                        size: graph.node(a).unwrap().size,
                    },
                ),
                (
                    b,
                    d2_rs_core::Rect {
                        origin: d2_rs_core::Point { x: 220.0, y: 0.0 },
                        size: graph.node(b).unwrap().size,
                    },
                ),
            ]),
            routes: BTreeMap::from([(
                edge,
                vec![
                    d2_rs_core::Point { x: 80.0, y: 25.0 },
                    d2_rs_core::Point { x: 220.0, y: 25.0 },
                ],
            )]),
            edge_labels: BTreeMap::from([(
                edge,
                d2_rs_core::EdgeLabel {
                    text: "request".into(),
                    size: d2_rs_core::Size {
                        width: 54.0,
                        height: 20.0,
                    },
                    position: d2_rs_core::LabelPosition::OutsideTopCenter,
                    percentage: 0.5,
                },
            )]),
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

        let output = render_png_at_scale(
            &graph,
            &result,
            &BTreeMap::new(),
            &BTreeMap::new(),
            RenderOptions {
                pad: 12.0,
                ..RenderOptions::default()
            },
            false,
            1,
        )
        .unwrap();
        let decoder = png::Decoder::new(Cursor::new(output));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        let bottom_right =
            ((info.height as usize - 1) * info.width as usize + info.width as usize - 1) * 3;
        assert_eq!(&pixels[bottom_right..bottom_right + 3], &[255, 255, 255]);
    }

    #[test]
    fn jpeg_exif_orientation_matches_v090_marker_rules() {
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(b"II");
        exif.extend_from_slice(&42_u16.to_le_bytes());
        exif.extend_from_slice(&8_u32.to_le_bytes());
        exif.extend_from_slice(&1_u16.to_le_bytes());
        exif.extend_from_slice(&0x0112_u16.to_le_bytes());
        exif.extend_from_slice(&3_u16.to_le_bytes());
        exif.extend_from_slice(&1_u32.to_le_bytes());
        exif.extend_from_slice(&6_u16.to_le_bytes());
        exif.extend_from_slice(&[0, 0, 0, 0]);
        exif.extend_from_slice(&[0, 0, 0, 0]);
        let segment_length = u16::try_from(exif.len() + 2).unwrap();
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&segment_length.to_be_bytes());
        jpeg.extend_from_slice(&exif);
        jpeg.extend_from_slice(&[0xff, 0xda]);
        assert_eq!(jpeg_exif_orientation(&jpeg), 6);
        assert_eq!(oriented_source_point(6, 0, 0, 2, 3), (0, 2));
        assert_eq!(jpeg_exif_orientation(b"\xff\xd8\xff\xe1\x00\x02"), 1);
        // Go stops at the first orientation tag when its type is invalid.
        // A later valid tag in the same TIFF directory cannot rotate the image.
        let mut duplicate = exif[..16].to_vec();
        duplicate[14..16].copy_from_slice(&2_u16.to_le_bytes());
        let mut invalid = exif[16..28].to_vec();
        invalid[2..4].copy_from_slice(&2_u16.to_le_bytes());
        duplicate.extend_from_slice(&invalid);
        duplicate.extend_from_slice(&exif[16..28]);
        let mut ambiguous = vec![0xff, 0xd8, 0xff, 0xe1];
        ambiguous.extend_from_slice(&u16::try_from(duplicate.len() + 2).unwrap().to_be_bytes());
        ambiguous.extend_from_slice(&duplicate);
        ambiguous.extend_from_slice(&[0xff, 0xda]);
        assert_eq!(jpeg_exif_orientation(&ambiguous), 1);
    }

    #[test]
    fn png_export_applies_canvas_padding_options() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let padded = export_with_options(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            ExportFormat::Png,
            RenderOptions::default(),
        )
        .unwrap();
        let unpadded = export_with_options(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            ExportFormat::Png,
            RenderOptions {
                pad: 0.0,
                ..RenderOptions::default()
            },
        )
        .unwrap();
        let png_width = |bytes: &[u8]| u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        // PNG uses D2's fixed 2x raster density, so the 100px logical pad on
        // each side contributes 400 encoded pixels to the total width.
        assert_eq!(png_width(&padded) - png_width(&unpadded), 400);
    }

    #[test]
    fn paged_frames_share_the_document_pixel_allowance() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let layout = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let render = |budget: &mut PagedFrameBudget| {
            render_paged_png_rgba_at_scale(
                &graph,
                &layout,
                &BTreeMap::new(),
                &BTreeMap::new(),
                RenderOptions::default(),
                false,
                2,
                budget,
            )
        };
        let png = render(&mut PagedFrameBudget::default()).unwrap();
        assert_eq!(render(&mut PagedFrameBudget::new(2)).unwrap(), png);
        assert_eq!(
            render(&mut PagedFrameBudget::new(0))
                .unwrap_err()
                .to_string(),
            "paged export exhausted its operation budget",
        );
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        let pixels = u64::from(width) * u64::from(height);
        // A reduced allowance exercises the same complete-document accounting
        // without allocating hundreds of megabytes in the test suite.
        let mut budget = PagedFrameBudget {
            remaining_pixels: 2 * pixels - 1,
            ..PagedFrameBudget::default()
        };
        assert_eq!(render(&mut budget).unwrap(), png);
        assert_eq!(budget.remaining_pixels, pixels - 1);
        assert_eq!(
            render(&mut budget).unwrap_err().to_string(),
            format!("d2raster: frame pixels exceed limit {}", pixels - 1),
        );
        assert_eq!(budget.remaining_pixels, pixels - 1);
        // Resolver limits span boards, while repeated locators use the same
        // admitted immutable resource. Reduced limits keep this boundary cheap.
        let mut resources = paged_admission::Resources::new().unwrap();
        resources.images = std::sync::Arc::new(
            d2_rs_scenebuild::RasterImageResolver::new(
                std::path::Path::new("."),
                d2_rs_scenebuild::ImageAssetLimits {
                    max_fetched_bytes: 1024,
                    max_encoded_bytes: 1024,
                    max_decompressed_bytes: 1024,
                    max_svg_bytes: 1024,
                    max_decoded_width: 100,
                    max_decoded_height: 100,
                    max_decoded_pixels: 10000,
                    max_assets: 1,
                    max_cumulative_encoded_bytes: 1024,
                    max_cumulative_decoded_bytes: 1024,
                },
                std::sync::Arc::new(d2_rs_scenebuild::UnrestrictedImageFiles::new()),
            )
            .unwrap(),
        );
        let mut budget = PagedFrameBudget {
            resources: Some(resources),
            ..PagedFrameBudget::new(3)
        };
        let image = go_png::encode(1, 1, &[255, 0, 0, 255], 4).unwrap();
        let render_image = |source: &str, budget: &mut PagedFrameBudget| {
            render_paged_png_rgba_at_scale(
                &graph,
                &layout,
                &BTreeMap::new(),
                &BTreeMap::from([(
                    node,
                    SourceNodeStyle {
                        icon: Some(source.into()),
                        ..Default::default()
                    },
                )]),
                RenderOptions {
                    asset_data: BTreeMap::from([(source.into(), image.clone())]),
                    ..Default::default()
                },
                false,
                2,
                budget,
            )
        };
        let first = render_image("acquired-one.png", &mut budget).unwrap();
        assert_eq!(
            render_image("acquired-one.png", &mut budget).unwrap(),
            first
        );
        let remaining = budget.remaining_pixels;
        assert_eq!(
            render_image("acquired-two.png", &mut budget)
                .unwrap_err()
                .to_string(),
            "scene: shape \"a\" image asset: imageasset: probe \"acquired-two.png\": assets 2 exceeds limit 1",
        );
        assert_eq!(budget.remaining_pixels, remaining);
        let mut resources = paged_admission::Resources::new().unwrap();
        resources.fonts = std::sync::Arc::new(
            d2_rs_scenebuild::BundledFallbackResolver::new(
                None,
                d2_rs_scenebuild::BundledFallbackLimits {
                    max_requested_runes: 1,
                    max_bundled_bytes: 8 << 20,
                    max_resolved_bytes: 136 << 20,
                },
            )
            .unwrap(),
        );
        let mut budget = PagedFrameBudget {
            resources: Some(resources),
            ..PagedFrameBudget::new(2)
        };
        let labels = BTreeMap::from([(node, "😀".into())]);
        let render_text = |budget: &mut PagedFrameBudget| {
            render_paged_png_rgba_at_scale(
                &graph,
                &layout,
                &labels,
                &BTreeMap::new(),
                RenderOptions::default(),
                false,
                2,
                budget,
            )
        };
        render_text(&mut budget).unwrap();
        let remaining = budget.remaining_pixels;
        assert!(render_text(&mut budget).unwrap_err().to_string().contains(
            "d2fonts: requested fallback rune count exceeds cumulative resolver limit 1"
        ));
        assert_eq!(budget.remaining_pixels, remaining);
    }

    #[test]
    fn pdf_encoder_returns_a_single_page_document() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let bytes = render_pdf(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4\n"));
        assert!(bytes.windows(8).any(|window| window == b"/Count 1"));
        assert!(
            bytes
                .windows(b"/MediaBox [0 0 576.00 576.00]".len())
                .any(|window| window == b"/MediaBox [0 0 576.00 576.00]")
        );
        assert!(bytes.ends_with(b"%%EOF\n"));
        assert!(
            bytes
                .windows(b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)".len())
                .any(|window| { window == b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)" })
        );
        assert!(
            bytes
                .windows(b"/CreationDate (D:".len())
                .any(|window| { window == b"/CreationDate (D:" })
        );
        assert!(
            bytes
                .windows(b"/ModDate (D:".len())
                .any(|window| { window == b"/ModDate (D:" })
        );
        assert!(
            bytes
                .windows(b"/Info ".len())
                .any(|window| window == b"/Info ")
        );
    }

    #[test]
    fn transparent_pdf_board_raster_keeps_the_padding_clear() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let png = render_png_rgba_at_scale(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
            2,
        )
        .unwrap();
        let page = decode_pdf_png(&png).unwrap();
        let alpha = page
            .alpha
            .expect("transparent PDF raster needs an alpha plane");
        assert!(alpha.contains(&0), "page padding must remain transparent");
        assert!(alpha.contains(&255), "diagram strokes must remain opaque");
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn pdf_navigation_matches_go_destinations_and_separator_color() {
        // Pinned lib/pdf.AddPDFPage + fpdf.putpages; see the
        // renderexport-export-public fixture recipes.
        // Unequal page heights expose both source/destination confusion and
        // omission of the target's navigation band.
        let expected: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/pdf-navigation-source-v090.json"
        ))
        .unwrap();
        let pages = [1200, 2]
            .into_iter()
            .map(|height| {
                let mut bytes = Vec::new();
                {
                    let mut encoder = png::Encoder::new(&mut bytes, 32, height);
                    encoder.set_color(png::ColorType::Rgb);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&vec![255; 32 * height as usize * 3])
                        .unwrap();
                }
                bytes
            })
            .collect::<Vec<_>>();
        let board_paths = vec![
            vec!["root".into()],
            vec!["root".into(), "root.layers.child".into()],
        ];
        let titles = vec![vec!["Root".into()], vec!["Root".into(), "Child".into()]];
        let links = ["root.layers.child", "root"].map(|href| {
            vec![DocumentLinkRegion {
                href: href.into(),
                rect: Rect {
                    origin: Point { x: 3.0, y: 4.0 },
                    size: d2_rs_core::Size {
                        width: 10.0,
                        height: 11.0,
                    },
                },
            }]
        });
        for row in expected.as_array().unwrap() {
            let bytes = package_pdf_pages_with_navigation(
                &pages,
                &links,
                &board_paths,
                &titles,
                row["navigation"].as_bool().unwrap(),
            )
            .unwrap();
            let text = String::from_utf8_lossy(&bytes);
            let kids = text
                .split("/Kids [")
                .nth(1)
                .unwrap()
                .split(']')
                .next()
                .unwrap();
            let page_ids = kids.split_whitespace().collect::<Vec<_>>();
            let expected_pages = row["pages"].as_array().unwrap();
            assert_eq!(page_ids.len(), expected_pages.len() * 3);
            let mut link_count = 0;
            for page in expected_pages {
                let width = page["size"][0].as_f64().unwrap();
                let height = page["size"][1].as_f64().unwrap();
                assert!(text.contains(&format!("/MediaBox [0 0 {width:.2} {height:.2}]")));
                for link in page["links"].as_array().unwrap() {
                    let target = link["destination_page"].as_u64().unwrap() as usize;
                    let rect = link["rect"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| format!("{:.2}", value.as_f64().unwrap()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    assert!(link["preserve_zoom"].as_bool().unwrap());
                    let annotation = format!(
                        "/Rect [{rect}] /Border [0 0 0] /Dest [{} 0 R {} {} {:.2} null]",
                        page_ids[target * 3],
                        link["destination_kind"].as_str().unwrap(),
                        link["destination_left"].as_f64().unwrap(),
                        link["destination_top"].as_f64().unwrap(),
                    );
                    assert!(text.contains(&annotation), "missing {annotation}");
                    link_count += 1;
                }
            }
            assert_eq!(text.matches("/Subtype /Link").count(), link_count);
            let commands = pdf_document::decoded_page_streams(&bytes).concat();
            assert_eq!(
                String::from_utf8_lossy(&commands)
                    .matches("0.039 0.059 0.145 RG")
                    .count(),
                3
            );
        }
    }

    #[test]
    fn pdf_metadata_timestamp_is_a_zoned_pdf_date() {
        let timestamp = pdf_timestamp_utc();
        assert!(timestamp.starts_with("D:"));
        assert_eq!(timestamp.len(), 23);
        assert!(timestamp.as_bytes()[2..16].iter().all(u8::is_ascii_digit));
        assert!(timestamp.ends_with("+00'00'"));
    }

    #[test]
    fn pdf_page_encoder_preserves_rgba_alpha_as_a_soft_mask() {
        let page = PdfRasterPage {
            width: 2,
            height: 2,
            rgb: vec![24, 48, 96, 10, 20, 30, 255, 0, 128, 17, 29, 41],
            alpha: Some(vec![128, 0, 255, 72]),
            rgb_png_idat: None,
            streams: None,
        };
        let bytes = encode_pdf_pages(&[page], true).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let mask_id = text
            .split("/SMask ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert!(text.contains(&format!("\n{mask_id} 0 obj\n")));
        assert!(text.contains("/ColorSpace /DeviceGray\n/BitsPerComponent 8"));
        // Decode the actual embedded RGB and mask streams, using their declared
        // lengths so binary data cannot be confused with PDF delimiters.
        for (color_space, channels, expected) in [
            (
                "DeviceRGB",
                3,
                vec![24, 48, 96, 10, 20, 30, 255, 0, 128, 17, 29, 41],
            ),
            ("DeviceGray", 1, vec![128, 0, 255, 72]),
        ] {
            let marker = format!("/ColorSpace /{color_space}\n");
            let start = bytes
                .windows(marker.len())
                .position(|s| s == marker.as_bytes())
                .unwrap();
            let object = &bytes[start..];
            let stream_start = object.windows(7).position(|s| s == b"stream\n").unwrap() + 7;
            let dictionary = std::str::from_utf8(&object[..stream_start]).unwrap();
            assert!(dictionary.contains("/Filter /FlateDecode"));
            assert!(dictionary.contains(&format!(
                "/Predictor 15 /Colors {channels} /BitsPerComponent 8 /Columns 2"
            )));
            let length = dictionary
                .split("/Length ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .trim_end_matches('>')
                .parse::<usize>()
                .unwrap();
            let stream = &object[stream_start..stream_start + length];
            assert!(object[stream_start + length..].starts_with(b"\nendstream\nendobj"));
            let decoded = go_png::decode_predictor(2, 2, channels, stream);
            assert_eq!(decoded, expected);
        }
        assert!(bytes.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn pdf_encoder_preserves_node_and_edge_links_as_annotations() {
        let mut graph = Graph::default();
        let source = graph.add_node(d2_rs_core::Node::new(
            "source",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let target = graph.add_node(d2_rs_core::Node::new(
            "target",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let edge = graph.add_edge(d2_rs_core::Edge { source, target });
        graph.set_edge_link(edge, Some("https://example.com/edge?q=(x)".to_owned()));
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "Edge".into(),
                size: d2_rs_core::Size {
                    width: 34.0,
                    height: 21.0,
                },
                position: d2_rs_core::LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(source, "source".to_owned());
        labels.insert(target, "target".to_owned());
        let mut styles = BTreeMap::new();
        styles.insert(
            source,
            SourceNodeStyle {
                link: Some("https://example.com/node".to_owned()),
                ..SourceNodeStyle::default()
            },
        );
        let bytes = render_pdf(&graph, &result, &labels, &styles).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("/Subtype /Link").count(), 2);
        assert!(text.contains("https://example.com/node"));
        assert!(text.contains("https://example.com/edge?q=\\(x\\)"));
        assert!(text.contains("/Annots ["));
        assert!(bytes.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn pdf_package_contains_all_rendered_pages() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let page = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let bytes = package_pdf_pages(&[page.clone(), page]).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.3\n"));
        assert!(bytes.windows(8).any(|window| window == b"/Count 2"));
        assert!(
            bytes
                .windows(b"/Kids [3 0 R 5 0 R ]".len())
                .any(|window| window == b"/Kids [3 0 R 5 0 R ]")
        );
        assert!(
            bytes
                .windows(b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)".len())
                .any(|window| window == b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)")
        );
        assert!(bytes.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn pdf_package_preserves_link_regions_for_each_page() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let page = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let link = DocumentLinkRegion {
            href: "https://example.com/multiboard".to_owned(),
            rect: Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: d2_rs_core::Size {
                    width: 40.0,
                    height: 30.0,
                },
            },
        };
        let bytes =
            package_pdf_pages_with_link_regions(&[page.clone(), page], &[vec![link], Vec::new()])
                .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Count 2"));
        assert_eq!(text.matches("/Subtype /Link").count(), 1);
        assert!(text.contains("https://example.com/multiboard"));
        assert!(
            bytes
                .windows(b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)".len())
                .any(|window| window == b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)")
        );
        assert!(bytes.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn pptx_encoder_returns_a_single_slide_package() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let bytes = render_pptx(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(
            archive.by_index(0).unwrap().name().unwrap(),
            "[Content_Types].xml"
        );
        let mut template = zip::ZipArchive::new(std::io::Cursor::new(include_bytes!(
            "../assets/d2-template.pptx"
        )))
        .unwrap();
        // Go preserves the complete template inventory, skips two rels,
        // then generates seven common parts and three members per slide.
        assert_eq!(archive.len(), template.len() - 2 + 7 + 3);
        let mut names = std::collections::BTreeSet::new();
        for index in 0..archive.len() {
            assert!(
                names.insert(
                    archive
                        .by_index(index)
                        .unwrap()
                        .name()
                        .unwrap()
                        .into_owned()
                )
            );
        }
        for index in 0..template.len() {
            let mut original = template.by_index(index).unwrap();
            let original_name = original.name().unwrap().into_owned();
            if matches!(
                original_name.as_str(),
                "_rels/.rels" | "ppt/slideMasters/_rels/slideMaster1.xml.rels"
            ) {
                continue;
            }
            let mut copied = archive.by_name(&original_name).unwrap();
            assert_eq!(copied.is_dir(), original.is_dir());
            assert_eq!(copied.compression(), original.compression());
            assert_eq!(copied.crc32(), original.crc32());
            assert_eq!(copied.compressed_size(), original.compressed_size());
            let mut original_bytes = Vec::new();
            let mut copied_bytes = Vec::new();
            original.read_to_end(&mut original_bytes).unwrap();
            copied.read_to_end(&mut copied_bytes).unwrap();
            assert_eq!(copied_bytes, original_bytes);
        }
        for entry in [
            "[Content_Types].xml",
            "ppt/presentation.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/media/slide1Image.png",
        ] {
            assert!(archive.by_name(entry).is_ok(), "missing PPTX entry {entry}");
        }
        assert!(archive.by_name("_rels/.rels").is_ok());
        let mut slide_xml = String::new();
        archive
            .by_name("ppt/slides/slide1.xml")
            .unwrap()
            .read_to_string(&mut slide_xml)
            .unwrap();
        assert!(slide_xml.contains("name=\"root\" descr=\"root\""));
        assert!(slide_xml.contains("r:embed=\"slide1Image\""));
        let mut slide_relationships = String::new();
        archive
            .by_name("ppt/slides/_rels/slide1.xml.rels")
            .unwrap()
            .read_to_string(&mut slide_relationships)
            .unwrap();
        assert!(slide_relationships.contains("Id=\"slide1Image\""));
        assert!(slide_relationships.contains("Id=\"navLink0\""));

        let png = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let deck = package_pptx_slides(&[png.clone(), png]).unwrap();
        let mut deck = zip::ZipArchive::new(std::io::Cursor::new(deck)).unwrap();
        assert!(deck.by_name("ppt/slides/slide2.xml").is_ok());
        assert!(deck.by_name("ppt/media/slide2Image.png").is_ok());
    }

    #[test]
    fn pptx_encoder_preserves_node_links_as_hyperlink_shapes() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "linked",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "linked".to_owned());
        let mut styles = BTreeMap::new();
        styles.insert(
            node,
            SourceNodeStyle {
                link: Some("https://example.com/pptx".to_owned()),
                ..SourceNodeStyle::default()
            },
        );
        let bytes = render_pptx(&graph, &result, &labels, &styles).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut slide_xml = String::new();
        archive
            .by_name("ppt/slides/slide1.xml")
            .unwrap()
            .read_to_string(&mut slide_xml)
            .unwrap();
        assert!(
            slide_xml
                .contains("<a:hlinkClick r:id=\"link1\" action=\"\" tooltip=\"\" history=\"1\"")
        );
        assert!(
            slide_xml.contains("<a:alpha val=\"0\" />") && slide_xml.contains("<a:ln w=\"12700\">")
        );
        let mut relationships = String::new();
        archive
            .by_name("ppt/slides/_rels/slide1.xml.rels")
            .unwrap()
            .read_to_string(&mut relationships)
            .unwrap();
        assert!(relationships.contains("Id=\"link1\""));
        assert!(relationships.contains("Target=\"https://example.com/pptx\""));
        assert!(relationships.contains("TargetMode=\"External\""));
    }

    #[test]
    fn pptx_package_preserves_link_regions_for_each_slide() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let page = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let link = DocumentLinkRegion {
            href: "https://example.com/slide".to_owned(),
            rect: Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: d2_rs_core::Size {
                    width: 40.0,
                    height: 30.0,
                },
            },
        };
        let bytes = package_pptx_slides_with_board_paths_and_link_regions(
            &[page.clone(), page],
            &[
                vec!["root".to_owned()],
                vec!["root".to_owned(), "root.board".to_owned()],
            ],
            &[vec![link], Vec::new()],
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut slide_xml = String::new();
        archive
            .by_name("ppt/slides/slide1.xml")
            .unwrap()
            .read_to_string(&mut slide_xml)
            .unwrap();
        assert_eq!(slide_xml.matches("hlinkClick").count(), 1);
        assert!(archive.by_name("ppt/slides/slide2.xml").is_ok());
        let mut relationships = String::new();
        archive
            .by_name("ppt/slides/_rels/slide1.xml.rels")
            .unwrap()
            .read_to_string(&mut relationships)
            .unwrap();
        assert!(relationships.contains("https://example.com/slide"));
    }

    #[test]
    fn paged_packages_emit_title_navigation_metadata() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let page = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let pages = vec![page.clone(), page];
        let board_paths = vec![
            vec!["root".to_owned()],
            vec!["root".to_owned(), "root.layers.one".to_owned()],
        ];
        let title_paths = vec![
            vec!["root".to_owned()],
            vec!["root".to_owned(), "one".to_owned()],
        ];
        let links = vec![
            Vec::new(),
            vec![DocumentLinkRegion {
                href: "root".to_owned(),
                rect: Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    size: d2_rs_core::Size {
                        width: 10.0,
                        height: 10.0,
                    },
                },
            }],
        ];
        let pdf =
            package_pdf_pages_with_navigation(&pages, &links, &board_paths, &title_paths, true)
                .unwrap();
        let pdf_text = String::from_utf8_lossy(&pdf);
        assert!(pdf.windows(7).any(|window| window == b"/Dest ["));
        assert!(
            pdf_document::decoded_page_streams(&pdf)
                .concat()
                .windows(8)
                .any(|w| w == b"\0r\0o\0o\0t")
        );
        assert!(pdf_text.contains("/Subtype /Type0"));
        // Original Go lib/pdf + FPDF CellFormat: links cover the regular
        // 14pt title text, with the cell inset, rather than the 72pt header.
        // Source witness: synthetic-root-raster-20261018/paged-labels.
        let labelled_titles = vec![
            vec!["Root title".to_owned()],
            vec!["Root title".to_owned(), "Section".to_owned()],
        ];
        let labelled_pdf =
            package_pdf_pages_with_navigation(&pages, &links, &board_paths, &labelled_titles, true)
                .unwrap();
        assert!(
            String::from_utf8_lossy(&labelled_pdf).contains("/Rect [30.84 619.00 84.93 605.00]")
        );
        assert!(pdf.windows(8).any(|window| window == b"\xfe\xff\0F\0P\0D"));
        let pdf_without_header =
            package_pdf_pages_with_navigation(&pages, &links, &board_paths, &title_paths, false)
                .unwrap();
        assert!(
            pdf_without_header
                .windows(7)
                .any(|window| window == b"/Dest [")
        );
        let dark_theme_pdf = package_pdf_pages_with_navigation_for_theme(
            &pages,
            &links,
            &board_paths,
            &title_paths,
            false,
            Some(200),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&pdf_document::decoded_page_streams(&dark_theme_pdf).concat())
                .contains("1.000 G")
        );

        let pptx =
            package_pptx_slides_with_navigation(&pages, &board_paths, &title_paths, &links, true)
                .unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(pptx)).unwrap();
        let mut slide = String::new();
        archive
            .by_name("ppt/slides/slide2.xml")
            .unwrap()
            .read_to_string(&mut slide)
            .unwrap();
        assert!(slide.contains("name=\"root.layers.one\"") && slide.contains("navLink0"));
    }

    #[test]
    fn gif_package_contains_one_delayed_frame_per_board() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let frame = render_png(&graph, &result, &labels, &BTreeMap::new()).unwrap();
        let bytes = package_gif_frames(&[frame.clone(), frame]).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            frames.push((frame.width, frame.height, frame.delay));
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], frames[1]);
        assert_eq!(frames[0].2, 100);
    }

    #[test]
    fn animated_shape_gif_frames_follow_the_shape_effect_schedule() {
        // shape_effects.go configureAnimatedShape: identity and transparent
        // shadows at t=0, then a lift and two growing drop shadows.
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let mut styles = BTreeMap::new();
        styles.insert(
            node,
            SourceNodeStyle {
                animated: Some("true".into()),
                ..SourceNodeStyle::default()
            },
        );
        let frames = render_animated_png_frames_with_options(
            &graph,
            &result,
            &labels,
            &styles,
            RenderOptions::default(),
            false,
            125,
        )
        .unwrap();
        assert_eq!(frames.len(), 4);
        let pixels = |png: &Vec<u8>| {
            let mut reader = png::Decoder::new(std::io::Cursor::new(png))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut pixels).unwrap();
            pixels
        };
        let (first, second, third) = (pixels(&frames[0]), pixels(&frames[1]), pixels(&frames[2]));
        assert_ne!(first, second);
        assert_ne!(second, third);
    }

    #[test]
    fn multiboard_gif_repeats_each_board_at_the_v090_sample_schedule() {
        let mut graph = Graph::default();
        let node = graph.add_node(d2_rs_core::Node::new(
            "a",
            d2_rs_core::Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let result = d2_rs_tala::Engine
            .layout(&graph, &d2_rs_core::LayoutOptions::default())
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert(node, "a".to_owned());
        let frame = render_png_at_scale(
            &graph,
            &result,
            &labels,
            &BTreeMap::new(),
            RenderOptions::default(),
            false,
            1,
        )
        .unwrap();
        let bytes = package_gif_boards_with_interval(&[frame.clone(), frame], 250).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).unwrap();
        let mut delays = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            delays.push(frame.delay);
        }
        assert_eq!(delays.len(), 16);
        assert_eq!(&delays[..8], &[3, 3, 3, 3, 3, 3, 3, 4]);
        assert_eq!(&delays[8..], &[3, 3, 3, 3, 3, 3, 3, 4]);
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod positioned_tooltip_admission_source_contract {
    use super::*;
    #[test]
    fn positioned_unicode_callouts_and_appendix_match_complete_source_pixels() {
        use sha2::{Digest, Sha256};
        let inputs: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-render"),
            "tests/fixtures/positioned-tooltip-source-v090.json",
        ))
        .unwrap();
        let expected: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/positioned-tooltip-unicode-png-source-v090.json"
        ))
        .unwrap();
        assert_eq!(expected.as_array().unwrap().len(), 48);
        for source in expected.as_array().unwrap() {
            let input = inputs
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == source["name"])
                .unwrap();
            let shape = &input["shape"];
            let size = d2_rs_core::Size {
                width: shape["width"].as_f64().unwrap(),
                height: shape["height"].as_f64().unwrap(),
            };
            let mut graph = Graph::default();
            let id = graph.add_node(d2_rs_core::Node::new("probe", size));
            let rect = Rect {
                origin: Point {
                    x: shape["pos"]["x"].as_f64().unwrap(),
                    y: shape["pos"]["y"].as_f64().unwrap(),
                },
                size,
            };
            let field = |key: &str| {
                shape
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    fill: field("fill"),
                    stroke: field("stroke"),
                    stroke_width: Some(shape["strokeWidth"].to_string()),
                    tooltip: field("tooltip"),
                    tooltip_position: field("tooltipPosition"),
                    link: field("link"),
                    pretty_link: Some(
                        shape
                            .get("prettyLink")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    ),
                    font: field("fontFamily"),
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
                report: d2_rs_core::LayoutReport {
                    selected_seed: 0,
                    selected_pass: 0,
                    score: d2_rs_core::Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            let bytes = render_png(&graph, &result, &BTreeMap::new(), &styles).unwrap();
            let image = decode_pdf_png(&bytes).unwrap();
            assert_eq!(
                serde_json::json!([image.width, image.height]),
                source["dimensions"],
                "{}",
                source["name"]
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(&image.rgb)),
                source["rgb_sha256"].as_str().unwrap(),
                "{}",
                source["name"]
            );
        }
    }
}

#[cfg(test)]
mod markdown_document_link_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use d2_rs_core::{LabelPosition, Node, ShapeKind, Size};
    #[cfg(feature = "oracle-fixtures")]
    use d2_rs_render::go_float::go_fmsub;
    #[cfg(feature = "oracle-fixtures")]
    use d2_rs_tala::{LayoutReport, Score};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_markdown_document_link_transport() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-document-links-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 27);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let size = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            let mut graph = Graph::default();
            let mut node = Node::new("markdown", size);
            node.shape = ShapeKind::Text;
            node.label_size = Some(size);
            node.label_position = LabelPosition::InsideTopLeft;
            let id = graph.add_node(node);
            let labels = BTreeMap::from([(id, r["Markdown"].as_str().unwrap().to_owned())]);
            let styles = BTreeMap::from([(
                id,
                SourceNodeStyle {
                    markdown: true,
                    language: Some("markdown".into()),
                    font: Some(
                        if r["Family"] == "mono" {
                            "mono"
                        } else if r["Family"] == "hand" {
                            "hand"
                        } else {
                            "default"
                        }
                        .into(),
                    ),
                    font_size: Some(16),
                    fill: Some("N7".into()),
                    stroke: Some("N1".into()),
                    stroke_width: Some("0".into()),
                    ..Default::default()
                },
            )]);
            let result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                boxes: BTreeMap::from([(
                    id,
                    Rect {
                        origin: Point::default(),
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
            let actual = document_link_regions(
                &graph,
                &result,
                &labels,
                &styles,
                RenderOptions {
                    scale: r["Scale"].as_f64(),
                    ..RenderOptions::default()
                },
                r["Family"] == "hand",
            )
            .unwrap();
            // Go's scene links mapped as d2cli/paged.go mapPagedLinks maps
            // them into the png.SCALE frame (lines 270-342: d2scenebuild's
            // meet viewport over the ceiling frame, uncentered without
            // --center (build.go:795-798), with the arm64 FMSUBD translation
            // and Matrix.Point), returned in logical units (frame pixels /
            // png.SCALE) as the document packagers take them.
            let view = &row["viewbox"];
            let number = |value: &serde_json::Value, key: &str| value[key].as_f64().unwrap();
            let (vx, vy) = (number(view, "X"), number(view, "Y"));
            let (vw, vh) = (number(view, "Width"), number(view, "Height"));
            let frame_width = (row["logical_width"].as_f64().unwrap() * 2.0).ceil();
            let frame_height = (row["logical_height"].as_f64().unwrap() * 2.0).ceil();
            let uniform = d2_rs_scene::go_min(frame_width / vw, frame_height / vh);
            let mapping = d2_rs_scene::Matrix {
                a: uniform,
                b: 0.0,
                c: 0.0,
                d: uniform,
                e: go_fmsub(vx, uniform, 0.0),
                f: go_fmsub(vy, uniform, 0.0),
            };
            let expected = row["links"]
                .as_array()
                .unwrap()
                .iter()
                .map(|link| {
                    let b = &link["Box"];
                    let (x, y) = (number(b, "X"), number(b, "Y"));
                    let top_left = mapping.point(d2_rs_scene::Point::new(x, y));
                    let bottom_right = mapping.point(d2_rs_scene::Point::new(
                        x + number(b, "Width"),
                        y + number(b, "Height"),
                    ));
                    DocumentLinkRegion {
                        href: link["URL"].as_str().unwrap().to_owned(),
                        rect: Rect {
                            origin: Point {
                                x: top_left.x / 2.0,
                                y: top_left.y / 2.0,
                            },
                            size: Size {
                                width: (bottom_right.x - top_left.x) / 2.0,
                                height: (bottom_right.y - top_left.y) / 2.0,
                            },
                        },
                    }
                })
                .collect::<Vec<_>>();
            if actual != expected {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"actual":actual.iter().map(|v|serde_json::json!({"href":v.href,"rect":v.rect})).collect::<Vec<_>>(),"expected":expected.iter().map(|v|serde_json::json!({"href":v.href,"rect":v.rect})).collect::<Vec<_>>() }));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MD_DOCUMENT_LINK_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "document link Source differences: {failures:?}"
        );
    }
}

#[doc(hidden)]
pub mod raster_asset_validation;
